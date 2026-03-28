use detonito_protocol::{
    AfkActionKind, AfkActionRequest, AfkCellSnapshot, AfkChatConnectionState, AfkClientMessage,
    AfkLossReason, AfkRoundPhase, AfkServerMessage, AfkSessionSnapshot, AfkStatusResponse,
};
use js_sys::encode_uri_component;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Event, MessageEvent, Request, RequestCredentials, RequestInit, Response, WebSocket};
use yew::prelude::*;

use crate::menu::{menu_blank_row, menu_header_row, menu_icon_button, menu_nav_enter_button};
use crate::runtime::{AppRoute, app_path, auth_return_to, frontend_runtime_config, websocket_path};
use crate::sprites::{Glyph, GlyphRun, GlyphSet, Icon, IconCrop, SpriteDefs};
use crate::utils::format_for_counter;

#[derive(Properties, PartialEq)]
pub(crate) struct AfkViewProps {
    pub on_menu: Callback<()>,
    #[prop_or_default]
    pub auth_error: Option<String>,
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

fn current_level_status_text(session: Option<&AfkSessionSnapshot>) -> Option<AttrValue> {
    session.map(|session| format!("Level {}", session.current_level).into())
}

fn menu_toggle_icon_button(
    icon: &'static str,
    title: impl Into<AttrValue>,
    selected: bool,
    disabled: bool,
    onclick: Callback<MouseEvent>,
) -> Html {
    html! {
        <button class={classes!(selected.then_some("pressed"))} {disabled} {onclick} title={title.into()}>
            <Icon name={icon} crop={IconCrop::CenteredSquare64} class={classes!("button-icon")}/>
        </button>
    }
}

fn menu_primary_row(label: impl Into<AttrValue>, button: Html) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-button-slot">{button}</td>
            <td class="menu-text" colspan="11">{label.into()}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

fn menu_toggle_row(label: impl Into<AttrValue>, left_button: Html, right_button: Html) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-text" colspan="9">{label.into()}</td>
            <td class="menu-button-slot">{left_button}</td>
            <td class="menu-button-slot">{right_button}</td>
            <td class="menu-pad"/>
            <td class="menu-pad"/>
        </tr>
    }
}

fn menu_copy_row(text: impl Into<AttrValue>) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-about-copy" colspan="12">{text.into()}</td>
            <td class="menu-pad"/>
        </tr>
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

fn afk_connect_href(base: String) -> String {
    let separator = if base.contains('?') { '&' } else { '?' };
    let afk_return_to = auth_return_to(AppRoute::Afk);
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

fn win_continue_prompt(secs: i32) -> AfkFacePrompt {
    AfkFacePrompt {
        message: format!("Nice! Next level? ({})", secs.max(0)).into(),
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

fn loss_continue_prompt(secs: i32) -> AfkFacePrompt {
    AfkFacePrompt {
        message: format!("Too bad. Play again? ({})", secs.max(0)).into(),
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

fn active_face_overlay(
    screen: AfkScreen,
    status: &LoadState<AfkStatusResponse>,
    manual_prompt: &Option<AfkFacePrompt>,
) -> Option<AfkFaceOverlay> {
    if matches!(screen, AfkScreen::Board) {
        if let Some(prompt) = manual_prompt.clone() {
            return Some(AfkFaceOverlay::Prompt(prompt));
        }
        if let LoadState::Ready(status) = status {
            if let Some(session) = &status.session {
                let level_status = current_level_status_text(Some(session));
                return match session.phase {
                    AfkRoundPhase::Countdown => Some(AfkFaceOverlay::Message {
                        message: format!(
                            "Starting in {}...",
                            session.phase_countdown_secs.unwrap_or_default().max(0)
                        )
                        .into(),
                        status: level_status,
                    }),
                    AfkRoundPhase::Won => Some(AfkFaceOverlay::Prompt(win_continue_prompt(
                        session.phase_countdown_secs.unwrap_or_default(),
                    ))),
                    AfkRoundPhase::TimedOut => Some(AfkFaceOverlay::Prompt(loss_continue_prompt(
                        session.phase_countdown_secs.unwrap_or_default(),
                    ))),
                    _ => level_status.map(AfkFaceOverlay::Status),
                };
            }
        }
        return None;
    }

    None
}

fn afk_face_icon(status: &LoadState<AfkStatusResponse>) -> &'static str {
    match status {
        LoadState::Loading => "mid-open",
        LoadState::Ready(status) => match status.session.as_ref().map(|session| session.phase) {
            Some(AfkRoundPhase::Countdown) => "not-started",
            Some(AfkRoundPhase::Active) => "in-progress",
            Some(AfkRoundPhase::Won) => "win",
            Some(AfkRoundPhase::TimedOut)
                if status
                    .session
                    .as_ref()
                    .is_some_and(|session| session.loss_reason == Some(AfkLossReason::Timer)) =>
            {
                "sleeping"
            }
            Some(AfkRoundPhase::TimedOut) => "lose",
            Some(AfkRoundPhase::Stopped) | None => "not-started",
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
    format!("{row}{column}")
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

fn should_show_cell_code(
    session: &AfkSessionSnapshot,
    x: usize,
    y: usize,
    cell: AfkCellSnapshot,
) -> bool {
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
    let manual_face_prompt = use_state_eq(|| None::<AfkFacePrompt>);
    let last_error = use_state_eq(|| None::<String>);
    let socket_path = match &*status {
        LoadState::Ready(status) if status.auth.identity.is_some() => status.websocket_path.clone(),
        _ => None,
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
        let status = status.clone();
        let last_error = last_error.clone();
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
                                    Ok(AfkServerMessage::Activity { .. }) => {}
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
        let last_error = last_error.clone();
        use_effect_with(status.clone(), move |status| {
            if matches!(*screen, AfkScreen::Board) {
                let ready_status = match &**status {
                    LoadState::Ready(status) => Some(status),
                    _ => None,
                };
                let has_session = ready_status.is_some_and(|status| status.session.is_some());
                if !has_session {
                    manual_face_prompt.set(None);
                    screen.set(AfkScreen::Menu);
                } else if let Some(status) = ready_status {
                    if has_critical_chat_failure(status) {
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

    let go_to_main_menu = {
        let manual_face_prompt = manual_face_prompt.clone();
        let on_menu = props.on_menu.clone();
        Callback::from(move |_: MouseEvent| {
            manual_face_prompt.set(None);
            on_menu.emit(());
        })
    };

    let resume_board = {
        let screen = screen.clone();
        let manual_face_prompt = manual_face_prompt.clone();
        let status = status.clone();
        let last_error = last_error.clone();
        Callback::from(move |_| {
            manual_face_prompt.set(None);
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
        let manual_face_prompt = manual_face_prompt.clone();
        let last_error = last_error.clone();
        Callback::from(move |_| {
            manual_face_prompt.set(None);
            let status = status.clone();
            let screen = screen.clone();
            let last_error = last_error.clone();
            spawn_local(async move {
                let _ = post_action("/api/afk/start").await;
                match fetch_status().await {
                    Ok(response) => {
                        let has_session = response.session.is_some();
                        let can_open_board = has_session
                            && !matches!(response.chat_connection, AfkChatConnectionState::Error);
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
        })
    };

    let disconnect_twitch = {
        let status = status.clone();
        let screen = screen.clone();
        let manual_face_prompt = manual_face_prompt.clone();
        Callback::from(move |_| {
            manual_face_prompt.set(None);
            screen.set(AfkScreen::Menu);
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

    let connect_twitch = {
        let href = match &*status {
            LoadState::Ready(status) => status
                .connect_url
                .clone()
                .unwrap_or_else(|| app_path("/auth/twitch/login")),
            _ => app_path("/auth/twitch/login"),
        };
        let href = afk_connect_href(href);
        Callback::from(move |_| {
            let _ = gloo::utils::window().location().set_href(&href);
        })
    };

    let set_timeout_on = {
        let status = status.clone();
        Callback::from(move |_| {
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
        Callback::from(move |_| {
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

    let current_overlay = active_face_overlay(*screen, &status, &manual_face_prompt);
    let face_button_locked = current_overlay
        .as_ref()
        .is_some_and(|overlay| matches!(overlay, AfkFaceOverlay::Prompt(_)));

    let on_face_button = {
        let manual_face_prompt = manual_face_prompt.clone();
        let face_button_locked = face_button_locked;
        Callback::from(move |e: MouseEvent| {
            e.stop_propagation();
            if !face_button_locked {
                manual_face_prompt.set(Some(board_menu_prompt()));
            }
        })
    };

    let on_face_action = {
        let manual_face_prompt = manual_face_prompt.clone();
        let screen = screen.clone();
        let status = status.clone();
        let last_error = last_error.clone();
        Callback::from(move |action: AfkFaceAction| match action {
            AfkFaceAction::DismissPrompt => {
                manual_face_prompt.set(None);
            }
            AfkFaceAction::OpenSubmenu => {
                manual_face_prompt.set(None);
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
                        {menu_blank_row()}
                        {menu_copy_row("AFK mode is unavailable in this build.")}
                    </>
                },
                LoadState::Loading => html! {
                    <>
                        {menu_blank_row()}
                        {menu_copy_row("Loading AFK status…")}
                    </>
                },
                LoadState::Error(error) => html! {
                    <>
                        {menu_blank_row()}
                        {menu_copy_row(error.clone())}
                    </>
                },
                LoadState::Ready(status) if status.auth.identity.is_none() => html! {
                    <>
                        {menu_blank_row()}
                        {
                            if let Some(error) = auth_error_html.clone() {
                                menu_copy_row(error)
                            } else {
                                Html::default()
                            }
                        }
                        {
                            if let Some(error) = status_error.clone().or_else(|| websocket_error.clone()) {
                                menu_copy_row(error)
                            } else {
                                Html::default()
                            }
                        }
                        {menu_primary_row(
                            "Connect Twitch",
                            menu_nav_enter_button("Connect with Twitch", false, connect_twitch),
                        )}
                    </>
                },
                LoadState::Ready(status) => html! {
                    <>
                        {menu_blank_row()}
                        {
                            if let Some(error) = auth_error_html.clone() {
                                menu_copy_row(error)
                            } else {
                                Html::default()
                            }
                        }
                        {
                            if let Some(error) = status_error.clone().or_else(|| websocket_error.clone()) {
                                menu_copy_row(error)
                            } else {
                                Html::default()
                            }
                        }
                        {
                            if status.session.is_some() {
                                html! {
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
                                        {menu_toggle_row(
                                            "Timeout on mistake",
                                            menu_toggle_icon_button(
                                                "ok",
                                                "Enable timeout on mistake",
                                                status.timeout_enabled,
                                                !status.timeout_supported,
                                                set_timeout_on.clone(),
                                            ),
                                            menu_toggle_icon_button(
                                                "cancel",
                                                "Disable timeout on mistake",
                                                !status.timeout_enabled,
                                                !status.timeout_supported,
                                                set_timeout_off.clone(),
                                            ),
                                        )}
                                    </>
                                }
                            } else {
                                html! {
                                    <>
                                        {menu_primary_row(
                                            "Start",
                                            menu_nav_enter_button(
                                                "Start AFK mode",
                                                false,
                                                start_new_board.clone(),
                                            ),
                                        )}
                                        {menu_toggle_row(
                                            "Timeout on mistake",
                                            menu_toggle_icon_button(
                                                "ok",
                                                "Enable timeout on mistake",
                                                status.timeout_enabled,
                                                !status.timeout_supported,
                                                set_timeout_on.clone(),
                                            ),
                                            menu_toggle_icon_button(
                                                "cancel",
                                                "Disable timeout on mistake",
                                                !status.timeout_enabled,
                                                !status.timeout_supported,
                                                set_timeout_off.clone(),
                                            ),
                                        )}
                                    </>
                                }
                            }
                        }
                        {menu_blank_row()}
                        {menu_primary_row(
                            "Disconnect Twitch",
                            menu_icon_button("cancel", "Disconnect Twitch", false, disconnect_twitch),
                        )}
                    </>
                },
                LoadState::Idle => html! {
                    <>
                        {menu_blank_row()}
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
                                {menu_blank_row()}
                                {menu_header_row("AFK Mode", go_to_main_menu)}
                                {body}
                                {menu_blank_row()}
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
            let game_state_icon = afk_face_icon(&status);
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

async fn post_board_action(request_body: AfkActionRequest) -> Result<AfkStatusResponse, String> {
    post_json_status("/api/afk/action", &request_body).await
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
        AfkBoardSnapshot, AfkChatConnectionState, AfkLossReason, AfkTimerProfileSnapshot,
        FrontendRuntimeConfig, StreamerAuthStatus,
    };

    #[test]
    fn face_icon_uses_sleeping_face_for_timer_losses() {
        let status = LoadState::Ready(AfkStatusResponse {
            runtime: FrontendRuntimeConfig { afk_enabled: true },
            auth: StreamerAuthStatus::default(),
            chat_connection: AfkChatConnectionState::Idle,
            chat_error: None,
            timeout_supported: true,
            timeout_enabled: true,
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
            }),
        });

        assert_eq!(afk_face_icon(&status), "sleeping");
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
                connect_url: None,
                websocket_path: None,
                session: Some(AfkSessionSnapshot {
                    streamer: None,
                    phase: AfkRoundPhase::Active,
                    paused: false,
                    board: AfkBoardSnapshot {
                        width: 0,
                        height: 0,
                        cells: Vec::new(),
                    },
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
                }),
            }),
            &None,
        );

        assert_eq!(overlay, Some(AfkFaceOverlay::Status("Level 3".into())));
    }
}
