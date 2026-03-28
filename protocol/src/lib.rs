use serde::{Deserialize, Serialize};

fn default_afk_current_level() -> u16 {
    1
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontendRuntimeConfig {
    pub afk_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkIdentity {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
}

impl AfkIdentity {
    pub fn new(
        user_id: impl Into<String>,
        login: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            user_id: user_id.into(),
            login: login.into(),
            display_name: display_name.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamerAuthStatus {
    pub identity: Option<AfkIdentity>,
    pub expires_at_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfkChatConnectionState {
    #[default]
    Idle,
    Connecting,
    Connected,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfkRoundPhase {
    Countdown,
    Active,
    Won,
    TimedOut,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfkLossReason {
    Mine,
    Timer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AfkCellSnapshot {
    Hidden,
    Flagged,
    Revealed(u8),
    Mine,
    Misflagged,
    Crater,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkCoordSnapshot {
    pub x: u8,
    pub y: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkBoardSnapshot {
    pub width: u8,
    pub height: u8,
    pub cells: Vec<AfkCellSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkTimerProfileSnapshot {
    pub start_secs: u32,
    pub safe_reveal_bonus_secs: u32,
    pub mine_penalty_secs: u32,
    pub start_delay_secs: u32,
    pub win_continue_delay_secs: u32,
    pub loss_continue_delay_secs: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkPenaltySnapshot {
    pub chatter: AfkIdentity,
    pub timer_delta_secs: i32,
    pub timeout_requested: bool,
    pub timeout_succeeded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkActivityRow {
    pub at_ms: i64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkSessionSnapshot {
    pub streamer: Option<AfkIdentity>,
    pub phase: AfkRoundPhase,
    pub paused: bool,
    pub board: AfkBoardSnapshot,
    pub timer_profile: AfkTimerProfileSnapshot,
    pub timer_remaining_secs: i32,
    #[serde(default)]
    pub phase_countdown_secs: Option<i32>,
    #[serde(default = "default_afk_current_level")]
    pub current_level: u16,
    pub live_mines_left: i32,
    pub crater_count: u16,
    #[serde(default)]
    pub loss_reason: Option<AfkLossReason>,
    pub timeout_enabled: bool,
    pub ignored_users: Vec<AfkIdentity>,
    pub recent_penalties: Vec<AfkPenaltySnapshot>,
    pub activity: Vec<AfkActivityRow>,
    pub last_action: Option<AfkActivityRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkStatusResponse {
    pub runtime: FrontendRuntimeConfig,
    pub auth: StreamerAuthStatus,
    pub chat_connection: AfkChatConnectionState,
    pub chat_error: Option<String>,
    pub timeout_supported: bool,
    pub timeout_enabled: bool,
    pub connect_url: Option<String>,
    pub websocket_path: Option<String>,
    pub session: Option<AfkSessionSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AfkServerMessage {
    Connected { status: AfkStatusResponse },
    Snapshot { session: AfkSessionSnapshot },
    Activity { row: AfkActivityRow },
    Error { message: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfkActionKind {
    Reveal,
    ToggleFlag,
    Chord,
    ChordFlag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkActionRequest {
    pub kind: AfkActionKind,
    pub x: u8,
    pub y: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AfkClientMessage {
    Ping,
}
