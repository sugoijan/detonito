use serde::{Deserialize, Serialize};

fn default_afk_current_level() -> u16 {
    1
}

fn default_afk_lives_remaining() -> u8 {
    3
}

fn default_afk_max_lives() -> u8 {
    3
}

fn default_afk_timer_preferences() -> AfkTimerPreferences {
    AfkTimerPreferences::default()
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkTimerPreferences {
    pub start_secs: u32,
    pub safe_reveal_bonus_secs: u32,
    pub mine_penalty_secs: u32,
}

impl Default for AfkTimerPreferences {
    fn default() -> Self {
        Self {
            start_secs: 180,
            safe_reveal_bonus_secs: 3,
            mine_penalty_secs: 10,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkPenaltySnapshot {
    pub chatter: AfkIdentity,
    pub timer_delta_secs: i32,
    pub timeout_requested: bool,
    pub timeout_succeeded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkUserStatsSnapshot {
    pub chatter: AfkIdentity,
    #[serde(default)]
    pub opened_cells: u32,
    #[serde(default)]
    pub correct_flags: u32,
    #[serde(default)]
    pub incorrect_flags: u32,
    #[serde(default)]
    pub correct_unflags: u32,
    #[serde(default)]
    pub died_this_round: bool,
    #[serde(default)]
    pub died_before_this_round: bool,
    #[serde(default)]
    pub died_every_round: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkStatsGroupSnapshot {
    #[serde(default)]
    pub users: Vec<AfkUserStatsSnapshot>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkRoundReportSnapshot {
    #[serde(default)]
    pub round_loser: Option<AfkIdentity>,
    #[serde(default)]
    pub round: AfkStatsGroupSnapshot,
    #[serde(default)]
    pub run: AfkStatsGroupSnapshot,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfkActivityKind {
    #[default]
    Generic,
    MineHit,
    OutForRound,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkActivityRow {
    pub at_ms: i64,
    pub text: String,
    #[serde(default)]
    pub kind: AfkActivityKind,
    #[serde(default)]
    pub actor: Option<AfkIdentity>,
    #[serde(default)]
    pub coord: Option<AfkCoordSnapshot>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfkBoardSize {
    Tiny,
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfkHazardVariant {
    #[default]
    Mines,
    Flowers,
}

impl AfkHazardVariant {
    pub const fn timeout_reason(self) -> &'static str {
        match self {
            Self::Mines => "BOOM! You found a mine.",
            Self::Flowers => "D: You stepped on a flower.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkSessionSnapshot {
    pub streamer: Option<AfkIdentity>,
    pub phase: AfkRoundPhase,
    pub paused: bool,
    #[serde(default)]
    pub hazard_variant: AfkHazardVariant,
    pub board: AfkBoardSnapshot,
    #[serde(default)]
    pub labeled_cells: Vec<bool>,
    pub timer_profile: AfkTimerProfileSnapshot,
    pub timer_remaining_secs: i32,
    #[serde(default)]
    pub phase_countdown_secs: Option<i32>,
    #[serde(default = "default_afk_current_level")]
    pub current_level: u16,
    #[serde(default = "default_afk_lives_remaining")]
    pub lives_remaining: u8,
    #[serde(default = "default_afk_max_lives")]
    pub max_lives: u8,
    #[serde(default)]
    pub game_over: bool,
    #[serde(default)]
    pub round_report: Option<AfkRoundReportSnapshot>,
    pub live_mines_left: i32,
    pub crater_count: u16,
    #[serde(default)]
    pub loss_reason: Option<AfkLossReason>,
    pub timeout_enabled: bool,
    pub ignored_users: Vec<AfkIdentity>,
    pub recent_penalties: Vec<AfkPenaltySnapshot>,
    pub activity: Vec<AfkActivityRow>,
    pub last_action: Option<AfkActivityRow>,
    #[serde(default)]
    pub last_user_activity_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkStatusResponse {
    pub runtime: FrontendRuntimeConfig,
    pub auth: StreamerAuthStatus,
    pub chat_connection: AfkChatConnectionState,
    pub chat_error: Option<String>,
    #[serde(default = "default_afk_timer_preferences")]
    pub timer_preferences: AfkTimerPreferences,
    pub timeout_supported: bool,
    pub timeout_enabled: bool,
    pub timeout_duration_secs: u32,
    pub board_size: AfkBoardSize,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn activity_row_deserialization_defaults_coord_to_none() {
        let row: AfkActivityRow = serde_json::from_value(json!({
            "at_ms": 1234,
            "text": "Jan hit a mine at 1A",
            "kind": "mine_hit",
            "actor": {
                "user_id": "1",
                "login": "jan",
                "display_name": "Jan"
            }
        }))
        .expect("activity row should deserialize");

        assert_eq!(row.coord, None);
    }

    #[test]
    fn activity_row_supports_embedded_coords() {
        let row = AfkActivityRow {
            at_ms: 1234,
            text: "Jan hit a mine at 1A".into(),
            kind: AfkActivityKind::MineHit,
            actor: Some(AfkIdentity::new("1", "jan", "Jan")),
            coord: Some(AfkCoordSnapshot { x: 0, y: 0 }),
        };

        let value = serde_json::to_value(&row).expect("activity row should serialize");
        assert_eq!(value["coord"], json!({ "x": 0, "y": 0 }));
    }

    #[test]
    fn timer_preferences_default_to_standard_values() {
        assert_eq!(
            AfkTimerPreferences::default(),
            AfkTimerPreferences {
                start_secs: 180,
                safe_reveal_bonus_secs: 3,
                mine_penalty_secs: 10,
            }
        );
    }

    #[test]
    fn session_snapshot_deserialization_defaults_hazard_variant_to_mines() {
        let session: AfkSessionSnapshot = serde_json::from_value(json!({
            "streamer": null,
            "phase": "active",
            "paused": false,
            "board": {
                "width": 1,
                "height": 1,
                "cells": ["Hidden"]
            },
            "labeled_cells": [],
            "timer_profile": {
                "start_secs": 180,
                "safe_reveal_bonus_secs": 3,
                "mine_penalty_secs": 10,
                "start_delay_secs": 5,
                "win_continue_delay_secs": 30,
                "loss_continue_delay_secs": 60
            },
            "timer_remaining_secs": 180,
            "phase_countdown_secs": null,
            "current_level": 1,
            "live_mines_left": 1,
            "crater_count": 0,
            "loss_reason": null,
            "timeout_enabled": true,
            "ignored_users": [],
            "recent_penalties": [],
            "activity": [],
            "last_action": null,
            "last_user_activity_at_ms": 0
        }))
        .expect("session snapshot should deserialize");

        assert_eq!(session.hazard_variant, AfkHazardVariant::Mines);
    }

    #[test]
    fn status_response_deserialization_defaults_timer_preferences() {
        let status: AfkStatusResponse = serde_json::from_value(json!({
            "runtime": { "afk_enabled": true },
            "auth": { "identity": null, "expires_at_ms": null },
            "chat_connection": "idle",
            "chat_error": null,
            "timeout_supported": true,
            "timeout_enabled": true,
            "timeout_duration_secs": 30,
            "board_size": "medium",
            "connect_url": null,
            "websocket_path": null,
            "session": null
        }))
        .expect("status response should deserialize");

        assert_eq!(status.timer_preferences, AfkTimerPreferences::default());
    }

    #[test]
    fn session_snapshot_deserialization_defaults_lives_and_report_fields() {
        let session: AfkSessionSnapshot = serde_json::from_value(json!({
            "streamer": null,
            "phase": "active",
            "paused": false,
            "board": {
                "width": 1,
                "height": 1,
                "cells": ["Hidden"]
            },
            "labeled_cells": [],
            "timer_profile": {
                "start_secs": 180,
                "safe_reveal_bonus_secs": 3,
                "mine_penalty_secs": 10,
                "start_delay_secs": 5,
                "win_continue_delay_secs": 30,
                "loss_continue_delay_secs": 60
            },
            "timer_remaining_secs": 180,
            "phase_countdown_secs": null,
            "current_level": 1,
            "live_mines_left": 1,
            "crater_count": 0,
            "loss_reason": null,
            "timeout_enabled": true,
            "ignored_users": [],
            "recent_penalties": [],
            "activity": [],
            "last_action": null,
            "last_user_activity_at_ms": 0
        }))
        .expect("session snapshot should deserialize");

        assert_eq!(session.lives_remaining, 3);
        assert_eq!(session.max_lives, 3);
        assert!(!session.game_over);
        assert_eq!(session.round_report, None);
    }

    #[test]
    fn user_stats_deserialization_defaults_death_flags_to_false() {
        let user: AfkUserStatsSnapshot = serde_json::from_value(json!({
            "chatter": {
                "user_id": "1",
                "login": "jan",
                "display_name": "Jan"
            },
            "opened_cells": 2,
            "correct_flags": 1,
            "incorrect_flags": 0,
            "correct_unflags": 0
        }))
        .expect("user stats snapshot should deserialize");

        assert!(!user.died_this_round);
        assert!(!user.died_before_this_round);
        assert!(!user.died_every_round);
    }
}
