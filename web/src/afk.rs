use std::{cell::Cell, rc::Rc};

use crate::board_input::{
    CellMsg as BoardCellMsg, CellPointerCallbacks, CellPointerState as BoardCellPointerState,
    MouseButtons, cell_pointer_callbacks, update_cell_pointer_state,
};
use crate::hazard_variant::HazardVariant;
use detonito_protocol::{
    AfkActionKind, AfkActionRequest, AfkActivityKind, AfkActivityRow, AfkBoardSize,
    AfkCellSnapshot, AfkChatConnectionState, AfkClientMessage, AfkCoordSnapshot, AfkLossReason,
    AfkRoundPhase, AfkRoundReportSnapshot, AfkServerMessage, AfkSessionSnapshot,
    AfkStatsGroupSnapshot, AfkStatusResponse, AfkUserStatsSnapshot,
};
use gloo::{render::request_animation_frame, timers::callback::Timeout};
use js_sys::{Reflect, encode_uri_component};
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Element, Event, MessageEvent, Request, RequestCredentials, RequestInit, Response, WebSocket,
};
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
    #[prop_or_default]
    pub restore_view_state: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum LoadState<T> {
    Idle,
    Loading,
    Ready(T),
    Error(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum AfkScreen {
    #[default]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AfkCountdownDemoCell {
    state: AfkCellSnapshot,
    code: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AfkCountdownDemoRow {
    before: AfkCountdownDemoCell,
    after: AfkCountdownDemoCell,
    command: &'static str,
    description: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AfkCountdownOverlay {
    title: &'static str,
    rows: [AfkCountdownDemoRow; 3],
    aria_label: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfkRoundReportLayout {
    SideBySide,
    Stacked,
    TotalOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfkRoundReportScope {
    Round,
    Total,
}

const AFK_FACE_NOTIFICATION_MS: u32 = 5_000;
const AFK_OUT_FOR_ROUND_NOTIFICATION_MS: u32 = 10_000;
const AFK_IDLE_SLEEPING_THRESHOLD_MS: i64 = 3 * 60 * 1_000;
const AFK_IDLE_PROMPT_THRESHOLD_MS: i64 = 10 * 60 * 1_000;
const AFK_IDLE_EXPIRY_THRESHOLD_MS: i64 = 60 * 60 * 1_000;
const AFK_CONNECTION_NOTICE_GRACE_MS: u32 = 2_000;
const AFK_WEBSOCKET_RECONNECT_BASE_DELAY_MS: u32 = 1_000;
const AFK_WEBSOCKET_RECONNECT_MAX_DELAY_MS: u32 = 60_000;
const AFK_WEBSOCKET_RECONNECT_TICK_MS: u32 = 1_000;
const AFK_TIMEOUT_DURATION_OPTIONS_SECS: [u32; 12] =
    [1, 5, 10, 15, 30, 45, 60, 90, 120, 180, 240, 300];
const AFK_DEFAULT_TIMEOUT_DURATION_INDEX: usize = 4;
const AFK_COUNTDOWN_DEMO_BEFORE_MS: u32 = 600;
const AFK_COUNTDOWN_DEMO_AFTER_MS: u32 = AFK_COUNTDOWN_DEMO_BEFORE_MS * 2;
const AFK_COUNTDOWN_OVERLAY: AfkCountdownOverlay = AfkCountdownOverlay {
    title: "How to play",
    rows: [
        AfkCountdownDemoRow {
            before: AfkCountdownDemoCell {
                state: AfkCellSnapshot::Hidden,
                code: Some("1A"),
            },
            after: AfkCountdownDemoCell {
                state: AfkCellSnapshot::Revealed(0),
                code: None,
            },
            command: "1a",
            description: "open",
        },
        AfkCountdownDemoRow {
            before: AfkCountdownDemoCell {
                state: AfkCellSnapshot::Hidden,
                code: Some("5C"),
            },
            after: AfkCountdownDemoCell {
                state: AfkCellSnapshot::Flagged,
                code: None,
            },
            command: "!f 5c",
            description: "flag",
        },
        AfkCountdownDemoRow {
            before: AfkCountdownDemoCell {
                state: AfkCellSnapshot::Flagged,
                code: Some("2E"),
            },
            after: AfkCountdownDemoCell {
                state: AfkCellSnapshot::Hidden,
                code: None,
            },
            command: "!u 2e",
            description: "unflag",
        },
    ],
    aria_label: "Chat commands: 1a opens, !f 5c flags, !u 2e unflags. Letter case does not matter.",
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfkIdleState {
    Sleeping,
    Prompt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfkBoardScreenResolution {
    Keep,
    FinishStartTransition,
    ReturnToMenu,
}

type AfkCellPointerState = BoardCellPointerState<(usize, usize)>;
type AfkCellMsg = BoardCellMsg<(usize, usize)>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AfkHeldCellPreview {
    pointer_state: AfkCellPointerState,
    session_before_action: AfkSessionSnapshot,
}

fn initial_afk_screen(restore_view_state: bool) -> AfkScreen {
    if restore_view_state {
        AfkScreen::local_or_default()
    } else {
        AfkScreen::Menu
    }
}

fn resolve_board_screen(
    status: &LoadState<AfkStatusResponse>,
    start_transition_pending: bool,
) -> AfkBoardScreenResolution {
    match status {
        LoadState::Ready(status) if status.session.is_some() && start_transition_pending => {
            AfkBoardScreenResolution::FinishStartTransition
        }
        LoadState::Ready(status) if status.session.is_none() && !start_transition_pending => {
            AfkBoardScreenResolution::ReturnToMenu
        }
        LoadState::Error(_) if start_transition_pending => AfkBoardScreenResolution::ReturnToMenu,
        _ => AfkBoardScreenResolution::Keep,
    }
}

fn current_level_status_text(session: Option<&AfkSessionSnapshot>) -> Option<AttrValue> {
    session
        .filter(|session| session.board.width >= 15)
        .map(|session| format!("Level {}", session.current_level).into())
}

fn visible_lives_count(session: Option<&AfkSessionSnapshot>) -> Option<u8> {
    session
        .filter(|session| session.board.width >= 15 && session.lives_remaining > 0)
        .map(|session| session.lives_remaining)
}

fn round_report_layout(session: &AfkSessionSnapshot) -> AfkRoundReportLayout {
    match session.board.width {
        0..=9 => AfkRoundReportLayout::TotalOnly,
        10..=14 => AfkRoundReportLayout::Stacked,
        _ => AfkRoundReportLayout::SideBySide,
    }
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
        AfkBoardSize::Tiny => "9x9 | 9+1/Lv",
        AfkBoardSize::Small => "16x16 | 20+4/Lv",
        AfkBoardSize::Medium => "24x18 | 36+7/Lv",
        AfkBoardSize::Large => "30x20 | 50+10/Lv",
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

impl StorageKey for AfkScreen {
    const KEY: &'static str = "detonito:afk:screen";
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

fn next_afk_reconnect_delay_ms(attempt: u32) -> u32 {
    let shift = attempt.saturating_sub(1).min(6);
    AFK_WEBSOCKET_RECONNECT_BASE_DELAY_MS
        .saturating_mul(1_u32 << shift)
        .min(AFK_WEBSOCKET_RECONNECT_MAX_DELAY_MS)
}

fn connection_notice_visible(started_at_ms: Option<i64>, now_ms: i64) -> bool {
    started_at_ms.is_some_and(|started_at_ms| {
        now_ms.saturating_sub(started_at_ms) >= i64::from(AFK_CONNECTION_NOTICE_GRACE_MS)
    })
}

fn next_connection_notice_refresh_delay_ms(started_at_ms: Option<i64>, now_ms: i64) -> Option<u32> {
    let started_at_ms = started_at_ms?;
    let elapsed_ms = now_ms.saturating_sub(started_at_ms);
    let remaining_ms = i64::from(AFK_CONNECTION_NOTICE_GRACE_MS).saturating_sub(elapsed_ms);
    (remaining_ms > 0).then(|| remaining_ms.min(i64::from(u32::MAX)) as u32)
}

fn afk_websocket_reconnect_notice(remaining_ms: i64) -> String {
    let remaining_secs = (remaining_ms.max(1).saturating_add(999)) / 1_000;
    format!("Reconnecting in {remaining_secs}...")
}

fn afk_websocket_connecting_notice() -> String {
    "Reconnecting...".to_string()
}

fn status_chat_notice(
    status: &AfkStatusResponse,
    reconnecting_notice_visible: bool,
) -> Option<String> {
    if status.session.is_none() {
        return None;
    }
    if reconnecting_notice_visible {
        Some("Chat reconnecting...".to_string())
    } else if matches!(status.chat_connection, AfkChatConnectionState::Error) {
        Some("Chat unavailable.".to_string())
    } else {
        None
    }
}

fn afk_board_is_interactive(
    status: &LoadState<AfkStatusResponse>,
    socket_connected: bool,
    face_button_locked: bool,
) -> bool {
    let LoadState::Ready(status) = status else {
        return false;
    };

    if status.auth.identity.is_none()
        || !socket_connected
        || !matches!(status.chat_connection, AfkChatConnectionState::Connected)
    {
        return false;
    }

    status.session.as_ref().is_some_and(|session| {
        matches!(
            session.phase,
            AfkRoundPhase::Countdown | AfkRoundPhase::Active
        ) && !session.paused
            && !face_button_locked
    })
}

fn handle_started_status(
    status: &UseStateHandle<LoadState<AfkStatusResponse>>,
    screen: &UseStateHandle<AfkScreen>,
    last_error: &UseStateHandle<Option<String>>,
    start_transition_in_progress: &UseStateHandle<bool>,
    response: AfkStatusResponse,
) {
    let has_session = response.session.is_some();
    start_transition_in_progress.set(false);
    status.set(LoadState::Ready(response));
    if has_session {
        last_error.set(None);
        screen.set(AfkScreen::Board);
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
    let prompt = if session.game_over {
        "Game over. Start over?"
    } else if session.loss_reason == Some(AfkLossReason::Timer) {
        "Too slow! Retry level?"
    } else {
        "Too bad. Retry level?"
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
                title: if session.game_over {
                    "Start a new run from level 1".into()
                } else {
                    "Retry the current level".into()
                },
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

fn active_round_report(session: Option<&AfkSessionSnapshot>) -> Option<&AfkRoundReportSnapshot> {
    session.and_then(|session| match session.phase {
        AfkRoundPhase::Won | AfkRoundPhase::TimedOut => session.round_report.as_ref(),
        AfkRoundPhase::Countdown | AfkRoundPhase::Active | AfkRoundPhase::Stopped => None,
    })
}

fn round_report_max_rows(session: &AfkSessionSnapshot) -> usize {
    match session.board.width {
        0..=9 => 2,
        10..=16 => 3,
        17..=24 => 4,
        _ => 5,
    }
}

fn displayed_report_users(
    group: &AfkStatsGroupSnapshot,
    target_rows: usize,
) -> Vec<AfkUserStatsSnapshot> {
    group
        .users
        .iter()
        .filter(|user| user.opened_cells > 0 || user.correct_flags > 0 || user.incorrect_flags > 0)
        .cloned()
        .take(target_rows)
        .collect()
}

fn report_user_has_wrong_flags_only(user: &AfkUserStatsSnapshot) -> bool {
    user.incorrect_flags > 0
        && user.opened_cells == 0
        && user.correct_flags == 0
        && user.correct_unflags == 0
}

fn round_report_user_icon_name(user: &AfkUserStatsSnapshot) -> &'static str {
    if user.died_this_round {
        "lose"
    } else if report_user_has_wrong_flags_only(user) {
        "woozy"
    } else if user.incorrect_flags > 0 {
        "win-decent"
    } else {
        "win"
    }
}

fn total_report_user_icon_name(game_over: bool, user: &AfkUserStatsSnapshot) -> &'static str {
    if game_over && user.died_every_round {
        "instant-loss"
    } else if user.died_this_round {
        "lose"
    } else if user.died_before_this_round {
        "in-progress"
    } else if report_user_has_wrong_flags_only(user) {
        "woozy"
    } else {
        "win"
    }
}

fn round_report_user_icon_name_for_scope(
    scope: AfkRoundReportScope,
    game_over: bool,
    user: &AfkUserStatsSnapshot,
) -> &'static str {
    match scope {
        AfkRoundReportScope::Round => round_report_user_icon_name(user),
        AfkRoundReportScope::Total => total_report_user_icon_name(game_over, user),
    }
}

fn render_round_report_header_cell(cell: AfkCellSnapshot) -> Html {
    html! {
        <span class="afk-round-report-header-icon-stack" aria-hidden="true">
            {render_demo_context_board(
                AfkCountdownDemoCell {
                    state: cell,
                    code: None,
                },
                "afk-round-report-header-window",
                "afk-round-report-header-board",
            )}
            <Icon
                name="ok"
                crop={IconCrop::CenteredSquare64}
                class={classes!("afk-round-report-header-ok")}
            />
        </span>
    }
}

fn render_round_report_colgroup() -> Html {
    html! {
        <div class="afk-round-report-colgroup" aria-hidden="true">
            <div class="afk-round-report-name-col" />
            <div class="afk-round-report-count-col" />
            <div class="afk-round-report-count-col" />
        </div>
    }
}

fn round_report_body_overflow_px(
    group_ref: &NodeRef,
    thead_ref: &NodeRef,
    tbody_ref: &NodeRef,
) -> f64 {
    let Some(group) = group_ref.cast::<Element>() else {
        return 0.0;
    };
    let Some(thead) = thead_ref.cast::<Element>() else {
        return 0.0;
    };
    let Some(tbody) = tbody_ref.cast::<Element>() else {
        return 0.0;
    };
    let Some(group_client_height) = element_number_property_from_element(&group, "clientHeight")
    else {
        return 0.0;
    };
    let Some(thead_height) = element_number_property_from_element(&thead, "offsetHeight") else {
        return 0.0;
    };
    let Some(tbody_height) = element_number_property_from_element(&tbody, "offsetHeight") else {
        return 0.0;
    };

    let body_viewport_height = (group_client_height - thead_height).max(0.0);
    let overflow_px = (tbody_height - body_viewport_height).max(0.0);
    (overflow_px > 0.0).then_some(overflow_px).unwrap_or(0.0)
}

fn element_number_property_from_element(element: &Element, property: &str) -> Option<f64> {
    Reflect::get(element.as_ref(), &JsValue::from_str(property))
        .ok()?
        .as_f64()
}

fn round_report_animation_duration_ms(overflow_px: f64) -> u32 {
    let travel_ms = overflow_px.max(0.0).round() as u32 * 35;
    travel_ms
        .saturating_mul(2)
        .saturating_add(2_000)
        .clamp(8_000, 30_000)
}

#[derive(Properties, PartialEq)]
struct AfkRoundReportGroupProps {
    title: AttrValue,
    scope: AfkRoundReportScope,
    game_over: bool,
    users: Vec<AfkUserStatsSnapshot>,
}

#[function_component]
fn AfkRoundReportGroupView(props: &AfkRoundReportGroupProps) -> Html {
    let group_ref = use_node_ref();
    let thead_ref = use_node_ref();
    let tbody_ref = use_node_ref();
    let overflow_px = use_state_eq(|| 0.0_f64);

    {
        let group_ref = group_ref.clone();
        let thead_ref = thead_ref.clone();
        let tbody_ref = tbody_ref.clone();
        let overflow_px = overflow_px.clone();
        use_effect(move || {
            let handle = request_animation_frame(move |_| {
                let next = round_report_body_overflow_px(&group_ref, &thead_ref, &tbody_ref);
                overflow_px.set(next);
            });
            move || drop(handle)
        });
    }

    let is_overflowing = *overflow_px > 0.0;
    let tbody_style = if is_overflowing {
        format!(
            "--afk-round-report-body-overflow: {:.3}px; --afk-round-report-body-duration: {}ms;",
            *overflow_px,
            round_report_animation_duration_ms(*overflow_px),
        )
    } else {
        String::new()
    };

    html! {
        <div
            class={classes!("afk-round-report-group", is_overflowing.then_some("overflowing"))}
            ref={group_ref}
        >
            <div class="afk-round-report-table" role="table">
                {render_round_report_colgroup()}
                <div class="afk-round-report-thead" ref={thead_ref} role="rowgroup">
                    <div class="afk-round-report-row" role="row">
                        <div class="afk-round-report-th afk-round-report-section" role="columnheader">
                            {props.title.clone()}
                        </div>
                        <div
                            class="afk-round-report-th afk-round-report-count-header"
                            title="Correct flags"
                            role="columnheader"
                        >
                            {render_round_report_header_cell(AfkCellSnapshot::Flagged)}
                        </div>
                        <div
                            class="afk-round-report-th afk-round-report-count-header"
                            title="Opened cells"
                            role="columnheader"
                        >
                            {render_round_report_header_cell(AfkCellSnapshot::Hidden)}
                        </div>
                    </div>
                </div>
                <div class="afk-round-report-tbody" ref={tbody_ref} style={tbody_style} role="rowgroup">
                    {
                        if props.users.is_empty() {
                            html! {
                                <div class="afk-round-report-row" role="row">
                                    <div class="afk-round-report-td afk-round-report-empty" role="cell">{"No moves"}</div>
                                    <div class="afk-round-report-td afk-round-report-empty-count" role="cell"></div>
                                    <div class="afk-round-report-td afk-round-report-empty-count" role="cell"></div>
                                </div>
                            }
                        } else {
                            html! {
                                <>
                                    {
                                        for props.users.iter().map(|user| html! {
                                            <div class="afk-round-report-row" role="row">
                                                <div class="afk-round-report-td afk-round-report-user" role="cell">
                                                    <span class="afk-round-report-user-inner">
                                                        <Icon
                                                            name={round_report_user_icon_name_for_scope(props.scope, props.game_over, user)}
                                                            crop={IconCrop::CenteredSquare64}
                                                            class={classes!("afk-round-report-user-icon")}
                                                        />
                                                        <span class="afk-round-report-user-name">
                                                            {user.chatter.display_name.clone()}
                                                        </span>
                                                    </span>
                                                </div>
                                                <div class="afk-round-report-td afk-round-report-count" role="cell">{user.correct_flags}</div>
                                                <div class="afk-round-report-td afk-round-report-count" role="cell">{user.opened_cells}</div>
                                            </div>
                                        })
                                    }
                                </>
                            }
                        }
                    }
                </div>
            </div>
        </div>
    }
}

fn render_round_report_group(
    title: &str,
    scope: AfkRoundReportScope,
    game_over: bool,
    group: &AfkStatsGroupSnapshot,
    max_rows: usize,
) -> Html {
    let users = displayed_report_users(group, max_rows);
    html! {
        <AfkRoundReportGroupView title={AttrValue::from(title)} {scope} {game_over} {users} />
    }
}

fn render_round_report_overlay(
    session: &AfkSessionSnapshot,
    report: &AfkRoundReportSnapshot,
    layout: AfkRoundReportLayout,
) -> Html {
    let max_rows = round_report_max_rows(session);
    html! {
        <div class="afk-round-report-overlay" aria-hidden="true">
            <div class={classes!(
                "afk-round-report-window",
                (session.board.width == 16 && matches!(layout, AfkRoundReportLayout::SideBySide))
                    .then_some("small-board"),
                matches!(layout, AfkRoundReportLayout::TotalOnly).then_some("total-only"),
                matches!(
                    layout,
                    AfkRoundReportLayout::Stacked | AfkRoundReportLayout::TotalOnly
                )
                .then_some("stacked")
            )}>
                {
                    if matches!(layout, AfkRoundReportLayout::TotalOnly) {
                        render_round_report_group("Total", AfkRoundReportScope::Total, session.game_over, &report.run, max_rows)
                    } else {
                        html! {
                            <div class="afk-round-report-columns">
                                {render_round_report_group("Round", AfkRoundReportScope::Round, session.game_over, &report.round, max_rows)}
                                {render_round_report_group("Total", AfkRoundReportScope::Total, session.game_over, &report.run, max_rows)}
                            </div>
                        }
                    }
                }
            </div>
        </div>
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

fn afk_activity_text(row: &AfkActivityRow, hazard_variant: HazardVariant) -> AttrValue {
    match (row.kind, row.actor.as_ref(), row.coord) {
        (AfkActivityKind::MineHit, Some(actor), Some(coord)) => hazard_variant
            .mine_hit_message(&actor.display_name, &format_afk_coord_snapshot(coord))
            .into(),
        _ => row.text.clone().into(),
    }
}

fn face_notification_event(
    row: &AfkActivityRow,
    hazard_variant: HazardVariant,
) -> Option<AfkFaceNotificationEvent> {
    match row.kind {
        AfkActivityKind::MineHit => Some(AfkFaceNotificationEvent {
            message: afk_activity_text(row, hazard_variant),
            timeout_ms: AFK_FACE_NOTIFICATION_MS,
        }),
        AfkActivityKind::OutForRound => row.actor.as_ref().map(|actor| AfkFaceNotificationEvent {
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
    status_notice: Option<AttrValue>,
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
                if let Some(message) = status_notice.clone() {
                    return Some(AfkFaceOverlay::Message {
                        message,
                        status: level_status,
                    });
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
    status_notice_present: bool,
) -> &'static str {
    if status_notice_present {
        return "spiral-eyes";
    }
    if notification.is_some() {
        return "dejected";
    }
    if idle_state.is_some() {
        return "yawning";
    }
    match status {
        LoadState::Loading => "mid-open",
        LoadState::Ready(status) => match status.session.as_ref() {
            Some(session) => match session.phase {
                AfkRoundPhase::Countdown => "starting-soon",
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

fn view_lives_rail(lives: u8) -> Html {
    html! {
        <div
            class="face-lives-rail"
            aria-label={format!("{lives} live{}", if lives == 1 { "" } else { "s" })}
        >
            {
                for (0..usize::from(lives)).map(|_| html! {
                    <Icon
                        name="heart"
                        crop={IconCrop::CenteredSquare64}
                        class={classes!("face-life-icon")}
                    />
                })
            }
        </div>
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

fn format_afk_coord_snapshot(coord: AfkCoordSnapshot) -> String {
    format_cell_code((usize::from(coord.x), usize::from(coord.y)))
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

fn afk_count_flagged_neighbors(session: &AfkSessionSnapshot, coords: (usize, usize)) -> u8 {
    let mut flagged_neighbors: u8 = 0;
    for dy in -1isize..=1 {
        for dx in -1isize..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let nx = coords.0.checked_add_signed(dx);
            let ny = coords.1.checked_add_signed(dy);
            let (Some(nx), Some(ny)) = (nx, ny) else {
                continue;
            };
            if matches!(
                board_cell_at(session, nx, ny),
                Some(AfkCellSnapshot::Flagged)
            ) {
                flagged_neighbors = flagged_neighbors.saturating_add(1);
            }
        }
    }
    flagged_neighbors
}

fn afk_can_chord_reveal_at(session: &AfkSessionSnapshot, coords: (usize, usize)) -> bool {
    match board_cell_at(session, coords.0, coords.1) {
        Some(AfkCellSnapshot::Revealed(count)) => {
            count == afk_count_flagged_neighbors(session, coords)
        }
        _ => false,
    }
}

fn request_for_streamer_pointer_release(
    session: &AfkSessionSnapshot,
    coords: (usize, usize),
    buttons: MouseButtons,
) -> Option<AfkActionRequest> {
    let cell = board_cell_at(session, coords.0, coords.1)?;
    let kind = match (buttons, cell) {
        (MouseButtons::LEFT, AfkCellSnapshot::Hidden) => AfkActionKind::Reveal,
        (MouseButtons::LEFT, AfkCellSnapshot::Revealed(count)) if count > 0 => AfkActionKind::Chord,
        (MouseButtons::RIGHT, AfkCellSnapshot::Revealed(count)) if count > 0 => {
            AfkActionKind::ChordFlag
        }
        (MouseButtons::RIGHT, AfkCellSnapshot::Hidden | AfkCellSnapshot::Flagged) => {
            AfkActionKind::ToggleFlag
        }
        _ => return None,
    };
    Some(AfkActionRequest {
        kind,
        x: coords.0 as u8,
        y: coords.1 as u8,
    })
}

fn held_afk_press_preview(
    session: Option<&AfkSessionSnapshot>,
    pointer_state: AfkCellPointerState,
) -> Option<AfkHeldCellPreview> {
    matches!(pointer_state.buttons, MouseButtons::LEFT)
        .then(|| session.cloned())
        .flatten()
        .map(|session_before_action| AfkHeldCellPreview {
            pointer_state,
            session_before_action,
        })
}

fn held_afk_pointer_state(
    preview: Option<&AfkHeldCellPreview>,
    session: Option<&AfkSessionSnapshot>,
) -> Option<AfkCellPointerState> {
    preview.and_then(|preview| {
        session
            .filter(|session| *session == &preview.session_before_action)
            .map(|_| preview.pointer_state)
    })
}

fn afk_is_pressed(
    session: &AfkSessionSnapshot,
    coords: (usize, usize),
    cell: AfkCellSnapshot,
    pointer_state: Option<AfkCellPointerState>,
) -> bool {
    let Some(AfkCellPointerState {
        pos,
        buttons: MouseButtons::LEFT,
    }) = pointer_state
    else {
        return false;
    };

    if !matches!(cell, AfkCellSnapshot::Hidden) {
        return false;
    }

    if pos == coords {
        return true;
    }

    pos.0.abs_diff(coords.0) <= 1
        && pos.1.abs_diff(coords.1) <= 1
        && afk_can_chord_reveal_at(session, pos)
}

fn afk_cell_classes(
    cell: AfkCellSnapshot,
    hazard_variant: HazardVariant,
    interactive: bool,
    pressed: bool,
) -> Classes {
    let mut class = match cell {
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
            hazard_variant.cell_class(),
            "oops",
            "afk-cell"
        ),
        AfkCellSnapshot::Mine => classes!(
            "cell",
            (!interactive).then_some("locked"),
            "open",
            "mine",
            hazard_variant.cell_class(),
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
    if pressed {
        class.push("open");
    }
    class
}

fn afk_cell_content(
    cell: AfkCellSnapshot,
    code: Option<AttrValue>,
    show_code: bool,
    hazard_variant: HazardVariant,
) -> Html {
    match cell {
        AfkCellSnapshot::Hidden => show_code
            .then(|| {
                code.map(|code| html! { <span class="afk-cell-code">{code}</span> })
                    .unwrap_or_default()
            })
            .unwrap_or_default(),
        AfkCellSnapshot::Flagged => html! {
            <>
                {
                    if show_code {
                        code.map(|code| html! { <span class="afk-cell-tag">{code}</span> })
                            .unwrap_or_default()
                    } else {
                        Html::default()
                    }
                }
                <Icon name="flag" class={classes!("cell-icon")}/>
            </>
        },
        AfkCellSnapshot::Crater => html! {
            <Icon name={hazard_variant.triggered_hazard_icon_name()} class={classes!("cell-icon")}/>
        },
        AfkCellSnapshot::Mine => html! {
            <Icon name={hazard_variant.hidden_hazard_icon_name()} class={classes!("cell-icon")}/>
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
    }
}

fn render_afk_cell(
    session: &AfkSessionSnapshot,
    (x, y): (usize, usize),
    cell: AfkCellSnapshot,
    hazard_variant: HazardVariant,
    interactive: bool,
    pointer_state: Option<AfkCellPointerState>,
    on_cell_event: Callback<AfkCellMsg>,
) -> Html {
    let code: AttrValue = format_cell_code((x, y)).into();
    let show_code = should_show_cell_code(session, x, y, cell);
    let pressed = interactive && afk_is_pressed(session, (x, y), cell, pointer_state);
    let class = afk_cell_classes(cell, hazard_variant, interactive, pressed);
    let content = afk_cell_content(cell, Some(code), show_code, hazard_variant);
    let CellPointerCallbacks {
        onmousedown,
        onmouseup,
        onmouseenter,
        onmouseleave,
    } = cell_pointer_callbacks((x, y), on_cell_event);

    let oncontextmenu = Callback::from(|e: MouseEvent| e.prevent_default());

    html! {
        <td class={class} {onmousedown} {onmouseup} {onmouseenter} {onmouseleave} {oncontextmenu}>
            {content}
        </td>
    }
}

fn active_countdown_overlay(session: Option<&AfkSessionSnapshot>) -> Option<AfkCountdownOverlay> {
    session
        .is_some_and(|session| matches!(session.phase, AfkRoundPhase::Countdown))
        .then_some(AFK_COUNTDOWN_OVERLAY)
}

fn countdown_demo_step_ms(show_after: bool) -> u32 {
    if show_after {
        AFK_COUNTDOWN_DEMO_AFTER_MS
    } else {
        AFK_COUNTDOWN_DEMO_BEFORE_MS
    }
}

fn render_countdown_demo_board_cell(cell: AfkCountdownDemoCell) -> Html {
    html! {
        <td class={afk_cell_classes(cell.state, HazardVariant::default(), false, false)}>
            {afk_cell_content(
                cell.state,
                cell.code.map(AttrValue::from),
                cell.code.is_some(),
                HazardVariant::default(),
            )}
        </td>
    }
}

fn render_demo_context_board(
    cell: AfkCountdownDemoCell,
    window_class: &'static str,
    board_class: &'static str,
) -> Html {
    html! {
        <div class={window_class} aria-hidden="true">
            <table class={board_class}>
                <tbody>
                    {
                        for (0..3).map(|y| html! {
                            <tr>
                                {
                                    for (0..3).map(|x| {
                                        let preview_cell = if x == 1 && y == 1 {
                                            cell
                                        } else {
                                            AfkCountdownDemoCell {
                                                state: AfkCellSnapshot::Hidden,
                                                code: None,
                                            }
                                        };
                                        render_countdown_demo_board_cell(preview_cell)
                                    })
                                }
                            </tr>
                        })
                    }
                </tbody>
            </table>
        </div>
    }
}

fn render_countdown_context_board(cell: AfkCountdownDemoCell) -> Html {
    render_demo_context_board(
        cell,
        "afk-countdown-board-window",
        "afk-countdown-board-preview",
    )
}

fn render_countdown_overlay_row(row: AfkCountdownDemoRow, show_after: bool) -> Html {
    let displayed_cell = if show_after { row.after } else { row.before };
    html! {
        <div class="afk-countdown-row">
            {render_countdown_context_board(displayed_cell)}
            <div class="afk-countdown-command">{row.command}</div>
            <div class="afk-countdown-description">{row.description}</div>
        </div>
    }
}

fn render_countdown_overlay(overlay: AfkCountdownOverlay, show_after: bool) -> Html {
    html! {
        <div
            class="afk-countdown-overlay"
            role="img"
            aria-label={overlay.aria_label}
        >
            <div class="afk-countdown-guide">
                <div class="afk-countdown-title">{overlay.title}</div>
                <div class="afk-countdown-rows" aria-hidden="true">
                    {for overlay.rows.into_iter().map(|row| render_countdown_overlay_row(row, show_after))}
                </div>
            </div>
        </div>
    }
}

fn render_afk_board(
    session: &AfkSessionSnapshot,
    hazard_variant: HazardVariant,
    interactive: bool,
    pointer_state: Option<AfkCellPointerState>,
    on_cell_event: Callback<AfkCellMsg>,
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
                                render_afk_cell(
                                    session,
                                    (x, y),
                                    cell,
                                    hazard_variant,
                                    interactive,
                                    pointer_state,
                                    on_cell_event.clone(),
                                )
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
    let local_hazard_variant = HazardVariant::local_or_default();
    let status = use_state_eq(|| LoadState::<AfkStatusResponse>::Idle);
    let hazard_variant = match &*status {
        LoadState::Ready(status) => status
            .session
            .as_ref()
            .map(|session| HazardVariant::from_afk_protocol(session.hazard_variant))
            .unwrap_or(local_hazard_variant),
        _ => local_hazard_variant,
    };
    let screen = use_state_eq({
        let restore_view_state = props.restore_view_state;
        move || initial_afk_screen(restore_view_state)
    });
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
    let current_cell_state = use_state_eq(|| None::<AfkCellPointerState>);
    let held_cell_preview = use_state_eq(|| None::<AfkHeldCellPreview>);
    let start_transition_in_progress = use_state_eq(|| false);
    let countdown_demo_show_after = use_state_eq(|| false);
    let countdown_demo_timeout = use_mut_ref(|| None::<Timeout>);
    let socket_connected = use_state_eq(|| false);
    let socket_reconnecting = use_state_eq(|| false);
    let socket_retry_deadline_ms = use_state_eq(|| None::<i64>);
    let socket_retry_tick = use_state_eq(|| 0_u64);
    let socket_retry_tick_timeout = use_mut_ref(|| None::<Timeout>);
    let socket_retry_version = use_state_eq(|| 0_u64);
    let socket_retry_timeout = use_mut_ref(|| None::<Timeout>);
    let socket_retry_attempt = use_mut_ref(|| 0_u32);
    let socket_notice_started_at_ms = use_state_eq(|| None::<i64>);
    let socket_notice_tick = use_state_eq(|| 0_u64);
    let socket_notice_timeout = use_mut_ref(|| None::<Timeout>);
    let chat_reconnect_active = use_state_eq(|| false);
    let chat_reconnect_version = use_state_eq(|| 0_u64);
    let chat_reconnect_timeout = use_mut_ref(|| None::<Timeout>);
    let chat_reconnect_attempt = use_mut_ref(|| 0_u32);
    let chat_notice_started_at_ms = use_state_eq(|| None::<i64>);
    let chat_notice_tick = use_state_eq(|| 0_u64);
    let chat_notice_timeout = use_mut_ref(|| None::<Timeout>);
    let live_hazard_variant = use_mut_ref(|| hazard_variant);
    let socket_path = match &*status {
        LoadState::Ready(status) if status.auth.identity.is_some() => status.websocket_path.clone(),
        _ => None,
    };
    let socket_notice_active = (*socket_retry_deadline_ms).is_some() || *socket_reconnecting;
    let chat_notice_active = *chat_reconnect_active
        || matches!(
            &*status,
            LoadState::Ready(status)
                if status.session.is_some()
                    && matches!(status.chat_connection, AfkChatConnectionState::Connecting)
        );
    let countdown_overlay_active = matches!(
        (&*status, *screen),
        (LoadState::Ready(status), AfkScreen::Board)
            if status
                .session
                .as_ref()
                .is_some_and(|session| matches!(session.phase, AfkRoundPhase::Countdown))
    );

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
        let live_hazard_variant = live_hazard_variant.clone();
        use_effect_with(hazard_variant, move |hazard_variant| {
            *live_hazard_variant.borrow_mut() = *hazard_variant;
            || ()
        });
    }

    {
        let status_handle = status.clone();
        use_effect_with(
            ((*status_handle).clone(), local_hazard_variant),
            move |(status_snapshot, hazard_variant)| {
                let variant_mismatch = matches!(
                    status_snapshot,
                    LoadState::Ready(AfkStatusResponse {
                        session: Some(session),
                        ..
                    }) if HazardVariant::from_afk_protocol(session.hazard_variant) != *hazard_variant
                );
                if variant_mismatch {
                    let status_handle = status_handle.clone();
                    let hazard_variant = *hazard_variant;
                    spawn_local(async move {
                        match post_hazard_variant_status(hazard_variant).await {
                            Ok(response) => status_handle.set(LoadState::Ready(response)),
                            Err(error) => status_handle.set(LoadState::Error(error)),
                        }
                    });
                }
                || ()
            },
        );
    }

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
        use_effect_with(*screen, move |screen| {
            screen.local_save();
            || ()
        });
    }

    {
        let countdown_demo_show_after = countdown_demo_show_after.clone();
        let countdown_demo_timeout = countdown_demo_timeout.clone();
        use_effect_with(
            (countdown_overlay_active, *countdown_demo_show_after),
            move |(countdown_overlay_active, countdown_demo_show_after_value)| {
                countdown_demo_timeout.borrow_mut().take();
                if *countdown_overlay_active {
                    let next_value = !*countdown_demo_show_after_value;
                    let countdown_demo_show_after = countdown_demo_show_after.clone();
                    let countdown_demo_timeout_for_store = countdown_demo_timeout.clone();
                    let countdown_demo_timeout_for_callback = countdown_demo_timeout.clone();
                    *countdown_demo_timeout_for_store.borrow_mut() = Some(Timeout::new(
                        countdown_demo_step_ms(*countdown_demo_show_after_value),
                        move || {
                            countdown_demo_show_after.set(next_value);
                            countdown_demo_timeout_for_callback.borrow_mut().take();
                        },
                    ));
                } else if *countdown_demo_show_after_value {
                    countdown_demo_show_after.set(false);
                }
                let countdown_demo_timeout = countdown_demo_timeout.clone();
                move || {
                    countdown_demo_timeout.borrow_mut().take();
                }
            },
        );
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
        let socket_retry_deadline_ms = socket_retry_deadline_ms.clone();
        let socket_retry_tick = socket_retry_tick.clone();
        let socket_retry_tick_timeout = socket_retry_tick_timeout.clone();
        let retry_deadline_ms = *socket_retry_deadline_ms;
        let retry_tick_version = *socket_retry_tick;
        use_effect_with(
            (retry_deadline_ms, retry_tick_version),
            move |(retry_deadline_ms, _)| {
                socket_retry_tick_timeout.borrow_mut().take();
                if let Some(retry_deadline_ms) = *retry_deadline_ms {
                    let remaining_ms = retry_deadline_ms.saturating_sub(browser_now_ms());
                    if remaining_ms > 0 {
                        let delay_ms = (remaining_ms as u32).min(AFK_WEBSOCKET_RECONNECT_TICK_MS);
                        let socket_retry_tick = socket_retry_tick.clone();
                        let socket_retry_tick_timeout_for_store = socket_retry_tick_timeout.clone();
                        let socket_retry_tick_timeout_for_callback =
                            socket_retry_tick_timeout.clone();
                        *socket_retry_tick_timeout_for_store.borrow_mut() =
                            Some(Timeout::new(delay_ms, move || {
                                socket_retry_tick.set((*socket_retry_tick).saturating_add(1));
                                socket_retry_tick_timeout_for_callback.borrow_mut().take();
                            }));
                    }
                }
                let socket_retry_tick_timeout = socket_retry_tick_timeout.clone();
                move || {
                    socket_retry_tick_timeout.borrow_mut().take();
                }
            },
        );
    }

    {
        let socket_notice_started_at_ms = socket_notice_started_at_ms.clone();
        use_effect_with(socket_notice_active, move |active| {
            if *active {
                if (*socket_notice_started_at_ms).is_none() {
                    socket_notice_started_at_ms.set(Some(browser_now_ms()));
                }
            } else if (*socket_notice_started_at_ms).is_some() {
                socket_notice_started_at_ms.set(None);
            }
            || ()
        });
    }

    {
        let socket_notice_tick = socket_notice_tick.clone();
        let socket_notice_timeout = socket_notice_timeout.clone();
        let notice_started_at_ms = *socket_notice_started_at_ms;
        let notice_tick_version = *socket_notice_tick;
        use_effect_with(
            (notice_started_at_ms, notice_tick_version),
            move |(notice_started_at_ms, _)| {
                socket_notice_timeout.borrow_mut().take();
                if let Some(delay_ms) =
                    next_connection_notice_refresh_delay_ms(*notice_started_at_ms, browser_now_ms())
                {
                    let socket_notice_tick = socket_notice_tick.clone();
                    let socket_notice_timeout_for_store = socket_notice_timeout.clone();
                    let socket_notice_timeout_for_callback = socket_notice_timeout.clone();
                    *socket_notice_timeout_for_store.borrow_mut() =
                        Some(Timeout::new(delay_ms, move || {
                            socket_notice_tick.set((*socket_notice_tick).saturating_add(1));
                            socket_notice_timeout_for_callback.borrow_mut().take();
                        }));
                }
                let socket_notice_timeout = socket_notice_timeout.clone();
                move || {
                    socket_notice_timeout.borrow_mut().take();
                }
            },
        );
    }

    {
        let status = status.clone();
        let last_error = last_error.clone();
        let screen = screen.clone();
        let show_face_notification = show_face_notification.clone();
        let live_hazard_variant = live_hazard_variant.clone();
        let socket_connected = socket_connected.clone();
        let socket_reconnecting = socket_reconnecting.clone();
        let socket_retry_deadline_ms = socket_retry_deadline_ms.clone();
        let socket_retry_version = socket_retry_version.clone();
        let socket_retry_timeout = socket_retry_timeout.clone();
        let socket_retry_attempt = socket_retry_attempt.clone();
        use_effect_with(
            (socket_path.clone(), *socket_retry_version),
            move |(socket_path, _)| {
                socket_retry_timeout.borrow_mut().take();
                let intentionally_closed = Rc::new(Cell::new(false));
                let reconnect_scheduled = Rc::new(Cell::new(false));
                let schedule_reconnect = {
                    let intentionally_closed = intentionally_closed.clone();
                    let reconnect_scheduled = reconnect_scheduled.clone();
                    let socket_reconnecting = socket_reconnecting.clone();
                    let socket_retry_deadline_ms = socket_retry_deadline_ms.clone();
                    let socket_retry_version = socket_retry_version.clone();
                    let socket_retry_timeout = socket_retry_timeout.clone();
                    let socket_retry_attempt = socket_retry_attempt.clone();
                    Rc::new(move || {
                        if intentionally_closed.get() || reconnect_scheduled.replace(true) {
                            return;
                        }

                        let attempt = {
                            let mut current = socket_retry_attempt.borrow_mut();
                            *current = current.saturating_add(1);
                            *current
                        };
                        let delay_ms = next_afk_reconnect_delay_ms(attempt);
                        socket_reconnecting.set(false);
                        socket_retry_deadline_ms
                            .set(Some(browser_now_ms().saturating_add(i64::from(delay_ms))));

                        let socket_retry_timeout_for_store = socket_retry_timeout.clone();
                        let socket_retry_timeout_for_callback = socket_retry_timeout.clone();
                        let socket_reconnecting = socket_reconnecting.clone();
                        let socket_retry_deadline_ms = socket_retry_deadline_ms.clone();
                        let socket_retry_version = socket_retry_version.clone();
                        *socket_retry_timeout_for_store.borrow_mut() =
                            Some(Timeout::new(delay_ms, move || {
                                socket_reconnecting.set(true);
                                socket_retry_deadline_ms.set(None);
                                socket_retry_version.set((*socket_retry_version).saturating_add(1));
                                socket_retry_timeout_for_callback.borrow_mut().take();
                            }));
                    })
                };

                let mut socket = None::<WebSocket>;
                let mut onmessage = None::<Closure<dyn FnMut(MessageEvent)>>;
                let mut onopen = None::<Closure<dyn FnMut(Event)>>;
                let mut onclose = None::<Closure<dyn FnMut(Event)>>;
                let mut onerror = None::<Closure<dyn FnMut(JsValue)>>;

                if let Some(socket_path) = socket_path.clone() {
                    let socket_url = websocket_path(&socket_path);
                    match WebSocket::new(&socket_url) {
                        Ok(ws) => {
                            let status_state = status.clone();
                            let last_error_for_message = last_error.clone();
                            let screen_for_message = screen.clone();
                            let show_face_notification = show_face_notification.clone();

                            let message_handler = Closure::<dyn FnMut(MessageEvent)>::new(
                                move |event: MessageEvent| {
                                    let Some(payload) = event.data().as_string() else {
                                        return;
                                    };
                                    match serde_json::from_str::<AfkServerMessage>(&payload) {
                                        Ok(AfkServerMessage::Connected { status: next }) => {
                                            last_error_for_message.set(None);
                                            status_state.set(LoadState::Ready(next));
                                        }
                                        Ok(AfkServerMessage::Snapshot { session }) => {
                                            if let LoadState::Ready(mut next) =
                                                (*status_state).clone()
                                            {
                                                last_error_for_message.set(None);
                                                next.session = Some(session);
                                                status_state.set(LoadState::Ready(next));
                                            }
                                        }
                                        Ok(AfkServerMessage::Activity { row }) => {
                                            if matches!(*screen_for_message, AfkScreen::Board)
                                                && let Some(notification) = face_notification_event(
                                                    &row,
                                                    *live_hazard_variant.borrow(),
                                                )
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
                                },
                            );
                            ws.set_onmessage(Some(message_handler.as_ref().unchecked_ref()));

                            let onopen_socket = ws.clone();
                            let last_error_for_open = last_error.clone();
                            let reconnect_scheduled_for_open = reconnect_scheduled.clone();
                            let socket_connected_for_open = socket_connected.clone();
                            let socket_reconnecting_for_open = socket_reconnecting.clone();
                            let socket_retry_deadline_ms_for_open =
                                socket_retry_deadline_ms.clone();
                            let socket_retry_timeout_for_open = socket_retry_timeout.clone();
                            let socket_retry_attempt_for_open = socket_retry_attempt.clone();
                            let open_handler =
                                Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
                                    reconnect_scheduled_for_open.set(false);
                                    socket_connected_for_open.set(true);
                                    socket_reconnecting_for_open.set(false);
                                    socket_retry_deadline_ms_for_open.set(None);
                                    socket_retry_timeout_for_open.borrow_mut().take();
                                    *socket_retry_attempt_for_open.borrow_mut() = 0;
                                    last_error_for_open.set(None);
                                    let _ = onopen_socket.send_with_str(
                                        &serde_json::to_string(&AfkClientMessage::Ping)
                                            .unwrap_or_default(),
                                    );
                                });
                            ws.set_onopen(Some(open_handler.as_ref().unchecked_ref()));

                            let schedule_reconnect_for_close = schedule_reconnect.clone();
                            let intentionally_closed_for_close = intentionally_closed.clone();
                            let socket_connected_for_close = socket_connected.clone();
                            let close_handler =
                                Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
                                    socket_connected_for_close.set(false);
                                    if intentionally_closed_for_close.get() {
                                        return;
                                    }
                                    schedule_reconnect_for_close();
                                });
                            ws.set_onclose(Some(close_handler.as_ref().unchecked_ref()));

                            let schedule_reconnect_for_error = schedule_reconnect.clone();
                            let intentionally_closed_for_error = intentionally_closed.clone();
                            let socket_connected_for_error = socket_connected.clone();
                            let socket_for_error = ws.clone();
                            let error_handler =
                                Closure::<dyn FnMut(JsValue)>::new(move |error: JsValue| {
                                    socket_connected_for_error.set(false);
                                    if intentionally_closed_for_error.get() {
                                        return;
                                    }
                                    log::warn!("afk websocket error: {error:?}");
                                    let _ = socket_for_error.close();
                                    schedule_reconnect_for_error();
                                });
                            ws.set_onerror(Some(error_handler.as_ref().unchecked_ref()));

                            socket = Some(ws);
                            onmessage = Some(message_handler);
                            onopen = Some(open_handler);
                            onclose = Some(close_handler);
                            onerror = Some(error_handler);
                        }
                        Err(error) => {
                            log::warn!("failed to open afk websocket: {error:?}");
                            last_error.set(None);
                            schedule_reconnect();
                        }
                    }
                } else {
                    socket_connected.set(false);
                    socket_reconnecting.set(false);
                    socket_retry_deadline_ms.set(None);
                    *socket_retry_attempt.borrow_mut() = 0;
                }

                let socket_retry_timeout = socket_retry_timeout.clone();
                move || {
                    intentionally_closed.set(true);
                    socket_connected.set(false);
                    socket_retry_timeout.borrow_mut().take();
                    if let Some(socket) = socket {
                        socket.close().ok();
                        socket.set_onmessage(None);
                        socket.set_onopen(None);
                        socket.set_onclose(None);
                        socket.set_onerror(None);
                    }
                    drop(onmessage);
                    drop(onopen);
                    drop(onclose);
                    drop(onerror);
                }
            },
        );
    }

    {
        let screen = screen.clone();
        let manual_face_prompt = manual_face_prompt.clone();
        let clear_face_notification = clear_face_notification.clone();
        let start_transition_in_progress = start_transition_in_progress.clone();
        use_effect_with(
            // Depend on the snapshot value so session teardown reruns this effect.
            ((*status).clone(), *screen, *start_transition_in_progress),
            move |(status, screen_value, start_transition_pending)| {
                if matches!(*screen_value, AfkScreen::Board) {
                    match resolve_board_screen(status, *start_transition_pending) {
                        AfkBoardScreenResolution::Keep => {}
                        AfkBoardScreenResolution::FinishStartTransition => {
                            start_transition_in_progress.set(false);
                        }
                        AfkBoardScreenResolution::ReturnToMenu => {
                            clear_face_notification();
                            manual_face_prompt.set(None);
                            start_transition_in_progress.set(false);
                            screen.set(AfkScreen::Menu);
                        }
                    }
                }
                || ()
            },
        );
    }

    {
        let status = status.clone();
        let socket_connected = socket_connected.clone();
        let chat_reconnect_active = chat_reconnect_active.clone();
        let chat_reconnect_version = chat_reconnect_version.clone();
        let chat_reconnect_timeout = chat_reconnect_timeout.clone();
        let chat_reconnect_attempt = chat_reconnect_attempt.clone();
        let chat_reconnect_needed = matches!(
            &*status,
            LoadState::Ready(status)
                if *socket_connected
                    && status.auth.identity.is_some()
                    && status.session.is_some()
                    && matches!(status.chat_connection, AfkChatConnectionState::Error)
        );
        use_effect_with(
            (chat_reconnect_needed, *chat_reconnect_version),
            move |(chat_reconnect_needed, _)| {
                chat_reconnect_timeout.borrow_mut().take();
                let cancelled = Rc::new(Cell::new(false));
                if *chat_reconnect_needed {
                    chat_reconnect_active.set(true);
                    let schedule_retry = {
                        let cancelled = cancelled.clone();
                        let chat_reconnect_active = chat_reconnect_active.clone();
                        let chat_reconnect_attempt = chat_reconnect_attempt.clone();
                        let chat_reconnect_timeout = chat_reconnect_timeout.clone();
                        let chat_reconnect_version = chat_reconnect_version.clone();
                        Rc::new(move || {
                            if cancelled.get() {
                                return;
                            }
                            chat_reconnect_active.set(true);
                            let attempt = {
                                let mut current = chat_reconnect_attempt.borrow_mut();
                                *current = current.saturating_add(1);
                                *current
                            };
                            let delay_ms = next_afk_reconnect_delay_ms(attempt);
                            let chat_reconnect_timeout_for_store = chat_reconnect_timeout.clone();
                            let chat_reconnect_timeout_for_callback =
                                chat_reconnect_timeout.clone();
                            let chat_reconnect_version = chat_reconnect_version.clone();
                            *chat_reconnect_timeout_for_store.borrow_mut() =
                                Some(Timeout::new(delay_ms, move || {
                                    chat_reconnect_version
                                        .set((*chat_reconnect_version).saturating_add(1));
                                    chat_reconnect_timeout_for_callback.borrow_mut().take();
                                }));
                        })
                    };

                    let status = status.clone();
                    let chat_reconnect_active = chat_reconnect_active.clone();
                    let chat_reconnect_attempt = chat_reconnect_attempt.clone();
                    let schedule_retry_for_task = schedule_retry.clone();
                    let cancelled_for_task = cancelled.clone();
                    spawn_local(async move {
                        match post_empty_status("/api/afk/chat-reconnect").await {
                            Ok(response) => {
                                if cancelled_for_task.get() {
                                    return;
                                }
                                let should_retry = response.auth.identity.is_some()
                                    && response.session.is_some()
                                    && matches!(
                                        response.chat_connection,
                                        AfkChatConnectionState::Error
                                    );
                                let keep_notice = matches!(
                                    response.chat_connection,
                                    AfkChatConnectionState::Connecting
                                );
                                status.set(LoadState::Ready(response));
                                if should_retry {
                                    schedule_retry_for_task();
                                } else {
                                    chat_reconnect_active.set(keep_notice);
                                    *chat_reconnect_attempt.borrow_mut() = 0;
                                }
                            }
                            Err(error) => {
                                if cancelled_for_task.get() {
                                    return;
                                }
                                log::warn!("failed to request afk chat reconnect: {error}");
                                schedule_retry_for_task();
                            }
                        }
                    });
                } else {
                    chat_reconnect_active.set(false);
                    *chat_reconnect_attempt.borrow_mut() = 0;
                }

                let chat_reconnect_timeout = chat_reconnect_timeout.clone();
                move || {
                    cancelled.set(true);
                    chat_reconnect_timeout.borrow_mut().take();
                }
            },
        );
    }

    {
        let chat_notice_started_at_ms = chat_notice_started_at_ms.clone();
        use_effect_with(chat_notice_active, move |active| {
            if *active {
                if (*chat_notice_started_at_ms).is_none() {
                    chat_notice_started_at_ms.set(Some(browser_now_ms()));
                }
            } else if (*chat_notice_started_at_ms).is_some() {
                chat_notice_started_at_ms.set(None);
            }
            || ()
        });
    }

    {
        let chat_notice_tick = chat_notice_tick.clone();
        let chat_notice_timeout = chat_notice_timeout.clone();
        let notice_started_at_ms = *chat_notice_started_at_ms;
        let notice_tick_version = *chat_notice_tick;
        use_effect_with(
            (notice_started_at_ms, notice_tick_version),
            move |(notice_started_at_ms, _)| {
                chat_notice_timeout.borrow_mut().take();
                if let Some(delay_ms) =
                    next_connection_notice_refresh_delay_ms(*notice_started_at_ms, browser_now_ms())
                {
                    let chat_notice_tick = chat_notice_tick.clone();
                    let chat_notice_timeout_for_store = chat_notice_timeout.clone();
                    let chat_notice_timeout_for_callback = chat_notice_timeout.clone();
                    *chat_notice_timeout_for_store.borrow_mut() =
                        Some(Timeout::new(delay_ms, move || {
                            chat_notice_tick.set((*chat_notice_tick).saturating_add(1));
                            chat_notice_timeout_for_callback.borrow_mut().take();
                        }));
                }
                let chat_notice_timeout = chat_notice_timeout.clone();
                move || {
                    chat_notice_timeout.borrow_mut().take();
                }
            },
        );
    }

    {
        let status = status.clone();
        let screen = screen.clone();
        let menu_page = menu_page.clone();
        let manual_face_prompt = manual_face_prompt.clone();
        let last_error = last_error.clone();
        let pre_auth_preferences = pre_auth_preferences.clone();
        let auto_start_in_progress = auto_start_in_progress.clone();
        let start_transition_in_progress = start_transition_in_progress.clone();
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
                    start_transition_in_progress.set(true);
                    screen.set(AfkScreen::Board);

                    let status = status.clone();
                    let screen = screen.clone();
                    let last_error = last_error.clone();
                    let start_transition_in_progress = start_transition_in_progress.clone();
                    spawn_local(async move {
                        match apply_preferences_and_start(
                            preferences,
                            HazardVariant::local_or_default(),
                        )
                        .await
                        {
                            Ok(response) => {
                                handle_started_status(
                                    &status,
                                    &screen,
                                    &last_error,
                                    &start_transition_in_progress,
                                    response,
                                );
                            }
                            Err(error) => {
                                start_transition_in_progress.set(false);
                                screen.set(AfkScreen::Menu);
                                status.set(LoadState::Error(error));
                            }
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
                            status.set(LoadState::Ready(response));
                            if has_session {
                                last_error.set(None);
                                screen.set(AfkScreen::Board);
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
        let start_transition_in_progress = start_transition_in_progress.clone();
        Callback::from(move |_| {
            manual_face_prompt.set(None);
            menu_page.set(AfkMenuPage::Root);
            start_transition_in_progress.set(true);
            screen.set(AfkScreen::Board);
            let status = status.clone();
            let screen = screen.clone();
            let last_error = last_error.clone();
            let start_transition_in_progress = start_transition_in_progress.clone();
            spawn_local(async move {
                last_error.set(None);
                match post_start_status(HazardVariant::local_or_default()).await {
                    Ok(response) => {
                        handle_started_status(
                            &status,
                            &screen,
                            &last_error,
                            &start_transition_in_progress,
                            response,
                        );
                    }
                    Err(error) => {
                        start_transition_in_progress.set(false);
                        screen.set(AfkScreen::Menu);
                        status.set(LoadState::Error(error));
                    }
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
        let start_transition_in_progress = start_transition_in_progress.clone();
        Callback::from(move |_| {
            let Some(next_board_size) = *pending_board_size else {
                return;
            };
            pending_board_size.set(None);
            menu_page.set(AfkMenuPage::Root);
            let status = status.clone();
            let screen = screen.clone();
            let last_error = last_error.clone();
            let start_transition_in_progress = start_transition_in_progress.clone();
            spawn_local(async move {
                let size_response = post_json_status(
                    "/api/afk/board-size",
                    &serde_json::json!({ "board_size": next_board_size }),
                )
                .await;
                match size_response {
                    Ok(_) => {
                        last_error.set(None);
                        start_transition_in_progress.set(true);
                        screen.set(AfkScreen::Board);
                        match post_start_status(HazardVariant::local_or_default()).await {
                            Ok(response) => {
                                handle_started_status(
                                    &status,
                                    &screen,
                                    &last_error,
                                    &start_transition_in_progress,
                                    response,
                                );
                            }
                            Err(error) => {
                                start_transition_in_progress.set(false);
                                screen.set(AfkScreen::Menu);
                                status.set(LoadState::Error(error));
                            }
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

    let now_ms = browser_now_ms();
    let socket_notice_visible =
        socket_notice_active && connection_notice_visible(*socket_notice_started_at_ms, now_ms);
    let chat_notice_visible =
        chat_notice_active && connection_notice_visible(*chat_notice_started_at_ms, now_ms);
    let idle_state = match &*status {
        LoadState::Ready(status) => afk_idle_state(status.session.as_ref(), now_ms),
        _ => None,
    };
    let websocket_status_notice = if socket_notice_visible {
        if let Some(retry_deadline_ms) = *socket_retry_deadline_ms {
            Some(AttrValue::from(afk_websocket_reconnect_notice(
                retry_deadline_ms.saturating_sub(now_ms),
            )))
        } else if *socket_reconnecting {
            Some(AttrValue::from(afk_websocket_connecting_notice()))
        } else {
            None
        }
    } else {
        None
    };
    let chat_status_notice = match &*status {
        LoadState::Ready(status) => {
            status_chat_notice(status, chat_notice_visible).map(AttrValue::from)
        }
        _ => None,
    };
    let face_status_notice = chat_status_notice
        .clone()
        .or_else(|| websocket_status_notice.clone());
    let current_overlay = active_face_overlay(
        *screen,
        &status,
        &manual_face_prompt,
        &face_notification,
        idle_state,
        face_status_notice.clone(),
    );
    let face_button_locked = current_overlay
        .as_ref()
        .is_some_and(|overlay| matches!(overlay, AfkFaceOverlay::Prompt(_)));
    let board_interactive =
        afk_board_is_interactive(&status, *socket_connected, face_button_locked);

    {
        let current_cell_state = current_cell_state.clone();
        let held_cell_preview = held_cell_preview.clone();
        use_effect_with(board_interactive, move |interactive| {
            if !*interactive && (*current_cell_state).is_some() {
                current_cell_state.set(None);
            }
            if !*interactive && (*held_cell_preview).is_some() {
                held_cell_preview.set(None);
            }
            || ()
        });
    }

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
                            status.set(LoadState::Ready(response));
                            if has_session {
                                last_error.set(None);
                                screen.set(AfkScreen::Board);
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

    let on_streamer_cell_event = {
        let current_cell_state = current_cell_state.clone();
        let held_cell_preview = held_cell_preview.clone();
        let status = status.clone();
        let last_error = last_error.clone();
        Callback::from(move |cell_msg: AfkCellMsg| {
            if !board_interactive {
                if (*current_cell_state).is_some() {
                    current_cell_state.set(None);
                }
                if (*held_cell_preview).is_some() {
                    held_cell_preview.set(None);
                }
                return;
            }

            let mut next_cell_state = *current_cell_state;
            let mut next_held_cell_preview = match cell_msg {
                AfkCellMsg::Update(AfkCellPointerState { buttons, .. }) if !buttons.is_empty() => {
                    None
                }
                _ => (*held_cell_preview).clone(),
            };
            let mut pending_request = None;
            update_cell_pointer_state(&mut next_cell_state, cell_msg, |pointer_state| {
                pending_request = match &*status {
                    LoadState::Ready(status) => status.session.as_ref().and_then(|session| {
                        request_for_streamer_pointer_release(
                            session,
                            pointer_state.pos,
                            pointer_state.buttons,
                        )
                    }),
                    _ => None,
                };
                next_held_cell_preview = pending_request.and_then(|_| match &*status {
                    LoadState::Ready(status) => {
                        held_afk_press_preview(status.session.as_ref(), pointer_state)
                    }
                    _ => None,
                });
                pending_request.is_some()
            });
            if next_cell_state != *current_cell_state {
                current_cell_state.set(next_cell_state);
            }
            if next_held_cell_preview != *held_cell_preview {
                held_cell_preview.set(next_held_cell_preview);
            }
            if let Some(request) = pending_request {
                let status = status.clone();
                let held_cell_preview = held_cell_preview.clone();
                let last_error = last_error.clone();
                spawn_local(async move {
                    match post_board_action(request).await {
                        Ok(response) => status.set(LoadState::Ready(response)),
                        Err(error) => {
                            held_cell_preview.set(None);
                            last_error.set(Some(error));
                        }
                    }
                });
            }
        })
    };

    let auth_error_html = props.auth_error.as_deref().map(render_auth_error);
    let menu_notice = face_status_notice
        .clone()
        .or_else(|| (*last_error).clone().map(AttrValue::from));
    let board_error = (*last_error).clone().map(AttrValue::from);

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
                                    menu_notice.clone(),
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
                                    menu_notice.clone(),
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
            let countdown_overlay = active_countdown_overlay(session);
            let round_report = active_round_report(session);
            let mines_left = mines_counter_text(session);
            let timer = board_counter_text(session);
            let visible_lives = visible_lives_count(session);
            let displayed_idle_state = if face_button_locked { None } else { idle_state };
            let game_state_icon = afk_face_icon(
                &status,
                if face_button_locked {
                    None
                } else {
                    (*face_notification).as_ref()
                },
                displayed_idle_state,
                !face_button_locked && face_status_notice.is_some(),
            );
            let face_button_title = if face_button_locked {
                "AFK prompt open"
            } else {
                "Open AFK submenu"
            };
            let timer_class = classes!("countdown-timer", afk_timer_phase_class(session));
            let board_pointer_state = if board_interactive {
                (*current_cell_state)
                    .or_else(|| held_afk_pointer_state((*held_cell_preview).as_ref(), session))
            } else {
                None
            };

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
                                visible_lives
                                    .map(view_lives_rail)
                                    .unwrap_or_default()
                            }
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
                                render_afk_board(
                                    session,
                                    hazard_variant,
                                    board_interactive,
                                    board_pointer_state,
                                    on_streamer_cell_event,
                                )
                            } else if *start_transition_in_progress {
                                html! { <div class="afk-board-note">{"Starting..."}</div> }
                            } else if matches!(&*status, LoadState::Loading | LoadState::Idle) {
                                html! { <div class="afk-board-note">{"Loading..."}</div> }
                            } else if matches!(&*status, LoadState::Ready(_)) {
                                html! { <div class="afk-board-note">{"Returning to menu..."}</div> }
                            } else {
                                Html::default()
                            }
                        }
                        {
                            countdown_overlay
                                .map(|overlay| render_countdown_overlay(overlay, *countdown_demo_show_after))
                                .or_else(|| {
                                    session.and_then(|session| {
                                        round_report.map(|report| {
                                            render_round_report_overlay(
                                                session,
                                                report,
                                                round_report_layout(session),
                                            )
                                        })
                                    })
                                })
                                .unwrap_or_default()
                        }
                    </div>
                    {
                        if let Some(error) = board_error {
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
    hazard_variant: HazardVariant,
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
    post_start_status(hazard_variant).await
}

async fn post_start_status(hazard_variant: HazardVariant) -> Result<AfkStatusResponse, String> {
    post_json_status(
        "/api/afk/start",
        &serde_json::json!({
            "hazard_variant": hazard_variant.to_afk_protocol(),
        }),
    )
    .await
}

async fn post_hazard_variant_status(
    hazard_variant: HazardVariant,
) -> Result<AfkStatusResponse, String> {
    post_json_status(
        "/api/afk/variant",
        &serde_json::json!({
            "hazard_variant": hazard_variant.to_afk_protocol(),
        }),
    )
    .await
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
        AfkActivityKind, AfkBoardSize, AfkBoardSnapshot, AfkChatConnectionState, AfkHazardVariant,
        AfkIdentity, AfkLossReason, AfkRoundReportSnapshot, AfkStatsGroupSnapshot,
        AfkTimerProfileSnapshot, AfkUserStatsSnapshot, FrontendRuntimeConfig, StreamerAuthStatus,
    };

    fn active_test_session(last_user_activity_at_ms: i64) -> AfkSessionSnapshot {
        AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::Active,
            paused: false,
            hazard_variant: AfkHazardVariant::Mines,
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
            lives_remaining: 3,
            max_lives: 3,
            game_over: false,
            round_report: None,
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

    fn countdown_test_session(board_size: AfkBoardSize) -> AfkSessionSnapshot {
        let (width, height, live_mines_left) = match board_size {
            AfkBoardSize::Tiny => (9, 9, 9),
            AfkBoardSize::Small => (16, 16, 20),
            AfkBoardSize::Medium => (24, 18, 36),
            AfkBoardSize::Large => (30, 20, 50),
        };
        AfkSessionSnapshot {
            phase: AfkRoundPhase::Countdown,
            board: AfkBoardSnapshot {
                width,
                height,
                cells: vec![AfkCellSnapshot::Hidden; usize::from(width) * usize::from(height)],
            },
            phase_countdown_secs: Some(5),
            current_level: 1,
            lives_remaining: 3,
            max_lives: 3,
            game_over: false,
            round_report: None,
            live_mines_left,
            ..active_test_session(1_000)
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

    fn sample_round_report() -> AfkRoundReportSnapshot {
        AfkRoundReportSnapshot {
            round_loser: None,
            round: AfkStatsGroupSnapshot {
                users: vec![
                    AfkUserStatsSnapshot {
                        chatter: AfkIdentity::new("1", "jan", "Jan"),
                        opened_cells: 4,
                        correct_flags: 2,
                        incorrect_flags: 0,
                        correct_unflags: 0,
                        died_this_round: false,
                        died_before_this_round: false,
                        died_every_round: false,
                    },
                    AfkUserStatsSnapshot {
                        chatter: AfkIdentity::new("2", "bea", "Bea"),
                        opened_cells: 1,
                        correct_flags: 0,
                        incorrect_flags: 1,
                        correct_unflags: 0,
                        died_this_round: false,
                        died_before_this_round: false,
                        died_every_round: false,
                    },
                ],
            },
            run: AfkStatsGroupSnapshot {
                users: vec![
                    AfkUserStatsSnapshot {
                        chatter: AfkIdentity::new("1", "jan", "Jan"),
                        opened_cells: 9,
                        correct_flags: 4,
                        incorrect_flags: 0,
                        correct_unflags: 1,
                        died_this_round: false,
                        died_before_this_round: false,
                        died_every_round: false,
                    },
                    AfkUserStatsSnapshot {
                        chatter: AfkIdentity::new("3", "zoe", "Zoe"),
                        opened_cells: 0,
                        correct_flags: 1,
                        incorrect_flags: 0,
                        correct_unflags: 0,
                        died_this_round: false,
                        died_before_this_round: false,
                        died_every_round: false,
                    },
                ],
            },
        }
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

    fn connected_streamer_status(
        session: Option<AfkSessionSnapshot>,
        chat_connection: AfkChatConnectionState,
    ) -> LoadState<AfkStatusResponse> {
        LoadState::Ready(AfkStatusResponse {
            auth: StreamerAuthStatus {
                identity: Some(AfkIdentity::new("1", "streamer", "Streamer")),
                ..StreamerAuthStatus::default()
            },
            chat_connection,
            websocket_path: Some("/ws/afk".into()),
            ..base_status(session)
        })
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
    fn board_screen_defaults_to_menu_when_not_restoring_view_state() {
        assert_eq!(initial_afk_screen(false), AfkScreen::Menu);
    }

    #[test]
    fn board_screen_stays_open_while_status_is_loading() {
        assert_eq!(
            resolve_board_screen(&LoadState::<AfkStatusResponse>::Loading, false),
            AfkBoardScreenResolution::Keep
        );
    }

    #[test]
    fn board_screen_finishes_start_transition_once_session_arrives() {
        assert_eq!(
            resolve_board_screen(&ready_status(active_test_session(1_000)), true),
            AfkBoardScreenResolution::FinishStartTransition
        );
    }

    #[test]
    fn board_screen_returns_to_menu_when_ready_status_has_no_session() {
        assert_eq!(
            resolve_board_screen(&LoadState::Ready(base_status(None)), false),
            AfkBoardScreenResolution::ReturnToMenu
        );
    }

    #[test]
    fn board_screen_stays_open_when_connection_issue_keeps_session_alive() {
        let status = AfkStatusResponse {
            chat_connection: AfkChatConnectionState::Error,
            ..base_status(Some(active_test_session(1_000)))
        };

        assert_eq!(
            resolve_board_screen(&LoadState::Ready(status), false),
            AfkBoardScreenResolution::Keep
        );
    }

    #[test]
    fn board_screen_keeps_restored_view_on_non_startup_errors() {
        assert_eq!(
            resolve_board_screen(&LoadState::<AfkStatusResponse>::Error("nope".into()), false),
            AfkBoardScreenResolution::Keep
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
    fn countdown_overlay_shows_only_during_countdown() {
        assert_eq!(
            active_countdown_overlay(Some(&countdown_test_session(AfkBoardSize::Medium))),
            Some(AFK_COUNTDOWN_OVERLAY)
        );

        for phase in [
            AfkRoundPhase::Active,
            AfkRoundPhase::Won,
            AfkRoundPhase::TimedOut,
            AfkRoundPhase::Stopped,
        ] {
            let mut session = active_test_session(1_000);
            session.phase = phase;
            assert_eq!(active_countdown_overlay(Some(&session)), None);
        }

        assert_eq!(active_countdown_overlay(None), None);
    }

    #[test]
    fn countdown_demo_target_state_lasts_twice_as_long() {
        assert_eq!(countdown_demo_step_ms(false), AFK_COUNTDOWN_DEMO_BEFORE_MS);
        assert_eq!(countdown_demo_step_ms(true), AFK_COUNTDOWN_DEMO_AFTER_MS);
        assert_eq!(
            countdown_demo_step_ms(true),
            countdown_demo_step_ms(false) * 2
        );
    }

    #[test]
    fn countdown_overlay_uses_expected_fixed_rows() {
        let overlay = active_countdown_overlay(Some(&countdown_test_session(AfkBoardSize::Medium)))
            .expect("countdown overlay should exist");

        assert_eq!(overlay.title, "How to play");
        assert_eq!(
            overlay.rows,
            [
                AfkCountdownDemoRow {
                    before: AfkCountdownDemoCell {
                        state: AfkCellSnapshot::Hidden,
                        code: Some("1A"),
                    },
                    after: AfkCountdownDemoCell {
                        state: AfkCellSnapshot::Revealed(0),
                        code: None,
                    },
                    command: "1a",
                    description: "open",
                },
                AfkCountdownDemoRow {
                    before: AfkCountdownDemoCell {
                        state: AfkCellSnapshot::Hidden,
                        code: Some("5C"),
                    },
                    after: AfkCountdownDemoCell {
                        state: AfkCellSnapshot::Flagged,
                        code: None,
                    },
                    command: "!f 5c",
                    description: "flag",
                },
                AfkCountdownDemoRow {
                    before: AfkCountdownDemoCell {
                        state: AfkCellSnapshot::Flagged,
                        code: Some("2E"),
                    },
                    after: AfkCountdownDemoCell {
                        state: AfkCellSnapshot::Hidden,
                        code: None,
                    },
                    command: "!u 2e",
                    description: "unflag",
                },
            ]
        );
        assert_eq!(
            overlay.aria_label,
            "Chat commands: 1a opens, !f 5c flags, !u 2e unflags. Letter case does not matter."
        );
    }

    #[test]
    fn countdown_overlay_keeps_same_copy_on_tiny_boards() {
        assert_eq!(
            active_countdown_overlay(Some(&countdown_test_session(AfkBoardSize::Tiny))),
            Some(AFK_COUNTDOWN_OVERLAY)
        );
    }

    #[test]
    fn visible_lives_count_matches_wide_board_capacity() {
        let session = active_test_session(1_000);
        assert_eq!(visible_lives_count(Some(&session)), Some(3));

        let narrow = countdown_test_session(AfkBoardSize::Tiny);
        assert_eq!(visible_lives_count(Some(&narrow)), None);
    }

    #[test]
    fn round_report_layout_uses_total_only_on_tiny_boards() {
        assert_eq!(
            round_report_layout(&countdown_test_session(AfkBoardSize::Tiny)),
            AfkRoundReportLayout::TotalOnly
        );
        assert_eq!(
            round_report_layout(&countdown_test_session(AfkBoardSize::Small)),
            AfkRoundReportLayout::SideBySide
        );
    }

    #[test]
    fn round_report_max_rows_scales_with_board_size() {
        assert_eq!(
            round_report_max_rows(&countdown_test_session(AfkBoardSize::Tiny)),
            2
        );
        assert_eq!(
            round_report_max_rows(&countdown_test_session(AfkBoardSize::Small)),
            3
        );
        assert_eq!(
            round_report_max_rows(&countdown_test_session(AfkBoardSize::Medium)),
            4
        );
        assert_eq!(
            round_report_max_rows(&countdown_test_session(AfkBoardSize::Large)),
            5
        );
    }

    #[test]
    fn displayed_report_users_filters_zero_rows_and_caps_at_requested_limit() {
        let group = AfkStatsGroupSnapshot {
            users: vec![
                AfkUserStatsSnapshot {
                    chatter: AfkIdentity::new("1", "jan", "Jan"),
                    opened_cells: 5,
                    correct_flags: 1,
                    incorrect_flags: 0,
                    correct_unflags: 0,
                    died_this_round: false,
                    died_before_this_round: false,
                    died_every_round: false,
                },
                AfkUserStatsSnapshot {
                    chatter: AfkIdentity::new("2", "bea", "Bea"),
                    opened_cells: 0,
                    correct_flags: 0,
                    incorrect_flags: 2,
                    correct_unflags: 0,
                    died_this_round: false,
                    died_before_this_round: false,
                    died_every_round: false,
                },
                AfkUserStatsSnapshot {
                    chatter: AfkIdentity::new("3", "zoe", "Zoe"),
                    opened_cells: 3,
                    correct_flags: 0,
                    incorrect_flags: 0,
                    correct_unflags: 0,
                    died_this_round: false,
                    died_before_this_round: false,
                    died_every_round: false,
                },
                AfkUserStatsSnapshot {
                    chatter: AfkIdentity::new("4", "max", "Max"),
                    opened_cells: 1,
                    correct_flags: 2,
                    incorrect_flags: 0,
                    correct_unflags: 0,
                    died_this_round: false,
                    died_before_this_round: false,
                    died_every_round: false,
                },
                AfkUserStatsSnapshot {
                    chatter: AfkIdentity::new("5", "ivy", "Ivy"),
                    opened_cells: 1,
                    correct_flags: 1,
                    incorrect_flags: 0,
                    correct_unflags: 0,
                    died_this_round: false,
                    died_before_this_round: false,
                    died_every_round: false,
                },
            ],
        };

        assert_eq!(
            displayed_report_users(&group, 3)
                .into_iter()
                .map(|user| user.chatter.display_name)
                .collect::<Vec<_>>(),
            vec!["Jan", "Bea", "Zoe"]
        );
    }

    #[test]
    fn round_report_user_icon_is_woozy_for_wrong_flags_only_rows() {
        let user = AfkUserStatsSnapshot {
            chatter: AfkIdentity::new("2", "bea", "Bea"),
            opened_cells: 0,
            correct_flags: 0,
            incorrect_flags: 2,
            correct_unflags: 0,
            died_this_round: false,
            died_before_this_round: false,
            died_every_round: false,
        };

        assert_eq!(round_report_user_icon_name(&user), "woozy");
        assert_eq!(total_report_user_icon_name(false, &user), "woozy");
    }

    #[test]
    fn round_report_user_icon_is_win_decent_for_mixed_mistake_rows() {
        let user = AfkUserStatsSnapshot {
            chatter: AfkIdentity::new("2", "bea", "Bea"),
            opened_cells: 1,
            correct_flags: 0,
            incorrect_flags: 1,
            correct_unflags: 0,
            died_this_round: false,
            died_before_this_round: false,
            died_every_round: false,
        };

        assert_eq!(round_report_user_icon_name(&user), "win-decent");
    }

    #[test]
    fn total_report_user_icon_is_in_progress_after_prior_death() {
        let user = AfkUserStatsSnapshot {
            chatter: AfkIdentity::new("2", "bea", "Bea"),
            opened_cells: 3,
            correct_flags: 1,
            incorrect_flags: 0,
            correct_unflags: 0,
            died_this_round: false,
            died_before_this_round: true,
            died_every_round: false,
        };

        assert_eq!(total_report_user_icon_name(false, &user), "in-progress");
    }

    #[test]
    fn total_report_user_icon_is_lose_for_current_round_death() {
        let user = AfkUserStatsSnapshot {
            chatter: AfkIdentity::new("2", "bea", "Bea"),
            opened_cells: 3,
            correct_flags: 1,
            incorrect_flags: 0,
            correct_unflags: 0,
            died_this_round: true,
            died_before_this_round: false,
            died_every_round: false,
        };

        assert_eq!(total_report_user_icon_name(false, &user), "lose");
    }

    #[test]
    fn total_report_user_icon_is_instant_loss_after_death_every_round() {
        let user = AfkUserStatsSnapshot {
            chatter: AfkIdentity::new("2", "bea", "Bea"),
            opened_cells: 3,
            correct_flags: 1,
            incorrect_flags: 0,
            correct_unflags: 0,
            died_this_round: true,
            died_before_this_round: false,
            died_every_round: true,
        };

        assert_eq!(total_report_user_icon_name(true, &user), "instant-loss");
    }

    #[test]
    fn active_round_report_requires_finished_round() {
        let mut session = active_test_session(1_000);
        session.round_report = Some(sample_round_report());
        assert_eq!(active_round_report(Some(&session)), None);

        session.phase = AfkRoundPhase::Won;
        assert_eq!(
            active_round_report(Some(&session)),
            session.round_report.as_ref()
        );
    }

    #[test]
    fn should_show_cell_code_uses_snapshot_label_mask_when_present() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::Active,
            paused: false,
            hazard_variant: AfkHazardVariant::Mines,
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
            lives_remaining: 3,
            max_lives: 3,
            game_over: false,
            round_report: None,
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
            hazard_variant: AfkHazardVariant::Mines,
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
            lives_remaining: 3,
            max_lives: 3,
            game_over: false,
            round_report: None,
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
    fn streamer_release_actions_follow_normal_mouseup_mapping() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::Active,
            paused: false,
            hazard_variant: AfkHazardVariant::Mines,
            board: AfkBoardSnapshot {
                width: 2,
                height: 2,
                cells: vec![
                    AfkCellSnapshot::Hidden,
                    AfkCellSnapshot::Flagged,
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
            lives_remaining: 3,
            max_lives: 3,
            game_over: false,
            round_report: None,
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

        assert_eq!(
            request_for_streamer_pointer_release(&session, (0, 0), MouseButtons::LEFT),
            Some(AfkActionRequest {
                kind: AfkActionKind::Reveal,
                x: 0,
                y: 0,
            })
        );
        assert_eq!(
            request_for_streamer_pointer_release(&session, (1, 0), MouseButtons::RIGHT),
            Some(AfkActionRequest {
                kind: AfkActionKind::ToggleFlag,
                x: 1,
                y: 0,
            })
        );
        assert_eq!(
            request_for_streamer_pointer_release(&session, (0, 1), MouseButtons::LEFT),
            Some(AfkActionRequest {
                kind: AfkActionKind::Chord,
                x: 0,
                y: 1,
            })
        );
        assert_eq!(
            request_for_streamer_pointer_release(&session, (0, 1), MouseButtons::RIGHT),
            Some(AfkActionRequest {
                kind: AfkActionKind::ChordFlag,
                x: 0,
                y: 1,
            })
        );
    }

    #[test]
    fn afk_pressed_preview_matches_left_button_feedback() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::Active,
            paused: false,
            hazard_variant: AfkHazardVariant::Mines,
            board: AfkBoardSnapshot {
                width: 3,
                height: 1,
                cells: vec![
                    AfkCellSnapshot::Hidden,
                    AfkCellSnapshot::Revealed(1),
                    AfkCellSnapshot::Flagged,
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
            lives_remaining: 3,
            max_lives: 3,
            game_over: false,
            round_report: None,
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

        assert!(afk_is_pressed(
            &session,
            (0, 0),
            AfkCellSnapshot::Hidden,
            Some(AfkCellPointerState {
                pos: (1, 0),
                buttons: MouseButtons::LEFT,
            }),
        ));
        assert!(!afk_is_pressed(
            &session,
            (0, 0),
            AfkCellSnapshot::Hidden,
            Some(AfkCellPointerState {
                pos: (1, 0),
                buttons: MouseButtons::RIGHT,
            }),
        ));
    }

    #[test]
    fn held_press_preview_only_survives_while_session_is_unchanged() {
        let session = active_test_session(1_000);
        let preview = held_afk_press_preview(
            Some(&session),
            AfkCellPointerState {
                pos: (3, 4),
                buttons: MouseButtons::LEFT,
            },
        )
        .expect("left-button release should create a held preview");

        assert_eq!(
            held_afk_pointer_state(Some(&preview), Some(&session)),
            Some(AfkCellPointerState {
                pos: (3, 4),
                buttons: MouseButtons::LEFT,
            })
        );

        let mut next_session = session.clone();
        next_session.current_level = next_session.current_level.saturating_add(1);
        assert_eq!(
            held_afk_pointer_state(Some(&preview), Some(&next_session)),
            None
        );
    }

    #[test]
    fn held_press_preview_is_not_kept_for_right_clicks() {
        let session = active_test_session(1_000);

        assert_eq!(
            held_afk_press_preview(
                Some(&session),
                AfkCellPointerState {
                    pos: (0, 0),
                    buttons: MouseButtons::RIGHT,
                },
            ),
            None
        );
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
                hazard_variant: AfkHazardVariant::Mines,
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
                lives_remaining: 3,
                max_lives: 3,
                game_over: false,
                round_report: None,
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

        assert_eq!(afk_face_icon(&status, None, None, false), "sleeping");
    }

    #[test]
    fn countdown_face_icon_uses_upside_down_face() {
        let session = countdown_test_session(AfkBoardSize::Medium);

        assert_eq!(
            afk_face_icon(&ready_status(session), None, None, false),
            "starting-soon"
        );
    }

    #[test]
    fn timer_loss_prompt_uses_too_slow_message() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::TimedOut,
            paused: false,
            hazard_variant: AfkHazardVariant::Mines,
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
            lives_remaining: 3,
            max_lives: 3,
            game_over: false,
            round_report: None,
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

        assert_eq!(loss_prompt_message(&session), "Too slow! Retry level? (60)");
    }

    #[test]
    fn mine_loss_prompt_keeps_too_bad_message() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::TimedOut,
            paused: false,
            hazard_variant: AfkHazardVariant::Mines,
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
            lives_remaining: 3,
            max_lives: 3,
            game_over: false,
            round_report: None,
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

        assert_eq!(loss_prompt_message(&session), "Too bad. Retry level? (60)");
    }

    #[test]
    fn game_over_prompt_uses_start_over_copy() {
        let mut session = active_test_session(1_000);
        session.phase = AfkRoundPhase::TimedOut;
        session.phase_countdown_secs = Some(60);
        session.loss_reason = Some(AfkLossReason::Mine);
        session.current_level = 4;
        session.live_mines_left = 0;
        session.lives_remaining = 0;
        session.game_over = true;

        assert_eq!(loss_prompt_message(&session), "Game over. Start over? (60)");
        assert_eq!(
            loss_continue_prompt(&session).choices[0].title,
            AttrValue::from("Start a new run from level 1")
        );
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
                false,
            ),
            "dejected"
        );
    }

    #[test]
    fn face_icon_uses_spiral_eyes_for_status_notices() {
        assert_eq!(
            afk_face_icon(&LoadState::<AfkStatusResponse>::Idle, None, None, true),
            "spiral-eyes"
        );
    }

    #[test]
    fn idle_state_uses_yawning_face_after_three_minutes() {
        let session = active_test_session(1_000);
        let idle_state = afk_idle_state(Some(&session), 1_000 + AFK_IDLE_SLEEPING_THRESHOLD_MS);

        assert_eq!(idle_state, Some(AfkIdleState::Sleeping));
        assert_eq!(
            afk_face_icon(&ready_status(session), None, idle_state, false),
            "yawning"
        );
    }

    #[test]
    fn idle_prompt_uses_yawning_face_after_ten_minutes() {
        let session = active_test_session(1_000);
        let idle_state = afk_idle_state(Some(&session), 1_000 + AFK_IDLE_PROMPT_THRESHOLD_MS);

        assert_eq!(idle_state, Some(AfkIdleState::Prompt));
        assert_eq!(
            afk_face_icon(&ready_status(session), None, idle_state, false),
            "yawning"
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
            None,
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
    fn websocket_reconnect_backoff_caps_at_one_minute() {
        assert_eq!(next_afk_reconnect_delay_ms(1), 1_000);
        assert_eq!(next_afk_reconnect_delay_ms(2), 2_000);
        assert_eq!(next_afk_reconnect_delay_ms(3), 4_000);
        assert_eq!(next_afk_reconnect_delay_ms(4), 8_000);
        assert_eq!(next_afk_reconnect_delay_ms(5), 16_000);
        assert_eq!(next_afk_reconnect_delay_ms(6), 32_000);
        assert_eq!(next_afk_reconnect_delay_ms(7), 60_000);
        assert_eq!(next_afk_reconnect_delay_ms(99), 60_000);
    }

    #[test]
    fn websocket_reconnect_notice_uses_plain_count() {
        assert_eq!(
            afk_websocket_reconnect_notice(4_000),
            "Reconnecting in 4..."
        );
    }

    #[test]
    fn connection_notice_visibility_waits_for_two_second_grace_period() {
        assert!(!connection_notice_visible(None, 3_000));
        assert!(!connection_notice_visible(Some(1_000), 2_999));
        assert!(connection_notice_visible(Some(1_000), 3_000));
    }

    #[test]
    fn connection_notice_refresh_delay_counts_down_to_grace_period() {
        assert_eq!(
            next_connection_notice_refresh_delay_ms(Some(1_000), 1_500),
            Some(1_500)
        );
        assert_eq!(
            next_connection_notice_refresh_delay_ms(Some(1_000), 2_999),
            Some(1)
        );
        assert_eq!(
            next_connection_notice_refresh_delay_ms(Some(1_000), 3_000),
            None
        );
    }

    #[test]
    fn afk_board_waits_for_live_connection_before_becoming_interactive() {
        let countdown = countdown_test_session(AfkBoardSize::Medium);

        assert!(!afk_board_is_interactive(
            &connected_streamer_status(Some(countdown.clone()), AfkChatConnectionState::Connected,),
            false,
            false,
        ));
        assert!(!afk_board_is_interactive(
            &connected_streamer_status(Some(countdown.clone()), AfkChatConnectionState::Connecting,),
            true,
            false,
        ));
        assert!(afk_board_is_interactive(
            &connected_streamer_status(Some(countdown), AfkChatConnectionState::Connected),
            true,
            false,
        ));
    }

    #[test]
    fn afk_board_still_respects_pause_phase_and_prompt_locks() {
        let mut paused = active_test_session(1_000);
        paused.paused = true;
        assert!(!afk_board_is_interactive(
            &connected_streamer_status(Some(paused), AfkChatConnectionState::Connected),
            true,
            false,
        ));

        let mut timed_out = active_test_session(1_000);
        timed_out.phase = AfkRoundPhase::TimedOut;
        assert!(!afk_board_is_interactive(
            &connected_streamer_status(Some(timed_out), AfkChatConnectionState::Connected),
            true,
            false,
        ));

        assert!(!afk_board_is_interactive(
            &connected_streamer_status(
                Some(active_test_session(1_000)),
                AfkChatConnectionState::Connected,
            ),
            true,
            true,
        ));
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
            None,
        );

        assert_eq!(
            overlay,
            Some(AfkFaceOverlay::Prompt(AfkFacePrompt {
                message: "Too bad. Retry level? (60)".into(),
                choices: vec![
                    AfkFaceChoice {
                        label: "Yes (!continue)".into(),
                        title: "Retry the current level".into(),
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
            coord: Some(AfkCoordSnapshot { x: 0, y: 0 }),
        };

        assert_eq!(
            face_notification_event(&row, HazardVariant::Mines),
            Some(AfkFaceNotificationEvent {
                message: "Jan hit a mine at 1A".into(),
                timeout_ms: AFK_FACE_NOTIFICATION_MS,
            })
        );
    }

    #[test]
    fn mine_hit_activity_formats_flower_notification() {
        let row = AfkActivityRow {
            at_ms: 1_234,
            text: "Jan hit a mine at 1A".into(),
            kind: AfkActivityKind::MineHit,
            actor: Some(AfkIdentity::new("1", "jan", "Jan")),
            coord: Some(AfkCoordSnapshot { x: 0, y: 0 }),
        };

        assert_eq!(
            face_notification_event(&row, HazardVariant::Flowers),
            Some(AfkFaceNotificationEvent {
                message: "Jan stepped on a flower at 1A".into(),
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
            coord: None,
        };

        assert_eq!(
            face_notification_event(&row, HazardVariant::Mines),
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
            hazard_variant: AfkHazardVariant::Mines,
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
            lives_remaining: 3,
            max_lives: 3,
            game_over: false,
            round_report: None,
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
    fn status_notices_override_face_notifications() {
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
            Some("Chat reconnecting...".into()),
        );

        assert_eq!(
            overlay,
            Some(AfkFaceOverlay::Message {
                message: "Chat reconnecting...".into(),
                status: Some("Level 3".into()),
            })
        );
    }

    #[test]
    fn chat_reconnect_notice_waits_for_grace_period() {
        let status = AfkStatusResponse {
            chat_connection: AfkChatConnectionState::Connecting,
            ..base_status(Some(active_test_session(1_000)))
        };

        assert_eq!(status_chat_notice(&status, false), None);
        assert_eq!(
            status_chat_notice(&status, true),
            Some("Chat reconnecting...".into())
        );
    }

    #[test]
    fn chat_error_notice_is_not_delayed() {
        let status = AfkStatusResponse {
            chat_connection: AfkChatConnectionState::Error,
            ..base_status(Some(active_test_session(1_000)))
        };

        assert_eq!(
            status_chat_notice(&status, false),
            Some("Chat unavailable.".into())
        );
    }

    #[test]
    fn win_face_icon_uses_decent_variant_after_mine_hits() {
        let session = AfkSessionSnapshot {
            streamer: None,
            phase: AfkRoundPhase::Won,
            paused: false,
            hazard_variant: AfkHazardVariant::Mines,
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
            lives_remaining: 3,
            max_lives: 3,
            game_over: false,
            round_report: None,
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
            hazard_variant: AfkHazardVariant::Mines,
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
            lives_remaining: 3,
            max_lives: 3,
            game_over: false,
            round_report: None,
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
                    hazard_variant: AfkHazardVariant::Mines,
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
                    lives_remaining: 3,
                    max_lives: 3,
                    game_over: false,
                    round_report: None,
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
                    hazard_variant: AfkHazardVariant::Mines,
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
                    lives_remaining: 3,
                    max_lives: 3,
                    game_over: false,
                    round_report: None,
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
                    hazard_variant: AfkHazardVariant::Mines,
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
                    lives_remaining: 3,
                    max_lives: 3,
                    game_over: false,
                    round_report: None,
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
                    hazard_variant: AfkHazardVariant::Mines,
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
                    lives_remaining: 3,
                    max_lives: 3,
                    game_over: false,
                    round_report: None,
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
            None,
        );

        assert_eq!(
            overlay,
            Some(AfkFaceOverlay::Prompt(AfkFacePrompt {
                message: "Too bad. Retry level? (60)".into(),
                choices: vec![
                    AfkFaceChoice {
                        label: "Yes (!continue)".into(),
                        title: "Retry the current level".into(),
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
