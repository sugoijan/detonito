use std::rc::Rc;

use detonito_protocol::{
    AfkActionKind, AfkActionRequest, AfkActivityKind, AfkActivityRow, AfkBoardSize,
    AfkCellSnapshot, AfkChatConnectionState, AfkClientMessage, AfkLossReason, AfkRoundPhase,
    AfkServerMessage, AfkSessionSnapshot, AfkStatusResponse,
};
use gloo::timers::callback::Timeout;
use js_sys::encode_uri_component;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Event, MessageEvent, Request, RequestCredentials, RequestInit, Response, WebSocket};
use yew::prelude::*;

use crate::menu::{
    menu_copy_row, menu_header_row, menu_icon_button, menu_nav_enter_button, menu_primary_row,
    menu_section_gap, menu_stepper_row, menu_toggle_row, menu_wide_detail_row,
};
use crate::runtime::{AppRoute, app_path, auth_return_to, frontend_runtime_config, websocket_path};
use crate::sprites::{Glyph, GlyphRun, GlyphSet, Icon, IconCrop, SpriteDefs};
use crate::utils::{
    LocalDelete, LocalOrDefault, LocalSave, StorageKey, browser_now_ms, format_for_counter,
};

#[derive(Properties, PartialEq)]
pub(crate) struct AfkViewProps {
    pub on_menu: Callback<()>,
    pub on_open_settings: Callback<()>,
    #[prop_or_default]
    pub auth_error: Option<String>,
    #[prop_or_default]
    pub start_after_connect: bool,
    pub on_consume_start_after_connect: Callback<()>,
}

#[derive(Clone, Debug, PartialEq)]
enum LoadState<T> {
    Idle,
    Loading,
    Ready(T),
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfkScreen {
    Menu,
    Board,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfkMenuPage {
    Root,
    BoardSize,
    ConfirmBoardSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfkFaceAction {
    DismissPrompt,
    OpenSubmenu,
    ContinueRound,
    StopRun,
}

#[derive(Clone, Debug, PartialEq)]
enum AfkFaceOverlay {
    Message {
        message: AttrValue,
        status: Option<AttrValue>,
    },
    Prompt(AfkFacePrompt),
    Status(AttrValue),
}

#[derive(Clone, Debug, PartialEq)]
struct AfkFaceChoice {
    label: AttrValue,
    title: AttrValue,
    action: AfkFaceAction,
}

#[derive(Clone, Debug, PartialEq)]
struct AfkFacePrompt {
    message: AttrValue,
    choices: Vec<AfkFaceChoice>,
}

#[derive(Clone, Debug, PartialEq)]
struct AfkFaceNotification {
    id: u64,
    message: AttrValue,
}

#[derive(Clone, Debug, PartialEq)]
struct AfkFaceNotificationEvent {
    message: AttrValue,
    timeout_ms: u32,
}

const AFK_FACE_NOTIFICATION_MS: u32 = 5_000;
const AFK_OUT_FOR_ROUND_NOTIFICATION_MS: u32 = 10_000;
const AFK_IDLE_SLEEPING_THRESHOLD_MS: i64 = 3 * 60 * 1_000;
const AFK_IDLE_PROMPT_THRESHOLD_MS: i64 = 10 * 60 * 1_000;
const AFK_IDLE_EXPIRY_THRESHOLD_MS: i64 = 60 * 60 * 1_000;
const AFK_TIMEOUT_DURATION_OPTIONS_SECS: [u32; 12] =
    [1, 5, 10, 15, 30, 45, 60, 90, 120, 180, 240, 300];
const AFK_DEFAULT_TIMEOUT_DURATION_INDEX: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfkIdleState {
    Sleeping,
    Prompt,
}

fn current_level_status_text(session: Option<&AfkSessionSnapshot>) -> Option<AttrValue> {
    session
        .filter(|session| session.board.width >= 15)
        .map(|session| format!("Level {}", session.current_level).into())
}

fn board_size_label(board_size: AfkBoardSize) -> &'static str {
    match board_size {
        AfkBoardSize::Tiny => "Tiny",
        AfkBoardSize::Small => "Small",
        AfkBoardSize::Medium => "Medium",
        AfkBoardSize::Large => "Large",
    }
}

fn board_size_detail(board_size: AfkBoardSize) -> &'static str {
    match board_size {
        AfkBoardSize::Tiny => "9x9 / 9 +1/level",
        AfkBoardSize::Small => "16x16 / 20 +4/level",
        AfkBoardSize::Medium => "24x18 / 36 +7/level",
        AfkBoardSize::Large => "30x20 / 50 +10/level",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct AfkMenuPreferences {
    board_size: AfkBoardSize,
    timeout_enabled: bool,
    timeout_duration_secs: u32,
}

impl Default for AfkMenuPreferences {
    fn default() -> Self {
        Self {
            board_size: AfkBoardSize::Medium,
            timeout_enabled: true,
            timeout_duration_secs: 30,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct AfkConnectStartDraft {
    preferences: AfkMenuPreferences,
}

impl StorageKey for AfkConnectStartDraft {
    const KEY: &'static str = "detonito:afk:connect-start";
}

fn afk_menu_preferences_from_status(status: &AfkStatusResponse) -> AfkMenuPreferences {
    AfkMenuPreferences {
        board_size: status.board_size,
        timeout_enabled: status.timeout_enabled,
        timeout_duration_secs: status.timeout_duration_secs,
    }
}

fn displayed_afk_menu_preferences(
    status: &AfkStatusResponse,
    pending_preferences: Option<AfkMenuPreferences>,
) -> AfkMenuPreferences {
    if status.auth.identity.is_none() {
        pending_preferences.unwrap_or_else(|| afk_menu_preferences_from_status(status))
    } else {
        afk_menu_preferences_from_status(status)
    }
}

fn persist_afk_connect_start_draft(preferences: Option<AfkMenuPreferences>) {
    match preferences {
        Some(preferences) => Some(AfkConnectStartDraft { preferences }).local_save(),
        None => Option::<AfkConnectStartDraft>::local_delete(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfkBoardSizeChangePlan {
    NoChange,
    ApplyOnly(AfkBoardSize),
    ConfirmRestart(AfkBoardSize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfkRootPrimaryAction {
    ResumeAndStartNew,
    Start,
    ConnectAndStart,
}

fn plan_board_size_change(
    status: &AfkStatusResponse,
    next_board_size: AfkBoardSize,
) -> AfkBoardSizeChangePlan {
    if status.board_size == next_board_size {
        AfkBoardSizeChangePlan::NoChange
    } else if status.session.is_some() {
        AfkBoardSizeChangePlan::ConfirmRestart(next_board_size)
    } else {
        AfkBoardSizeChangePlan::ApplyOnly(next_board_size)
    }
}

fn afk_root_primary_action(status: &AfkStatusResponse) -> AfkRootPrimaryAction {
    if status.session.is_some() {
        AfkRootPrimaryAction::ResumeAndStartNew
    } else if status.auth.identity.is_some() {
        AfkRootPrimaryAction::Start
    } else {
        AfkRootPrimaryAction::ConnectAndStart
    }
}

fn status_chat_error(status: &AfkStatusResponse) -> Option<String> {
    status
        .chat_error
        .clone()
        .filter(|message| !message.is_empty())
}

fn has_critical_chat_failure(status: &AfkStatusResponse) -> bool {
    status.session.is_some() && matches!(status.chat_connection, AfkChatConnectionState::Error)
}

fn handle_started_status(
    status: &UseStateHandle<LoadState<AfkStatusResponse>>,
    screen: &UseStateHandle<AfkScreen>,
    last_error: &UseStateHandle<Option<String>>,
    response: AfkStatusResponse,
) {
    let has_session = response.session.is_some();
    let can_open_board =
        has_session && !matches!(response.chat_connection, AfkChatConnectionState::Error);
    let next_error = status_chat_error(&response);
    status.set(LoadState::Ready(response));
    if can_open_board {
        last_error.set(None);
        screen.set(AfkScreen::Board);
    } else if let Some(error) = next_error {
        last_error.set(Some(error));
    }
}

/// Optional AFK notices own their trailing separator so root menus do not end
/// up with a second blank row when no notice is visible.
fn afk_menu_notice_block(
    primary_notice: Option<AttrValue>,
    secondary_notice: Option<AttrValue>,
) -> Html {
    let rows: Vec<Html> = [primary_notice, secondary_notice]
        .into_iter()
        .flatten()
        .map(menu_copy_row)
        .collect();

    if rows.is_empty() {
        Html::default()
    } else {
        html! {
            <>
                {for rows}
                {menu_section_gap()}
            </>
        }
    }
}

fn afk_root_primary_rows(
    action: AfkRootPrimaryAction,
    resume_board: &Callback<MouseEvent>,
    start_new_board: &Callback<MouseEvent>,
    connect_twitch_and_start: &Callback<MouseEvent>,
) -> Html {
    match action {
        AfkRootPrimaryAction::ResumeAndStartNew => html! {
            <>
                {menu_primary_row(
                    "Resume",
                    menu_nav_enter_button(
                        "Resume live board",
                        false,
                        resume_board.clone(),
                    ),
                )}
                {menu_primary_row(
                    "Start New",
                    menu_nav_enter_button(
                        "Start a new AFK round",
                        false,
                        start_new_board.clone(),
                    ),
                )}
            </>
        },
        AfkRootPrimaryAction::Start => html! {
            {menu_primary_row(
                "Start",
                menu_nav_enter_button(
                    "Start AFK mode",
                    false,
                    start_new_board.clone(),
                ),
            )}
        },
        AfkRootPrimaryAction::ConnectAndStart => html! {
            {menu_primary_row(
                "Connect and Start",
                menu_nav_enter_button(
                    "Connect Twitch and start AFK mode",
                    false,
                    connect_twitch_and_start.clone(),
                ),
            )}
        },
    }
}

fn afk_root_option_rows(
    displayed_preferences: AfkMenuPreferences,
    timeout_controls_disabled: bool,
    open_board_size_menu: &Callback<MouseEvent>,
    set_timeout_on: &Callback<MouseEvent>,
    set_timeout_off: &Callback<MouseEvent>,
    decrease_timeout_duration: &Callback<MouseEvent>,
    increase_timeout_duration: &Callback<MouseEvent>,
    open_settings: &Callback<MouseEvent>,
) -> Html {
    html! {
        <>
            {menu_wide_detail_row(
                "Board Size",
                board_size_label(displayed_preferences.board_size),
                menu_nav_enter_button(
                    "Open board size menu",
                    false,
                    open_board_size_menu.clone(),
                ),
            )}
            {menu_toggle_row(
                "Timeout on mistake",
                menu_icon_button(
                    "ok",
                    "Enable timeout on mistake",
                    displayed_preferences.timeout_enabled,
                    timeout_controls_disabled,
                    set_timeout_on.clone(),
                ),
                menu_icon_button(
                    "cancel",
                    "Disable timeout on mistake",
                    !displayed_preferences.timeout_enabled,
                    timeout_controls_disabled,
                    set_timeout_off.clone(),
                ),
            )}
            {
                if displayed_preferences.timeout_enabled {
                    menu_stepper_row(
                        None,
                        "Timeout length",
                        html! { format!("{}s", displayed_preferences.timeout_duration_secs) },
                        menu_icon_button(
                            "minus",
                            "Decrease timeout length",
                            false,
                            timeout_controls_disabled
                                || previous_timeout_duration_secs(
                                    displayed_preferences.timeout_duration_secs,
                                ) == displayed_preferences.timeout_duration_secs,
                            decrease_timeout_duration.clone(),
                        ),
                        menu_icon_button(
                            "plus",
                            "Increase timeout length",
                            false,
                            timeout_controls_disabled
                                || next_timeout_duration_secs(
                                    displayed_preferences.timeout_duration_secs,
                                ) == displayed_preferences.timeout_duration_secs,
                            increase_timeout_duration.clone(),
                        ),
                    )
                } else {
                    Html::default()
                }
            }
            {menu_section_gap()}
            {menu_wide_detail_row(
                "Settings",
                "",
                menu_nav_enter_button(
                    "Open settings",
                    false,
                    open_settings.clone(),
                ),
            )}
        </>
    }
}

/// The AFK root menu keeps one shared option block for resumable, fresh, and
/// disconnected states so new settings rows cannot drift between branches.
fn afk_root_menu_rows(
    action: AfkRootPrimaryAction,
    displayed_preferences: AfkMenuPreferences,
    timeout_controls_disabled: bool,
    resume_board: &Callback<MouseEvent>,
    start_new_board: &Callback<MouseEvent>,
    connect_twitch_and_start: &Callback<MouseEvent>,
    open_board_size_menu: &Callback<MouseEvent>,
    set_timeout_on: &Callback<MouseEvent>,
    set_timeout_off: &Callback<MouseEvent>,
    decrease_timeout_duration: &Callback<MouseEvent>,
    increase_timeout_duration: &Callback<MouseEvent>,
    open_settings: &Callback<MouseEvent>,
) -> Html {
    html! {
        <>
            {afk_root_primary_rows(
                action,
                resume_board,
                start_new_board,
                connect_twitch_and_start,
            )}
            {menu_section_gap()}
            {afk_root_option_rows(
                displayed_preferences,
                timeout_controls_disabled,
                open_board_size_menu,
                set_timeout_on,
                set_timeout_off,
                decrease_timeout_duration,
                increase_timeout_duration,
                open_settings,
            )}
        </>
    }
}

fn afk_return_to_path(start_after_connect: bool) -> String {
    let mut return_to = auth_return_to(AppRoute::Afk);
    if start_after_connect {
        let separator = if return_to.contains('?') { '&' } else { '?' };
        return_to.push(separator);
        return_to.push_str("afk_start=1");
    }
    return_to
}

fn afk_connect_href(base: String, start_after_connect: bool) -> String {
    let separator = if base.contains('?') { '&' } else { '?' };
    let afk_return_to = afk_return_to_path(start_after_connect);
    let return_to = encode_uri_component(&afk_return_to)
        .as_string()
        .unwrap_or(afk_return_to);
    format!("{base}{separator}return_to={return_to}")
}

fn render_auth_error(code: &str) -> AttrValue {
    match code {
        "access_denied" => {
            "Twitch authorization was cancelled. Connect again when you are ready.".into()
        }
        "expired_oauth_state" => {
            "That Twitch connect link expired. Start the connection again from AFK mode.".into()
        }
        "invalid_oauth_state" => {
            "That Twitch connect link is no longer valid. Start the connection again from AFK mode."
                .into()
        }
        "oauth_exchange_failed" => {
            "Twitch authorization completed, but the token exchange failed. Try connecting again."
                .into()
        }
        "missing_code" | "missing_state" | "oauth_error" => {
            "Twitch authorization did not complete correctly. Try connecting again.".into()
        }
        _ => "Twitch authorization failed. Try connecting again.".into(),
    }
}

fn board_menu_prompt() -> AfkFacePrompt {
    AfkFacePrompt {
        message: "Quit?".into(),
        choices: vec![
            AfkFaceChoice {
                label: "Yes".into(),
                title: "Quit the current AFK run".into(),
                action: AfkFaceAction::StopRun,
            },
            AfkFaceChoice {
                label: "No".into(),
                title: "Keep watching the board".into(),
                action: AfkFaceAction::DismissPrompt,
            },
            AfkFaceChoice {
                label: "Menu".into(),
                title: "Open AFK submenu and pause the round".into(),
                action: AfkFaceAction::OpenSubmenu,
            },
        ],
    }
}

fn timeout_duration_index(current: u32) -> usize {
    AFK_TIMEOUT_DURATION_OPTIONS_SECS
        .iter()
        .position(|&candidate| candidate == current)
        .unwrap_or(AFK_DEFAULT_TIMEOUT_DURATION_INDEX)
}

fn previous_timeout_duration_secs(current: u32) -> u32 {
    let index = timeout_duration_index(current);
    AFK_TIMEOUT_DURATION_OPTIONS_SECS[index.saturating_sub(1)]
}

fn next_timeout_duration_secs(current: u32) -> u32 {
    let index = timeout_duration_index(current);
    let next_index = (index + 1).min(AFK_TIMEOUT_DURATION_OPTIONS_SECS.len() - 1);
    AFK_TIMEOUT_DURATION_OPTIONS_SECS[next_index]
}

fn win_prompt_message(session: &AfkSessionSnapshot) -> AttrValue {
    if session.timer_remaining_secs < 10 {
        format!(
            "Close call! Next level? ({})",
            session.phase_countdown_secs.unwrap_or_default().max(0)
        )
        .into()
    } else if session.crater_count > 0 {
        format!(
            "Decent! Next level? ({})",
            session.phase_countdown_secs.unwrap_or_default().max(0)
        )
        .into()
    } else {
        format!(
            "NICE! Next level? ({})",
            session.phase_countdown_secs.unwrap_or_default().max(0)
        )
        .into()
    }
}

fn win_face_icon(session: &AfkSessionSnapshot) -> &'static str {
    if session.timer_remaining_secs < 10 {
        "win-close-call"
    } else if session.crater_count > 0 {
        "win-decent"
    } else {
        "win"
    }
}

fn win_continue_prompt(session: &AfkSessionSnapshot) -> AfkFacePrompt {
    AfkFacePrompt {
        message: win_prompt_message(session),
        choices: vec![
            AfkFaceChoice {
                label: "Yes (!continue)".into(),
                title: "Start the next round now".into(),
                action: AfkFaceAction::ContinueRound,
            },
            AfkFaceChoice {
                label: "No".into(),
                title: "Stop AFK mode".into(),
                action: AfkFaceAction::StopRun,
            },
        ],
    }
}

fn loss_prompt_message(session: &AfkSessionSnapshot) -> AttrValue {
    let prompt = if session.loss_reason == Some(AfkLossReason::Timer) {
        "Too slow! Play again?"
    } else {
        "Too bad. Play again?"
    };
    format!(
        "{prompt} ({})",
        session.phase_countdown_secs.unwrap_or_default().max(0)
    )
    .into()
}

fn loss_continue_prompt(session: &AfkSessionSnapshot) -> AfkFacePrompt {
    AfkFacePrompt {
        message: loss_prompt_message(session),
        choices: vec![
            AfkFaceChoice {
                label: "Yes (!continue)".into(),
                title: "Start the next round now".into(),
                action: AfkFaceAction::ContinueRound,
            },
            AfkFaceChoice {
                label: "No".into(),
                title: "Stop AFK mode".into(),
                action: AfkFaceAction::StopRun,
            },
        ],
    }
}

fn automatic_face_prompt(session: &AfkSessionSnapshot) -> Option<AfkFacePrompt> {
    match session.phase {
        AfkRoundPhase::Won => Some(win_continue_prompt(session)),
        AfkRoundPhase::TimedOut => Some(loss_continue_prompt(session)),
        _ => None,
    }
}

fn has_active_face_prompt(
    screen: AfkScreen,
    status: &LoadState<AfkStatusResponse>,
    manual_prompt: &Option<AfkFacePrompt>,
) -> bool {
    if !matches!(screen, AfkScreen::Board) {
        return false;
    }
    if manual_prompt.is_some() {
        return true;
    }
    matches!(
        status,
        LoadState::Ready(status)
            if status
                .session
                .as_ref()
                .is_some_and(|session| automatic_face_prompt(session).is_some())
    )
}

fn face_notification_event(row: &AfkActivityRow) -> Option<AfkFaceNotificationEvent> {
    let actor = row.actor.as_ref()?;
    match row.kind {
        AfkActivityKind::MineHit => Some(AfkFaceNotificationEvent {
            message: format!("{} found a mine! o7", actor.display_name).into(),
            timeout_ms: AFK_FACE_NOTIFICATION_MS,
        }),
        AfkActivityKind::OutForRound => Some(AfkFaceNotificationEvent {
            message: format!("{} is out for the rest of the round.", actor.display_name).into(),
            timeout_ms: AFK_OUT_FOR_ROUND_NOTIFICATION_MS,
        }),
        AfkActivityKind::Generic => None,
    }
}

fn idle_duration_ms(last_user_activity_at_ms: i64, now_ms: i64) -> Option<i64> {
    (last_user_activity_at_ms > 0).then(|| now_ms.saturating_sub(last_user_activity_at_ms))
}

fn afk_idle_state(session: Option<&AfkSessionSnapshot>, now_ms: i64) -> Option<AfkIdleState> {
    let idle_ms = idle_duration_ms(session?.last_user_activity_at_ms, now_ms)?;
    if idle_ms >= AFK_IDLE_EXPIRY_THRESHOLD_MS {
        return None;
    }
    if idle_ms >= AFK_IDLE_PROMPT_THRESHOLD_MS {
        Some(AfkIdleState::Prompt)
    } else if idle_ms >= AFK_IDLE_SLEEPING_THRESHOLD_MS {
        Some(AfkIdleState::Sleeping)
    } else {
        None
    }
}

fn next_idle_refresh_delay_ms(last_user_activity_at_ms: i64, now_ms: i64) -> Option<u32> {
    let idle_ms = idle_duration_ms(last_user_activity_at_ms, now_ms)?;
    let next_threshold = if idle_ms < AFK_IDLE_SLEEPING_THRESHOLD_MS {
        AFK_IDLE_SLEEPING_THRESHOLD_MS
    } else if idle_ms < AFK_IDLE_PROMPT_THRESHOLD_MS {
        AFK_IDLE_PROMPT_THRESHOLD_MS
    } else {
        return None;
    };
    let remaining_ms = next_threshold.saturating_sub(idle_ms).max(1);
    Some(remaining_ms.min(u32::MAX as i64) as u32)
}

fn active_face_overlay(
    screen: AfkScreen,
    status: &LoadState<AfkStatusResponse>,
    manual_prompt: &Option<AfkFacePrompt>,
    notification: &Option<AfkFaceNotification>,
    idle_state: Option<AfkIdleState>,
) -> Option<AfkFaceOverlay> {
    if matches!(screen, AfkScreen::Board) {
        if let Some(prompt) = manual_prompt.clone() {
            return Some(AfkFaceOverlay::Prompt(prompt));
        }
        if let LoadState::Ready(status) = status {
            if let Some(session) = &status.session {
                let level_status = current_level_status_text(Some(session));
                if let Some(prompt) = automatic_face_prompt(session) {
                    return Some(AfkFaceOverlay::Prompt(prompt));
                }
                return match session.phase {
                    AfkRoundPhase::Countdown => Some(AfkFaceOverlay::Message {
                        message: format!(
                            "Starting in {}...",
                            session.phase_countdown_secs.unwrap_or_default().max(0)
                        )
                        .into(),
                        status: level_status,
                    }),
                    _ => notification
                        .as_ref()
                        .map(|notification| AfkFaceOverlay::Message {
                            message: notification.message.clone(),
                            status: level_status.clone(),
                        })
                        .or_else(|| {
                            matches!(idle_state, Some(AfkIdleState::Prompt)).then_some(
                                AfkFaceOverlay::Message {
                                    message: "Is anyone there?".into(),
                                    status: level_status.clone(),
                                },
                            )
                        })
                        .or_else(|| level_status.map(AfkFaceOverlay::Status)),
                };
            }
        }
        return None;
    }

    None
}

fn afk_face_icon(
    status: &LoadState<AfkStatusResponse>,
    notification: Option<&AfkFaceNotification>,
    idle_state: Option<AfkIdleState>,
) -> &'static str {
    if notification.is_some() {
        return "dejected";
    }
    if idle_state.is_some() {
        return "sleeping";
    }
    match status {
        LoadState::Loading => "mid-open",
        LoadState::Ready(status) => match status.session.as_ref() {
            Some(session) => match session.phase {
                AfkRoundPhase::Countdown => "not-started",
                AfkRoundPhase::Active => "in-progress",
                AfkRoundPhase::Won => win_face_icon(session),
                AfkRoundPhase::TimedOut if session.loss_reason == Some(AfkLossReason::Timer) => {
                    "sleeping"
                }
                AfkRoundPhase::TimedOut => "lose",
                AfkRoundPhase::Stopped => "not-started",
            },
            None => "not-started",
        },
        LoadState::Error(_) | LoadState::Idle => "not-started",
    }
}

fn afk_timer_phase_class(session: Option<&AfkSessionSnapshot>) -> &'static str {
    match session.map(|session| session.phase) {
        Some(AfkRoundPhase::Countdown) => "phase-countdown",
        Some(AfkRoundPhase::Active) => "phase-active",
        Some(AfkRoundPhase::Won) => "phase-won",
        Some(AfkRoundPhase::TimedOut) => "phase-timed-out",
        Some(AfkRoundPhase::Stopped) | None => "phase-idle",
    }
}

fn board_counter_text(session: Option<&AfkSessionSnapshot>) -> String {
    let value = session
        .map(|session| session.timer_remaining_secs)
        .unwrap_or_default();
    format_for_counter(value)
}

fn mines_counter_text(session: Option<&AfkSessionSnapshot>) -> String {
    format_for_counter(
        session
            .map(|session| session.live_mines_left)
            .unwrap_or_default(),
    )
}

fn view_face_overlay(overlay: &AfkFaceOverlay, on_action: Callback<AfkFaceAction>) -> Html {
    match overlay {
        AfkFaceOverlay::Message { message, status } => html! {
            <div class="face-prompt-rail" aria-live="polite">
                <div class="face-prompt-bubble">{message.clone()}</div>
                {
                    if let Some(status) = status.clone() {
                        html! { <div class="face-prompt-status">{status}</div> }
                    } else {
                        Html::default()
                    }
                }
            </div>
        },
        AfkFaceOverlay::Prompt(prompt) => html! {
            <div class="face-prompt-rail" aria-live="polite">
                <div class="face-prompt-bubble">{prompt.message.clone()}</div>
                <div class="face-prompt-choices">
                    {
                        for prompt.choices.iter().map(|choice| {
                            let title = choice.title.clone();
                            let label = choice.label.clone();
                            let action = choice.action;
                            let on_action = on_action.clone();
                            let onclick = Callback::from(move |e: MouseEvent| {
                                e.stop_propagation();
                                on_action.emit(action);
                            });
                            html! {
                                <button class="face-prompt-choice" {title} {onclick}>{label}</button>
                            }
                        })
                    }
                </div>
            </div>
        },
        AfkFaceOverlay::Status(status) => html! {
            <div class="face-prompt-rail" aria-live="polite">
                <div class="face-prompt-status">{status.clone()}</div>
            </div>
        },
    }
}

fn format_cell_code((x, y): (usize, usize)) -> String {
    let row = AFK_ROW_LABELS
        .as_bytes()
        .get(y)
        .copied()
        .map(char::from)
        .unwrap_or('?');
    let column = AFK_COLUMN_LABELS
        .as_bytes()
        .get(x)
        .copied()
        .map(char::from)
        .unwrap_or('?');
    let mut code = String::with_capacity(2);
    code.push(display_afk_label_char(row));
    code.push(display_afk_label_char(column));
    code
}

fn display_afk_label_char(label: char) -> char {
    match label {
        'a'..='z' if label != 'o' => label.to_ascii_uppercase(),
        _ => label,
    }
}

const AFK_ROW_LABELS: &str = "123456789abcdefghijk";
const AFK_COLUMN_LABELS: &str = "abcdefghijklmnopqrstuvwxyz0123";

fn board_cell_at(session: &AfkSessionSnapshot, x: usize, y: usize) -> Option<AfkCellSnapshot> {
    let width = usize::from(session.board.width);
    let height = usize::from(session.board.height);
    if x >= width || y >= height {
        return None;
    }
    session.board.cells.get(y * width + x).copied()
}

fn board_label_at(session: &AfkSessionSnapshot, x: usize, y: usize) -> Option<bool> {
    let width = usize::from(session.board.width);
    let height = usize::from(session.board.height);
    if x >= width || y >= height || session.labeled_cells.len() != width * height {
        return None;
    }
    session.labeled_cells.get(y * width + x).copied()
}

fn should_show_cell_code(
    session: &AfkSessionSnapshot,
    x: usize,
    y: usize,
    cell: AfkCellSnapshot,
) -> bool {
    if let Some(show_code) = board_label_at(session, x, y) {
        return show_code;
    }
    if matches!(session.phase, AfkRoundPhase::Countdown) {
        return false;
    }
    if matches!(cell, AfkCellSnapshot::Flagged) {
        return true;
    }
    if !matches!(cell, AfkCellSnapshot::Hidden) {
        return false;
    }

    for dy in -1isize..=1 {
        for dx in -1isize..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let nx = x.checked_add_signed(dx);
            let ny = y.checked_add_signed(dy);
            let (Some(nx), Some(ny)) = (nx, ny) else {
                continue;
            };
            match board_cell_at(session, nx, ny) {
                Some(AfkCellSnapshot::Revealed(count)) if count > 0 => return true,
                Some(AfkCellSnapshot::Crater) => return true,
                _ => {}
            }
        }
    }

    false
}

fn request_for_streamer_click(
    cell: AfkCellSnapshot,
    coords: (usize, usize),
    button: i16,
) -> Option<AfkActionRequest> {
    let kind = match (button, cell) {
        (0, AfkCellSnapshot::Hidden) => AfkActionKind::Reveal,
        (0, AfkCellSnapshot::Revealed(count)) if count > 0 => AfkActionKind::Chord,
        (2, AfkCellSnapshot::Revealed(count)) if count > 0 => AfkActionKind::ChordFlag,
        (2, AfkCellSnapshot::Hidden | AfkCellSnapshot::Flagged) => AfkActionKind::ToggleFlag,
        _ => return None,
    };
    Some(AfkActionRequest {
        kind,
        x: coords.0 as u8,
        y: coords.1 as u8,
    })
}

fn render_afk_cell(
    session: &AfkSessionSnapshot,
    (x, y): (usize, usize),
    cell: AfkCellSnapshot,
    interactive: bool,
    on_cell_action: Callback<AfkActionRequest>,
) -> Html {
    let code = format_cell_code((x, y));
    let show_code = should_show_cell_code(session, x, y, cell);
    let class = match cell {
        AfkCellSnapshot::Hidden => classes!(
            "cell",
            (!interactive).then_some("locked"),
            "afk-cell",
            "afk-cell-hidden"
        ),
        AfkCellSnapshot::Flagged => {
            classes!(
                "cell",
                (!interactive).then_some("locked"),
                "flag",
                "afk-cell"
            )
        }
        AfkCellSnapshot::Crater => classes!(
            "cell",
            (!interactive).then_some("locked"),
            "open",
            "mine",
            "oops",
            "afk-cell"
        ),
        AfkCellSnapshot::Mine => classes!(
            "cell",
            (!interactive).then_some("locked"),
            "open",
            "mine",
            "afk-cell"
        ),
        AfkCellSnapshot::Misflagged => classes!(
            "cell",
            (!interactive).then_some("locked"),
            "flag",
            "wrong",
            "afk-cell"
        ),
        AfkCellSnapshot::Revealed(count) => {
            classes!(
                "cell",
                (!interactive).then_some("locked"),
                "open",
                "afk-cell",
                format!("num-{count}")
            )
        }
    };

    let content = match cell {
        AfkCellSnapshot::Hidden => show_code
            .then(|| html! { <span class="afk-cell-code">{code}</span> })
            .unwrap_or_default(),
        AfkCellSnapshot::Flagged => html! {
            <>
                {
                    if show_code {
                        html! { <span class="afk-cell-tag">{code}</span> }
                    } else {
                        Html::default()
                    }
                }
                <Icon name="flag" class={classes!("cell-icon")}/>
            </>
        },
        AfkCellSnapshot::Crater => html! {
            <Icon name="mine-exploded" class={classes!("cell-icon")}/>
        },
        AfkCellSnapshot::Mine => html! {
            <Icon name="mine" class={classes!("cell-icon")}/>
        },
        AfkCellSnapshot::Misflagged => html! {
            <Icon name="flag" class={classes!("cell-icon")}/>
        },
        AfkCellSnapshot::Revealed(0) => Html::default(),
        AfkCellSnapshot::Revealed(count) => html! {
            <Glyph
                set={GlyphSet::Cell}
                ch={char::from_digit(count.into(), 10).expect("Cell numbers fit in a single digit")}
                class={classes!("cell-glyph")}
            />
        },
    };

    let onmousedown = Callback::from(move |e: MouseEvent| {
        e.prevent_default();
        e.stop_propagation();
        if !interactive {
            return;
        }
        if let Some(request) = request_for_streamer_click(cell, (x, y), e.button()) {
            on_cell_action.emit(request);
        }
    });
    let oncontextmenu = Callback::from(|e: MouseEvent| e.prevent_default());

    html! { <td class={class} {onmousedown} {oncontextmenu}>{content}</td> }
}

fn render_afk_board(
    session: &AfkSessionSnapshot,
    interactive: bool,
    on_cell_action: Callback<AfkActionRequest>,
) -> Html {
    let width = usize::from(session.board.width);
    let height = usize::from(session.board.height);
    html! {
        <table class={classes!("afk-board-grid", interactive.then_some("playable"))}>
            {
                for (0..height).map(|y| html! {
                    <tr>
                        {
                            for (0..width).map(|x| {
                                let cell = session.board.cells[y * width + x];
                                render_afk_cell(session, (x, y), cell, interactive, on_cell_action.clone())
                            })
                        }
                    </tr>
                })
            }
        </table>
    }
}

#[function_component]
pub(crate) fn AfkView(props: &AfkViewProps) -> Html {
    let runtime = frontend_runtime_config();
    let status = use_state_eq(|| LoadState::<AfkStatusResponse>::Idle);
    let screen = use_state_eq(|| AfkScreen::Menu);
    let menu_page = use_state_eq(|| AfkMenuPage::Root);
    let pending_board_size = use_state_eq(|| None::<AfkBoardSize>);
    let pre_auth_preferences = use_state_eq(|| {
        Option::<AfkConnectStartDraft>::local_or_default().map(|draft| draft.preferences)
    });
    let auto_start_in_progress = use_state_eq(|| false);
    let manual_face_prompt = use_state_eq(|| None::<AfkFacePrompt>);
    let face_notification = use_state_eq(|| None::<AfkFaceNotification>);
    let face_notification_timeout = use_mut_ref(|| None::<Timeout>);
    let next_face_notification_id = use_mut_ref(|| 0_u64);
    let idle_refresh_tick = use_state_eq(|| 0_u64);
    let idle_refresh_timeout = use_mut_ref(|| None::<Timeout>);
    let last_error = use_state_eq(|| None::<String>);
    let socket_path = match &*status {
        LoadState::Ready(status) if status.auth.identity.is_some() => status.websocket_path.clone(),
        _ => None,
    };

    let clear_face_notification = {
        let face_notification = face_notification.clone();
        let face_notification_timeout = face_notification_timeout.clone();
        Rc::new(move || {
            face_notification_timeout.borrow_mut().take();
            face_notification.set(None);
        })
    };

    let show_face_notification = {
        let face_notification = face_notification.clone();
        let face_notification_timeout = face_notification_timeout.clone();
        let next_face_notification_id = next_face_notification_id.clone();
        Rc::new(move |event: AfkFaceNotificationEvent| {
            let notification_id = {
                let mut next_id = next_face_notification_id.borrow_mut();
                *next_id += 1;
                *next_id
            };
            face_notification_timeout.borrow_mut().take();
            face_notification.set(Some(AfkFaceNotification {
                id: notification_id,
                message: event.message,
            }));

            let face_notification = face_notification.clone();
            let face_notification_timeout_for_store = face_notification_timeout.clone();
            let face_notification_timeout_for_callback = face_notification_timeout.clone();
            *face_notification_timeout_for_store.borrow_mut() =
                Some(Timeout::new(event.timeout_ms, move || {
                    let still_active = matches!(
                        &*face_notification,
                        Some(notification) if notification.id == notification_id
                    );
                    if still_active {
                        face_notification.set(None);
                    }
                    face_notification_timeout_for_callback.borrow_mut().take();
                }));
        })
    };

    {
        let status = status.clone();
        use_effect_with(runtime.afk_enabled, move |enabled| {
            if *enabled {
                status.set(LoadState::Loading);
                let status = status.clone();
                spawn_local(async move {
                    match fetch_status().await {
                        Ok(response) => status.set(LoadState::Ready(response)),
                        Err(error) => status.set(LoadState::Error(error)),
                    }
                });
            } else {
                status.set(LoadState::Idle);
            }
            || ()
        });
    }

    {
        let pre_auth_preferences = pre_auth_preferences.clone();
        use_effect_with(
            ((*status).clone(), props.start_after_connect),
            move |(status_snapshot, start_after_connect)| {
                if let LoadState::Ready(status) = status_snapshot {
                    if status.auth.identity.is_none() {
                        if (*pre_auth_preferences).is_none() {
                            let preferences = afk_menu_preferences_from_status(status);
                            pre_auth_preferences.set(Some(preferences));
                            persist_afk_connect_start_draft(Some(preferences));
                        }
                    } else if !*start_after_connect && (*pre_auth_preferences).is_some() {
                        pre_auth_preferences.set(None);
                        persist_afk_connect_start_draft(None);
                    }
                }
                || ()
            },
        );
    }

    {
        let auto_start_in_progress = auto_start_in_progress.clone();
        use_effect_with(props.start_after_connect, move |start_after_connect| {
            if !*start_after_connect && *auto_start_in_progress {
                auto_start_in_progress.set(false);
            }
            || ()
        });
    }

    {
        let idle_refresh_tick = idle_refresh_tick.clone();
        let idle_refresh_timeout = idle_refresh_timeout.clone();
        let last_user_activity_at_ms = match &*status {
            LoadState::Ready(status) => status
                .session
                .as_ref()
                .map(|session| session.last_user_activity_at_ms),
            _ => None,
        };
        let idle_refresh_version = *idle_refresh_tick;
        use_effect_with(
            (*screen, last_user_activity_at_ms, idle_refresh_version),
            move |(screen, last_user_activity_at_ms, _)| {
                idle_refresh_timeout.borrow_mut().take();
                if let Some(last_user_activity_at_ms) = *last_user_activity_at_ms {
                    if matches!(*screen, AfkScreen::Board) {
                        if let Some(delay_ms) =
                            next_idle_refresh_delay_ms(last_user_activity_at_ms, browser_now_ms())
                        {
                            let idle_refresh_tick = idle_refresh_tick.clone();
                            let idle_refresh_timeout_for_store = idle_refresh_timeout.clone();
                            let idle_refresh_timeout_for_callback = idle_refresh_timeout.clone();
                            *idle_refresh_timeout_for_store.borrow_mut() =
                                Some(Timeout::new(delay_ms, move || {
                                    idle_refresh_tick.set(*idle_refresh_tick + 1);
                                    idle_refresh_timeout_for_callback.borrow_mut().take();
                                }));
                        }
                    }
                }
                let idle_refresh_timeout = idle_refresh_timeout.clone();
                move || {
                    idle_refresh_timeout.borrow_mut().take();
                }
            },
        );
    }

    {
        let status = status.clone();
        let last_error = last_error.clone();
        let screen = screen.clone();
        let show_face_notification = show_face_notification.clone();
        use_effect_with(socket_path.clone(), move |socket_path| {
            let mut socket = None::<WebSocket>;
            let mut onmessage = None::<Closure<dyn FnMut(MessageEvent)>>;
            let mut onopen = None::<Closure<dyn FnMut(Event)>>;
            let mut onerror = None::<Closure<dyn FnMut(JsValue)>>;

            if let Some(socket_path) = socket_path.clone() {
                let socket_url = websocket_path(&socket_path);
                match WebSocket::new(&socket_url) {
                    Ok(ws) => {
                        let status_state = status.clone();
                        let last_error_for_message = last_error.clone();
                        let last_error_for_socket = last_error.clone();
                        let screen_for_message = screen.clone();
                        let show_face_notification = show_face_notification.clone();

                        let message_handler =
                            Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                                let Some(payload) = event.data().as_string() else {
                                    return;
                                };
                                match serde_json::from_str::<AfkServerMessage>(&payload) {
                                    Ok(AfkServerMessage::Connected { status: next }) => {
                                        last_error_for_message.set(None);
                                        status_state.set(LoadState::Ready(next));
                                    }
                                    Ok(AfkServerMessage::Snapshot { session }) => {
                                        if let LoadState::Ready(mut next) = (*status_state).clone()
                                        {
                                            last_error_for_message.set(None);
                                            next.session = Some(session);
                                            status_state.set(LoadState::Ready(next));
                                        }
                                    }
                                    Ok(AfkServerMessage::Activity { row }) => {
                                        if matches!(*screen_for_message, AfkScreen::Board)
                                            && let Some(notification) =
                                                face_notification_event(&row)
                                        {
                                            show_face_notification(notification);
                                        }
                                    }
                                    Ok(AfkServerMessage::Error { message }) => {
                                        last_error_for_message.set(Some(message));
                                    }
                                    Err(error) => {
                                        last_error_for_message.set(Some(error.to_string()));
                                    }
                                }
                            });
                        ws.set_onmessage(Some(message_handler.as_ref().unchecked_ref()));

                        let onopen_socket = ws.clone();
                        let last_error_for_open = last_error.clone();
                        let open_handler =
                            Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
                                last_error_for_open.set(None);
                                let _ = onopen_socket.send_with_str(
                                    &serde_json::to_string(&AfkClientMessage::Ping)
                                        .unwrap_or_default(),
                                );
                            });
                        ws.set_onopen(Some(open_handler.as_ref().unchecked_ref()));

                        let error_handler =
                            Closure::<dyn FnMut(JsValue)>::new(move |error: JsValue| {
                                log::warn!("afk websocket error: {error:?}");
                                last_error_for_socket
                                    .set(Some("Live updates are unavailable right now.".into()));
                            });
                        ws.set_onerror(Some(error_handler.as_ref().unchecked_ref()));

                        socket = Some(ws);
                        onmessage = Some(message_handler);
                        onopen = Some(open_handler);
                        onerror = Some(error_handler);
                    }
                    Err(error) => {
                        log::warn!("failed to open afk websocket: {error:?}");
                        last_error.set(Some("Live updates could not connect.".into()));
                    }
                }
            }

            move || {
                if let Some(socket) = socket {
                    socket.close().ok();
                    socket.set_onmessage(None);
                    socket.set_onopen(None);
                    socket.set_onerror(None);
                }
                drop(onmessage);
                drop(onopen);
                drop(onerror);
            }
        });
    }

    {
        let screen = screen.clone();
        let manual_face_prompt = manual_face_prompt.clone();
        let clear_face_notification = clear_face_notification.clone();
        let last_error = last_error.clone();
        use_effect_with(status.clone(), move |status| {
            if matches!(*screen, AfkScreen::Board) {
                let ready_status = match &**status {
                    LoadState::Ready(status) => Some(status),
                    _ => None,
                };
                let has_session = ready_status.is_some_and(|status| status.session.is_some());
                if !has_session {
                    clear_face_notification();
                    manual_face_prompt.set(None);
                    screen.set(AfkScreen::Menu);
                } else if let Some(status) = ready_status {
                    if has_critical_chat_failure(status) {
                        clear_face_notification();
                        manual_face_prompt.set(None);
                        last_error.set(
                            status_chat_error(status)
                                .or_else(|| Some("Twitch chat is disconnected.".to_string())),
                        );
                        screen.set(AfkScreen::Menu);
                    }
                }
            }
            || ()
        });
    }

    {
        let status = status.clone();
        let screen = screen.clone();
        let menu_page = menu_page.clone();
        let manual_face_prompt = manual_face_prompt.clone();
        let last_error = last_error.clone();
        let pre_auth_preferences = pre_auth_preferences.clone();
        let auto_start_in_progress = auto_start_in_progress.clone();
        let on_consume_start_after_connect = props.on_consume_start_after_connect.clone();
        use_effect_with(
            (
                props.start_after_connect,
                (*status).clone(),
                *pre_auth_preferences,
                *auto_start_in_progress,
            ),
            move |(
                start_after_connect,
                status_snapshot,
                pending_preferences,
                auto_start_pending,
            )| {
                if *start_after_connect
                    && !*auto_start_pending
                    && let LoadState::Ready(current_status) = status_snapshot
                    && current_status.auth.identity.is_some()
                {
                    let preferences = pending_preferences
                        .unwrap_or_else(|| afk_menu_preferences_from_status(current_status));
                    auto_start_in_progress.set(true);
                    on_consume_start_after_connect.emit(());
                    pre_auth_preferences.set(None);
                    persist_afk_connect_start_draft(None);
                    manual_face_prompt.set(None);
                    menu_page.set(AfkMenuPage::Root);
                    last_error.set(None);

                    let status = status.clone();
                    let screen = screen.clone();
                    let last_error = last_error.clone();
                    spawn_local(async move {
                        match apply_preferences_and_start(preferences).await {
                            Ok(response) => {
                                handle_started_status(&status, &screen, &last_error, response);
                            }
                            Err(error) => status.set(LoadState::Error(error)),
                        }
                    });
                }
                || ()
            },
        );
    }

    {
        let clear_face_notification = clear_face_notification.clone();
        let notification_id = (*face_notification)
            .as_ref()
            .map(|notification| notification.id);
        use_effect_with(
            (*screen, notification_id),
            move |(screen, notification_id)| {
                if matches!(*screen, AfkScreen::Menu) && notification_id.is_some() {
                    clear_face_notification();
                }
                || ()
            },
        );
    }

    {
        let clear_face_notification = clear_face_notification.clone();
        let prompt_active = has_active_face_prompt(*screen, &status, &manual_face_prompt);
        let notification_id = (*face_notification)
            .as_ref()
            .map(|notification| notification.id);
        use_effect_with(
            (prompt_active, notification_id),
            move |(prompt_active, notification_id)| {
                if *prompt_active && notification_id.is_some() {
                    clear_face_notification();
                }
                || ()
            },
        );
    }

    let go_to_main_menu = {
        let manual_face_prompt = manual_face_prompt.clone();
        let menu_page = menu_page.clone();
        let clear_face_notification = clear_face_notification.clone();
        let on_menu = props.on_menu.clone();
        Callback::from(move |_: MouseEvent| {
            clear_face_notification();
            manual_face_prompt.set(None);
            menu_page.set(AfkMenuPage::Root);
            on_menu.emit(());
        })
    };

    let open_board_size_menu = {
        let menu_page = menu_page.clone();
        Callback::from(move |_| menu_page.set(AfkMenuPage::BoardSize))
    };

    let close_board_size_menu = {
        let menu_page = menu_page.clone();
        Callback::from(move |_| menu_page.set(AfkMenuPage::Root))
    };

    let resume_board = {
        let screen = screen.clone();
        let menu_page = menu_page.clone();
        let manual_face_prompt = manual_face_prompt.clone();
        let status = status.clone();
        let last_error = last_error.clone();
        Callback::from(move |_| {
            manual_face_prompt.set(None);
            menu_page.set(AfkMenuPage::Root);
            let should_resume = matches!(
                &*status,
                LoadState::Ready(AfkStatusResponse {
                    session: Some(AfkSessionSnapshot { paused: true, .. }),
                    ..
                })
            );
            if should_resume {
                let status = status.clone();
                let screen = screen.clone();
                let last_error = last_error.clone();
                spawn_local(async move {
                    let _ = post_action("/api/afk/resume").await;
                    match fetch_status().await {
                        Ok(response) => {
                            let has_session = response.session.is_some();
                            let can_open_board = has_session
                                && !matches!(
                                    response.chat_connection,
                                    AfkChatConnectionState::Error
                                );
                            let next_error = status_chat_error(&response);
                            status.set(LoadState::Ready(response));
                            if can_open_board {
                                screen.set(AfkScreen::Board);
                            } else if let Some(error) = next_error {
                                last_error.set(Some(error));
                            }
                        }
                        Err(error) => status.set(LoadState::Error(error)),
                    }
                });
            } else {
                screen.set(AfkScreen::Board);
            }
        })
    };

    let start_new_board = {
        let status = status.clone();
        let screen = screen.clone();
        let menu_page = menu_page.clone();
        let manual_face_prompt = manual_face_prompt.clone();
        let last_error = last_error.clone();
        Callback::from(move |_| {
            manual_face_prompt.set(None);
            menu_page.set(AfkMenuPage::Root);
            let status = status.clone();
            let screen = screen.clone();
            let last_error = last_error.clone();
            spawn_local(async move {
                last_error.set(None);
                match post_empty_status("/api/afk/start").await {
                    Ok(response) => {
                        handle_started_status(&status, &screen, &last_error, response);
                    }
                    Err(error) => status.set(LoadState::Error(error)),
                }
            });
        })
    };

    let disconnect_twitch = {
        let status = status.clone();
        let screen = screen.clone();
        let menu_page = menu_page.clone();
        let manual_face_prompt = manual_face_prompt.clone();
        Callback::from(move |_| {
            manual_face_prompt.set(None);
            screen.set(AfkScreen::Menu);
            menu_page.set(AfkMenuPage::Root);
            let status = status.clone();
            spawn_local(async move {
                let _ = post_action("/auth/logout").await;
                match fetch_status().await {
                    Ok(response) => status.set(LoadState::Ready(response)),
                    Err(error) => status.set(LoadState::Error(error)),
                }
            });
        })
    };

    let connect_twitch_and_start = {
        let href = match &*status {
            LoadState::Ready(status) => status
                .connect_url
                .clone()
                .unwrap_or_else(|| app_path("/auth/twitch/login")),
            _ => app_path("/auth/twitch/login"),
        };
        let pre_auth_preferences = pre_auth_preferences.clone();
        let status = status.clone();
        Callback::from(move |_| {
            let preferences = match &*status {
                LoadState::Ready(current_status) => {
                    displayed_afk_menu_preferences(current_status, *pre_auth_preferences)
                }
                _ => AfkMenuPreferences::default(),
            };
            persist_afk_connect_start_draft(Some(preferences));
            let href = afk_connect_href(href.clone(), true);
            let _ = gloo::utils::window().location().set_href(&href);
        })
    };

    let open_settings = {
        let on_open_settings = props.on_open_settings.clone();
        Callback::from(move |_| on_open_settings.emit(()))
    };

    let change_board_size = {
        let status = status.clone();
        let menu_page = menu_page.clone();
        let pending_board_size = pending_board_size.clone();
        let pre_auth_preferences = pre_auth_preferences.clone();
        Rc::new(move |next_board_size: AfkBoardSize| {
            let LoadState::Ready(current_status) = &*status else {
                return;
            };

            if current_status.auth.identity.is_none() {
                let current_preferences =
                    displayed_afk_menu_preferences(current_status, *pre_auth_preferences);
                let next_preferences = AfkMenuPreferences {
                    board_size: next_board_size,
                    ..current_preferences
                };
                pre_auth_preferences.set(Some(next_preferences));
                persist_afk_connect_start_draft(Some(next_preferences));
                menu_page.set(AfkMenuPage::Root);
                return;
            }

            match plan_board_size_change(current_status, next_board_size) {
                AfkBoardSizeChangePlan::NoChange => {
                    menu_page.set(AfkMenuPage::Root);
                }
                AfkBoardSizeChangePlan::ApplyOnly(next_board_size) => {
                    let status = status.clone();
                    let menu_page = menu_page.clone();
                    spawn_local(async move {
                        match post_json_status(
                            "/api/afk/board-size",
                            &serde_json::json!({ "board_size": next_board_size }),
                        )
                        .await
                        {
                            Ok(response) => {
                                status.set(LoadState::Ready(response));
                                menu_page.set(AfkMenuPage::Root);
                            }
                            Err(error) => status.set(LoadState::Error(error)),
                        }
                    });
                }
                AfkBoardSizeChangePlan::ConfirmRestart(next_board_size) => {
                    pending_board_size.set(Some(next_board_size));
                    menu_page.set(AfkMenuPage::ConfirmBoardSize);
                }
            }
        })
    };

    let set_board_size_tiny = {
        let change_board_size = change_board_size.clone();
        Callback::from(move |_| change_board_size(AfkBoardSize::Tiny))
    };

    let set_board_size_small = {
        let change_board_size = change_board_size.clone();
        Callback::from(move |_| change_board_size(AfkBoardSize::Small))
    };

    let set_board_size_medium = {
        let change_board_size = change_board_size.clone();
        Callback::from(move |_| change_board_size(AfkBoardSize::Medium))
    };

    let set_board_size_large = {
        let change_board_size = change_board_size.clone();
        Callback::from(move |_| change_board_size(AfkBoardSize::Large))
    };

    let cancel_board_size_restart = {
        let menu_page = menu_page.clone();
        let pending_board_size = pending_board_size.clone();
        Callback::from(move |_| {
            pending_board_size.set(None);
            menu_page.set(AfkMenuPage::BoardSize);
        })
    };

    let confirm_board_size_restart = {
        let status = status.clone();
        let screen = screen.clone();
        let menu_page = menu_page.clone();
        let pending_board_size = pending_board_size.clone();
        let last_error = last_error.clone();
        Callback::from(move |_| {
            let Some(next_board_size) = *pending_board_size else {
                return;
            };
            pending_board_size.set(None);
            menu_page.set(AfkMenuPage::Root);
            let status = status.clone();
            let screen = screen.clone();
            let last_error = last_error.clone();
            spawn_local(async move {
                let size_response = post_json_status(
                    "/api/afk/board-size",
                    &serde_json::json!({ "board_size": next_board_size }),
                )
                .await;
                match size_response {
                    Ok(_) => {
                        last_error.set(None);
                        match post_empty_status("/api/afk/start").await {
                            Ok(response) => {
                                handle_started_status(&status, &screen, &last_error, response);
                            }
                            Err(error) => status.set(LoadState::Error(error)),
                        }
                    }
                    Err(error) => status.set(LoadState::Error(error)),
                }
            });
        })
    };

    let set_timeout_on = {
        let status = status.clone();
        let pre_auth_preferences = pre_auth_preferences.clone();
        Callback::from(move |_| {
            let LoadState::Ready(current_status) = &*status else {
                return;
            };
            if current_status.auth.identity.is_none() {
                let next_preferences = AfkMenuPreferences {
                    timeout_enabled: true,
                    ..displayed_afk_menu_preferences(current_status, *pre_auth_preferences)
                };
                pre_auth_preferences.set(Some(next_preferences));
                persist_afk_connect_start_draft(Some(next_preferences));
                return;
            }
            let status = status.clone();
            spawn_local(async move {
                match post_json_status("/api/afk/timeout", &serde_json::json!({ "enabled": true }))
                    .await
                {
                    Ok(response) => status.set(LoadState::Ready(response)),
                    Err(error) => status.set(LoadState::Error(error)),
                }
            });
        })
    };

    let set_timeout_off = {
        let status = status.clone();
        let pre_auth_preferences = pre_auth_preferences.clone();
        Callback::from(move |_| {
            let LoadState::Ready(current_status) = &*status else {
                return;
            };
            if current_status.auth.identity.is_none() {
                let next_preferences = AfkMenuPreferences {
                    timeout_enabled: false,
                    ..displayed_afk_menu_preferences(current_status, *pre_auth_preferences)
                };
                pre_auth_preferences.set(Some(next_preferences));
                persist_afk_connect_start_draft(Some(next_preferences));
                return;
            }
            let status = status.clone();
            spawn_local(async move {
                match post_json_status("/api/afk/timeout", &serde_json::json!({ "enabled": false }))
                    .await
                {
                    Ok(response) => status.set(LoadState::Ready(response)),
                    Err(error) => status.set(LoadState::Error(error)),
                }
            });
        })
    };

    let decrease_timeout_duration = {
        let status = status.clone();
        let pre_auth_preferences = pre_auth_preferences.clone();
        Callback::from(move |_| {
            let next_duration = match &*status {
                LoadState::Ready(current_status) if current_status.auth.identity.is_none() => {
                    previous_timeout_duration_secs(
                        displayed_afk_menu_preferences(current_status, *pre_auth_preferences)
                            .timeout_duration_secs,
                    )
                }
                LoadState::Ready(status) => {
                    previous_timeout_duration_secs(status.timeout_duration_secs)
                }
                _ => return,
            };
            if let LoadState::Ready(current_status) = &*status
                && current_status.auth.identity.is_none()
            {
                let next_preferences = AfkMenuPreferences {
                    timeout_duration_secs: next_duration,
                    ..displayed_afk_menu_preferences(current_status, *pre_auth_preferences)
                };
                pre_auth_preferences.set(Some(next_preferences));
                persist_afk_connect_start_draft(Some(next_preferences));
                return;
            }
            let status = status.clone();
            spawn_local(async move {
                match post_json_status(
                    "/api/afk/timeout",
                    &serde_json::json!({ "duration_secs": next_duration }),
                )
                .await
                {
                    Ok(response) => status.set(LoadState::Ready(response)),
                    Err(error) => status.set(LoadState::Error(error)),
                }
            });
        })
    };

    let increase_timeout_duration = {
        let status = status.clone();
        let pre_auth_preferences = pre_auth_preferences.clone();
        Callback::from(move |_| {
            let next_duration = match &*status {
                LoadState::Ready(current_status) if current_status.auth.identity.is_none() => {
                    next_timeout_duration_secs(
                        displayed_afk_menu_preferences(current_status, *pre_auth_preferences)
                            .timeout_duration_secs,
                    )
                }
                LoadState::Ready(status) => {
                    next_timeout_duration_secs(status.timeout_duration_secs)
                }
                _ => return,
            };
            if let LoadState::Ready(current_status) = &*status
                && current_status.auth.identity.is_none()
            {
                let next_preferences = AfkMenuPreferences {
                    timeout_duration_secs: next_duration,
                    ..displayed_afk_menu_preferences(current_status, *pre_auth_preferences)
                };
                pre_auth_preferences.set(Some(next_preferences));
                persist_afk_connect_start_draft(Some(next_preferences));
                return;
            }
            let status = status.clone();
            spawn_local(async move {
                match post_json_status(
                    "/api/afk/timeout",
                    &serde_json::json!({ "duration_secs": next_duration }),
                )
                .await
                {
                    Ok(response) => status.set(LoadState::Ready(response)),
                    Err(error) => status.set(LoadState::Error(error)),
                }
            });
        })
    };

    let idle_state = match &*status {
        LoadState::Ready(status) => afk_idle_state(status.session.as_ref(), browser_now_ms()),
        _ => None,
    };
    let current_overlay = active_face_overlay(
        *screen,
        &status,
        &manual_face_prompt,
        &face_notification,
        idle_state,
    );
    let face_button_locked = current_overlay
        .as_ref()
        .is_some_and(|overlay| matches!(overlay, AfkFaceOverlay::Prompt(_)));

    let on_face_button = {
        let manual_face_prompt = manual_face_prompt.clone();
        let clear_face_notification = clear_face_notification.clone();
        let face_button_locked = face_button_locked;
        Callback::from(move |e: MouseEvent| {
            e.stop_propagation();
            if !face_button_locked {
                clear_face_notification();
                manual_face_prompt.set(Some(board_menu_prompt()));
            }
        })
    };

    let on_face_action = {
        let manual_face_prompt = manual_face_prompt.clone();
        let menu_page = menu_page.clone();
        let screen = screen.clone();
        let status = status.clone();
        let last_error = last_error.clone();
        Callback::from(move |action: AfkFaceAction| match action {
            AfkFaceAction::DismissPrompt => {
                manual_face_prompt.set(None);
            }
            AfkFaceAction::OpenSubmenu => {
                manual_face_prompt.set(None);
                menu_page.set(AfkMenuPage::Root);
                let status = status.clone();
                let screen = screen.clone();
                spawn_local(async move {
                    let _ = post_action("/api/afk/pause").await;
                    match fetch_status().await {
                        Ok(response) => {
                            status.set(LoadState::Ready(response));
                            screen.set(AfkScreen::Menu);
                        }
                        Err(error) => status.set(LoadState::Error(error)),
                    }
                });
            }
            AfkFaceAction::ContinueRound => {
                manual_face_prompt.set(None);
                menu_page.set(AfkMenuPage::Root);
                let status = status.clone();
                let screen = screen.clone();
                let last_error = last_error.clone();
                spawn_local(async move {
                    let _ = post_action("/api/afk/continue").await;
                    match fetch_status().await {
                        Ok(response) => {
                            let has_session = response.session.is_some();
                            let can_open_board = has_session
                                && !matches!(
                                    response.chat_connection,
                                    AfkChatConnectionState::Error
                                );
                            let next_error = status_chat_error(&response);
                            status.set(LoadState::Ready(response));
                            if can_open_board {
                                screen.set(AfkScreen::Board);
                            } else if let Some(error) = next_error {
                                last_error.set(Some(error));
                            }
                        }
                        Err(error) => status.set(LoadState::Error(error)),
                    }
                });
            }
            AfkFaceAction::StopRun => {
                manual_face_prompt.set(None);
                menu_page.set(AfkMenuPage::Root);
                screen.set(AfkScreen::Menu);
                let status = status.clone();
                spawn_local(async move {
                    let _ = post_action("/api/afk/stop").await;
                    match fetch_status().await {
                        Ok(response) => status.set(LoadState::Ready(response)),
                        Err(error) => status.set(LoadState::Error(error)),
                    }
                });
            }
        })
    };

    let on_streamer_action = {
        let status = status.clone();
        let last_error = last_error.clone();
        Callback::from(move |request: AfkActionRequest| {
            let status = status.clone();
            let last_error = last_error.clone();
            spawn_local(async move {
                match post_board_action(request).await {
                    Ok(response) => status.set(LoadState::Ready(response)),
                    Err(error) => last_error.set(Some(error)),
                }
            });
        })
    };

    let auth_error_html = props.auth_error.as_deref().map(render_auth_error);
    let websocket_error = (*last_error).clone();
    let status_error = match &*status {
        LoadState::Ready(status) => status_chat_error(status),
        _ => None,
    };

    match *screen {
        AfkScreen::Menu => {
            let body = match &*status {
                LoadState::Idle if !runtime.afk_enabled => html! {
                    <>
                        {menu_section_gap()}
                        {menu_copy_row("AFK mode is unavailable in this build.")}
                    </>
                },
                LoadState::Loading => html! {
                    <>
                        {menu_section_gap()}
                        {menu_copy_row("Loading AFK status…")}
                    </>
                },
                LoadState::Error(error) => html! {
                    <>
                        {menu_section_gap()}
                        {menu_copy_row(error.clone())}
                    </>
                },
                LoadState::Ready(status) => {
                    if matches!(*menu_page, AfkMenuPage::BoardSize) {
                        let displayed_preferences =
                            displayed_afk_menu_preferences(status, *pre_auth_preferences);
                        html! {
                            <>
                                {menu_section_gap()}
                                {afk_menu_notice_block(
                                    auth_error_html.clone(),
                                    status_error
                                        .clone()
                                        .or_else(|| websocket_error.clone())
                                        .map(AttrValue::from),
                                )}
                                {menu_wide_detail_row(
                                    "Tiny",
                                    board_size_detail(AfkBoardSize::Tiny),
                                    menu_icon_button(
                                        "ok",
                                        "Use tiny AFK board size",
                                        displayed_preferences.board_size == AfkBoardSize::Tiny,
                                        false,
                                        set_board_size_tiny.clone(),
                                    ),
                                )}
                                {menu_wide_detail_row(
                                    "Small",
                                    board_size_detail(AfkBoardSize::Small),
                                    menu_icon_button(
                                        "ok",
                                        "Use small AFK board size",
                                        displayed_preferences.board_size == AfkBoardSize::Small,
                                        false,
                                        set_board_size_small.clone(),
                                    ),
                                )}
                                {menu_wide_detail_row(
                                    "Medium",
                                    board_size_detail(AfkBoardSize::Medium),
                                    menu_icon_button(
                                        "ok",
                                        "Use medium AFK board size",
                                        displayed_preferences.board_size == AfkBoardSize::Medium,
                                        false,
                                        set_board_size_medium.clone(),
                                    ),
                                )}
                                {menu_wide_detail_row(
                                    "Large",
                                    board_size_detail(AfkBoardSize::Large),
                                    menu_icon_button(
                                        "ok",
                                        "Use large AFK board size",
                                        displayed_preferences.board_size == AfkBoardSize::Large,
                                        false,
                                        set_board_size_large.clone(),
                                    ),
                                )}
                                {menu_section_gap()}
                            </>
                        }
                    } else if matches!(*menu_page, AfkMenuPage::ConfirmBoardSize)
                        && status.auth.identity.is_some()
                    {
                        let pending_label = (*pending_board_size)
                            .map(board_size_label)
                            .unwrap_or("New size");
                        html! {
                            <>
                                {menu_section_gap()}
                                {menu_copy_row("Changing board size starts a new AFK round immediately.")}
                                {menu_copy_row(format!("New board size: {pending_label}"))}
                                {menu_section_gap()}
                                {menu_primary_row(
                                    "Start New Round",
                                    menu_icon_button(
                                        "ok",
                                        "Apply board size and start a new AFK round",
                                        false,
                                        false,
                                        confirm_board_size_restart.clone(),
                                    ),
                                )}
                                {menu_primary_row(
                                    "Keep Current Round",
                                    menu_icon_button(
                                        "cancel",
                                        "Discard the pending board size change",
                                        false,
                                        false,
                                        cancel_board_size_restart.clone(),
                                    ),
                                )}
                                {menu_section_gap()}
                            </>
                        }
                    } else {
                        let displayed_preferences =
                            displayed_afk_menu_preferences(status, *pre_auth_preferences);
                        let primary_action = afk_root_primary_action(status);
                        let timeout_controls_disabled =
                            status.auth.identity.is_some() && !status.timeout_supported;
                        html! {
                            <>
                                {menu_section_gap()}
                                {afk_menu_notice_block(
                                    auth_error_html.clone(),
                                    status_error
                                        .clone()
                                        .or_else(|| websocket_error.clone())
                                        .map(AttrValue::from),
                                )}
                                {afk_root_menu_rows(
                                    primary_action,
                                    displayed_preferences,
                                    timeout_controls_disabled,
                                    &resume_board,
                                    &start_new_board,
                                    &connect_twitch_and_start,
                                    &open_board_size_menu,
                                    &set_timeout_on,
                                    &set_timeout_off,
                                    &decrease_timeout_duration,
                                    &increase_timeout_duration,
                                    &open_settings,
                                )}
                                {
                                    if status.auth.identity.is_some() {
                                        html! {
                                            <>
                                                {menu_section_gap()}
                                                {menu_primary_row(
                                                    "Disconnect Twitch",
                                                    menu_icon_button(
                                                        "cancel",
                                                        "Disconnect Twitch",
                                                        false,
                                                        false,
                                                        disconnect_twitch,
                                                    ),
                                                )}
                                            </>
                                        }
                                    } else {
                                        Html::default()
                                    }
                                }
                            </>
                        }
                    }
                }
                LoadState::Idle => html! {
                    <>
                        {menu_section_gap()}
                        {menu_copy_row("AFK mode is idle.")}
                    </>
                },
            };

            html! {
                <div class="detonito settings-open afk-menu-shell">
                    <SpriteDefs/>
                    <dialog open=true>
                        <table class="menu-grid">
                            <tbody>
                                {menu_section_gap()}
                                {
                                    if matches!(*menu_page, AfkMenuPage::BoardSize)
                                        && matches!(&*status, LoadState::Ready(_))
                                    {
                                        menu_header_row("Board Size", close_board_size_menu)
                                    } else if matches!(*menu_page, AfkMenuPage::ConfirmBoardSize)
                                        && matches!(&*status, LoadState::Ready(status) if status.auth.identity.is_some())
                                    {
                                        menu_header_row("Start New Round", cancel_board_size_restart)
                                    } else {
                                        menu_header_row("AFK Mode", go_to_main_menu)
                                    }
                                }
                                {body}
                                {menu_section_gap()}
                            </tbody>
                        </table>
                    </dialog>
                </div>
            }
        }
        AfkScreen::Board => {
            let session = match &*status {
                LoadState::Ready(status) => status.session.as_ref(),
                _ => None,
            };
            let mines_left = mines_counter_text(session);
            let timer = board_counter_text(session);
            let displayed_idle_state = if face_button_locked { None } else { idle_state };
            let game_state_icon = afk_face_icon(
                &status,
                if face_button_locked {
                    None
                } else {
                    (*face_notification).as_ref()
                },
                displayed_idle_state,
            );
            let face_button_title = if face_button_locked {
                "AFK prompt open"
            } else {
                "Open AFK submenu"
            };
            let timer_class = classes!("countdown-timer", afk_timer_phase_class(session));
            let board_interactive = session.is_some_and(|session| {
                matches!(
                    session.phase,
                    AfkRoundPhase::Countdown | AfkRoundPhase::Active
                ) && !session.paused
                    && !face_button_locked
            });

            html! {
                <div
                    class="detonito afk-board-mode"
                    oncontextmenu={Callback::from(move |e: MouseEvent| e.prevent_default())}
                >
                    <SpriteDefs/>
                    <nav>
                        <aside>
                            <GlyphRun set={GlyphSet::Counter} text={mines_left} class={classes!("counter-glyphs")}/>
                        </aside>
                        <span class={classes!("face-slot", face_button_locked.then_some("prompt-open"))}>
                            {
                                if let Some(overlay) = current_overlay.as_ref() {
                                    view_face_overlay(overlay, on_face_action)
                                } else {
                                    Html::default()
                                }
                            }
                            <button
                                class={classes!("face-button", game_state_icon, face_button_locked.then_some("locked"))}
                                title={face_button_title}
                                onclick={on_face_button}
                                disabled={face_button_locked}
                            >
                                <Icon
                                    name={game_state_icon}
                                    crop={IconCrop::CenteredSquare64}
                                    class={classes!("state-icon")}
                                />
                            </button>
                        </span>
                        <aside class={timer_class}>
                            <GlyphRun set={GlyphSet::Counter} text={timer} class={classes!("counter-glyphs")}/>
                        </aside>
                    </nav>
                    <div class="board-shell">
                        {
                            if let Some(session) = session {
                                render_afk_board(session, board_interactive, on_streamer_action)
                            } else {
                                Html::default()
                            }
                        }
                    </div>
                    {
                        if let Some(error) = status_error.or(websocket_error) {
                            html! { <div class="afk-board-note error">{error}</div> }
                        } else {
                            Html::default()
                        }
                    }
                </div>
            }
        }
    }
}

async fn fetch_status() -> Result<AfkStatusResponse, String> {
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_credentials(RequestCredentials::Include);
    let request =
        Request::new_with_str_and_init(&app_path("/api/afk/status"), &init).map_err(js_error)?;
    let response = fetch_request(request).await?;
    if !response.ok() {
        return Err(format!("status request failed with {}", response.status()));
    }
    read_json(response).await
}

async fn post_action(path: &str) -> Result<(), String> {
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_credentials(RequestCredentials::Include);
    let request = Request::new_with_str_and_init(&app_path(path), &init).map_err(js_error)?;
    let response = fetch_request(request).await?;
    if response.ok() {
        Ok(())
    } else {
        Err(format!("request failed with {}", response.status()))
    }
}

async fn post_empty_status(path: &str) -> Result<AfkStatusResponse, String> {
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_credentials(RequestCredentials::Include);
    let request = Request::new_with_str_and_init(&app_path(path), &init).map_err(js_error)?;
    let response = fetch_request(request).await?;
    if !response.ok() {
        return Err(format!("request failed with {}", response.status()));
    }
    read_json(response).await
}

async fn post_board_action(request_body: AfkActionRequest) -> Result<AfkStatusResponse, String> {
    post_json_status("/api/afk/action", &request_body).await
}

async fn apply_preferences_and_start(
    preferences: AfkMenuPreferences,
) -> Result<AfkStatusResponse, String> {
    post_json_status(
        "/api/afk/timeout",
        &serde_json::json!({
            "enabled": preferences.timeout_enabled,
            "duration_secs": preferences.timeout_duration_secs,
        }),
    )
    .await?;
    post_json_status(
        "/api/afk/board-size",
        &serde_json::json!({ "board_size": preferences.board_size }),
    )
    .await?;
    post_empty_status("/api/afk/start").await
}

async fn post_json_status<T>(path: &str, request_body: &T) -> Result<AfkStatusResponse, String>
where
    T: serde::Serialize,
{
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_credentials(RequestCredentials::Include);
    init.set_body(&JsValue::from_str(
        &serde_json::to_string(&request_body).map_err(|error| error.to_string())?,
    ));
    let request = Request::new_with_str_and_init(&app_path(path), &init).map_err(js_error)?;
    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(js_error)?;
    let response = fetch_request(request).await?;
    if !response.ok() {
        return Err(format!("request failed with {}", response.status()));
    }
    read_json(response).await
}

async fn fetch_request(request: Request) -> Result<Response, String> {
    let window = gloo::utils::window();
    let response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(js_error)?;
    response.dyn_into().map_err(js_error)
}

async fn read_json<T>(response: Response) -> Result<T, String>
where
    T: for<'a> serde::Deserialize<'a>,
{
    let json = JsFuture::from(response.json().map_err(js_error)?)
        .await
        .map_err(js_error)?;
    let serialized = js_sys::JSON::stringify(&json)
        .map_err(js_error)?
        .as_string()
        .ok_or_else(|| "response JSON could not be stringified".to_string())?;
    serde_json::from_str(&serialized).map_err(|error| error.to_string())
}

fn js_error(error: impl core::fmt::Debug) -> String {
    format!("{error:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use detonito_protocol::{
        AfkActivityKind, AfkBoardSize, AfkBoardSnapshot, AfkChatConnectionState, AfkIdentity,
        AfkLossReason, AfkTimerProfileSnapshot, FrontendRuntimeConfig, StreamerAuthStatus,
    };

    fn active_test_session(last_user_activity_at_ms: i64) -> AfkSessionSnapshot {
        AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::Active,
            paused: false,
            board: AfkBoardSnapshot {
                width: 24,
                height: 18,
                cells: Vec::new(),
            },
            labeled_cells: Vec::new(),
            timer_profile: AfkTimerProfileSnapshot {
                start_secs: 120,
                safe_reveal_bonus_secs: 1,
                mine_penalty_secs: 15,
                start_delay_secs: 5,
                win_continue_delay_secs: 30,
                loss_continue_delay_secs: 60,
            },
            timer_remaining_secs: 120,
            phase_countdown_secs: None,
            current_level: 3,
            live_mines_left: 50,
            crater_count: 0,
            loss_reason: None,
            timeout_enabled: true,
            ignored_users: Vec::new(),
            recent_penalties: Vec::new(),
            activity: Vec::new(),
            last_action: None,
            last_user_activity_at_ms,
        }
    }

    fn ready_status(session: AfkSessionSnapshot) -> LoadState<AfkStatusResponse> {
        LoadState::Ready(AfkStatusResponse {
            runtime: FrontendRuntimeConfig { afk_enabled: true },
            auth: StreamerAuthStatus::default(),
            chat_connection: AfkChatConnectionState::Idle,
            chat_error: None,
            timeout_supported: true,
            timeout_enabled: true,
            timeout_duration_secs: 30,
            board_size: AfkBoardSize::Medium,
            connect_url: None,
            websocket_path: None,
            session: Some(session),
        })
    }

    fn base_status(session: Option<AfkSessionSnapshot>) -> AfkStatusResponse {
        AfkStatusResponse {
            runtime: FrontendRuntimeConfig { afk_enabled: true },
            auth: StreamerAuthStatus::default(),
            chat_connection: AfkChatConnectionState::Idle,
            chat_error: None,
            timeout_supported: true,
            timeout_enabled: true,
            timeout_duration_secs: 30,
            board_size: AfkBoardSize::Medium,
            connect_url: None,
            websocket_path: None,
            session,
        }
    }

    #[test]
    fn board_size_change_requires_confirmation_when_run_is_active() {
        let status = base_status(Some(active_test_session(1_000)));

        assert_eq!(
            plan_board_size_change(&status, AfkBoardSize::Large),
            AfkBoardSizeChangePlan::ConfirmRestart(AfkBoardSize::Large)
        );
    }

    #[test]
    fn board_size_change_applies_immediately_when_no_run_exists() {
        let status = base_status(None);

        assert_eq!(
            plan_board_size_change(&status, AfkBoardSize::Large),
            AfkBoardSizeChangePlan::ApplyOnly(AfkBoardSize::Large)
        );
    }

    #[test]
    fn root_primary_action_depends_on_connection_and_session_state() {
        let disconnected = base_status(None);
        assert_eq!(
            afk_root_primary_action(&disconnected),
            AfkRootPrimaryAction::ConnectAndStart
        );

        let connected_without_session = AfkStatusResponse {
            auth: StreamerAuthStatus {
                identity: Some(AfkIdentity::new("1", "streamer", "Streamer")),
                ..StreamerAuthStatus::default()
            },
            ..base_status(None)
        };
        assert_eq!(
            afk_root_primary_action(&connected_without_session),
            AfkRootPrimaryAction::Start
        );

        let connected_with_session = AfkStatusResponse {
            auth: StreamerAuthStatus {
                identity: Some(AfkIdentity::new("1", "streamer", "Streamer")),
                ..StreamerAuthStatus::default()
            },
            ..base_status(Some(active_test_session(1_000)))
        };
        assert_eq!(
            afk_root_primary_action(&connected_with_session),
            AfkRootPrimaryAction::ResumeAndStartNew
        );
    }

    #[test]
    fn disconnected_preferences_use_pending_draft_values() {
        let status = base_status(None);

        assert_eq!(
            displayed_afk_menu_preferences(
                &status,
                Some(AfkMenuPreferences {
                    board_size: AfkBoardSize::Large,
                    timeout_enabled: false,
                    timeout_duration_secs: 90,
                }),
            ),
            AfkMenuPreferences {
                board_size: AfkBoardSize::Large,
                timeout_enabled: false,
                timeout_duration_secs: 90,
            }
        );
    }

    #[test]
    fn afk_return_to_path_can_request_start_after_connect() {
        assert_eq!(afk_return_to_path(false), "/?view=afk");
        assert_eq!(afk_return_to_path(true), "/?view=afk&afk_start=1");
    }

    #[test]
    fn format_cell_code_uppercases_letters_except_o() {
        assert_eq!(format_cell_code((0, 0)), "1A");
        assert_eq!(format_cell_code((14, 9)), "Ao");
    }

    #[test]
    fn should_show_cell_code_uses_snapshot_label_mask_when_present() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::Active,
            paused: false,
            board: AfkBoardSnapshot {
                width: 3,
                height: 1,
                cells: vec![
                    AfkCellSnapshot::Hidden,
                    AfkCellSnapshot::Hidden,
                    AfkCellSnapshot::Hidden,
                ],
            },
            labeled_cells: vec![false, false, true],
            timer_profile: AfkTimerProfileSnapshot {
                start_secs: 120,
                safe_reveal_bonus_secs: 1,
                mine_penalty_secs: 15,
                start_delay_secs: 5,
                win_continue_delay_secs: 30,
                loss_continue_delay_secs: 60,
            },
            timer_remaining_secs: 120,
            phase_countdown_secs: None,
            current_level: 1,
            live_mines_left: 1,
            crater_count: 0,
            loss_reason: None,
            timeout_enabled: true,
            ignored_users: Vec::new(),
            recent_penalties: Vec::new(),
            activity: Vec::new(),
            last_action: None,
            last_user_activity_at_ms: 1,
        };

        assert!(should_show_cell_code(
            &session,
            2,
            0,
            AfkCellSnapshot::Hidden
        ));
        assert!(!should_show_cell_code(
            &session,
            1,
            0,
            AfkCellSnapshot::Hidden
        ));

        let flagged_session = AfkSessionSnapshot {
            board: AfkBoardSnapshot {
                width: 1,
                height: 1,
                cells: vec![AfkCellSnapshot::Flagged],
            },
            labeled_cells: vec![false],
            ..session
        };
        assert!(!should_show_cell_code(
            &flagged_session,
            0,
            0,
            AfkCellSnapshot::Flagged
        ));
    }

    #[test]
    fn should_show_cell_code_falls_back_to_legacy_frontier_rule_when_mask_is_missing() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::Active,
            paused: false,
            board: AfkBoardSnapshot {
                width: 3,
                height: 1,
                cells: vec![
                    AfkCellSnapshot::Hidden,
                    AfkCellSnapshot::Revealed(1),
                    AfkCellSnapshot::Hidden,
                ],
            },
            labeled_cells: Vec::new(),
            timer_profile: AfkTimerProfileSnapshot {
                start_secs: 120,
                safe_reveal_bonus_secs: 1,
                mine_penalty_secs: 15,
                start_delay_secs: 5,
                win_continue_delay_secs: 30,
                loss_continue_delay_secs: 60,
            },
            timer_remaining_secs: 120,
            phase_countdown_secs: None,
            current_level: 1,
            live_mines_left: 1,
            crater_count: 0,
            loss_reason: None,
            timeout_enabled: true,
            ignored_users: Vec::new(),
            recent_penalties: Vec::new(),
            activity: Vec::new(),
            last_action: None,
            last_user_activity_at_ms: 1,
        };

        assert!(should_show_cell_code(
            &session,
            0,
            0,
            AfkCellSnapshot::Hidden
        ));
        assert!(should_show_cell_code(
            &session,
            2,
            0,
            AfkCellSnapshot::Hidden
        ));
    }

    #[test]
    fn face_icon_uses_sleeping_face_for_timer_losses() {
        let status = LoadState::Ready(AfkStatusResponse {
            runtime: FrontendRuntimeConfig { afk_enabled: true },
            auth: StreamerAuthStatus::default(),
            chat_connection: AfkChatConnectionState::Idle,
            chat_error: None,
            timeout_supported: true,
            timeout_enabled: true,
            timeout_duration_secs: 30,
            board_size: AfkBoardSize::Medium,
            connect_url: None,
            websocket_path: None,
            session: Some(AfkSessionSnapshot {
                streamer: None,
                phase: AfkRoundPhase::TimedOut,
                paused: false,
                board: AfkBoardSnapshot {
                    width: 0,
                    height: 0,
                    cells: Vec::new(),
                },
                labeled_cells: Vec::new(),
                timer_profile: AfkTimerProfileSnapshot {
                    start_secs: 120,
                    safe_reveal_bonus_secs: 1,
                    mine_penalty_secs: 15,
                    start_delay_secs: 5,
                    win_continue_delay_secs: 30,
                    loss_continue_delay_secs: 60,
                },
                timer_remaining_secs: 0,
                phase_countdown_secs: Some(60),
                current_level: 1,
                live_mines_left: 0,
                crater_count: 0,
                loss_reason: Some(AfkLossReason::Timer),
                timeout_enabled: true,
                ignored_users: Vec::new(),
                recent_penalties: Vec::new(),
                activity: Vec::new(),
                last_action: None,
                last_user_activity_at_ms: 1,
            }),
        });

        assert_eq!(afk_face_icon(&status, None, None), "sleeping");
    }

    #[test]
    fn timer_loss_prompt_uses_too_slow_message() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::TimedOut,
            paused: false,
            board: AfkBoardSnapshot {
                width: 0,
                height: 0,
                cells: Vec::new(),
            },
            labeled_cells: Vec::new(),
            timer_profile: AfkTimerProfileSnapshot {
                start_secs: 120,
                safe_reveal_bonus_secs: 1,
                mine_penalty_secs: 15,
                start_delay_secs: 5,
                win_continue_delay_secs: 30,
                loss_continue_delay_secs: 60,
            },
            timer_remaining_secs: 0,
            phase_countdown_secs: Some(60),
            current_level: 1,
            live_mines_left: 0,
            crater_count: 0,
            loss_reason: Some(AfkLossReason::Timer),
            timeout_enabled: true,
            ignored_users: Vec::new(),
            recent_penalties: Vec::new(),
            activity: Vec::new(),
            last_action: None,
            last_user_activity_at_ms: 1,
        };

        assert_eq!(loss_prompt_message(&session), "Too slow! Play again? (60)");
    }

    #[test]
    fn mine_loss_prompt_keeps_too_bad_message() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::TimedOut,
            paused: false,
            board: AfkBoardSnapshot {
                width: 0,
                height: 0,
                cells: Vec::new(),
            },
            labeled_cells: Vec::new(),
            timer_profile: AfkTimerProfileSnapshot {
                start_secs: 120,
                safe_reveal_bonus_secs: 1,
                mine_penalty_secs: 15,
                start_delay_secs: 5,
                win_continue_delay_secs: 30,
                loss_continue_delay_secs: 60,
            },
            timer_remaining_secs: 0,
            phase_countdown_secs: Some(60),
            current_level: 1,
            live_mines_left: 0,
            crater_count: 0,
            loss_reason: Some(AfkLossReason::Mine),
            timeout_enabled: true,
            ignored_users: Vec::new(),
            recent_penalties: Vec::new(),
            activity: Vec::new(),
            last_action: None,
            last_user_activity_at_ms: 1,
        };

        assert_eq!(loss_prompt_message(&session), "Too bad. Play again? (60)");
    }

    #[test]
    fn face_icon_uses_dejected_face_for_notifications() {
        assert_eq!(
            afk_face_icon(
                &LoadState::<AfkStatusResponse>::Idle,
                Some(&AfkFaceNotification {
                    id: 1,
                    message: "Jan found a mine! o7".into(),
                }),
                None,
            ),
            "dejected"
        );
    }

    #[test]
    fn idle_state_uses_sleeping_face_after_three_minutes() {
        let session = active_test_session(1_000);
        let idle_state = afk_idle_state(Some(&session), 1_000 + AFK_IDLE_SLEEPING_THRESHOLD_MS);

        assert_eq!(idle_state, Some(AfkIdleState::Sleeping));
        assert_eq!(
            afk_face_icon(&ready_status(session), None, idle_state),
            "sleeping"
        );
    }

    #[test]
    fn idle_overlay_shows_is_anyone_there_after_ten_minutes() {
        let session = active_test_session(1_000);
        let overlay = active_face_overlay(
            AfkScreen::Board,
            &ready_status(session),
            &None,
            &None,
            Some(AfkIdleState::Prompt),
        );

        assert_eq!(
            overlay,
            Some(AfkFaceOverlay::Message {
                message: "Is anyone there?".into(),
                status: Some("Level 3".into()),
            })
        );
    }

    #[test]
    fn idle_overlay_stays_hidden_for_recent_activity() {
        let session = active_test_session(1_000);
        assert_eq!(
            afk_idle_state(Some(&session), 1_000 + AFK_IDLE_SLEEPING_THRESHOLD_MS - 1),
            None
        );
    }

    #[test]
    fn notifications_override_idle_prompt() {
        let session = active_test_session(1_000);
        let overlay = active_face_overlay(
            AfkScreen::Board,
            &ready_status(session),
            &None,
            &Some(AfkFaceNotification {
                id: 1,
                message: "Jan found a mine! o7".into(),
            }),
            Some(AfkIdleState::Prompt),
        );

        assert_eq!(
            overlay,
            Some(AfkFaceOverlay::Message {
                message: "Jan found a mine! o7".into(),
                status: Some("Level 3".into()),
            })
        );
    }

    #[test]
    fn automatic_prompts_override_idle_prompt() {
        let mut session = active_test_session(1_000);
        session.phase = AfkRoundPhase::TimedOut;
        session.phase_countdown_secs = Some(60);
        session.loss_reason = Some(AfkLossReason::Mine);
        session.current_level = 1;
        session.live_mines_left = 0;

        let overlay = active_face_overlay(
            AfkScreen::Board,
            &ready_status(session),
            &None,
            &None,
            Some(AfkIdleState::Prompt),
        );

        assert_eq!(
            overlay,
            Some(AfkFaceOverlay::Prompt(AfkFacePrompt {
                message: "Too bad. Play again? (60)".into(),
                choices: vec![
                    AfkFaceChoice {
                        label: "Yes (!continue)".into(),
                        title: "Start the next round now".into(),
                        action: AfkFaceAction::ContinueRound,
                    },
                    AfkFaceChoice {
                        label: "No".into(),
                        title: "Stop AFK mode".into(),
                        action: AfkFaceAction::StopRun,
                    },
                ],
            }))
        );
    }

    #[test]
    fn mine_hit_activity_formats_face_notification() {
        let row = AfkActivityRow {
            at_ms: 1_234,
            text: "Jan hit a mine at 1A".into(),
            kind: AfkActivityKind::MineHit,
            actor: Some(AfkIdentity::new("1", "jan", "Jan")),
        };

        assert_eq!(
            face_notification_event(&row),
            Some(AfkFaceNotificationEvent {
                message: "Jan found a mine! o7".into(),
                timeout_ms: AFK_FACE_NOTIFICATION_MS,
            })
        );
    }

    #[test]
    fn out_for_round_activity_formats_face_notification() {
        let row = AfkActivityRow {
            at_ms: 1_234,
            text: "Jan is out for the rest of the round.".into(),
            kind: AfkActivityKind::OutForRound,
            actor: Some(AfkIdentity::new("1", "jan", "Jan")),
        };

        assert_eq!(
            face_notification_event(&row),
            Some(AfkFaceNotificationEvent {
                message: "Jan is out for the rest of the round.".into(),
                timeout_ms: AFK_OUT_FOR_ROUND_NOTIFICATION_MS,
            })
        );
    }

    #[test]
    fn win_face_icon_uses_close_call_variant_for_low_timer_wins() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::Won,
            paused: false,
            board: AfkBoardSnapshot {
                width: 0,
                height: 0,
                cells: Vec::new(),
            },
            labeled_cells: Vec::new(),
            timer_profile: AfkTimerProfileSnapshot {
                start_secs: 120,
                safe_reveal_bonus_secs: 1,
                mine_penalty_secs: 15,
                start_delay_secs: 5,
                win_continue_delay_secs: 30,
                loss_continue_delay_secs: 60,
            },
            timer_remaining_secs: 9,
            phase_countdown_secs: Some(30),
            current_level: 2,
            live_mines_left: 0,
            crater_count: 1,
            loss_reason: None,
            timeout_enabled: true,
            ignored_users: Vec::new(),
            recent_penalties: Vec::new(),
            activity: Vec::new(),
            last_action: None,
            last_user_activity_at_ms: 1,
        };

        assert_eq!(win_face_icon(&session), "win-close-call");
        assert_eq!(win_prompt_message(&session), "Close call! Next level? (30)");
    }

    #[test]
    fn win_face_icon_uses_decent_variant_after_mine_hits() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::Won,
            paused: false,
            board: AfkBoardSnapshot {
                width: 0,
                height: 0,
                cells: Vec::new(),
            },
            labeled_cells: Vec::new(),
            timer_profile: AfkTimerProfileSnapshot {
                start_secs: 120,
                safe_reveal_bonus_secs: 1,
                mine_penalty_secs: 15,
                start_delay_secs: 5,
                win_continue_delay_secs: 30,
                loss_continue_delay_secs: 60,
            },
            timer_remaining_secs: 20,
            phase_countdown_secs: Some(30),
            current_level: 2,
            live_mines_left: 0,
            crater_count: 1,
            loss_reason: None,
            timeout_enabled: true,
            ignored_users: Vec::new(),
            recent_penalties: Vec::new(),
            activity: Vec::new(),
            last_action: None,
            last_user_activity_at_ms: 1,
        };

        assert_eq!(win_face_icon(&session), "win-decent");
        assert_eq!(win_prompt_message(&session), "Decent! Next level? (30)");
    }

    #[test]
    fn win_face_icon_keeps_nice_variant_for_clean_wins() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::Won,
            paused: false,
            board: AfkBoardSnapshot {
                width: 0,
                height: 0,
                cells: Vec::new(),
            },
            labeled_cells: Vec::new(),
            timer_profile: AfkTimerProfileSnapshot {
                start_secs: 120,
                safe_reveal_bonus_secs: 1,
                mine_penalty_secs: 15,
                start_delay_secs: 5,
                win_continue_delay_secs: 30,
                loss_continue_delay_secs: 60,
            },
            timer_remaining_secs: 20,
            phase_countdown_secs: Some(30),
            current_level: 2,
            live_mines_left: 0,
            crater_count: 0,
            loss_reason: None,
            timeout_enabled: true,
            ignored_users: Vec::new(),
            recent_penalties: Vec::new(),
            activity: Vec::new(),
            last_action: None,
            last_user_activity_at_ms: 1,
        };

        assert_eq!(win_face_icon(&session), "win");
        assert_eq!(win_prompt_message(&session), "NICE! Next level? (30)");
    }

    #[test]
    fn active_board_overlay_shows_current_level_status() {
        let overlay = active_face_overlay(
            AfkScreen::Board,
            &LoadState::Ready(AfkStatusResponse {
                runtime: FrontendRuntimeConfig { afk_enabled: true },
                auth: StreamerAuthStatus::default(),
                chat_connection: AfkChatConnectionState::Idle,
                chat_error: None,
                timeout_supported: true,
                timeout_enabled: true,
                timeout_duration_secs: 30,
                board_size: AfkBoardSize::Medium,
                connect_url: None,
                websocket_path: None,
                session: Some(AfkSessionSnapshot {
                    streamer: None,
                    phase: AfkRoundPhase::Active,
                    paused: false,
                    board: AfkBoardSnapshot {
                        width: 24,
                        height: 18,
                        cells: Vec::new(),
                    },
                    labeled_cells: Vec::new(),
                    timer_profile: AfkTimerProfileSnapshot {
                        start_secs: 120,
                        safe_reveal_bonus_secs: 1,
                        mine_penalty_secs: 15,
                        start_delay_secs: 5,
                        win_continue_delay_secs: 30,
                        loss_continue_delay_secs: 60,
                    },
                    timer_remaining_secs: 120,
                    phase_countdown_secs: None,
                    current_level: 3,
                    live_mines_left: 50,
                    crater_count: 0,
                    loss_reason: None,
                    timeout_enabled: true,
                    ignored_users: Vec::new(),
                    recent_penalties: Vec::new(),
                    activity: Vec::new(),
                    last_action: None,
                    last_user_activity_at_ms: 1,
                }),
            }),
            &None,
            &None,
            None,
        );

        assert_eq!(overlay, Some(AfkFaceOverlay::Status("Level 3".into())));
    }

    #[test]
    fn active_board_overlay_hides_current_level_status_on_narrow_boards() {
        let overlay = active_face_overlay(
            AfkScreen::Board,
            &LoadState::Ready(AfkStatusResponse {
                runtime: FrontendRuntimeConfig { afk_enabled: true },
                auth: StreamerAuthStatus::default(),
                chat_connection: AfkChatConnectionState::Idle,
                chat_error: None,
                timeout_supported: true,
                timeout_enabled: true,
                timeout_duration_secs: 30,
                board_size: AfkBoardSize::Tiny,
                connect_url: None,
                websocket_path: None,
                session: Some(AfkSessionSnapshot {
                    streamer: None,
                    phase: AfkRoundPhase::Active,
                    paused: false,
                    board: AfkBoardSnapshot {
                        width: 9,
                        height: 9,
                        cells: Vec::new(),
                    },
                    labeled_cells: Vec::new(),
                    timer_profile: AfkTimerProfileSnapshot {
                        start_secs: 120,
                        safe_reveal_bonus_secs: 1,
                        mine_penalty_secs: 15,
                        start_delay_secs: 5,
                        win_continue_delay_secs: 30,
                        loss_continue_delay_secs: 60,
                    },
                    timer_remaining_secs: 120,
                    phase_countdown_secs: None,
                    current_level: 3,
                    live_mines_left: 9,
                    crater_count: 0,
                    loss_reason: None,
                    timeout_enabled: true,
                    ignored_users: Vec::new(),
                    recent_penalties: Vec::new(),
                    activity: Vec::new(),
                    last_action: None,
                    last_user_activity_at_ms: 1,
                }),
            }),
            &None,
            &None,
            None,
        );

        assert_eq!(overlay, None);
    }

    #[test]
    fn active_board_overlay_shows_notification_until_a_prompt_replaces_it() {
        let overlay = active_face_overlay(
            AfkScreen::Board,
            &LoadState::Ready(AfkStatusResponse {
                runtime: FrontendRuntimeConfig { afk_enabled: true },
                auth: StreamerAuthStatus::default(),
                chat_connection: AfkChatConnectionState::Idle,
                chat_error: None,
                timeout_supported: true,
                timeout_enabled: true,
                timeout_duration_secs: 30,
                board_size: AfkBoardSize::Medium,
                connect_url: None,
                websocket_path: None,
                session: Some(AfkSessionSnapshot {
                    streamer: None,
                    phase: AfkRoundPhase::Active,
                    paused: false,
                    board: AfkBoardSnapshot {
                        width: 24,
                        height: 18,
                        cells: Vec::new(),
                    },
                    labeled_cells: Vec::new(),
                    timer_profile: AfkTimerProfileSnapshot {
                        start_secs: 120,
                        safe_reveal_bonus_secs: 1,
                        mine_penalty_secs: 15,
                        start_delay_secs: 5,
                        win_continue_delay_secs: 30,
                        loss_continue_delay_secs: 60,
                    },
                    timer_remaining_secs: 120,
                    phase_countdown_secs: None,
                    current_level: 3,
                    live_mines_left: 50,
                    crater_count: 0,
                    loss_reason: None,
                    timeout_enabled: true,
                    ignored_users: Vec::new(),
                    recent_penalties: Vec::new(),
                    activity: Vec::new(),
                    last_action: None,
                    last_user_activity_at_ms: 1,
                }),
            }),
            &None,
            &Some(AfkFaceNotification {
                id: 7,
                message: "Jan found a mine! o7".into(),
            }),
            None,
        );

        assert_eq!(
            overlay,
            Some(AfkFaceOverlay::Message {
                message: "Jan found a mine! o7".into(),
                status: Some("Level 3".into()),
            })
        );
    }

    #[test]
    fn active_board_prompt_replaces_notification() {
        let overlay = active_face_overlay(
            AfkScreen::Board,
            &LoadState::Ready(AfkStatusResponse {
                runtime: FrontendRuntimeConfig { afk_enabled: true },
                auth: StreamerAuthStatus::default(),
                chat_connection: AfkChatConnectionState::Idle,
                chat_error: None,
                timeout_supported: true,
                timeout_enabled: true,
                timeout_duration_secs: 30,
                board_size: AfkBoardSize::Medium,
                connect_url: None,
                websocket_path: None,
                session: Some(AfkSessionSnapshot {
                    streamer: None,
                    phase: AfkRoundPhase::TimedOut,
                    paused: false,
                    board: AfkBoardSnapshot {
                        width: 0,
                        height: 0,
                        cells: Vec::new(),
                    },
                    labeled_cells: Vec::new(),
                    timer_profile: AfkTimerProfileSnapshot {
                        start_secs: 120,
                        safe_reveal_bonus_secs: 1,
                        mine_penalty_secs: 15,
                        start_delay_secs: 5,
                        win_continue_delay_secs: 30,
                        loss_continue_delay_secs: 60,
                    },
                    timer_remaining_secs: 0,
                    phase_countdown_secs: Some(60),
                    current_level: 1,
                    live_mines_left: 0,
                    crater_count: 0,
                    loss_reason: Some(AfkLossReason::Mine),
                    timeout_enabled: true,
                    ignored_users: Vec::new(),
                    recent_penalties: Vec::new(),
                    activity: Vec::new(),
                    last_action: None,
                    last_user_activity_at_ms: 1,
                }),
            }),
            &None,
            &Some(AfkFaceNotification {
                id: 7,
                message: "Jan found a mine! o7".into(),
            }),
            None,
        );

        assert_eq!(
            overlay,
            Some(AfkFaceOverlay::Prompt(AfkFacePrompt {
                message: "Too bad. Play again? (60)".into(),
                choices: vec![
                    AfkFaceChoice {
                        label: "Yes (!continue)".into(),
                        title: "Start the next round now".into(),
                        action: AfkFaceAction::ContinueRound,
                    },
                    AfkFaceChoice {
                        label: "No".into(),
                        title: "Stop AFK mode".into(),
                        action: AfkFaceAction::StopRun,
                    },
                ],
            }))
        );
    }
}
