mod auth;

use std::cell::RefCell;
use std::mem;
use std::time::Duration;

use auth::{
    AuthError, SignedAuthClaims, auth_cookie_header, auth_token_from_authorization_header,
    auth_token_from_cookie_header, build_twitch_authorize_url, cleared_auth_cookie_header,
    complete_twitch_callback, refreshed_auth_claims, should_refresh_auth_token, sign_auth_token,
    verify_auth_token, verify_oauth_state,
};
use barbed::cloudflare_worker::{
    create_eventsub_subscription, delete_eventsub_subscription, exchange_twitch_code,
    list_eventsub_subscriptions, refresh_access_token, send_prepared_request,
};
use barbed::eventsub::{
    CHANNEL_CHAT_MESSAGE, EventSubChatMessage, EventSubMessageType, EventSubWebSocketEnvelope,
    EventSubWebSocketSession, chat_message_subscription_request, decode_eventsub_websocket_message,
};
use barbed::helix::{HttpMethod, PreparedRequest};
use barbed::oauth::{
    TwitchAuthOutcome, TwitchTokenState, refreshed_twitch_token_state, should_refresh_twitch_token,
};
use detonito_core::{
    AfkAction, AfkBoardSize as CoreAfkBoardSize, AfkCellState as CoreAfkCellState, AfkEngine,
    AfkLossReason as CoreAfkLossReason, AfkPreset, AfkRoundPhase as CoreAfkRoundPhase, flat_index,
};
use detonito_protocol::{
    AfkActionKind, AfkActionRequest, AfkActivityKind, AfkActivityRow,
    AfkBoardSize as ProtocolAfkBoardSize, AfkBoardSnapshot, AfkCellSnapshot,
    AfkChatConnectionState, AfkClientMessage, AfkIdentity, AfkLossReason, AfkPenaltySnapshot,
    AfkRoundPhase, AfkServerMessage, AfkSessionSnapshot, AfkStatusResponse,
    AfkTimerProfileSnapshot, FrontendRuntimeConfig, StreamerAuthStatus,
};
use futures_channel::mpsc::{UnboundedReceiver, unbounded};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use worker::*;

const AFK_SESSIONS: &str = "AFK_SESSIONS";

// === Cloudflare Durable Object Storage Limits ===
//
// All state is persisted as a single JSON value under STATE_KEY.
// Cloudflare DO storage enforces a hard 128 KiB (131072 byte) per-value limit.
//
// Every Vec field in PersistedAfkState / PersistedAfkSession MUST have a
// MAX_* constant and be capped via drain(0..overflow) after every push.
// When adding new Vec fields, calculate per-element JSON size and ensure
// the combined worst-case stays under PERSISTED_STATE_SIZE_LIMIT.
const STATE_KEY: &str = "detonito:afk:state";
const PERSISTED_STATE_SIZE_LIMIT: usize = 100 * 1024;
const MAX_ACTIVITY_ROWS: usize = 64;
const MAX_PENALTIES: usize = 16;
const MAX_EVENTSUB_IDS: usize = 64;
const MAX_IGNORED_USERS: usize = 200;
const MAX_TIMED_OUT_USERS: usize = 200;
/// Entries beyond this cap are dropped (oldest first). This is safe because
/// Twitch timeouts are short-lived and released again when rounds end; by the
/// time this many entries accumulate across rounds, the oldest timeouts have
/// already expired on Twitch's side. If the configured timeout durations are
/// ever raised significantly, revisit this cap or add expiry-based eviction.
const MAX_PENDING_UNTIMEOUTS: usize = 64;
const DEFAULT_TIMEOUT_DURATION_SECS: u32 = 30;
const TIMEOUT_DURATION_OPTIONS_SECS: [u32; 12] = [1, 5, 10, 15, 30, 45, 60, 90, 120, 180, 240, 300];
const AFK_FRONTEND_ABSENCE_TIMEOUT_MS: i64 = 10 * 60 * 1_000;
const AFK_SESSION_INACTIVITY_TIMEOUT_MS: i64 = 60 * 60 * 1_000;

const EVENTSUB_WS_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
const EVENTSUB_RECONNECT_RETRY_SECS: u64 = 5;
const EVENTSUB_SESSION_RECONNECT_DELAY_SECS: u64 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedEventSubState {
    connection_status: Option<String>,
    websocket_session_id: Option<String>,
    reconnect_url: Option<String>,
    reconnect_due_at_ms: Option<i64>,
    subscription_id: Option<String>,
    last_message_id: Option<String>,
    last_received_at_ms: Option<i64>,
    last_error: Option<String>,
}

/// Per-round session state stored inside [`PersistedAfkState`].
///
/// The `ignored_users` and `timed_out_users` vectors are cleared on each
/// `restart_round()` but capped at [`MAX_IGNORED_USERS`] / [`MAX_TIMED_OUT_USERS`]
/// as a safety net against the 128 KiB DO storage limit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PersistedAfkSession {
    engine: AfkEngine,
    /// Users who hit mines this round. Capped at [`MAX_IGNORED_USERS`].
    ignored_users: Vec<AfkIdentity>,
    /// Recent mine-hit penalty records. Capped at [`MAX_PENALTIES`].
    recent_penalties: Vec<AfkPenaltySnapshot>,
    /// Users successfully timed out this round. Capped at [`MAX_TIMED_OUT_USERS`].
    #[serde(default)]
    timed_out_users: Vec<AfkIdentity>,
    /// Chronological game event log. Capped at [`MAX_ACTIVITY_ROWS`].
    activity: Vec<AfkActivityRow>,
    last_action: Option<AfkActivityRow>,
    timeout_enabled: bool,
    #[serde(default)]
    last_user_activity_at_ms: i64,
    #[serde(default)]
    frontend_missing_since_at_ms: Option<i64>,
}

impl PersistedAfkSession {
    fn new(board_size: CoreAfkBoardSize, timeout_enabled: bool, now_ms: i64) -> Self {
        let mut session = Self {
            engine: AfkEngine::new(random_seed(), AfkPreset::for_board_size(board_size), now_ms),
            ignored_users: Vec::new(),
            recent_penalties: Vec::new(),
            timed_out_users: Vec::new(),
            activity: Vec::new(),
            last_action: None,
            timeout_enabled,
            last_user_activity_at_ms: now_ms,
            frontend_missing_since_at_ms: None,
        };
        session.push_activity("AFK run started", now_ms);
        session
    }

    fn normalize_loaded_state(&mut self, now_ms: i64) {
        if self.last_user_activity_at_ms <= 0 {
            self.last_user_activity_at_ms = now_ms;
        }
    }

    fn restart_round(&mut self, now_ms: i64) {
        let board_size = self
            .engine
            .preset()
            .board_size()
            .unwrap_or_else(default_board_size);
        let next_mines = if matches!(self.engine.phase(), CoreAfkRoundPhase::Won) {
            board_size.next_mine_count(self.engine.preset().config.mines)
        } else {
            self.engine
                .preset()
                .config
                .mines
                .clamp(board_size.initial_mines(), board_size.max_mines())
        };
        self.engine = AfkEngine::new(
            random_seed(),
            AfkPreset::for_board_size_and_mines(board_size, next_mines),
            now_ms,
        );
        self.ignored_users.clear();
        self.recent_penalties.clear();
        self.timed_out_users.clear();
        self.last_action = None;
        self.push_activity("Round restarted", now_ms);
    }

    fn record_user_activity(&mut self, now_ms: i64) {
        self.last_user_activity_at_ms = now_ms;
    }

    fn mark_frontend_present(&mut self) -> bool {
        self.frontend_missing_since_at_ms.take().is_some()
    }

    fn mark_frontend_missing(&mut self, now_ms: i64) -> bool {
        if self.frontend_missing_since_at_ms.is_some() {
            return false;
        }
        self.frontend_missing_since_at_ms = Some(now_ms);
        true
    }

    fn inactivity_deadline_at_ms(&self) -> i64 {
        self.last_user_activity_at_ms
            .saturating_add(AFK_SESSION_INACTIVITY_TIMEOUT_MS)
    }

    fn frontend_missing_deadline_at_ms(&self) -> Option<i64> {
        self.frontend_missing_since_at_ms
            .map(|since| since.saturating_add(AFK_FRONTEND_ABSENCE_TIMEOUT_MS))
    }

    fn next_policy_alarm_at_ms(&self) -> i64 {
        self.frontend_missing_deadline_at_ms()
            .map(|deadline| deadline.min(self.inactivity_deadline_at_ms()))
            .unwrap_or_else(|| self.inactivity_deadline_at_ms())
    }

    fn pause(&mut self, now_ms: i64) -> bool {
        if self.engine.is_paused() {
            return false;
        }
        self.engine.pause(now_ms);
        true
    }

    fn resume(&mut self, now_ms: i64) -> bool {
        if !self.engine.is_paused() {
            return false;
        }
        self.engine.resume(now_ms);
        true
    }

    fn push_activity(&mut self, text: impl Into<String>, now_ms: i64) -> AfkActivityRow {
        self.push_activity_with_details(text, now_ms, AfkActivityKind::Generic, None)
    }

    fn push_activity_with_details(
        &mut self,
        text: impl Into<String>,
        now_ms: i64,
        kind: AfkActivityKind,
        actor: Option<AfkIdentity>,
    ) -> AfkActivityRow {
        let row = AfkActivityRow {
            at_ms: now_ms,
            text: text.into(),
            kind,
            actor,
        };
        self.activity.push(row.clone());
        if self.activity.len() > MAX_ACTIVITY_ROWS {
            let overflow = self.activity.len() - MAX_ACTIVITY_ROWS;
            self.activity.drain(0..overflow);
        }
        self.last_action = Some(row.clone());
        row
    }

    fn push_penalty(&mut self, penalty: AfkPenaltySnapshot) {
        self.recent_penalties.push(penalty);
        if self.recent_penalties.len() > MAX_PENALTIES {
            let overflow = self.recent_penalties.len() - MAX_PENALTIES;
            self.recent_penalties.drain(0..overflow);
        }
    }

    fn snapshot(
        &self,
        streamer: Option<AfkIdentity>,
        timeout_supported: bool,
        now_ms: i64,
    ) -> AfkSessionSnapshot {
        let (width, height) = self.engine.size();
        let labeled_cells = self.engine.labeled_cells();
        let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
        for y in 0..height {
            for x in 0..width {
                let cell = self
                    .engine
                    .cell_state_at((x, y))
                    .unwrap_or(CoreAfkCellState::Hidden);
                cells.push(match cell {
                    CoreAfkCellState::Hidden => AfkCellSnapshot::Hidden,
                    CoreAfkCellState::Flagged => AfkCellSnapshot::Flagged,
                    CoreAfkCellState::Revealed(count) => AfkCellSnapshot::Revealed(count),
                    CoreAfkCellState::Mine => AfkCellSnapshot::Mine,
                    CoreAfkCellState::Misflagged => AfkCellSnapshot::Misflagged,
                    CoreAfkCellState::Crater => AfkCellSnapshot::Crater,
                });
            }
        }

        let timer = self.engine.preset().timer;
        AfkSessionSnapshot {
            streamer,
            phase: match self.engine.phase() {
                CoreAfkRoundPhase::Countdown => AfkRoundPhase::Countdown,
                CoreAfkRoundPhase::Active => AfkRoundPhase::Active,
                CoreAfkRoundPhase::Won => AfkRoundPhase::Won,
                CoreAfkRoundPhase::TimedOut => AfkRoundPhase::TimedOut,
            },
            paused: self.engine.is_paused(),
            board: AfkBoardSnapshot {
                width,
                height,
                cells,
            },
            labeled_cells,
            timer_profile: AfkTimerProfileSnapshot {
                start_secs: timer.start_secs,
                safe_reveal_bonus_secs: timer.safe_reveal_bonus_secs,
                mine_penalty_secs: timer.mine_penalty_secs,
                start_delay_secs: timer.start_delay_secs,
                win_continue_delay_secs: timer.win_continue_delay_secs,
                loss_continue_delay_secs: timer.loss_continue_delay_secs,
            },
            timer_remaining_secs: self.engine.board_timer_remaining_secs(),
            phase_countdown_secs: self.engine.phase_countdown_secs(now_ms),
            current_level: self.engine.preset().current_level(),
            live_mines_left: self.engine.live_mines_left_for_display(),
            crater_count: self.engine.crater_count(),
            loss_reason: self.engine.loss_reason().map(|reason| match reason {
                CoreAfkLossReason::Mine => AfkLossReason::Mine,
                CoreAfkLossReason::Timer => AfkLossReason::Timer,
            }),
            timeout_enabled: self.timeout_enabled && timeout_supported,
            ignored_users: self.ignored_users.clone(),
            recent_penalties: self.recent_penalties.clone(),
            activity: self.activity.clone(),
            last_action: self.last_action.clone(),
            last_user_activity_at_ms: self.last_user_activity_at_ms,
        }
    }
}

/// Top-level persisted state for an AFK session Durable Object.
///
/// Serialized as JSON and stored under a single DO storage key. All `Vec` fields
/// must be capped to stay within Cloudflare's 128 KiB per-value storage limit.
/// See [`PERSISTED_STATE_SIZE_LIMIT`] and the `MAX_*` constants.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PersistedAfkState {
    broadcaster: Option<AfkIdentity>,
    tokens: Option<TwitchTokenState>,
    #[serde(default = "default_timeout_enabled")]
    timeout_enabled: bool,
    #[serde(default = "default_timeout_duration_secs")]
    timeout_duration_secs: u32,
    #[serde(default = "default_protocol_board_size")]
    board_size: ProtocolAfkBoardSize,
    session: Option<PersistedAfkSession>,
    /// Users awaiting untimeout across rounds. Capped at [`MAX_PENDING_UNTIMEOUTS`].
    #[serde(default)]
    pending_untimeouts: Vec<AfkIdentity>,
    /// EventSub message dedup buffer. Capped at [`MAX_EVENTSUB_IDS`].
    recent_eventsub_ids: Vec<String>,
    eventsub: PersistedEventSubState,
}

impl Default for PersistedAfkState {
    fn default() -> Self {
        Self {
            broadcaster: None,
            tokens: None,
            timeout_enabled: default_timeout_enabled(),
            timeout_duration_secs: default_timeout_duration_secs(),
            board_size: default_protocol_board_size(),
            session: None,
            pending_untimeouts: Vec::new(),
            recent_eventsub_ids: Vec::new(),
            eventsub: PersistedEventSubState::default(),
        }
    }
}

impl PersistedAfkState {
    fn timeout_supported(&self) -> bool {
        self.tokens.as_ref().is_some_and(|tokens| {
            tokens
                .scope
                .iter()
                .any(|scope| scope == "moderator:manage:banned_users")
        })
    }

    fn status_response(
        &self,
        base_path: &str,
        chat_connection: AfkChatConnectionState,
        chat_error: Option<String>,
    ) -> AfkStatusResponse {
        AfkStatusResponse {
            runtime: FrontendRuntimeConfig { afk_enabled: true },
            auth: StreamerAuthStatus {
                identity: self.broadcaster.clone(),
                expires_at_ms: None,
            },
            chat_connection,
            chat_error,
            timeout_supported: self.timeout_supported(),
            timeout_enabled: self.timeout_enabled,
            timeout_duration_secs: self.timeout_duration_secs,
            board_size: self.board_size,
            connect_url: Some(join_base_path(base_path, "/auth/twitch/login")),
            websocket_path: self
                .broadcaster
                .as_ref()
                .map(|_| join_base_path(base_path, "/ws/afk")),
            session: self.session.as_ref().map(|session| {
                session.snapshot(self.broadcaster.clone(), self.timeout_supported(), now_ms())
            }),
        }
    }

    fn remember_eventsub_message_id(&mut self, message_id: &str) -> bool {
        if self
            .recent_eventsub_ids
            .iter()
            .any(|known| known == message_id)
        {
            return false;
        }
        self.recent_eventsub_ids.push(message_id.to_string());
        if self.recent_eventsub_ids.len() > MAX_EVENTSUB_IDS {
            let overflow = self.recent_eventsub_ids.len() - MAX_EVENTSUB_IDS;
            self.recent_eventsub_ids.drain(0..overflow);
        }
        true
    }
}

fn default_eventsub_error_message(state: &PersistedAfkState) -> String {
    state.eventsub.last_error.clone().unwrap_or_else(|| {
        "Twitch chat is disconnected. Return to AFK mode and start again.".to_string()
    })
}

fn chat_connection_for_response(
    runtime_active: bool,
    state: &PersistedAfkState,
) -> (AfkChatConnectionState, Option<String>) {
    let requires_chat = state.broadcaster.is_some() && state.session.is_some();
    if !requires_chat {
        return (AfkChatConnectionState::Idle, None);
    }

    match (runtime_active, state.eventsub.connection_status.as_deref()) {
        (true, Some("connected")) => (AfkChatConnectionState::Connected, None),
        (true, Some("connecting" | "reconnecting")) => (AfkChatConnectionState::Connecting, None),
        (_, Some("connected")) => (
            AfkChatConnectionState::Error,
            Some(
                "Twitch chat connection was lost. Return to AFK mode and start again.".to_string(),
            ),
        ),
        (_, Some("connecting" | "reconnecting")) => (AfkChatConnectionState::Connecting, None),
        _ => (
            AfkChatConnectionState::Error,
            Some(default_eventsub_error_message(state)),
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LinkStreamerRequest {
    identity: AfkIdentity,
    tokens: TwitchTokenState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct EnsureEventSubRequest {
    force: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SetTimeoutPreferenceRequest {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    duration_secs: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SetBoardSizePreferenceRequest {
    board_size: ProtocolAfkBoardSize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct EventSubWebSocketMessageRequest {
    connection_id: String,
    envelope: EventSubWebSocketEnvelope,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct EventSubWebSocketClosedRequest {
    connection_id: String,
    code: u16,
    reason: String,
    was_clean: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct EventSubWebSocketErrorRequest {
    connection_id: String,
    message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EventSubRuntime {
    connection_id: String,
    socket: WebSocket,
}

enum EventSubSocketEvent {
    Message(Option<String>),
    Close {
        code: u16,
        reason: String,
        was_clean: bool,
    },
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestAuthError {
    Missing,
    Invalid,
}

#[derive(Debug, Deserialize)]
struct AuthLoginQuery {
    #[serde(default)]
    return_to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthCallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let path = normalized_request_path(&req, &env);
    let method = req.method();

    match (method.clone(), path.as_str()) {
        (Method::Get, "/healthz") => Response::ok("ok"),
        (Method::Get, "/auth/twitch/login") => handle_twitch_login(req, env).await,
        (Method::Get, "/auth/twitch/callback") => handle_twitch_callback(req, env).await,
        (Method::Get, "/auth/logout") | (Method::Post, "/auth/logout") => {
            handle_logout(req, env).await
        }
        (Method::Get, "/api/afk/status") => handle_afk_status(req, env).await,
        (Method::Post, "/api/afk/action") => handle_afk_action(req, env).await,
        (Method::Post, "/api/afk/board-size") => handle_afk_action(req, env).await,
        (Method::Post, "/api/afk/timeout") => handle_afk_action(req, env).await,
        (Method::Post, "/api/afk/pause") | (Method::Post, "/api/afk/resume") => {
            handle_afk_action(req, env).await
        }
        (Method::Post, "/api/afk/start")
        | (Method::Post, "/api/afk/continue")
        | (Method::Post, "/api/afk/stop")
        | (Method::Post, "/api/afk/panic-reset") => handle_afk_action(req, env).await,
        (Method::Get, "/ws/afk") => handle_afk_websocket(req, env).await,
        _ if matches!(method, Method::Get | Method::Head) => serve_assets(req, &env).await,
        _ => Response::error("not found", 404),
    }
}

async fn serve_assets(req: Request, env: &Env) -> Result<Response> {
    env.assets("ASSETS")?.fetch_request(req).await
}

async fn handle_twitch_login(req: Request, env: Env) -> Result<Response> {
    let query: AuthLoginQuery = req.query().unwrap_or(AuthLoginQuery { return_to: None });
    let authorize_url = build_twitch_authorize_url(
        &configured_var(&env, "TWITCH_CLIENT_ID"),
        &public_base_url(&env, &req)?,
        query.return_to.as_deref(),
        now_ms(),
        &auth_signing_secret(&env),
    )
    .map_err(error_from_display)?;
    Response::redirect(authorize_url)
}

async fn handle_twitch_callback(req: Request, env: Env) -> Result<Response> {
    let query: AuthCallbackQuery = req.query()?;
    if query.error.is_some() || query.error_description.is_some() {
        let code = query.error.as_deref().unwrap_or("oauth_error");
        let detail = query.error_description.as_deref().unwrap_or(code);
        return auth_callback_error_response(&req, &env, code, detail);
    }

    let Some(code) = query.code else {
        return auth_callback_error_response(
            &req,
            &env,
            "missing_code",
            "missing code query param",
        );
    };
    let Some(state_token) = query.state else {
        return auth_callback_error_response(
            &req,
            &env,
            "missing_state",
            "missing state query param",
        );
    };
    let signing_secret = auth_signing_secret(&env);
    let state = match verify_oauth_state(&signing_secret, &state_token, now_ms()) {
        Ok(state) => state,
        Err(AuthError::Expired) => {
            return auth_callback_error_response(
                &req,
                &env,
                "expired_oauth_state",
                "signed oauth state is expired",
            );
        }
        Err(error) => {
            return auth_callback_error_response(
                &req,
                &env,
                "invalid_oauth_state",
                &error.to_string(),
            );
        }
    };
    let public_url = public_base_url(&env, &req)?;
    let outcome = match exchange_twitch_code(
        &configured_var(&env, "TWITCH_CLIENT_ID"),
        &configured_var(&env, "TWITCH_CLIENT_SECRET"),
        &code,
        &format!("{}/auth/twitch/callback", public_url),
        now_ms(),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            return auth_callback_error_response(
                &req,
                &env,
                "oauth_exchange_failed",
                &error.to_string(),
            );
        }
    };

    let identity = to_afk_identity(&outcome);
    let stub = afk_session_stub(&env, &identity.user_id)?;
    let _ = post_json(
        &stub,
        "https://internal/internal/link",
        &LinkStreamerRequest {
            identity: identity.clone(),
            tokens: outcome.tokens.clone(),
        },
    )
    .await?;
    let _ = post_json(
        &stub,
        "https://internal/internal/eventsub/ensure",
        &EnsureEventSubRequest { force: false },
    )
    .await;

    let completed =
        complete_twitch_callback(&public_url, &signing_secret, state, identity, now_ms())
            .map_err(error_from_display)?;
    redirect_with_cookie(
        completed.redirect_url.as_str(),
        &auth_cookie_header(
            &completed.auth_token,
            &configured_base_path(&env),
            public_url.starts_with("https://"),
        ),
    )
}

async fn handle_logout(req: Request, env: Env) -> Result<Response> {
    let secure = public_base_url(&env, &req)?.starts_with("https://");
    let cookie_path = configured_base_path(&env);
    if let Ok(Some(auth)) = optional_auth_from_request(&req, &env, now_ms()) {
        if let Ok(stub) = afk_session_stub(&env, &auth.identity.user_id) {
            let _ = post_empty(&stub, "https://internal/internal/unlink").await;
        }
    }
    if req.method() == Method::Get {
        redirect_with_cookie(
            &public_base_url(&env, &req)?,
            &cleared_auth_cookie_header(&cookie_path, secure),
        )
    } else {
        Ok(ResponseBuilder::new()
            .with_status(204)
            .with_header(
                "Set-Cookie",
                &cleared_auth_cookie_header(&cookie_path, secure),
            )?
            .with_header("Cache-Control", "no-store")?
            .empty())
    }
}

async fn handle_afk_status(req: Request, env: Env) -> Result<Response> {
    let secure = public_base_url(&env, &req)?.starts_with("https://");
    let auth = match optional_auth_from_request(&req, &env, now_ms()) {
        Ok(auth) => auth,
        Err(error) => return auth_error_response(error),
    };
    let Some(auth) = auth else {
        return Response::from_json(&AfkStatusResponse {
            runtime: FrontendRuntimeConfig { afk_enabled: true },
            auth: StreamerAuthStatus::default(),
            chat_connection: AfkChatConnectionState::Idle,
            chat_error: None,
            timeout_supported: false,
            timeout_enabled: true,
            timeout_duration_secs: default_timeout_duration_secs(),
            board_size: default_protocol_board_size(),
            connect_url: Some(join_base_path(
                &configured_base_path(&env),
                "/auth/twitch/login",
            )),
            websocket_path: None,
            session: None,
        });
    };

    let stub = afk_session_stub(&env, &auth.identity.user_id)?;
    let mut response = stub.fetch_with_request(req).await?;
    maybe_refresh_auth_cookie(&mut response, &env, Some(&auth), secure)?;
    Ok(response)
}

async fn handle_afk_action(req: Request, env: Env) -> Result<Response> {
    let secure = public_base_url(&env, &req)?.starts_with("https://");
    let auth = match require_identity_auth(&req, &env, now_ms()) {
        Ok(auth) => auth,
        Err(error) => return auth_error_response(error),
    };
    let stub = afk_session_stub(&env, &auth.identity.user_id)?;
    let mut response = stub.fetch_with_request(req).await?;
    maybe_refresh_auth_cookie(&mut response, &env, Some(&auth), secure)?;
    Ok(response)
}

async fn handle_afk_websocket(req: Request, env: Env) -> Result<Response> {
    let auth = match require_identity_auth(&req, &env, now_ms()) {
        Ok(auth) => auth,
        Err(error) => return auth_error_response(error),
    };
    let stub = afk_session_stub(&env, &auth.identity.user_id)?;
    stub.fetch_with_request(req).await
}

#[durable_object]
pub struct AfkSessionDO {
    state: State,
    env: Env,
    cache: RefCell<Option<PersistedAfkState>>,
    eventsub_runtime: RefCell<Option<EventSubRuntime>>,
    eventsub_connection_seq: RefCell<u64>,
}

impl AfkSessionDO {
    async fn load(&self) -> Result<PersistedAfkState> {
        if let Some(cached) = self.cache.borrow().clone() {
            return Ok(cached);
        }
        let storage = self.state.storage();
        let mut loaded = match load_storage_json::<PersistedAfkState>(&storage, STATE_KEY).await {
            Ok(Some(loaded)) => loaded,
            Ok(None) => PersistedAfkState::default(),
            Err(error) => {
                log::warn!("resetting invalid AFK saved state: {error}");
                let _ = storage.delete(STATE_KEY).await;
                PersistedAfkState::default()
            }
        };
        loaded.timeout_duration_secs =
            normalize_timeout_duration_secs(loaded.timeout_duration_secs);
        loaded.board_size = normalize_protocol_board_size(loaded.board_size);
        if let Some(session) = loaded.session.as_mut() {
            session.normalize_loaded_state(now_ms());
        }
        *self.cache.borrow_mut() = Some(loaded.clone());
        Ok(loaded)
    }

    async fn persist(&self, next: &PersistedAfkState) -> Result<()> {
        let storage = self.state.storage();
        persist_storage_json(&storage, STATE_KEY, next).await?;
        *self.cache.borrow_mut() = Some(next.clone());
        Ok(())
    }

    async fn schedule_alarm(&self, state: &PersistedAfkState) -> Result<()> {
        let now = now_ms();
        let session_deadline = state.session.as_ref().map(|session| {
            let policy_deadline = session.next_policy_alarm_at_ms();
            match session.engine.next_alarm_at_ms(now) {
                Some(engine_deadline) => engine_deadline.min(policy_deadline),
                None => policy_deadline,
            }
        });
        let eventsub_deadline = state.eventsub.reconnect_due_at_ms;
        let deadline = match (session_deadline, eventsub_deadline) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(left), None) => Some(left),
            (None, Some(right)) => Some(right),
            (None, None) => None,
        };
        if let Some(deadline_ms) = deadline {
            let delay_ms = deadline_ms.saturating_sub(now) as u64;
            self.state
                .storage()
                .set_alarm(Duration::from_millis(delay_ms))
                .await?;
        }
        Ok(())
    }

    fn status_response(&self, state: &PersistedAfkState) -> AfkStatusResponse {
        let (chat_connection, chat_error) =
            chat_connection_for_response(self.eventsub_runtime.borrow().is_some(), state);
        state.status_response(
            &configured_base_path(&self.env),
            chat_connection,
            chat_error,
        )
    }

    async fn mark_eventsub_runtime_missing(&self, state: &mut PersistedAfkState) -> Result<bool> {
        let requires_chat = state.broadcaster.is_some() && state.session.is_some();
        if !requires_chat
            || self.eventsub_runtime.borrow().is_some()
            || state.eventsub.reconnect_due_at_ms.is_some()
            || matches!(state.eventsub.connection_status.as_deref(), Some("error"))
        {
            return Ok(false);
        }

        state.eventsub.connection_status = Some("error".to_string());
        if state.eventsub.last_error.is_none() {
            state.eventsub.last_error = Some(
                "Twitch chat connection is inactive. Return to AFK mode and start again."
                    .to_string(),
            );
        }
        self.persist(state).await?;
        Ok(true)
    }

    async fn set_eventsub_error(
        &self,
        state: &mut PersistedAfkState,
        message: impl Into<String>,
    ) -> Result<()> {
        state.eventsub.connection_status = Some("error".to_string());
        state.eventsub.last_error = Some(message.into());
        state.eventsub.reconnect_due_at_ms = None;
        self.persist(state).await?;
        self.schedule_alarm(state).await?;
        self.broadcast_status(state);
        Ok(())
    }

    fn runtime_matches(&self, connection_id: &str) -> bool {
        self.eventsub_runtime
            .borrow()
            .as_ref()
            .is_some_and(|runtime| runtime.connection_id == connection_id)
    }

    fn next_eventsub_connection_id(&self) -> String {
        let mut seq = self.eventsub_connection_seq.borrow_mut();
        *seq += 1;
        format!("afk-eventsub-{}-{}", now_ms(), *seq)
    }

    fn set_runtime(&self, connection_id: String, socket: WebSocket) {
        *self.eventsub_runtime.borrow_mut() = Some(EventSubRuntime {
            connection_id,
            socket,
        });
    }

    fn clear_runtime_if_matches(&self, connection_id: &str) {
        if self.runtime_matches(connection_id) {
            self.eventsub_runtime.borrow_mut().take();
        }
    }

    fn has_frontend_websockets_excluding(&self, exclude: Option<&WebSocket>) -> bool {
        self.state
            .get_websockets()
            .into_iter()
            .any(|socket| exclude.is_none_or(|exclude| socket != *exclude))
    }

    fn sync_frontend_presence(
        &self,
        state: &mut PersistedAfkState,
        frontend_present: bool,
        now_ms: i64,
    ) -> bool {
        let Some(session) = state.session.as_mut() else {
            return false;
        };
        if frontend_present {
            session.mark_frontend_present()
        } else {
            session.mark_frontend_missing(now_ms)
        }
    }

    async fn delete_eventsub_subscription_if_present(
        &self,
        state: &mut PersistedAfkState,
    ) -> Result<()> {
        let Some(subscription_id) = state.eventsub.subscription_id.clone() else {
            return Ok(());
        };
        if let Some(access_token) = self.ensure_fresh_access_token(state, false).await? {
            let _ = delete_eventsub_subscription(
                &configured_var(&self.env, "TWITCH_CLIENT_ID"),
                &access_token,
                &subscription_id,
            )
            .await;
        }
        Ok(())
    }

    async fn cleanup_live_session(&self, state: &mut PersistedAfkState) -> Result<bool> {
        let had_session = state.session.is_some();
        let had_eventsub_state = state.eventsub != PersistedEventSubState::default();
        if !had_session && !had_eventsub_state && self.eventsub_runtime.borrow().is_none() {
            return Ok(false);
        }

        self.release_round_timeouts(state).await?;
        self.delete_eventsub_subscription_if_present(state).await?;

        if let Some(runtime) = self.eventsub_runtime.borrow_mut().take() {
            let _ = runtime.socket.close(Some(1000), Some("AFK session ended"));
        }

        state.session = None;
        state.recent_eventsub_ids.clear();
        state.eventsub = PersistedEventSubState::default();
        Ok(true)
    }

    async fn ensure_fresh_access_token(
        &self,
        state: &mut PersistedAfkState,
        force_refresh: bool,
    ) -> Result<Option<String>> {
        let Some(tokens) = state.tokens.clone() else {
            return Ok(None);
        };
        if !force_refresh && !should_refresh_twitch_token(&tokens, now_ms()) {
            return Ok(Some(tokens.access_token));
        }

        let refreshed = refresh_access_token(
            &configured_var(&self.env, "TWITCH_CLIENT_ID"),
            &configured_var(&self.env, "TWITCH_CLIENT_SECRET"),
            &tokens.refresh_token,
        )
        .await
        .map_err(error_from_display)?;
        let next_tokens = refreshed_twitch_token_state(
            &tokens,
            refreshed.access_token,
            refreshed.refresh_token,
            refreshed.expires_in,
            refreshed.scope,
            refreshed.token_type,
            now_ms(),
        );
        let access_token = next_tokens.access_token.clone();
        state.tokens = Some(next_tokens);
        let timeout_supported = state.timeout_supported();
        if let Some(session) = state.session.as_mut() {
            session.timeout_enabled = state.timeout_enabled && timeout_supported;
        }
        self.persist(state).await?;
        Ok(Some(access_token))
    }

    async fn reconcile_eventsub_subscription(
        &self,
        state: &mut PersistedAfkState,
        session: &EventSubWebSocketSession,
    ) -> Result<()> {
        let Some(broadcaster) = state.broadcaster.clone() else {
            return Ok(());
        };
        let Some(access_token) = self.ensure_fresh_access_token(state, false).await? else {
            return Ok(());
        };

        let client_id = configured_var(&self.env, "TWITCH_CLIENT_ID");
        let existing = list_eventsub_subscriptions(&client_id, &access_token)
            .await
            .map_err(error_from_display)?;
        for subscription in existing.data {
            if subscription.subscription_type == CHANNEL_CHAT_MESSAGE {
                if let Some(subscription_id) = subscription.id {
                    let _ =
                        delete_eventsub_subscription(&client_id, &access_token, &subscription_id)
                            .await;
                }
            }
        }

        let created = create_eventsub_subscription(
            &client_id,
            &access_token,
            &chat_message_subscription_request(&broadcaster.user_id, &session.id),
        )
        .await
        .map_err(error_from_display)?;
        state.eventsub.subscription_id = created.data.first().and_then(|sub| sub.id.clone());
        self.persist(state).await?;
        Ok(())
    }

    async fn request_timeout(
        &self,
        state: &mut PersistedAfkState,
        chatter_user_id: &str,
    ) -> Result<bool> {
        let Some(broadcaster) = state.broadcaster.clone() else {
            return Ok(false);
        };
        if chatter_user_id.is_empty()
            || chatter_user_id == broadcaster.user_id
            || !state.timeout_supported()
        {
            return Ok(false);
        }
        let Some(access_token) = self.ensure_fresh_access_token(state, false).await? else {
            return Ok(false);
        };

        let request = PreparedRequest {
            url: format!(
                "https://api.twitch.tv/helix/moderation/bans?broadcaster_id={}&moderator_id={}",
                broadcaster.user_id, broadcaster.user_id
            ),
            method: HttpMethod::Post,
            headers: vec![
                (
                    "Authorization".to_string(),
                    format!("Bearer {access_token}"),
                ),
                (
                    "Client-Id".to_string(),
                    configured_var(&self.env, "TWITCH_CLIENT_ID"),
                ),
                ("Content-Type".to_string(), "application/json".to_string()),
            ],
            body: Some(
                serde_json::json!({
                    "data": {
                        "user_id": chatter_user_id,
                        "duration": state.timeout_duration_secs,
                        "reason": "BOOM! You found a mine.",
                    }
                })
                .to_string(),
            ),
        };
        let response = send_prepared_request(request)
            .await
            .map_err(error_from_display)?;
        Ok(matches!(response.status, 200 | 201 | 204))
    }

    async fn request_untimeout(
        &self,
        state: &mut PersistedAfkState,
        chatter_user_id: &str,
    ) -> Result<bool> {
        let Some(broadcaster) = state.broadcaster.clone() else {
            return Ok(false);
        };
        if chatter_user_id.is_empty()
            || chatter_user_id == broadcaster.user_id
            || !state.timeout_supported()
        {
            return Ok(false);
        }
        let Some(access_token) = self.ensure_fresh_access_token(state, false).await? else {
            return Ok(false);
        };

        let request = PreparedRequest {
            url: format!(
                "https://api.twitch.tv/helix/moderation/bans?broadcaster_id={}&moderator_id={}&user_id={}",
                broadcaster.user_id, broadcaster.user_id, chatter_user_id
            ),
            method: HttpMethod::Delete,
            headers: vec![
                (
                    "Authorization".to_string(),
                    format!("Bearer {access_token}"),
                ),
                (
                    "Client-Id".to_string(),
                    configured_var(&self.env, "TWITCH_CLIENT_ID"),
                ),
            ],
            body: None,
        };
        let response = send_prepared_request(request)
            .await
            .map_err(error_from_display)?;
        let body_lower = response.body.to_ascii_lowercase();
        Ok(matches!(response.status, 200 | 204)
            || (response.status == 400
                && (body_lower.contains("not banned") || body_lower.contains("not timed out"))))
    }

    async fn release_round_timeouts(&self, state: &mut PersistedAfkState) -> Result<()> {
        let mut releasing = mem::take(&mut state.pending_untimeouts);
        if let Some(session) = state.session.as_mut() {
            for identity in mem::take(&mut session.timed_out_users) {
                if !releasing
                    .iter()
                    .any(|known| known.user_id == identity.user_id)
                {
                    releasing.push(identity);
                }
            }
        }
        if releasing.is_empty() {
            return Ok(());
        }

        let mut still_pending = Vec::new();
        for identity in releasing {
            let released = self
                .request_untimeout(state, &identity.user_id)
                .await
                .unwrap_or(false);
            if !released {
                still_pending.push(identity);
            }
        }
        state.pending_untimeouts = still_pending;
        if state.pending_untimeouts.len() > MAX_PENDING_UNTIMEOUTS {
            let overflow = state.pending_untimeouts.len() - MAX_PENDING_UNTIMEOUTS;
            state.pending_untimeouts.drain(0..overflow);
        }
        Ok(())
    }

    async fn apply_chat_board_action(
        &self,
        state: &mut PersistedAfkState,
        chat: &EventSubChatMessage,
        actor_label: &str,
        parsed: ParsedBoardAction,
    ) -> Result<Option<AfkActivityRow>> {
        let Some(session) = state.session.as_ref() else {
            return Ok(None);
        };
        if session
            .ignored_users
            .iter()
            .any(|identity| identity.user_id == chat.chatter_user_id)
        {
            return Ok(None);
        }
        if !chat_board_action_targets_labeled_cell(session, parsed)? {
            return Ok(None);
        }

        let now = now_ms();
        let (row, timed_out_identity, should_request_timeout) = {
            let session = state
                .session
                .as_mut()
                .expect("session existence checked above");
            let outcome = session
                .engine
                .apply_action(parsed.action, now)
                .map_err(error_from_display)?;
            if !outcome.changed {
                return Ok(None);
            }

            let coord_label = format_coord(parsed.coords);
            let verb = match parsed.action {
                AfkAction::Reveal(_) => "revealed",
                AfkAction::ToggleFlag(_) | AfkAction::SetFlag(_) => "flagged",
                AfkAction::ClearFlag(_) => "unflagged",
                AfkAction::Chord(_) => "chorded",
                AfkAction::ChordFlag(_) => "chord-flagged",
            };
            let row = if outcome.mine_triggered {
                let actor = AfkIdentity::new(
                    chat.chatter_user_id.clone(),
                    chat.chatter_user_login.clone(),
                    actor_label.to_string(),
                );
                session.push_activity_with_details(
                    format!("{actor_label} hit a mine at {coord_label}"),
                    now,
                    AfkActivityKind::MineHit,
                    Some(actor),
                )
            } else if outcome.won {
                session.push_activity(format!("{actor_label} cleared {coord_label}"), now)
            } else {
                session.push_activity(format!("{actor_label} {verb} {coord_label}"), now)
            };

            let (timed_out_identity, should_request_timeout) = if outcome.mine_triggered {
                let identity = row
                    .actor
                    .clone()
                    .expect("mine-hit activity rows must include an actor");
                session.ignored_users.push(identity.clone());
                if session.ignored_users.len() > MAX_IGNORED_USERS {
                    let overflow = session.ignored_users.len() - MAX_IGNORED_USERS;
                    session.ignored_users.drain(0..overflow);
                }
                let should_request_timeout =
                    session.timeout_enabled && !chat_actor_is_timeout_exempt(chat);
                let penalty = AfkPenaltySnapshot {
                    chatter: identity.clone(),
                    timer_delta_secs: outcome.timer_delta_secs,
                    timeout_requested: should_request_timeout,
                    timeout_succeeded: false,
                };
                session.push_penalty(penalty);
                (Some(identity), should_request_timeout)
            } else {
                (None, false)
            };

            (row, timed_out_identity, should_request_timeout)
        };

        let timeout_succeeded = if should_request_timeout && state.timeout_supported() {
            self.request_timeout(state, &chat.chatter_user_id)
                .await
                .unwrap_or(false)
        } else {
            false
        };
        if timed_out_identity.is_some() {
            if let Some(last_penalty) = state
                .session
                .as_mut()
                .and_then(|session| session.recent_penalties.last_mut())
            {
                last_penalty.timeout_succeeded = timeout_succeeded;
            }
        }
        if timeout_succeeded {
            if let (Some(identity), Some(session)) = (timed_out_identity, state.session.as_mut()) {
                if !session
                    .timed_out_users
                    .iter()
                    .any(|known| known.user_id == identity.user_id)
                {
                    session.timed_out_users.push(identity);
                    if session.timed_out_users.len() > MAX_TIMED_OUT_USERS {
                        let overflow = session.timed_out_users.len() - MAX_TIMED_OUT_USERS;
                        session.timed_out_users.drain(0..overflow);
                    }
                }
            }
        }

        Ok(Some(row))
    }

    async fn ingest_chat_message(
        &self,
        state: &mut PersistedAfkState,
        message_id: &str,
        chat: EventSubChatMessage,
    ) -> Result<bool> {
        if !state.remember_eventsub_message_id(message_id) {
            return Ok(false);
        }
        if state.session.is_none() {
            self.persist(state).await?;
            return Ok(false);
        }

        let Some(command) = parse_chat_command(&chat.message.text) else {
            self.persist(state).await?;
            return Ok(false);
        };
        let actor_label = if chat.chatter_user_name.is_empty() {
            chat.chatter_user_login.clone()
        } else {
            chat.chatter_user_name.clone()
        };
        let now = now_ms();
        let rows = match command {
            ParsedChatCommand::Continue => {
                let Some(session) = state.session.as_mut() else {
                    self.persist(state).await?;
                    return Ok(false);
                };
                if !matches!(
                    session.engine.phase(),
                    CoreAfkRoundPhase::Won | CoreAfkRoundPhase::TimedOut
                ) {
                    self.persist(state).await?;
                    return Ok(false);
                }
                session.record_user_activity(now);
                session.restart_round(now);
                vec![session.push_activity(format!("{actor_label} continued the run"), now)]
            }
            ParsedChatCommand::BoardBatch(actions) => {
                let has_targeted_move = state.session.as_ref().is_some_and(|session| {
                    actions.iter().copied().any(|parsed| {
                        chat_board_action_targets_labeled_cell(session, parsed).unwrap_or(false)
                    })
                });
                if !has_targeted_move {
                    self.persist(state).await?;
                    return Ok(false);
                }

                if let Some(session) = state.session.as_mut() {
                    session.record_user_activity(now);
                }

                let user_ignored = state.session.as_ref().is_some_and(|session| {
                    session
                        .ignored_users
                        .iter()
                        .any(|identity| identity.user_id == chat.chatter_user_id)
                });
                if user_ignored {
                    let actor = AfkIdentity::new(
                        chat.chatter_user_id.clone(),
                        chat.chatter_user_login.clone(),
                        actor_label.clone(),
                    );
                    let row = state
                        .session
                        .as_mut()
                        .expect("session existence checked above")
                        .push_activity_with_details(
                            format!("{actor_label} is out for the rest of the round."),
                            now,
                            AfkActivityKind::OutForRound,
                            Some(actor),
                        );
                    vec![row]
                } else {
                    let mut rows = Vec::new();
                    for parsed in actions {
                        if let Some(row) = self
                            .apply_chat_board_action(state, &chat, &actor_label, parsed)
                            .await?
                        {
                            rows.push(row);
                        }

                        let Some(session) = state.session.as_ref() else {
                            break;
                        };
                        let user_ignored = session
                            .ignored_users
                            .iter()
                            .any(|identity| identity.user_id == chat.chatter_user_id);
                        if user_ignored
                            || !matches!(session.engine.phase(), CoreAfkRoundPhase::Active)
                        {
                            break;
                        }
                    }
                    if rows.is_empty() {
                        self.persist(state).await?;
                        return Ok(false);
                    }
                    rows
                }
            }
        };

        if state.session.as_ref().is_some_and(|session| {
            matches!(
                session.engine.phase(),
                CoreAfkRoundPhase::Won | CoreAfkRoundPhase::TimedOut
            )
        }) {
            self.release_round_timeouts(state).await?;
        }

        self.persist(state).await?;
        for row in &rows {
            self.broadcast_activity(row);
        }
        self.broadcast_snapshot(state);
        Ok(true)
    }

    async fn apply_streamer_action(
        &self,
        state: &mut PersistedAfkState,
        action: AfkAction,
    ) -> Result<bool> {
        let Some(streamer) = state.broadcaster.clone() else {
            self.persist(state).await?;
            return Ok(false);
        };
        let Some(session) = state.session.as_mut() else {
            self.persist(state).await?;
            return Ok(false);
        };

        let coords = match action {
            AfkAction::Reveal(coords)
            | AfkAction::ToggleFlag(coords)
            | AfkAction::SetFlag(coords)
            | AfkAction::ClearFlag(coords)
            | AfkAction::Chord(coords)
            | AfkAction::ChordFlag(coords) => coords,
        };
        if coords.0 >= session.engine.size().0 || coords.1 >= session.engine.size().1 {
            self.persist(state).await?;
            return Ok(false);
        }
        let now = now_ms();
        session.record_user_activity(now);

        if matches!(session.engine.phase(), CoreAfkRoundPhase::Countdown) {
            let AfkAction::Reveal(coords) = action else {
                self.persist(state).await?;
                return Ok(false);
            };
            let started = session
                .engine
                .open_starting_cell(coords, now)
                .map_err(error_from_display)?;
            if !started {
                self.persist(state).await?;
                return Ok(false);
            }
            let row = session.push_activity(
                format!("{} opened {}", streamer.display_name, format_coord(coords)),
                now,
            );
            self.persist(state).await?;
            self.broadcast_activity(&row);
            self.broadcast_snapshot(state);
            return Ok(true);
        }

        let outcome = session
            .engine
            .apply_action(action, now)
            .map_err(error_from_display)?;
        if !outcome.changed {
            self.persist(state).await?;
            return Ok(false);
        }

        let coord_label = format_coord(coords);
        let actor_label = streamer.display_name.clone();
        let verb = match action {
            AfkAction::Reveal(_) => "revealed",
            AfkAction::ToggleFlag(_) | AfkAction::SetFlag(_) => "flagged",
            AfkAction::ClearFlag(_) => "unflagged",
            AfkAction::Chord(_) => "chorded",
            AfkAction::ChordFlag(_) => "chord-flagged",
        };
        let row = if outcome.mine_triggered {
            session.engine.force_timed_out(CoreAfkLossReason::Mine, now);
            session.push_activity_with_details(
                format!("{actor_label} hit a mine at {coord_label}"),
                now,
                AfkActivityKind::MineHit,
                Some(streamer.clone()),
            )
        } else if outcome.won {
            session.push_activity(format!("{actor_label} cleared {coord_label}"), now)
        } else {
            session.push_activity(format!("{actor_label} {verb} {coord_label}"), now)
        };

        if matches!(
            session.engine.phase(),
            CoreAfkRoundPhase::Won | CoreAfkRoundPhase::TimedOut
        ) {
            self.release_round_timeouts(state).await?;
        }

        self.persist(state).await?;
        self.broadcast_activity(&row);
        self.broadcast_snapshot(state);
        Ok(true)
    }

    fn broadcast_status(&self, state: &PersistedAfkState) {
        let message = AfkServerMessage::Connected {
            status: self.status_response(state),
        };
        for socket in self.state.get_websockets() {
            let _ = socket.send(&message);
        }
    }

    fn broadcast_snapshot(&self, state: &PersistedAfkState) {
        let Some(session) = state.session.as_ref().map(|session| {
            session.snapshot(
                state.broadcaster.clone(),
                state.timeout_supported(),
                now_ms(),
            )
        }) else {
            self.broadcast_status(state);
            return;
        };
        let message = AfkServerMessage::Snapshot { session };
        for socket in self.state.get_websockets() {
            let _ = socket.send(&message);
        }
    }

    fn broadcast_activity(&self, row: &AfkActivityRow) {
        let message = AfkServerMessage::Activity { row: row.clone() };
        for socket in self.state.get_websockets() {
            let _ = socket.send(&message);
        }
    }

    fn spawn_eventsub_loop(
        &self,
        connection_id: String,
        broadcaster_user_id: String,
        ws: WebSocket,
    ) {
        let stub = match afk_session_stub(&self.env, &broadcaster_user_id) {
            Ok(stub) => stub,
            Err(_) => return,
        };
        let raw_socket = ws.as_ref().clone();
        let (tx, rx) = unbounded::<EventSubSocketEvent>();

        let message_listener = Closure::wrap(Box::new({
            let tx = tx.clone();
            move |event: web_sys::MessageEvent| {
                let _ = tx.unbounded_send(EventSubSocketEvent::Message(event.data().as_string()));
            }
        }) as Box<dyn FnMut(web_sys::MessageEvent)>);
        if raw_socket
            .add_event_listener_with_callback("message", message_listener.as_ref().unchecked_ref())
            .is_err()
        {
            return;
        }

        let close_listener = Closure::wrap(Box::new({
            let tx = tx.clone();
            move |event: web_sys::CloseEvent| {
                let _ = tx.unbounded_send(EventSubSocketEvent::Close {
                    code: event.code(),
                    reason: event.reason(),
                    was_clean: event.was_clean(),
                });
            }
        }) as Box<dyn FnMut(web_sys::CloseEvent)>);
        if raw_socket
            .add_event_listener_with_callback("close", close_listener.as_ref().unchecked_ref())
            .is_err()
        {
            let _ = raw_socket.remove_event_listener_with_callback(
                "message",
                message_listener.as_ref().unchecked_ref(),
            );
            return;
        }

        let error_listener = Closure::wrap(Box::new({
            let tx = tx.clone();
            move |event: web_sys::ErrorEvent| {
                let detail = if event.message().is_empty() {
                    "websocket error".to_string()
                } else {
                    event.message()
                };
                let _ = tx.unbounded_send(EventSubSocketEvent::Error(detail));
            }
        }) as Box<dyn FnMut(web_sys::ErrorEvent)>);
        if raw_socket
            .add_event_listener_with_callback("error", error_listener.as_ref().unchecked_ref())
            .is_err()
        {
            let _ = raw_socket.remove_event_listener_with_callback(
                "message",
                message_listener.as_ref().unchecked_ref(),
            );
            let _ = raw_socket.remove_event_listener_with_callback(
                "close",
                close_listener.as_ref().unchecked_ref(),
            );
            return;
        }

        if ws.accept().is_err() {
            let _ = raw_socket.remove_event_listener_with_callback(
                "message",
                message_listener.as_ref().unchecked_ref(),
            );
            let _ = raw_socket.remove_event_listener_with_callback(
                "close",
                close_listener.as_ref().unchecked_ref(),
            );
            let _ = raw_socket.remove_event_listener_with_callback(
                "error",
                error_listener.as_ref().unchecked_ref(),
            );
            return;
        }

        self.state.wait_until(async move {
            drain_eventsub_socket_events(&stub, &connection_id, rx).await;
            let _ = raw_socket.remove_event_listener_with_callback(
                "message",
                message_listener.as_ref().unchecked_ref(),
            );
            let _ = raw_socket.remove_event_listener_with_callback(
                "close",
                close_listener.as_ref().unchecked_ref(),
            );
            let _ = raw_socket.remove_event_listener_with_callback(
                "error",
                error_listener.as_ref().unchecked_ref(),
            );
        });
    }

    async fn start_eventsub_connection(&self, force: bool) -> Result<()> {
        let mut state = self.load().await?;
        let Some(broadcaster) = state.broadcaster.clone() else {
            return Ok(());
        };
        if state.session.is_none() {
            return Ok(());
        }
        if self.eventsub_runtime.borrow().is_some() && !force {
            return Ok(());
        }
        let Some(_access_token) = self.ensure_fresh_access_token(&mut state, false).await? else {
            return Ok(());
        };
        let connection_url = if force {
            state
                .eventsub
                .reconnect_url
                .clone()
                .unwrap_or_else(|| EVENTSUB_WS_URL.to_string())
        } else {
            EVENTSUB_WS_URL.to_string()
        };
        if !force {
            state.eventsub.reconnect_url = None;
            state.eventsub.websocket_session_id = None;
            state.eventsub.subscription_id = None;
        }
        let connection_id = self.next_eventsub_connection_id();
        let socket = WebSocket::connect(Url::parse(&connection_url)?)
            .await
            .map_err(error_from_display)?;
        state.eventsub.connection_status = Some(if connection_url == EVENTSUB_WS_URL {
            "connecting".to_string()
        } else {
            "reconnecting".to_string()
        });
        state.eventsub.reconnect_due_at_ms = None;
        state.eventsub.last_error = None;
        self.persist(&state).await?;
        self.schedule_alarm(&state).await?;
        self.set_runtime(connection_id.clone(), socket.clone());
        self.spawn_eventsub_loop(connection_id, broadcaster.user_id, socket);
        self.broadcast_status(&state);
        Ok(())
    }

    async fn disconnect_streamer(&self) -> Result<PersistedAfkState> {
        let mut state = self.load().await?;
        let _ = self.cleanup_live_session(&mut state).await?;
        state.broadcaster = None;
        state.tokens = None;
        state.timeout_enabled = default_timeout_enabled();
        state.timeout_duration_secs = default_timeout_duration_secs();
        state.board_size = default_protocol_board_size();
        state.pending_untimeouts.clear();
        self.persist(&state).await?;
        self.schedule_alarm(&state).await?;
        self.broadcast_status(&state);
        Ok(state)
    }

    async fn handle_eventsub_ws_message(
        &self,
        payload: EventSubWebSocketMessageRequest,
    ) -> Result<Response> {
        if !self.runtime_matches(&payload.connection_id) {
            return Response::ok("stale eventsub message");
        }

        let mut state = self.load().await?;
        state.eventsub.last_message_id = Some(payload.envelope.metadata.message_id.clone());
        state.eventsub.last_received_at_ms = Some(now_ms());

        match payload
            .envelope
            .message_type()
            .map_err(error_from_display)?
        {
            EventSubMessageType::SessionWelcome => {
                let session = payload
                    .envelope
                    .session()
                    .cloned()
                    .ok_or_else(|| Error::RustError("missing EventSub session".to_string()))?;
                state.eventsub.connection_status = Some("connected".to_string());
                state.eventsub.websocket_session_id = Some(session.id.clone());
                state.eventsub.reconnect_url = session.reconnect_url.clone();
                state.eventsub.last_error = None;
                self.persist(&state).await?;
                self.reconcile_eventsub_subscription(&mut state, &session)
                    .await?;
                self.broadcast_status(&state);
            }
            EventSubMessageType::SessionKeepalive => {
                state.eventsub.connection_status = Some("connected".to_string());
                self.persist(&state).await?;
                self.broadcast_status(&state);
            }
            EventSubMessageType::SessionReconnect => {
                let session = payload
                    .envelope
                    .session()
                    .cloned()
                    .ok_or_else(|| Error::RustError("missing reconnect session".to_string()))?;
                state.eventsub.connection_status = Some("reconnecting".to_string());
                state.eventsub.reconnect_url = session.reconnect_url.clone();
                state.eventsub.reconnect_due_at_ms =
                    Some(now_ms() + (EVENTSUB_SESSION_RECONNECT_DELAY_SECS as i64 * 1_000));
                self.clear_runtime_if_matches(&payload.connection_id);
                self.persist(&state).await?;
                self.broadcast_status(&state);
            }
            EventSubMessageType::SessionDisconnect => {
                state.eventsub.connection_status = Some("disconnected".to_string());
                state.eventsub.last_error = Some(
                    "Twitch chat disconnected. Return to AFK mode and start again.".to_string(),
                );
                state.eventsub.reconnect_due_at_ms =
                    Some(now_ms() + (EVENTSUB_RECONNECT_RETRY_SECS as i64 * 1_000));
                self.clear_runtime_if_matches(&payload.connection_id);
                self.persist(&state).await?;
                self.broadcast_status(&state);
            }
            EventSubMessageType::Revocation => {
                state.eventsub.connection_status = Some("revoked".to_string());
                state.eventsub.last_error = Some("EventSub subscription revoked".to_string());
                state.eventsub.reconnect_due_at_ms =
                    Some(now_ms() + (EVENTSUB_RECONNECT_RETRY_SECS as i64 * 1_000));
                self.clear_runtime_if_matches(&payload.connection_id);
                self.persist(&state).await?;
                self.broadcast_status(&state);
            }
            EventSubMessageType::Notification => {
                let chat = payload.envelope.chat_message();
                let changed = if let Some(chat) = chat {
                    self.ingest_chat_message(
                        &mut state,
                        &payload.envelope.metadata.message_id,
                        chat,
                    )
                    .await?
                } else {
                    false
                };
                if !changed {
                    self.persist(&state).await?;
                }
            }
        }

        let state = self.load().await?;
        self.schedule_alarm(&state).await?;
        Response::ok("ok")
    }

    async fn handle_eventsub_ws_closed(
        &self,
        payload: EventSubWebSocketClosedRequest,
    ) -> Result<Response> {
        if !self.runtime_matches(&payload.connection_id) {
            return Response::ok("stale close");
        }
        self.clear_runtime_if_matches(&payload.connection_id);
        let mut state = self.load().await?;
        state.eventsub.connection_status = Some("disconnected".to_string());
        state.eventsub.last_error = Some(format!(
            "EventSub socket closed (code {}): {}",
            payload.code, payload.reason
        ));
        state.eventsub.reconnect_due_at_ms =
            Some(now_ms() + (EVENTSUB_RECONNECT_RETRY_SECS as i64 * 1_000));
        self.persist(&state).await?;
        self.schedule_alarm(&state).await?;
        self.broadcast_status(&state);
        Response::ok("ok")
    }

    async fn handle_eventsub_ws_error(
        &self,
        payload: EventSubWebSocketErrorRequest,
    ) -> Result<Response> {
        if !self.runtime_matches(&payload.connection_id) {
            return Response::ok("stale error");
        }
        self.clear_runtime_if_matches(&payload.connection_id);
        let mut state = self.load().await?;
        state.eventsub.connection_status = Some("error".to_string());
        state.eventsub.last_error = Some(payload.message);
        state.eventsub.reconnect_due_at_ms =
            Some(now_ms() + (EVENTSUB_RECONNECT_RETRY_SECS as i64 * 1_000));
        self.persist(&state).await?;
        self.schedule_alarm(&state).await?;
        self.broadcast_status(&state);
        Response::ok("ok")
    }

    async fn handle_websocket(&self) -> Result<Response> {
        let mut state = self.load().await?;
        if self.mark_eventsub_runtime_missing(&mut state).await? {
            self.broadcast_status(&state);
        }
        let pair = WebSocketPair::new()?;
        self.state.accept_web_socket(&pair.server);
        if self.sync_frontend_presence(&mut state, true, now_ms()) {
            self.persist(&state).await?;
            self.schedule_alarm(&state).await?;
        }
        let _ = pair.server.send(&AfkServerMessage::Connected {
            status: self.status_response(&state),
        });
        if let Some(session) = state.session.as_ref().map(|session| {
            session.snapshot(
                state.broadcaster.clone(),
                state.timeout_supported(),
                now_ms(),
            )
        }) {
            let _ = pair.server.send(&AfkServerMessage::Snapshot { session });
        }
        Response::from_websocket(pair.client)
    }

    async fn tick(&self) -> Result<()> {
        let mut state = self.load().await?;
        let mut broadcast_snapshot = false;
        let mut phase_ended = false;
        let mut needs_restart = false;
        let now = now_ms();

        if !self.has_frontend_websockets_excluding(None) {
            let _ = self.sync_frontend_presence(&mut state, false, now);
        }

        if state.session.as_ref().is_some_and(|session| {
            session.inactivity_deadline_at_ms() <= now
                || session
                    .frontend_missing_deadline_at_ms()
                    .is_some_and(|deadline| deadline <= now)
        }) {
            if self.cleanup_live_session(&mut state).await? {
                self.persist(&state).await?;
                self.schedule_alarm(&state).await?;
                self.broadcast_status(&state);
            }
            return Ok(());
        }

        if let Some(session) = state.session.as_mut() {
            let before_phase = session.engine.phase();
            let should_broadcast_countdown = !session.engine.is_paused()
                && matches!(
                    before_phase,
                    CoreAfkRoundPhase::Countdown
                        | CoreAfkRoundPhase::Won
                        | CoreAfkRoundPhase::TimedOut
                );
            let settle = session.engine.settle(now);
            if settle.round_started {
                session.push_activity("Round live", now);
                broadcast_snapshot = true;
            } else if settle.changed {
                broadcast_snapshot = true;
            } else if should_broadcast_countdown {
                broadcast_snapshot = true;
            }
            phase_ended = matches!(before_phase, CoreAfkRoundPhase::Active)
                && matches!(
                    session.engine.phase(),
                    CoreAfkRoundPhase::Won | CoreAfkRoundPhase::TimedOut
                );
            needs_restart = settle.needs_restart;
            if matches!(before_phase, CoreAfkRoundPhase::Active)
                && matches!(session.engine.phase(), CoreAfkRoundPhase::TimedOut)
            {
                session.push_activity("Round timed out", now);
                broadcast_snapshot = true;
            }
        }

        if phase_ended || needs_restart {
            self.release_round_timeouts(&mut state).await?;
        }
        if needs_restart {
            if let Some(session) = state.session.as_mut() {
                session.restart_round(now);
            }
            broadcast_snapshot = true;
        }

        if self.eventsub_runtime.borrow().is_none()
            && state
                .eventsub
                .reconnect_due_at_ms
                .is_some_and(|deadline| deadline <= now_ms())
        {
            state.eventsub.reconnect_due_at_ms = None;
            self.persist(&state).await?;
            if let Err(error) = self.start_eventsub_connection(true).await {
                state = self.load().await?;
                self.set_eventsub_error(
                    &mut state,
                    format!("Failed to reconnect Twitch chat: {error}"),
                )
                .await?;
            }
            state = self.load().await?;
        } else {
            self.persist(&state).await?;
        }

        if broadcast_snapshot {
            self.broadcast_snapshot(&state);
        }
        self.schedule_alarm(&state).await?;
        Ok(())
    }
}

impl DurableObject for AfkSessionDO {
    fn new(state: State, env: Env) -> Self {
        Self {
            state,
            env,
            cache: RefCell::new(None),
            eventsub_runtime: RefCell::new(None),
            eventsub_connection_seq: RefCell::new(0),
        }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = normalized_request_path(&req, &self.env);
        match (req.method(), path.as_str()) {
            (Method::Get, "/api/afk/status") => {
                let mut state = self.load().await?;
                if self.mark_eventsub_runtime_missing(&mut state).await? {
                    self.broadcast_status(&state);
                }
                Response::from_json(&self.status_response(&state))
            }
            (Method::Post, "/api/afk/start") => {
                let mut state = self.load().await?;
                self.release_round_timeouts(&mut state).await?;
                let mut session = PersistedAfkSession::new(
                    protocol_board_size_to_core(state.board_size),
                    state.timeout_enabled && state.timeout_supported(),
                    now_ms(),
                );
                if !self.has_frontend_websockets_excluding(None) {
                    session.mark_frontend_missing(now_ms());
                }
                state.session = Some(session);
                self.persist(&state).await?;
                self.schedule_alarm(&state).await?;
                self.broadcast_snapshot(&state);
                if let Err(error) = self.start_eventsub_connection(false).await {
                    state = self.load().await?;
                    self.set_eventsub_error(
                        &mut state,
                        format!("Failed to connect Twitch chat: {error}"),
                    )
                    .await?;
                    state = self.load().await?;
                } else {
                    state = self.load().await?;
                }
                Response::from_json(&self.status_response(&state))
            }
            (Method::Post, "/api/afk/action") => {
                let payload: AfkActionRequest = read_json(&mut req).await?;
                let mut state = self.load().await?;
                let _ = self
                    .apply_streamer_action(&mut state, request_action_to_core(payload)?)
                    .await?;
                self.schedule_alarm(&state).await?;
                Response::from_json(&self.status_response(&state))
            }
            (Method::Post, "/api/afk/board-size") => {
                let payload: SetBoardSizePreferenceRequest = read_json(&mut req).await?;
                let mut state = self.load().await?;
                state.board_size = normalize_protocol_board_size(payload.board_size);
                self.persist(&state).await?;
                self.broadcast_status(&state);
                Response::from_json(&self.status_response(&state))
            }
            (Method::Post, "/api/afk/pause") => {
                let mut state = self.load().await?;
                let now = now_ms();
                let changed = state.session.as_mut().is_some_and(|session| {
                    session.record_user_activity(now);
                    session.pause(now)
                });
                self.persist(&state).await?;
                self.schedule_alarm(&state).await?;
                if changed {
                    self.broadcast_snapshot(&state);
                }
                Response::from_json(&self.status_response(&state))
            }
            (Method::Post, "/api/afk/resume") => {
                let mut state = self.load().await?;
                let now = now_ms();
                let changed = state.session.as_mut().is_some_and(|session| {
                    session.record_user_activity(now);
                    session.resume(now)
                });
                self.persist(&state).await?;
                if let Err(error) = self.start_eventsub_connection(false).await {
                    state = self.load().await?;
                    self.set_eventsub_error(
                        &mut state,
                        format!("Failed to connect Twitch chat: {error}"),
                    )
                    .await?;
                    state = self.load().await?;
                } else {
                    state = self.load().await?;
                }
                self.schedule_alarm(&state).await?;
                if changed {
                    self.broadcast_snapshot(&state);
                }
                Response::from_json(&self.status_response(&state))
            }
            (Method::Post, "/api/afk/stop") => {
                let mut state = self.load().await?;
                let _ = self.cleanup_live_session(&mut state).await?;
                self.persist(&state).await?;
                self.schedule_alarm(&state).await?;
                self.broadcast_status(&state);
                Response::from_json(&self.status_response(&state))
            }
            (Method::Post, "/api/afk/continue") => {
                let mut state = self.load().await?;
                self.release_round_timeouts(&mut state).await?;
                let now = now_ms();
                if let Some(session) = state.session.as_mut() {
                    session.record_user_activity(now);
                    session.restart_round(now);
                }
                self.persist(&state).await?;
                self.schedule_alarm(&state).await?;
                self.broadcast_snapshot(&state);
                Response::from_json(&self.status_response(&state))
            }
            (Method::Post, "/api/afk/timeout") => {
                let payload: SetTimeoutPreferenceRequest = read_json(&mut req).await?;
                let mut state = self.load().await?;
                if let Some(enabled) = payload.enabled {
                    state.timeout_enabled = enabled;
                }
                if let Some(duration_secs) = payload.duration_secs {
                    state.timeout_duration_secs = normalize_timeout_duration_secs(duration_secs);
                }
                let timeout_supported = state.timeout_supported();
                if let Some(session) = state.session.as_mut() {
                    session.timeout_enabled = state.timeout_enabled && timeout_supported;
                }
                self.persist(&state).await?;
                self.broadcast_status(&state);
                Response::from_json(&self.status_response(&state))
            }
            (Method::Post, "/api/afk/panic-reset") => {
                let mut state = self.load().await?;
                let _ = self.cleanup_live_session(&mut state).await?;
                self.persist(&state).await?;
                self.schedule_alarm(&state).await?;
                self.broadcast_status(&state);
                Response::from_json(&self.status_response(&state))
            }
            (Method::Get, "/ws/afk") => self.handle_websocket().await,
            (Method::Post, "/internal/link") => {
                let payload: LinkStreamerRequest = read_json(&mut req).await?;
                let mut state = self.load().await?;
                state.broadcaster = Some(payload.identity);
                state.tokens = Some(payload.tokens);
                let timeout_supported = state.timeout_supported();
                if let Some(session) = state.session.as_mut() {
                    session.timeout_enabled = state.timeout_enabled && timeout_supported;
                }
                self.persist(&state).await?;
                self.broadcast_status(&state);
                Response::from_json(&self.status_response(&state))
            }
            (Method::Post, "/internal/unlink") => {
                let state = self.disconnect_streamer().await?;
                Response::from_json(&self.status_response(&state))
            }
            (Method::Post, "/internal/eventsub/ensure") => {
                let payload: EnsureEventSubRequest = read_json_or_default(&mut req).await?;
                self.start_eventsub_connection(payload.force).await?;
                Response::ok("ok")
            }
            (Method::Post, "/internal/eventsub/message") => {
                let payload: EventSubWebSocketMessageRequest = read_json(&mut req).await?;
                self.handle_eventsub_ws_message(payload).await
            }
            (Method::Post, "/internal/eventsub/closed") => {
                let payload: EventSubWebSocketClosedRequest = read_json(&mut req).await?;
                self.handle_eventsub_ws_closed(payload).await
            }
            (Method::Post, "/internal/eventsub/error") => {
                let payload: EventSubWebSocketErrorRequest = read_json(&mut req).await?;
                self.handle_eventsub_ws_error(payload).await
            }
            _ => Response::error("not found", 404),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        self.tick().await?;
        Response::ok("ok")
    }

    async fn websocket_message(
        &self,
        ws: WebSocket,
        message: WebSocketIncomingMessage,
    ) -> Result<()> {
        if let WebSocketIncomingMessage::String(text) = message {
            match serde_json::from_str::<AfkClientMessage>(&text) {
                Ok(AfkClientMessage::Ping) => {
                    let mut state = self.load().await?;
                    if self.sync_frontend_presence(&mut state, true, now_ms()) {
                        self.persist(&state).await?;
                        self.schedule_alarm(&state).await?;
                    }
                    let _ = ws.send(&AfkServerMessage::Connected {
                        status: self.status_response(&state),
                    });
                    if let Some(session) = state.session.as_ref().map(|session| {
                        session.snapshot(
                            state.broadcaster.clone(),
                            state.timeout_supported(),
                            now_ms(),
                        )
                    }) {
                        let _ = ws.send(&AfkServerMessage::Snapshot { session });
                    }
                }
                Err(_) => {
                    let _ = ws.send(&AfkServerMessage::Error {
                        message: "invalid websocket payload".to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    async fn websocket_close(
        &self,
        ws: WebSocket,
        _code: usize,
        _reason: String,
        _was_clean: bool,
    ) -> Result<()> {
        let mut state = self.load().await?;
        if !self.has_frontend_websockets_excluding(Some(&ws))
            && self.sync_frontend_presence(&mut state, false, now_ms())
        {
            self.persist(&state).await?;
            self.schedule_alarm(&state).await?;
        }
        Ok(())
    }

    async fn websocket_error(&self, _ws: WebSocket, _error: Error) -> Result<()> {
        Ok(())
    }
}

fn parse_chat_command(text: &str) -> Option<ParsedChatCommand> {
    let trimmed = text.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return None;
    }

    let tokens: Vec<_> = trimmed.split_whitespace().collect();
    if tokens.as_slice() == ["!continue"] {
        return Some(ParsedChatCommand::Continue);
    }

    match tokens.as_slice() {
        [] => None,
        ["!f" | "!flag", rest @ ..] => parse_board_batch(rest, AfkAction::SetFlag),
        ["!u" | "!unflag", rest @ ..] => parse_board_batch(rest, AfkAction::ClearFlag),
        ["!c", rest @ ..] => parse_board_batch(rest, AfkAction::Chord),
        [first, ..] if first.starts_with('!') => None,
        _ => parse_board_batch(&tokens, AfkAction::Reveal),
    }
}

fn chat_actor_is_timeout_exempt(chat: &EventSubChatMessage) -> bool {
    chat.badges
        .iter()
        .any(|badge| matches!(badge.set_id.as_str(), "moderator" | "broadcaster"))
}

fn parse_board_batch<F>(tokens: &[&str], make_action: F) -> Option<ParsedChatCommand>
where
    F: Fn((u8, u8)) -> AfkAction,
{
    let actions = tokens
        .iter()
        .map(|token| {
            parse_coord(token).map(|coords| ParsedBoardAction {
                action: make_action(coords),
                coords,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    (!actions.is_empty()).then_some(ParsedChatCommand::BoardBatch(actions))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ParsedBoardAction {
    action: AfkAction,
    coords: (u8, u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ParsedChatCommand {
    Continue,
    BoardBatch(Vec<ParsedBoardAction>),
}

/// Row labels rendered on each cell: digits for the first 9 rows, then lowercase letters.
/// Parsed case-insensitively (input is lowercased before lookup).
const AFK_ROW_LABELS: &str = "123456789abcdefghijk";
/// Column labels rendered on each cell: lowercase letters followed by digits for columns 27+.
/// Parsed case-insensitively (input is lowercased before lookup).
const AFK_COLUMN_LABELS: &str = "abcdefghijklmnopqrstuvwxyz0123";

fn parse_coord(value: &str) -> Option<(u8, u8)> {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.len() != 2 {
        return None;
    }
    let mut chars = trimmed.chars();
    let row = chars.next()?;
    let column = chars.next()?;
    let y = AFK_ROW_LABELS.find(row)?.try_into().ok()?;
    let x = AFK_COLUMN_LABELS.find(column)?.try_into().ok()?;
    Some((x, y))
}

fn format_coord((x, y): (u8, u8)) -> String {
    let row = AFK_ROW_LABELS
        .as_bytes()
        .get(usize::from(y))
        .copied()
        .map(char::from)
        .unwrap_or('?');
    let column = AFK_COLUMN_LABELS
        .as_bytes()
        .get(usize::from(x))
        .copied()
        .map(char::from)
        .unwrap_or('?');
    format!("{row}{column}")
}

fn request_action_to_core(request: AfkActionRequest) -> Result<AfkAction> {
    let coords = (request.x, request.y);
    Ok(match request.kind {
        AfkActionKind::Reveal => AfkAction::Reveal(coords),
        AfkActionKind::ToggleFlag => AfkAction::ToggleFlag(coords),
        AfkActionKind::Chord => AfkAction::Chord(coords),
        AfkActionKind::ChordFlag => AfkAction::ChordFlag(coords),
    })
}

fn random_seed() -> u64 {
    use js_sys::Math::random;
    u64::from_be_bytes([
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
    ])
}

fn to_afk_identity(outcome: &TwitchAuthOutcome) -> AfkIdentity {
    AfkIdentity::new(
        outcome.identity.user_id.clone(),
        outcome.identity.login.clone(),
        outcome.identity.display_name.clone(),
    )
}

fn now_ms() -> i64 {
    js_sys::Date::now() as i64
}

fn error_from_display(error: impl core::fmt::Display) -> Error {
    Error::RustError(error.to_string())
}

const fn default_timeout_enabled() -> bool {
    true
}

const fn default_board_size() -> CoreAfkBoardSize {
    CoreAfkBoardSize::Medium
}

const fn default_timeout_duration_secs() -> u32 {
    DEFAULT_TIMEOUT_DURATION_SECS
}

const fn default_protocol_board_size() -> ProtocolAfkBoardSize {
    ProtocolAfkBoardSize::Medium
}

fn normalize_timeout_duration_secs(value: u32) -> u32 {
    let mut best = TIMEOUT_DURATION_OPTIONS_SECS[0];
    let mut best_diff = best.abs_diff(value);
    for candidate in TIMEOUT_DURATION_OPTIONS_SECS.iter().copied().skip(1) {
        let diff = candidate.abs_diff(value);
        if diff < best_diff {
            best = candidate;
            best_diff = diff;
        }
    }
    best
}

const fn normalize_protocol_board_size(value: ProtocolAfkBoardSize) -> ProtocolAfkBoardSize {
    value
}

const fn protocol_board_size_to_core(value: ProtocolAfkBoardSize) -> CoreAfkBoardSize {
    match value {
        ProtocolAfkBoardSize::Tiny => CoreAfkBoardSize::Tiny,
        ProtocolAfkBoardSize::Small => CoreAfkBoardSize::Small,
        ProtocolAfkBoardSize::Medium => CoreAfkBoardSize::Medium,
        ProtocolAfkBoardSize::Large => CoreAfkBoardSize::Large,
    }
}

fn chat_board_action_targets_labeled_cell(
    session: &PersistedAfkSession,
    parsed: ParsedBoardAction,
) -> Result<bool> {
    if !matches!(session.engine.phase(), CoreAfkRoundPhase::Active) {
        return Ok(false);
    }
    if parsed.coords.0 >= session.engine.size().0 || parsed.coords.1 >= session.engine.size().1 {
        return Ok(false);
    }
    let labels = session.engine.labeled_cells();
    Ok(labels[flat_index(session.engine.size(), parsed.coords)])
}

fn configured_var(env: &Env, name: &str) -> String {
    env.var(name)
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn auth_signing_secret(env: &Env) -> String {
    configured_var(env, "AUTH_SIGNING_SECRET")
}

fn configured_base_path(env: &Env) -> String {
    let configured = configured_var(env, "BASE_PATH");
    if !configured.trim().is_empty() {
        return normalize_base_path(&configured);
    }
    let public_url = configured_var(env, "PUBLIC_URL");
    if public_url.is_empty() {
        "/".to_string()
    } else {
        Url::parse(&public_url)
            .ok()
            .map(|url| normalize_base_path(url.path()))
            .unwrap_or_else(|| "/".to_string())
    }
}

fn public_base_url(env: &Env, req: &Request) -> Result<String> {
    let configured = configured_var(env, "PUBLIC_URL");
    if !configured.is_empty() {
        return Ok(configured.trim_end_matches('/').to_string());
    }

    let mut url = req.url()?;
    url.set_path(&configured_base_path(env));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn normalize_base_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    let normalized = trimmed.trim_end_matches('/');
    if normalized.starts_with('/') {
        normalized.to_string()
    } else {
        format!("/{normalized}")
    }
}

fn join_base_path(base_path: &str, path: &str) -> String {
    let base_path = normalize_base_path(base_path);
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };

    if base_path == "/" {
        path
    } else if path == "/" {
        format!("{base_path}/")
    } else {
        format!("{base_path}{path}")
    }
}

fn path_without_base_prefix<'a>(path: &'a str, base_path: &str) -> &'a str {
    let base_path = normalize_base_path(base_path);
    if base_path == "/" {
        return path;
    }
    path.strip_prefix(&base_path).unwrap_or(path)
}

fn normalized_request_path(req: &Request, env: &Env) -> String {
    path_without_base_prefix(&req.path(), &configured_base_path(env)).to_string()
}

fn redirect_with_cookie(url: &str, cookie: &str) -> Result<Response> {
    let _ = Url::parse(url)?;
    Ok(ResponseBuilder::new()
        .with_status(302)
        .with_header("Location", url)?
        .with_header("Set-Cookie", cookie)?
        .with_header("Cache-Control", "no-store")?
        .empty())
}

fn auth_callback_error_response(
    req: &Request,
    env: &Env,
    code: &str,
    detail: &str,
) -> Result<Response> {
    log::warn!("twitch callback failed: {code}: {detail}");
    let public_url = public_base_url(env, req)?;
    let redirect_url = auth_callback_error_redirect_url(&public_url, code)?;
    redirect_with_cookie(
        &redirect_url,
        &cleared_auth_cookie_header(
            &configured_base_path(env),
            public_url.starts_with("https://"),
        ),
    )
}

fn auth_callback_error_redirect_url(public_url: &str, code: &str) -> Result<String> {
    let mut url = Url::parse(public_url)?;
    let base_path = normalize_base_path(url.path());
    let root_path = if base_path == "/" {
        "/".to_string()
    } else {
        format!("{base_path}/")
    };
    url.set_path(&root_path);
    url.set_query(Some(&format!(
        "view=afk&afk_auth_error={}",
        sanitize_auth_callback_error_code(code)
    )));
    url.set_fragment(None);
    Ok(url.to_string())
}

fn sanitize_auth_callback_error_code(code: &str) -> String {
    let sanitized: String = code
        .chars()
        .map(|ch| match ch {
            'a'..='z' | '0'..='9' | '_' | '-' => ch,
            'A'..='Z' => ch.to_ascii_lowercase(),
            _ => '_',
        })
        .collect();
    if sanitized.is_empty() {
        "auth_error".to_string()
    } else {
        sanitized
    }
}

fn auth_error_response(error: RequestAuthError) -> Result<Response> {
    match error {
        RequestAuthError::Missing => Response::error("missing auth token", 401),
        RequestAuthError::Invalid => Response::error("invalid auth token", 401),
    }
}

fn request_auth_token(req: &Request) -> Result<Option<String>> {
    if let Some(header) = req.headers().get("Authorization")? {
        if let Some(token) = auth_token_from_authorization_header(&header) {
            return Ok(Some(token.to_string()));
        }
    }
    if let Some(header) = req.headers().get("Cookie")? {
        if let Some(token) = auth_token_from_cookie_header(&header) {
            return Ok(Some(token.to_string()));
        }
    }
    Ok(None)
}

fn optional_auth_from_request(
    req: &Request,
    env: &Env,
    now_ms: i64,
) -> std::result::Result<Option<SignedAuthClaims>, RequestAuthError> {
    let token = request_auth_token(req).map_err(|_| RequestAuthError::Invalid)?;
    let Some(token) = token else {
        return Ok(None);
    };
    verify_auth_token(&auth_signing_secret(env), &token, now_ms)
        .map(Some)
        .map_err(|_| RequestAuthError::Invalid)
}

fn require_identity_auth(
    req: &Request,
    env: &Env,
    now_ms: i64,
) -> std::result::Result<SignedAuthClaims, RequestAuthError> {
    optional_auth_from_request(req, env, now_ms)?.ok_or(RequestAuthError::Missing)
}

fn maybe_refresh_auth_cookie(
    response: &mut Response,
    env: &Env,
    claims: Option<&SignedAuthClaims>,
    secure: bool,
) -> Result<()> {
    let Some(claims) = claims else {
        return Ok(());
    };
    if !should_refresh_auth_token(claims, now_ms()) {
        return Ok(());
    }
    detach_response_headers(response)?;
    let refreshed = refreshed_auth_claims(claims, now_ms());
    let token =
        sign_auth_token(&auth_signing_secret(env), &refreshed).map_err(error_from_display)?;
    response.headers_mut().set(
        "Set-Cookie",
        &auth_cookie_header(&token, &configured_base_path(env), secure),
    )?;
    response.headers_mut().set("Cache-Control", "no-store")?;
    Ok(())
}

/// Durable-object and fetch responses can carry an immutable JS `Headers` guard.
/// Rebuild the response with a cloned header bag before mutating it so refresh
/// cookies do not depend on the upstream response type.
fn detach_response_headers(response: &mut Response) -> Result<()> {
    let mutable_headers = response.headers().clone();
    let original = mem::replace(response, Response::empty()?);
    let (builder, body) = original.into_parts();
    *response = builder.with_headers(mutable_headers).body(body);
    Ok(())
}

fn afk_session_stub(env: &Env, broadcaster_user_id: &str) -> Result<Stub> {
    env.durable_object(AFK_SESSIONS)?
        .id_from_name(broadcaster_user_id)?
        .get_stub()
}

async fn post_json<T: Serialize>(stub: &Stub, url: &str, body: &T) -> Result<Response> {
    let headers = Headers::new();
    headers.set("Content-Type", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post);
    init.with_headers(headers);
    init.with_body(Some(JsValue::from_str(&serde_json::to_string(body)?)));
    let request = Request::new_with_init(url, &init)?;
    stub.fetch_with_request(request).await
}

async fn post_empty(stub: &Stub, url: &str) -> Result<Response> {
    let mut init = RequestInit::new();
    init.with_method(Method::Post);
    let request = Request::new_with_init(url, &init)?;
    stub.fetch_with_request(request).await
}

async fn drain_eventsub_socket_events(
    stub: &Stub,
    connection_id: &str,
    mut rx: UnboundedReceiver<EventSubSocketEvent>,
) {
    while let Some(event) = rx.next().await {
        match event {
            EventSubSocketEvent::Message(Some(text)) => {
                if let Ok(envelope) = decode_eventsub_websocket_message(&text) {
                    let _ = post_json(
                        stub,
                        "https://internal/internal/eventsub/message",
                        &EventSubWebSocketMessageRequest {
                            connection_id: connection_id.to_string(),
                            envelope,
                        },
                    )
                    .await;
                }
            }
            EventSubSocketEvent::Close {
                code,
                reason,
                was_clean,
            } => {
                let _ = post_json(
                    stub,
                    "https://internal/internal/eventsub/closed",
                    &EventSubWebSocketClosedRequest {
                        connection_id: connection_id.to_string(),
                        code,
                        reason,
                        was_clean,
                    },
                )
                .await;
                break;
            }
            EventSubSocketEvent::Error(message) => {
                let _ = post_json(
                    stub,
                    "https://internal/internal/eventsub/error",
                    &EventSubWebSocketErrorRequest {
                        connection_id: connection_id.to_string(),
                        message,
                    },
                )
                .await;
            }
            EventSubSocketEvent::Message(None) => {}
        }
    }
}

async fn read_json<T>(req: &mut Request) -> Result<T>
where
    T: DeserializeOwned,
{
    req.json().await
}

async fn read_json_or_default<T>(req: &mut Request) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    let body = req.text().await.unwrap_or_default();
    if body.trim().is_empty() {
        return Ok(T::default());
    }
    serde_json::from_str(&body).map_err(error_from_display)
}

async fn load_storage_json<T>(storage: &Storage, key: &str) -> Result<Option<T>>
where
    T: DeserializeOwned,
{
    match storage.get::<String>(key).await {
        Ok(Some(raw)) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(error_from_display),
        Ok(None) => match storage.get::<T>(key).await {
            Ok(value) => Ok(value),
            Err(error) => Err(error_from_display(error)),
        },
        Err(_) => match storage.get::<T>(key).await {
            Ok(value) => Ok(value),
            Err(error) => Err(error_from_display(error)),
        },
    }
}

async fn persist_storage_json<T>(storage: &Storage, key: &str, value: &T) -> Result<()>
where
    T: Serialize,
{
    let json = serde_json::to_string(value)?;
    debug_assert!(
        json.len() <= PERSISTED_STATE_SIZE_LIMIT,
        "Serialized state for key '{key}' is {} bytes, exceeding the \
         {PERSISTED_STATE_SIZE_LIMIT} byte safety threshold \
         (Cloudflare DO limit is 128 KiB). Review unbounded Vec fields.",
        json.len(),
    );
    storage.put(key, json).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session() -> PersistedAfkSession {
        PersistedAfkSession {
            engine: AfkEngine::new(0, AfkPreset::v1(), 0),
            ignored_users: Vec::new(),
            recent_penalties: Vec::new(),
            timed_out_users: Vec::new(),
            activity: Vec::new(),
            last_action: None,
            timeout_enabled: true,
            last_user_activity_at_ms: 1,
            frontend_missing_since_at_ms: None,
        }
    }

    fn line_session(width: u8, mine_xs: &[u8]) -> PersistedAfkSession {
        let mine_coords: Vec<(u8, u8)> = mine_xs.iter().copied().map(|x| (x, 0)).collect();
        let layout = detonito_core::MineLayout::from_mine_coords((width, 1), &mine_coords).unwrap();
        let mut session = test_session();
        session.engine = AfkEngine::with_layout_for_tests(
            layout,
            AfkPreset {
                config: detonito_core::GameConfig::new_unchecked((width, 1), mine_xs.len() as u16),
                timer: AfkPreset::v1().timer,
            },
            1_000,
        );
        session
    }

    #[test]
    fn reveal_command_parses() {
        let parsed = parse_chat_command("1a").expect("command should parse");
        assert_eq!(
            parsed,
            ParsedChatCommand::BoardBatch(vec![ParsedBoardAction {
                action: AfkAction::Reveal((0, 0)),
                coords: (0, 0),
            }])
        );
        assert_eq!(
            parse_chat_command("a1"),
            Some(ParsedChatCommand::BoardBatch(vec![ParsedBoardAction {
                action: AfkAction::Reveal((27, 9)),
                coords: (27, 9),
            }]))
        );
        assert_eq!(
            parse_chat_command("1a 2b"),
            Some(ParsedChatCommand::BoardBatch(vec![
                ParsedBoardAction {
                    action: AfkAction::Reveal((0, 0)),
                    coords: (0, 0),
                },
                ParsedBoardAction {
                    action: AfkAction::Reveal((1, 1)),
                    coords: (1, 1),
                },
            ]))
        );
    }

    #[test]
    fn reveal_command_is_case_insensitive() {
        assert_eq!(parse_chat_command("1A"), parse_chat_command("1a"));
        assert_eq!(parse_chat_command("!F 3C"), parse_chat_command("!f 3c"));
        assert_eq!(
            parse_chat_command("!FLAG 3C 4D"),
            parse_chat_command("!flag 3c 4d")
        );
    }

    #[test]
    fn flag_and_chord_commands_parse() {
        assert_eq!(
            parse_chat_command("!f 3c"),
            Some(ParsedChatCommand::BoardBatch(vec![ParsedBoardAction {
                action: AfkAction::SetFlag((2, 2)),
                coords: (2, 2),
            }]))
        );
        assert_eq!(
            parse_chat_command("!flag 3c"),
            Some(ParsedChatCommand::BoardBatch(vec![ParsedBoardAction {
                action: AfkAction::SetFlag((2, 2)),
                coords: (2, 2),
            }]))
        );
        assert_eq!(
            parse_chat_command("!f 3c 4d"),
            Some(ParsedChatCommand::BoardBatch(vec![
                ParsedBoardAction {
                    action: AfkAction::SetFlag((2, 2)),
                    coords: (2, 2),
                },
                ParsedBoardAction {
                    action: AfkAction::SetFlag((3, 3)),
                    coords: (3, 3),
                },
            ]))
        );
        assert_eq!(
            parse_chat_command("!u 3c"),
            Some(ParsedChatCommand::BoardBatch(vec![ParsedBoardAction {
                action: AfkAction::ClearFlag((2, 2)),
                coords: (2, 2),
            }]))
        );
        assert_eq!(
            parse_chat_command("!unflag 3c"),
            Some(ParsedChatCommand::BoardBatch(vec![ParsedBoardAction {
                action: AfkAction::ClearFlag((2, 2)),
                coords: (2, 2),
            }]))
        );
        assert_eq!(
            parse_chat_command("!c 9b"),
            Some(ParsedChatCommand::BoardBatch(vec![ParsedBoardAction {
                action: AfkAction::Chord((1, 8)),
                coords: (1, 8),
            }]))
        );
    }

    #[test]
    fn continue_command_parses() {
        assert_eq!(
            parse_chat_command("!continue"),
            Some(ParsedChatCommand::Continue)
        );
    }

    #[test]
    fn malformed_commands_are_rejected() {
        assert_eq!(parse_chat_command("!f nope"), None);
        assert_eq!(parse_chat_command("!flag 3c nope 4d"), None);
        assert_eq!(parse_chat_command("hi there"), None);
        assert_eq!(parse_chat_command(""), None);
        assert_eq!(parse_chat_command("!!"), None);
    }

    #[test]
    fn timeout_duration_defaults_to_thirty_seconds() {
        let state = PersistedAfkState::default();
        assert_eq!(state.timeout_duration_secs, 30);
    }

    #[test]
    fn board_size_defaults_to_medium() {
        let state = PersistedAfkState::default();
        assert_eq!(state.board_size, ProtocolAfkBoardSize::Medium);
    }

    #[cfg(target_arch = "wasm32")]
    #[test]
    fn detaching_response_headers_preserves_response_and_allows_mutation() {
        let mut response = ResponseBuilder::new()
            .with_status(202)
            .with_header("X-Test", "before")
            .expect("header should be set")
            .body(ResponseBody::Body(b"ok".to_vec()));

        detach_response_headers(&mut response).expect("response should be rebuilt");
        response
            .headers_mut()
            .set("X-After", "after")
            .expect("detached headers should be mutable");

        assert_eq!(response.status_code(), 202);
        assert_eq!(
            response
                .headers()
                .get("X-Test")
                .expect("header read should succeed"),
            Some("before".into())
        );
        assert_eq!(
            response
                .headers()
                .get("X-After")
                .expect("header read should succeed"),
            Some("after".into())
        );
        match response.body() {
            ResponseBody::Body(bytes) => assert_eq!(bytes.as_slice(), b"ok"),
            body => panic!("expected fixed body, got {body:?}"),
        }
    }

    #[test]
    fn timeout_duration_normalizes_to_supported_steps() {
        assert_eq!(normalize_timeout_duration_secs(0), 1);
        assert_eq!(normalize_timeout_duration_secs(33), 30);
        assert_eq!(normalize_timeout_duration_secs(59), 60);
        assert_eq!(normalize_timeout_duration_secs(600), 300);
    }

    #[test]
    fn automatic_activity_rows_do_not_refresh_user_activity() {
        let mut session = test_session();
        session.last_user_activity_at_ms = 42;

        session.push_activity("Round restarted", 5_000);
        session.push_activity("Round live", 5_000);

        assert_eq!(session.last_user_activity_at_ms, 42);
    }

    #[test]
    fn session_policy_alarm_prefers_frontend_absence_timeout() {
        let mut session = test_session();
        session.last_user_activity_at_ms = 1_000;
        session.frontend_missing_since_at_ms = Some(2_000);

        assert_eq!(
            session.next_policy_alarm_at_ms(),
            2_000 + AFK_FRONTEND_ABSENCE_TIMEOUT_MS
        );
    }

    #[test]
    fn session_policy_alarm_uses_inactivity_timeout_when_frontend_is_present() {
        let mut session = test_session();
        session.last_user_activity_at_ms = 1_000;

        assert_eq!(
            session.next_policy_alarm_at_ms(),
            1_000 + AFK_SESSION_INACTIVITY_TIMEOUT_MS
        );
    }

    #[test]
    fn paused_sessions_still_have_policy_cleanup_deadlines() {
        let mut session = test_session();
        session.pause(2_000);
        session.last_user_activity_at_ms = 1_000;

        assert_eq!(session.engine.next_alarm_at_ms(2_000), None);
        assert_eq!(
            session.next_policy_alarm_at_ms(),
            1_000 + AFK_SESSION_INACTIVITY_TIMEOUT_MS
        );
    }

    #[test]
    fn frontend_presence_tracking_clears_absence_window_on_return() {
        let mut session = test_session();

        assert!(session.mark_frontend_missing(1_000));
        assert_eq!(session.frontend_missing_since_at_ms, Some(1_000));
        assert!(session.mark_frontend_present());
        assert_eq!(session.frontend_missing_since_at_ms, None);
    }

    #[test]
    fn chat_connection_is_idle_without_an_active_run() {
        let mut state = PersistedAfkState::default();
        state.broadcaster = Some(AfkIdentity::new("1", "streamer", "Streamer"));

        let (connection, error) = chat_connection_for_response(false, &state);
        assert_eq!(connection, AfkChatConnectionState::Idle);
        assert_eq!(error, None);
    }

    #[test]
    fn chat_connection_is_connected_with_live_runtime() {
        let mut state = PersistedAfkState::default();
        state.broadcaster = Some(AfkIdentity::new("1", "streamer", "Streamer"));
        state.session = Some(test_session());
        state.eventsub.connection_status = Some("connected".to_string());

        let (connection, error) = chat_connection_for_response(true, &state);
        assert_eq!(connection, AfkChatConnectionState::Connected);
        assert_eq!(error, None);
    }

    #[test]
    fn chat_connection_reports_stale_connected_state_as_error() {
        let mut state = PersistedAfkState::default();
        state.broadcaster = Some(AfkIdentity::new("1", "streamer", "Streamer"));
        state.session = Some(test_session());
        state.eventsub.connection_status = Some("connected".to_string());

        let (connection, error) = chat_connection_for_response(false, &state);
        assert_eq!(connection, AfkChatConnectionState::Error);
        assert_eq!(
            error.as_deref(),
            Some("Twitch chat connection was lost. Return to AFK mode and start again.")
        );
    }

    #[test]
    fn chat_connection_uses_persisted_eventsub_error() {
        let mut state = PersistedAfkState::default();
        state.broadcaster = Some(AfkIdentity::new("1", "streamer", "Streamer"));
        state.session = Some(test_session());
        state.eventsub.connection_status = Some("error".to_string());
        state.eventsub.last_error = Some("socket failed".to_string());

        let (connection, error) = chat_connection_for_response(false, &state);
        assert_eq!(connection, AfkChatConnectionState::Error);
        assert_eq!(error.as_deref(), Some("socket failed"));
    }

    #[test]
    fn auth_callback_error_codes_are_sanitized_for_query_use() {
        assert_eq!(
            sanitize_auth_callback_error_code("Expired OAuth State"),
            "expired_oauth_state"
        );
        assert_eq!(
            sanitize_auth_callback_error_code("access_denied"),
            "access_denied"
        );
    }

    #[test]
    fn auth_callback_error_redirect_url_preserves_base_path() {
        let redirect_url = auth_callback_error_redirect_url(
            "http://localhost:4365/detonito",
            "expired_oauth_state",
        )
        .expect("redirect url should build");
        assert_eq!(
            redirect_url,
            "http://localhost:4365/detonito/?view=afk&afk_auth_error=expired_oauth_state"
        );
    }

    #[test]
    fn snapshot_keeps_board_timer_for_finished_rounds_and_exposes_prompt_countdown() {
        let now_ms = 10_000;
        let preset = AfkPreset {
            config: detonito_core::GameConfig::new_unchecked((2, 1), 1),
            timer: AfkPreset::v1().timer,
        };

        let mut won_session = test_session();
        let won_layout = detonito_core::MineLayout::from_mine_coords((2, 1), &[(0, 0)]).unwrap();
        won_session.engine = AfkEngine::with_layout_for_tests(won_layout, preset, now_ms);
        won_session
            .engine
            .apply_action(AfkAction::Reveal((1, 0)), now_ms)
            .expect("winning reveal should succeed");
        let won_timer = won_session.engine.timer_remaining_secs();
        let won_snapshot = won_session.snapshot(None, true, now_ms + 5_000);
        assert_eq!(won_snapshot.timer_remaining_secs, won_timer);
        assert_eq!(won_snapshot.phase_countdown_secs, Some(25));
        assert_eq!(won_snapshot.current_level, 1);

        let mut timed_out_session = test_session();
        let timed_out_layout =
            detonito_core::MineLayout::from_mine_coords((2, 1), &[(0, 0)]).unwrap();
        timed_out_session.engine =
            AfkEngine::with_layout_for_tests(timed_out_layout, preset, now_ms);
        timed_out_session
            .engine
            .force_timed_out(CoreAfkLossReason::Timer, now_ms);
        let timed_out_snapshot = timed_out_session.snapshot(None, true, now_ms + 7_000);
        assert_eq!(timed_out_snapshot.timer_remaining_secs, 0);
        assert_eq!(timed_out_snapshot.phase_countdown_secs, Some(53));
        assert_eq!(timed_out_snapshot.current_level, 1);
    }

    #[test]
    fn snapshot_reveals_mines_after_a_timeout_loss() {
        let now_ms = 10_000;
        let preset = AfkPreset {
            config: detonito_core::GameConfig::new_unchecked((3, 1), 1),
            timer: AfkPreset::v1().timer,
        };
        let layout = detonito_core::MineLayout::from_mine_coords((3, 1), &[(0, 0)]).unwrap();
        let mut session = test_session();
        session.engine = AfkEngine::with_layout_for_tests(layout, preset, now_ms);
        session
            .engine
            .apply_action(AfkAction::SetFlag((1, 0)), now_ms)
            .expect("flagging should succeed");
        session
            .engine
            .force_timed_out(CoreAfkLossReason::Timer, now_ms);

        let snapshot = session.snapshot(None, true, now_ms);

        assert_eq!(
            snapshot.board.cells,
            vec![
                AfkCellSnapshot::Mine,
                AfkCellSnapshot::Misflagged,
                AfkCellSnapshot::Hidden,
            ]
        );
    }

    #[test]
    fn snapshot_marks_elapsed_timer_losses_with_timer_reason() {
        let now_ms = 10_000;
        let mut preset = AfkPreset {
            config: detonito_core::GameConfig::new_unchecked((2, 1), 1),
            timer: AfkPreset::v1().timer,
        };
        preset.timer.start_secs = 1;
        let layout = detonito_core::MineLayout::from_mine_coords((2, 1), &[(0, 0)]).unwrap();
        let mut session = test_session();
        session.engine = AfkEngine::with_layout_for_tests(layout, preset, now_ms);

        let settle = session.engine.settle(now_ms + 2_000);
        assert!(settle.changed);

        let snapshot = session.snapshot(None, true, now_ms + 2_000);
        assert_eq!(snapshot.loss_reason, Some(AfkLossReason::Timer));
    }

    #[test]
    fn snapshot_auto_flags_mines_when_the_round_is_won() {
        let now_ms = 10_000;
        let preset = AfkPreset {
            config: detonito_core::GameConfig::new_unchecked((2, 1), 1),
            timer: AfkPreset::v1().timer,
        };
        let layout = detonito_core::MineLayout::from_mine_coords((2, 1), &[(0, 0)]).unwrap();
        let mut session = test_session();
        session.engine = AfkEngine::with_layout_for_tests(layout, preset, now_ms);
        session
            .engine
            .apply_action(AfkAction::Reveal((1, 0)), now_ms)
            .expect("winning reveal should succeed");

        let snapshot = session.snapshot(None, true, now_ms);

        assert_eq!(snapshot.live_mines_left, 0);
        assert_eq!(
            snapshot.board.cells,
            vec![AfkCellSnapshot::Flagged, AfkCellSnapshot::Revealed(1)]
        );
    }

    #[test]
    fn snapshot_exposes_endgame_unlocked_back_cells() {
        let mut session = line_session(6, &[0, 2, 4, 5]);
        session
            .engine
            .apply_action(AfkAction::Reveal((0, 0)), 1_000)
            .expect("mine reveal should succeed");
        session
            .engine
            .apply_action(AfkAction::Reveal((1, 0)), 1_000)
            .expect("safe reveal should succeed");

        let snapshot = session.snapshot(None, true, 1_000);

        assert_eq!(snapshot.labeled_cells.len(), snapshot.board.cells.len());
        assert!(snapshot.labeled_cells[flat_index((6, 1), (3, 0))]);
    }

    #[test]
    fn chat_validation_accepts_actions_on_endgame_unlocked_back_cells() {
        let mut session = line_session(6, &[0, 2, 4, 5]);
        session
            .engine
            .apply_action(AfkAction::Reveal((0, 0)), 1_000)
            .expect("mine reveal should succeed");
        session
            .engine
            .apply_action(AfkAction::Reveal((1, 0)), 1_000)
            .expect("safe reveal should succeed");

        let parsed = ParsedBoardAction {
            action: AfkAction::Reveal((3, 0)),
            coords: (3, 0),
        };

        assert!(chat_board_action_targets_labeled_cell(&session, parsed).unwrap());
    }

    // --- DO storage size limit tests ---

    fn test_identity(n: usize) -> AfkIdentity {
        AfkIdentity::new(
            format!("{n}"),
            format!("user_{n:0>20}"),
            format!("DisplayName_{n:0>12}"),
        )
    }

    fn worst_case_state() -> PersistedAfkState {
        let mut session = test_session();
        session.ignored_users = (0..MAX_IGNORED_USERS).map(test_identity).collect();
        session.timed_out_users = (0..MAX_TIMED_OUT_USERS).map(test_identity).collect();
        for i in 0..MAX_PENALTIES {
            session.push_penalty(AfkPenaltySnapshot {
                chatter: test_identity(i),
                timer_delta_secs: -15,
                timeout_requested: true,
                timeout_succeeded: true,
            });
        }
        for i in 0..MAX_ACTIVITY_ROWS {
            session.push_activity(format!("user_{i} hit a mine at 1A"), 1000 * i as i64);
        }

        PersistedAfkState {
            broadcaster: Some(test_identity(9999)),
            tokens: None,
            timeout_enabled: true,
            timeout_duration_secs: default_timeout_duration_secs(),
            board_size: default_protocol_board_size(),
            session: Some(session),
            pending_untimeouts: (0..MAX_PENDING_UNTIMEOUTS).map(test_identity).collect(),
            recent_eventsub_ids: (0..MAX_EVENTSUB_IDS)
                .map(|i| format!("eventsub-msg-id-{i:0>30}"))
                .collect(),
            eventsub: PersistedEventSubState {
                connection_status: Some("connected".into()),
                websocket_session_id: Some("session-id-placeholder-value".into()),
                reconnect_url: Some("wss://eventsub.wss.twitch.tv/ws?reconnect=true".into()),
                reconnect_due_at_ms: Some(9_999_999_999),
                subscription_id: Some("subscription-id-placeholder".into()),
                last_message_id: Some("last-message-id-placeholder".into()),
                last_received_at_ms: Some(9_999_999_999),
                last_error: Some("Some error message for testing".into()),
            },
        }
    }

    #[test]
    fn worst_case_state_fits_within_safety_threshold() {
        let state = worst_case_state();
        let json = serde_json::to_string(&state).expect("state should serialize");
        let size = json.len();
        assert!(
            size <= PERSISTED_STATE_SIZE_LIMIT,
            "Worst-case serialized state is {size} bytes, exceeding the \
             {PERSISTED_STATE_SIZE_LIMIT} byte safety threshold"
        );
    }

    #[test]
    fn worst_case_state_fits_within_do_hard_limit() {
        let state = worst_case_state();
        let json = serde_json::to_string(&state).expect("state should serialize");
        let hard_limit = 128 * 1024;
        assert!(
            json.len() <= hard_limit,
            "Worst-case serialized state is {} bytes, exceeding the \
             Cloudflare DO hard limit of {hard_limit} bytes",
            json.len(),
        );
    }

    #[test]
    fn ignored_users_cap_drains_oldest_entries() {
        let mut session = test_session();
        for i in 0..MAX_IGNORED_USERS + 10 {
            session.ignored_users.push(test_identity(i));
            if session.ignored_users.len() > MAX_IGNORED_USERS {
                let overflow = session.ignored_users.len() - MAX_IGNORED_USERS;
                session.ignored_users.drain(0..overflow);
            }
        }
        assert_eq!(session.ignored_users.len(), MAX_IGNORED_USERS);
        assert_eq!(session.ignored_users[0].user_id, "10");
    }

    #[test]
    fn pending_untimeouts_cap_drains_oldest_entries() {
        let mut state = PersistedAfkState::default();
        for i in 0..MAX_PENDING_UNTIMEOUTS + 5 {
            state.pending_untimeouts.push(test_identity(i));
            if state.pending_untimeouts.len() > MAX_PENDING_UNTIMEOUTS {
                let overflow = state.pending_untimeouts.len() - MAX_PENDING_UNTIMEOUTS;
                state.pending_untimeouts.drain(0..overflow);
            }
        }
        assert_eq!(state.pending_untimeouts.len(), MAX_PENDING_UNTIMEOUTS);
        assert_eq!(state.pending_untimeouts[0].user_id, "5");
    }

    #[test]
    fn minimal_state_serializes_small() {
        let state = PersistedAfkState::default();
        let json = serde_json::to_string(&state).expect("state should serialize");
        assert!(json.len() < 1024, "Empty state is {} bytes", json.len());
    }
}
