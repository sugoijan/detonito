use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::num::Saturating;

use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::{
    Coord2, FirstMovePolicy, GameConfig, GameError, LayoutGenerator, MineLayout, NeighborIterExt,
    RandomLayoutGenerator, Result, ToNdIndex,
};

const AFK_ENDGAME_LABEL_CUSHION: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkTimerProfile {
    pub start_secs: u32,
    pub safe_reveal_bonus_secs: u32,
    pub mine_penalty_secs: u32,
    pub start_delay_secs: u32,
    pub win_continue_delay_secs: u32,
    pub loss_continue_delay_secs: u32,
}

impl AfkTimerProfile {
    pub const fn v1() -> Self {
        Self {
            start_secs: 120,
            safe_reveal_bonus_secs: 1,
            mine_penalty_secs: 15,
            start_delay_secs: 8,
            win_continue_delay_secs: 30,
            loss_continue_delay_secs: 60,
        }
    }
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

impl AfkBoardSize {
    pub const fn dimensions(self) -> Coord2 {
        match self {
            Self::Tiny => (9, 9),
            Self::Small => (16, 16),
            Self::Medium => (24, 18),
            Self::Large => (30, 20),
        }
    }

    pub const fn initial_mines(self) -> u16 {
        match self {
            Self::Tiny => 9,
            Self::Small => 20,
            Self::Medium => 36,
            Self::Large => 50,
        }
    }

    pub const fn mine_increment(self) -> u16 {
        match self {
            Self::Tiny => 1,
            Self::Small => 4,
            Self::Medium => 7,
            Self::Large => 10,
        }
    }

    pub const fn max_mines(self) -> u16 {
        match self {
            Self::Tiny => 27,
            Self::Small => 84,
            Self::Medium => 141,
            Self::Large => 200,
        }
    }

    pub const fn from_size(size: Coord2) -> Option<Self> {
        match size {
            (9, 9) => Some(Self::Tiny),
            (16, 16) => Some(Self::Small),
            (24, 18) => Some(Self::Medium),
            (30, 20) => Some(Self::Large),
            _ => None,
        }
    }

    pub const fn for_mines(self, mines: u16) -> AfkPreset {
        AfkPreset {
            config: GameConfig::new_unchecked(self.dimensions(), mines),
            timer: AfkTimerProfile::v1(),
        }
    }

    pub const fn initial_preset(self) -> AfkPreset {
        self.for_mines(self.initial_mines())
    }

    pub fn level_number_for_mines(self, current: u16) -> u16 {
        let normalized = current.clamp(self.initial_mines(), self.max_mines());
        ((normalized - self.initial_mines()) / self.mine_increment()) + 1
    }

    pub const fn next_mine_count(self, current: u16) -> u16 {
        let incremented = current.saturating_add(self.mine_increment());
        if incremented > self.max_mines() {
            self.max_mines()
        } else {
            incremented
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AfkPreset {
    pub config: GameConfig,
    pub timer: AfkTimerProfile,
}

impl AfkPreset {
    pub const fn for_board_size(board_size: AfkBoardSize) -> Self {
        board_size.initial_preset()
    }

    pub const fn v1() -> Self {
        Self::for_board_size(AfkBoardSize::Medium)
    }

    pub const fn for_board_size_and_mines(board_size: AfkBoardSize, mines: u16) -> Self {
        board_size.for_mines(mines)
    }

    pub fn board_size(&self) -> Option<AfkBoardSize> {
        AfkBoardSize::from_size(self.config.size)
    }

    pub fn current_level(&self) -> u16 {
        self.board_size()
            .map(|board_size| board_size.level_number_for_mines(self.config.mines))
            .unwrap_or(1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfkRoundPhase {
    Countdown,
    Active,
    Won,
    TimedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfkLossReason {
    Mine,
    Timer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AfkAction {
    Reveal(Coord2),
    ToggleFlag(Coord2),
    SetFlag(Coord2),
    ClearFlag(Coord2),
    Chord(Coord2),
    ChordFlag(Coord2),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkPenalty {
    pub actor_user_id: String,
    pub actor_login: String,
    pub timer_delta_secs: i32,
    pub timeout_requested: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AfkCellState {
    Hidden,
    Flagged,
    Revealed(u8),
    Mine,
    Misflagged,
    Crater,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
enum AfkBoardCell {
    #[default]
    Hidden,
    Flagged,
    RevealedSafe,
    Crater,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkActionOutcome {
    pub changed: bool,
    pub safe_reveals: u16,
    pub mine_triggered: bool,
    pub cratered_mines: u16,
    pub timer_delta_secs: i32,
    pub won: bool,
}

impl AfkActionOutcome {
    const fn no_change() -> Self {
        Self {
            changed: false,
            safe_reveals: 0,
            mine_triggered: false,
            cratered_mines: 0,
            timer_delta_secs: 0,
            won: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfkSettleOutcome {
    pub changed: bool,
    pub round_started: bool,
    pub needs_restart: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AfkEngine {
    preset: AfkPreset,
    seed: u64,
    phase: AfkRoundPhase,
    paused_at_ms: Option<i64>,
    countdown_ends_at_ms: i64,
    auto_restart_at_ms: i64,
    timer_remaining_secs: i32,
    last_timer_tick_at_ms: i64,
    #[serde(default)]
    loss_reason: Option<AfkLossReason>,
    mine_layout: Option<MineLayout>,
    board: Array2<AfkBoardCell>,
    revealed_safe_count: Saturating<u16>,
    crater_count: Saturating<u16>,
}

impl AfkEngine {
    pub fn new(seed: u64, preset: AfkPreset, now_ms: i64) -> Self {
        Self {
            preset,
            seed,
            phase: AfkRoundPhase::Countdown,
            paused_at_ms: None,
            countdown_ends_at_ms: now_ms + i64::from(preset.timer.start_delay_secs) * 1_000,
            auto_restart_at_ms: 0,
            timer_remaining_secs: preset.timer.start_secs as i32,
            last_timer_tick_at_ms: now_ms,
            loss_reason: None,
            mine_layout: None,
            board: Array2::default(preset.config.size.to_nd_index()),
            revealed_safe_count: Saturating(0),
            crater_count: Saturating(0),
        }
    }

    pub fn with_layout_for_tests(layout: MineLayout, preset: AfkPreset, now_ms: i64) -> Self {
        let mut engine = Self::new(0, preset, now_ms);
        engine.phase = AfkRoundPhase::Active;
        engine.countdown_ends_at_ms = now_ms;
        engine.last_timer_tick_at_ms = now_ms;
        engine.mine_layout = Some(layout);
        engine
    }

    pub fn phase(&self) -> AfkRoundPhase {
        self.phase
    }

    pub fn loss_reason(&self) -> Option<AfkLossReason> {
        self.loss_reason
    }

    pub fn is_paused(&self) -> bool {
        self.paused_at_ms.is_some()
    }

    pub fn preset(&self) -> AfkPreset {
        self.preset
    }

    pub fn size(&self) -> Coord2 {
        self.preset.config.size
    }

    pub fn timer_remaining_secs(&self) -> i32 {
        self.timer_remaining_secs
    }

    pub fn board_timer_remaining_secs(&self) -> i32 {
        match self.phase {
            AfkRoundPhase::Countdown => self.preset.timer.start_secs as i32,
            AfkRoundPhase::Active | AfkRoundPhase::Won | AfkRoundPhase::TimedOut => {
                self.timer_remaining_secs
            }
        }
    }

    pub fn phase_countdown_secs(&self, now_ms: i64) -> Option<i32> {
        let now_ms = self.effective_now_ms(now_ms);
        match self.phase {
            AfkRoundPhase::Countdown => {
                Some((((self.countdown_ends_at_ms - now_ms).max(0) + 999) / 1_000) as i32)
            }
            AfkRoundPhase::Won | AfkRoundPhase::TimedOut => {
                Some((((self.auto_restart_at_ms - now_ms).max(0) + 999) / 1_000) as i32)
            }
            AfkRoundPhase::Active => None,
        }
    }

    pub fn display_timer_remaining_secs(&self, now_ms: i64) -> i32 {
        let now_ms = self.effective_now_ms(now_ms);
        match self.phase {
            AfkRoundPhase::Countdown => {
                (((self.countdown_ends_at_ms - now_ms).max(0) + 999) / 1_000) as i32
            }
            AfkRoundPhase::Active => self.timer_remaining_secs,
            AfkRoundPhase::Won | AfkRoundPhase::TimedOut => {
                (((self.auto_restart_at_ms - now_ms).max(0) + 999) / 1_000) as i32
            }
        }
    }

    pub fn next_alarm_at_ms(&self, now_ms: i64) -> Option<i64> {
        if self.is_paused() {
            return None;
        }
        match self.phase {
            AfkRoundPhase::Countdown => {
                Some(next_display_alarm_at_ms(now_ms, self.countdown_ends_at_ms))
            }
            AfkRoundPhase::Active => Some((self.last_timer_tick_at_ms + 1_000).max(now_ms)),
            AfkRoundPhase::Won | AfkRoundPhase::TimedOut => {
                Some(next_display_alarm_at_ms(now_ms, self.auto_restart_at_ms))
            }
        }
    }

    pub fn crater_count(&self) -> u16 {
        self.crater_count.0
    }

    pub fn labeled_cells(&self) -> Vec<bool> {
        let size = self.size();
        let width = usize::from(size.0);
        let height = usize::from(size.1);
        let total = width * height;
        if matches!(self.phase, AfkRoundPhase::Countdown) {
            return alloc::vec![false; total];
        }

        let states = self.visible_cell_states();
        let forced_mines = self.locally_forced_mine_mask(&states);
        let mut labels = alloc::vec![false; total];
        let mut back_hidden_count = 0usize;

        for y in 0..size.1 {
            for x in 0..size.0 {
                let coords = (x, y);
                let idx = flat_index(size, coords);
                match states[idx] {
                    AfkCellState::Flagged => labels[idx] = !forced_mines[idx],
                    AfkCellState::Hidden => {
                        if self.hidden_cell_has_frontier_neighbor(&states, coords) {
                            labels[idx] = true;
                        } else {
                            back_hidden_count += 1;
                        }
                    }
                    AfkCellState::Revealed(_)
                    | AfkCellState::Mine
                    | AfkCellState::Misflagged
                    | AfkCellState::Crater => {}
                }
            }
        }

        let observed_mines = usize::from(self.crater_count.0)
            + forced_mines.into_iter().filter(|forced| *forced).count();
        let total_mines = usize::from(self.preset.config.mines);
        let remaining_mines = total_mines.saturating_sub(observed_mines);
        let safe_unlock = observed_mines.saturating_add(AFK_ENDGAME_LABEL_CUSHION) >= total_mines;
        let mine_unlock = back_hidden_count >= remaining_mines
            && back_hidden_count - remaining_mines <= AFK_ENDGAME_LABEL_CUSHION;

        if safe_unlock || mine_unlock {
            for idx in 0..total {
                if matches!(states[idx], AfkCellState::Hidden) && !labels[idx] {
                    labels[idx] = true;
                }
            }
        }

        labels
    }

    pub fn live_mines_left_for_display(&self) -> i32 {
        if matches!(self.phase, AfkRoundPhase::Won) {
            return 0;
        }
        let total = i32::from(self.preset.config.mines);
        let craters = i32::from(self.crater_count.0);
        let flags = self
            .board
            .iter()
            .filter(|&&cell| matches!(cell, AfkBoardCell::Flagged))
            .count() as i32;
        (total - craters - flags).max(0)
    }

    pub fn cell_state_at(&self, coords: Coord2) -> Result<AfkCellState> {
        self.validate_coords(coords)?;
        if matches!(self.phase, AfkRoundPhase::Won) {
            return Ok(self.cell_state_won(coords));
        }
        if matches!(self.phase, AfkRoundPhase::TimedOut) {
            return Ok(self.cell_state_timed_out(coords));
        }
        Ok(self.cell_state_active(coords))
    }

    fn cell_state_active(&self, coords: Coord2) -> AfkCellState {
        match self.board[coords.to_nd_index()] {
            AfkBoardCell::Hidden => AfkCellState::Hidden,
            AfkBoardCell::Flagged => AfkCellState::Flagged,
            AfkBoardCell::Crater => AfkCellState::Crater,
            AfkBoardCell::RevealedSafe => AfkCellState::Revealed(self.adjacent_mine_count(coords)),
        }
    }

    fn cell_state_won(&self, coords: Coord2) -> AfkCellState {
        let board_cell = self.board[coords.to_nd_index()];
        if matches!(board_cell, AfkBoardCell::Crater) {
            return AfkCellState::Crater;
        }
        if self.has_mine_at(coords).unwrap_or(false) {
            return AfkCellState::Flagged;
        }
        match board_cell {
            AfkBoardCell::Hidden => AfkCellState::Hidden,
            AfkBoardCell::Flagged => AfkCellState::Misflagged,
            AfkBoardCell::Crater => AfkCellState::Crater,
            AfkBoardCell::RevealedSafe => AfkCellState::Revealed(self.adjacent_mine_count(coords)),
        }
    }

    fn cell_state_timed_out(&self, coords: Coord2) -> AfkCellState {
        let board_cell = self.board[coords.to_nd_index()];
        let has_mine = self.has_mine_at(coords).unwrap_or(false);
        match board_cell {
            AfkBoardCell::Crater => AfkCellState::Crater,
            AfkBoardCell::RevealedSafe => AfkCellState::Revealed(self.adjacent_mine_count(coords)),
            AfkBoardCell::Flagged if has_mine => AfkCellState::Flagged,
            AfkBoardCell::Flagged => AfkCellState::Misflagged,
            AfkBoardCell::Hidden if has_mine => AfkCellState::Mine,
            AfkBoardCell::Hidden => AfkCellState::Hidden,
        }
    }

    pub fn settle(&mut self, now_ms: i64) -> AfkSettleOutcome {
        if self.is_paused() {
            return AfkSettleOutcome {
                changed: false,
                round_started: false,
                needs_restart: false,
            };
        }
        match self.phase {
            AfkRoundPhase::Countdown if now_ms >= self.countdown_ends_at_ms => {
                self.auto_open_starting_cell(now_ms)
            }
            AfkRoundPhase::Active => {
                let elapsed_secs = ((now_ms - self.last_timer_tick_at_ms) / 1_000).max(0) as i32;
                if elapsed_secs == 0 {
                    return AfkSettleOutcome {
                        changed: false,
                        round_started: false,
                        needs_restart: false,
                    };
                }

                self.last_timer_tick_at_ms += i64::from(elapsed_secs) * 1_000;
                self.timer_remaining_secs -= elapsed_secs;
                if self.timer_remaining_secs <= 0 {
                    self.timer_remaining_secs = 0;
                    self.phase = AfkRoundPhase::TimedOut;
                    self.loss_reason = Some(AfkLossReason::Timer);
                    self.auto_restart_at_ms =
                        now_ms + i64::from(self.preset.timer.loss_continue_delay_secs) * 1_000;
                }

                AfkSettleOutcome {
                    changed: true,
                    round_started: false,
                    needs_restart: false,
                }
            }
            AfkRoundPhase::Won | AfkRoundPhase::TimedOut => AfkSettleOutcome {
                changed: false,
                round_started: false,
                needs_restart: now_ms >= self.auto_restart_at_ms,
            },
            _ => AfkSettleOutcome {
                changed: false,
                round_started: false,
                needs_restart: false,
            },
        }
    }

    pub fn restart(&mut self, seed: u64, now_ms: i64) {
        *self = Self::new(seed, self.preset, now_ms);
    }

    pub fn pause(&mut self, now_ms: i64) {
        if self.paused_at_ms.is_none() {
            self.paused_at_ms = Some(now_ms);
        }
    }

    pub fn resume(&mut self, now_ms: i64) {
        let Some(paused_at_ms) = self.paused_at_ms.take() else {
            return;
        };
        let delta_ms = now_ms.saturating_sub(paused_at_ms);
        match self.phase {
            AfkRoundPhase::Countdown => {
                self.countdown_ends_at_ms += delta_ms;
            }
            AfkRoundPhase::Active => {
                self.last_timer_tick_at_ms += delta_ms;
            }
            AfkRoundPhase::Won | AfkRoundPhase::TimedOut => {
                self.auto_restart_at_ms += delta_ms;
            }
        }
    }

    pub fn force_timed_out(&mut self, reason: AfkLossReason, now_ms: i64) {
        self.paused_at_ms = None;
        self.phase = AfkRoundPhase::TimedOut;
        self.loss_reason = Some(reason);
        self.timer_remaining_secs = 0;
        self.auto_restart_at_ms =
            now_ms + i64::from(self.preset.timer.loss_continue_delay_secs) * 1_000;
    }

    pub fn apply_action(&mut self, action: AfkAction, now_ms: i64) -> Result<AfkActionOutcome> {
        let _ = self.settle(now_ms);
        if !matches!(self.phase, AfkRoundPhase::Active) {
            return Ok(AfkActionOutcome::no_change());
        }

        let outcome = match action {
            AfkAction::Reveal(coords) => self.apply_reveal(coords)?,
            AfkAction::ToggleFlag(coords) => self.apply_toggle_flag(coords)?,
            AfkAction::SetFlag(coords) => self.apply_flag(coords)?,
            AfkAction::ClearFlag(coords) => self.apply_unflag(coords)?,
            AfkAction::Chord(coords) => self.apply_chord(coords)?,
            AfkAction::ChordFlag(coords) => self.apply_chord_flag(coords)?,
        };

        if outcome.changed {
            self.timer_remaining_secs += outcome.timer_delta_secs;
            if self.timer_remaining_secs <= 0 {
                self.timer_remaining_secs = 0;
                self.phase = AfkRoundPhase::TimedOut;
                self.loss_reason = Some(if outcome.mine_triggered {
                    AfkLossReason::Mine
                } else {
                    AfkLossReason::Timer
                });
                self.auto_restart_at_ms =
                    now_ms + i64::from(self.preset.timer.loss_continue_delay_secs) * 1_000;
            } else if outcome.won {
                self.phase = AfkRoundPhase::Won;
                self.loss_reason = None;
                self.auto_restart_at_ms =
                    now_ms + i64::from(self.preset.timer.win_continue_delay_secs) * 1_000;
            }
        }

        Ok(outcome)
    }

    fn apply_reveal(&mut self, coords: Coord2) -> Result<AfkActionOutcome> {
        self.validate_coords(coords)?;
        if !matches!(self.board[coords.to_nd_index()], AfkBoardCell::Hidden) {
            return Ok(AfkActionOutcome::no_change());
        }

        self.ensure_layout(coords);
        if self.has_live_mine_at(coords)? {
            self.board[coords.to_nd_index()] = AfkBoardCell::Crater;
            self.crater_count += 1;
            return Ok(self.finalize_outcome(0, 1, true));
        }

        let safe_reveals = self.reveal_safe_region(coords)?;
        Ok(self.finalize_outcome(safe_reveals, 0, false))
    }

    fn apply_flag(&mut self, coords: Coord2) -> Result<AfkActionOutcome> {
        self.set_flag_state(coords, Some(AfkBoardCell::Flagged))
    }

    fn apply_unflag(&mut self, coords: Coord2) -> Result<AfkActionOutcome> {
        self.set_flag_state(coords, Some(AfkBoardCell::Hidden))
    }

    fn apply_toggle_flag(&mut self, coords: Coord2) -> Result<AfkActionOutcome> {
        self.validate_coords(coords)?;
        let next = match self.board[coords.to_nd_index()] {
            AfkBoardCell::Hidden => Some(AfkBoardCell::Flagged),
            AfkBoardCell::Flagged => Some(AfkBoardCell::Hidden),
            AfkBoardCell::RevealedSafe | AfkBoardCell::Crater => None,
        };

        if let Some(next) = next {
            self.board[coords.to_nd_index()] = next;
            Ok(AfkActionOutcome {
                changed: true,
                ..AfkActionOutcome::no_change()
            })
        } else {
            Ok(AfkActionOutcome::no_change())
        }
    }

    fn set_flag_state(
        &mut self,
        coords: Coord2,
        desired: Option<AfkBoardCell>,
    ) -> Result<AfkActionOutcome> {
        self.validate_coords(coords)?;
        let Some(desired) = desired else {
            return Ok(AfkActionOutcome::no_change());
        };
        let current = self.board[coords.to_nd_index()];
        let next = match (current, desired) {
            (AfkBoardCell::Hidden, AfkBoardCell::Flagged) => Some(AfkBoardCell::Flagged),
            (AfkBoardCell::Flagged, AfkBoardCell::Hidden) => Some(AfkBoardCell::Hidden),
            _ => None,
        };
        if let Some(next) = next {
            self.board[coords.to_nd_index()] = next;
            Ok(AfkActionOutcome {
                changed: true,
                ..AfkActionOutcome::no_change()
            })
        } else {
            Ok(AfkActionOutcome::no_change())
        }
    }

    fn apply_chord(&mut self, coords: Coord2) -> Result<AfkActionOutcome> {
        self.validate_coords(coords)?;
        if !matches!(self.board[coords.to_nd_index()], AfkBoardCell::RevealedSafe) {
            return Ok(AfkActionOutcome::no_change());
        }

        let required_flags = usize::from(self.adjacent_mine_count(coords));
        let resolved_mine_neighbors = self.count_marked_mine_neighbors(coords);
        if resolved_mine_neighbors != required_flags {
            return Ok(AfkActionOutcome::no_change());
        }

        let mut safe_reveals = 0u16;
        let mut cratered_mines = 0u16;
        for neighbor in self.board.iter_neighbors(coords) {
            if !matches!(self.board[neighbor.to_nd_index()], AfkBoardCell::Hidden) {
                continue;
            }
            if self.has_live_mine_at(neighbor)? {
                self.board[neighbor.to_nd_index()] = AfkBoardCell::Crater;
                self.crater_count += 1;
                cratered_mines = cratered_mines.saturating_add(1);
            } else {
                safe_reveals = safe_reveals.saturating_add(self.reveal_safe_region(neighbor)?);
            }
        }

        Ok(self.finalize_outcome(safe_reveals, cratered_mines, cratered_mines > 0))
    }

    fn apply_chord_flag(&mut self, coords: Coord2) -> Result<AfkActionOutcome> {
        self.validate_coords(coords)?;
        if !matches!(self.board[coords.to_nd_index()], AfkBoardCell::RevealedSafe) {
            return Ok(AfkActionOutcome::no_change());
        }

        let required_mines = usize::from(self.adjacent_mine_count(coords));
        let unresolved_neighbors = self.count_unrevealed_neighbors(coords);
        let crater_neighbors = self.count_crater_neighbors(coords);
        if required_mines != unresolved_neighbors + crater_neighbors {
            return Ok(AfkActionOutcome::no_change());
        }

        let mut updated = false;
        for neighbor in self.board.iter_neighbors(coords) {
            if matches!(self.board[neighbor.to_nd_index()], AfkBoardCell::Hidden) {
                self.board[neighbor.to_nd_index()] = AfkBoardCell::Flagged;
                updated = true;
            }
        }

        if updated {
            Ok(AfkActionOutcome {
                changed: true,
                ..AfkActionOutcome::no_change()
            })
        } else {
            Ok(AfkActionOutcome::no_change())
        }
    }

    fn finalize_outcome(
        &self,
        safe_reveals: u16,
        cratered_mines: u16,
        mine_triggered: bool,
    ) -> AfkActionOutcome {
        let mut timer_delta_secs = if safe_reveals > 0 {
            self.preset.timer.safe_reveal_bonus_secs as i32
        } else {
            0
        };
        if mine_triggered {
            timer_delta_secs -= self.preset.timer.mine_penalty_secs as i32;
        }
        AfkActionOutcome {
            changed: safe_reveals > 0 || cratered_mines > 0 || mine_triggered,
            safe_reveals,
            mine_triggered,
            cratered_mines,
            timer_delta_secs,
            won: self.is_won(),
        }
    }

    fn reveal_safe_region(&mut self, start: Coord2) -> Result<u16> {
        let mut queue = VecDeque::from([start]);
        let mut safe_reveals = 0u16;

        while let Some(coords) = queue.pop_front() {
            self.validate_coords(coords)?;
            if !matches!(self.board[coords.to_nd_index()], AfkBoardCell::Hidden) {
                continue;
            }
            if self.has_live_mine_at(coords)? {
                continue;
            }

            self.board[coords.to_nd_index()] = AfkBoardCell::RevealedSafe;
            self.revealed_safe_count += 1;
            safe_reveals = safe_reveals.saturating_add(1);

            if self.adjacent_mine_count(coords) == 0 {
                queue.extend(self.board.iter_neighbors(coords));
            }
        }

        Ok(safe_reveals)
    }

    fn is_won(&self) -> bool {
        self.mine_layout
            .as_ref()
            .is_some_and(|layout| self.revealed_safe_count == Saturating(layout.safe_cell_count()))
    }

    fn ensure_layout(&mut self, first_move: Coord2) {
        if self.mine_layout.is_some() {
            return;
        }
        let layout =
            RandomLayoutGenerator::new(self.seed, first_move, FirstMovePolicy::FirstMoveZero)
                .generate(self.preset.config);
        self.mine_layout = Some(layout);
    }

    pub fn has_mine_at(&self, coords: Coord2) -> Result<bool> {
        self.validate_coords(coords)?;
        Ok(self
            .mine_layout
            .as_ref()
            .is_some_and(|layout| layout.contains_mine(coords)))
    }

    fn has_live_mine_at(&self, coords: Coord2) -> Result<bool> {
        Ok(self.has_mine_at(coords)?
            && !matches!(self.board[coords.to_nd_index()], AfkBoardCell::Crater))
    }

    fn adjacent_mine_count(&self, coords: Coord2) -> u8 {
        self.board
            .iter_neighbors(coords)
            .filter(|&pos| {
                self.mine_layout
                    .as_ref()
                    .is_some_and(|layout| layout.contains_mine(pos))
            })
            .count()
            .try_into()
            .unwrap()
    }

    fn count_flagged_neighbors(&self, coords: Coord2) -> usize {
        self.board
            .iter_neighbors(coords)
            .filter(|&pos| matches!(self.board[pos.to_nd_index()], AfkBoardCell::Flagged))
            .count()
    }

    fn count_crater_neighbors(&self, coords: Coord2) -> usize {
        self.board
            .iter_neighbors(coords)
            .filter(|&pos| matches!(self.board[pos.to_nd_index()], AfkBoardCell::Crater))
            .count()
    }

    fn count_marked_mine_neighbors(&self, coords: Coord2) -> usize {
        self.count_flagged_neighbors(coords) + self.count_crater_neighbors(coords)
    }

    fn count_unrevealed_neighbors(&self, coords: Coord2) -> usize {
        self.board
            .iter_neighbors(coords)
            .filter(|&pos| {
                matches!(
                    self.board[pos.to_nd_index()],
                    AfkBoardCell::Hidden | AfkBoardCell::Flagged
                )
            })
            .count()
    }

    fn validate_coords(&self, coords: Coord2) -> Result<Coord2> {
        let (width, height) = self.size();
        if coords.0 < width && coords.1 < height {
            Ok(coords)
        } else {
            Err(GameError::InvalidCoords)
        }
    }

    fn effective_now_ms(&self, now_ms: i64) -> i64 {
        self.paused_at_ms.unwrap_or(now_ms)
    }

    pub fn cell_has_label(&self, coords: Coord2) -> Result<bool> {
        let coords = self.validate_coords(coords)?;
        Ok(self.labeled_cells()[flat_index(self.size(), coords)])
    }

    pub fn open_starting_cell(&mut self, coords: Coord2, now_ms: i64) -> Result<bool> {
        self.validate_coords(coords)?;
        if !matches!(self.phase, AfkRoundPhase::Countdown) {
            return Ok(false);
        }
        self.ensure_layout(coords);
        let _ = self.reveal_safe_region(coords)?;
        self.phase = AfkRoundPhase::Active;
        self.countdown_ends_at_ms = now_ms;
        self.last_timer_tick_at_ms = now_ms;
        Ok(true)
    }

    fn auto_open_starting_cell(&mut self, now_ms: i64) -> AfkSettleOutcome {
        let coords = self.random_starting_coord();
        let round_started = self.open_starting_cell(coords, now_ms).unwrap_or(false);
        AfkSettleOutcome {
            changed: round_started,
            round_started,
            needs_restart: false,
        }
    }

    fn random_starting_coord(&self) -> Coord2 {
        let (width, height) = self.size();
        let width_u64 = u64::from(width.max(1));
        let height_u64 = u64::from(height.max(1));
        (
            (self.seed % width_u64) as u8,
            ((self.seed / width_u64) % height_u64) as u8,
        )
    }

    fn visible_cell_states(&self) -> Vec<AfkCellState> {
        let size = self.size();
        let mut states = Vec::with_capacity(usize::from(size.0) * usize::from(size.1));
        for y in 0..size.1 {
            for x in 0..size.0 {
                states.push(
                    self.cell_state_at((x, y))
                        .expect("board iteration should only visit valid coordinates"),
                );
            }
        }
        states
    }

    fn hidden_cell_has_frontier_neighbor(&self, states: &[AfkCellState], coords: Coord2) -> bool {
        self.board.iter_neighbors(coords).any(|neighbor| {
            match states[flat_index(self.size(), neighbor)] {
                AfkCellState::Crater => true,
                AfkCellState::Revealed(count) => count > 0,
                _ => false,
            }
        })
    }

    fn locally_forced_mine_mask(&self, states: &[AfkCellState]) -> Vec<bool> {
        let size = self.size();
        // Only hide a flag when a visible clue already proves the cell is a mine.
        let mut forced = alloc::vec![false; usize::from(size.0) * usize::from(size.1)];

        for y in 0..size.1 {
            for x in 0..size.0 {
                let coords = (x, y);
                let AfkCellState::Revealed(required_mines) = states[flat_index(size, coords)]
                else {
                    continue;
                };
                if required_mines == 0 {
                    continue;
                }

                let mut crater_neighbors = 0usize;
                let mut unresolved_neighbors = Vec::new();
                for neighbor in self.board.iter_neighbors(coords) {
                    match states[flat_index(size, neighbor)] {
                        AfkCellState::Crater => crater_neighbors += 1,
                        AfkCellState::Hidden | AfkCellState::Flagged => {
                            unresolved_neighbors.push(flat_index(size, neighbor));
                        }
                        AfkCellState::Revealed(_)
                        | AfkCellState::Mine
                        | AfkCellState::Misflagged => {}
                    }
                }

                if usize::from(required_mines) == crater_neighbors + unresolved_neighbors.len() {
                    for idx in unresolved_neighbors {
                        forced[idx] = true;
                    }
                }
            }
        }

        forced
    }
}

const fn next_display_alarm_at_ms(now_ms: i64, deadline_ms: i64) -> i64 {
    if now_ms >= deadline_ms {
        deadline_ms
    } else {
        let next_tick = now_ms + 1_000;
        if next_tick < deadline_ms {
            next_tick
        } else {
            deadline_ms
        }
    }
}

pub fn flat_index(size: Coord2, coords: Coord2) -> usize {
    usize::from(coords.1) * usize::from(size.0) + usize::from(coords.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> i64 {
        1_000
    }

    fn line_preset(width: u8, mines: u16) -> AfkPreset {
        AfkPreset {
            config: GameConfig::new_unchecked((width, 1), mines),
            timer: AfkTimerProfile::v1(),
        }
    }

    fn line_engine(width: u8, mine_xs: &[u8]) -> AfkEngine {
        let mine_coords: Vec<Coord2> = mine_xs.iter().copied().map(|x| (x, 0)).collect();
        let layout = MineLayout::from_mine_coords((width, 1), &mine_coords).unwrap();
        AfkEngine::with_layout_for_tests(layout, line_preset(width, mine_xs.len() as u16), now())
    }

    fn label_at(engine: &AfkEngine, coords: Coord2) -> bool {
        engine.labeled_cells()[flat_index(engine.size(), coords)]
    }

    #[test]
    fn safe_reveal_gains_timer_bonus() {
        let layout = MineLayout::from_mine_coords((2, 2), &[(1, 1)]).unwrap();
        let mut engine = AfkEngine::with_layout_for_tests(layout, AfkPreset::v1(), now());
        let before = engine.timer_remaining_secs();

        let outcome = engine
            .apply_action(AfkAction::Reveal((0, 0)), now())
            .expect("reveal should succeed");

        assert!(outcome.changed);
        assert!(outcome.safe_reveals > 0);
        assert!(engine.timer_remaining_secs() > before);
    }

    #[test]
    fn safe_reveal_bonus_is_flat_for_single_cell_and_cascade_reveals() {
        let single_layout = MineLayout::from_mine_coords((2, 2), &[(1, 1)]).unwrap();
        let cascade_layout = MineLayout::from_mine_coords((3, 1), &[(2, 0)]).unwrap();
        let mut single = AfkEngine::with_layout_for_tests(single_layout, AfkPreset::v1(), now());
        let mut cascade =
            AfkEngine::with_layout_for_tests(cascade_layout, line_preset(3, 1), now());

        let single_outcome = single
            .apply_action(AfkAction::Reveal((0, 0)), now())
            .expect("single reveal should succeed");
        let cascade_outcome = cascade
            .apply_action(AfkAction::Reveal((0, 0)), now())
            .expect("cascade reveal should succeed");

        assert_eq!(single_outcome.safe_reveals, 1);
        assert!(cascade_outcome.safe_reveals > 1);
        assert_eq!(
            single_outcome.timer_delta_secs,
            AfkPreset::v1().timer.safe_reveal_bonus_secs as i32
        );
        assert_eq!(
            cascade_outcome.timer_delta_secs,
            line_preset(3, 1).timer.safe_reveal_bonus_secs as i32
        );
    }

    #[test]
    fn mixed_chord_mine_outcome_still_only_gets_one_safe_bonus() {
        let layout = MineLayout::from_mine_coords((2, 2), &[(1, 1)]).unwrap();
        let preset = AfkPreset {
            config: GameConfig::new_unchecked((2, 2), 1),
            timer: AfkTimerProfile::v1(),
        };
        let mut engine = AfkEngine::with_layout_for_tests(layout, preset, now());
        engine
            .apply_action(AfkAction::Reveal((0, 0)), now())
            .expect("revealing clue should succeed");
        engine
            .apply_action(AfkAction::SetFlag((1, 0)), now())
            .expect("misflag should succeed");

        let outcome = engine
            .apply_action(AfkAction::Chord((0, 0)), now())
            .expect("chord should resolve hidden neighbors");

        assert_eq!(outcome.safe_reveals, 1);
        assert!(outcome.mine_triggered);
        assert_eq!(
            outcome.timer_delta_secs,
            preset.timer.safe_reveal_bonus_secs as i32 - preset.timer.mine_penalty_secs as i32
        );
    }

    #[test]
    fn preset_level_tracks_mine_progression() {
        assert_eq!(AfkPreset::v1().current_level(), 1);
        assert_eq!(
            AfkPreset::for_board_size_and_mines(AfkBoardSize::Tiny, 10).current_level(),
            2
        );
        assert_eq!(
            AfkPreset::for_board_size_and_mines(AfkBoardSize::Small, 24).current_level(),
            2
        );
        assert_eq!(
            AfkPreset::for_board_size_and_mines(AfkBoardSize::Medium, 43).current_level(),
            2
        );
        assert_eq!(
            AfkPreset::for_board_size_and_mines(AfkBoardSize::Large, 60).current_level(),
            2
        );
        assert_eq!(
            AfkPreset::for_board_size_and_mines(
                AfkBoardSize::Large,
                AfkBoardSize::Large.max_mines(),
            )
            .current_level(),
            16
        );
    }

    #[test]
    fn mine_hit_creates_a_crater_without_ending_the_round() {
        let layout = MineLayout::from_mine_coords((2, 2), &[(1, 1)]).unwrap();
        let mut engine = AfkEngine::with_layout_for_tests(layout, AfkPreset::v1(), now());

        let outcome = engine
            .apply_action(AfkAction::Reveal((1, 1)), now())
            .expect("mine reveal should succeed");

        assert!(outcome.mine_triggered);
        assert_eq!(engine.phase(), AfkRoundPhase::Active);
        assert_eq!(engine.cell_state_at((1, 1)).unwrap(), AfkCellState::Crater);
    }

    #[test]
    fn timeout_transition_requests_restart_after_delay() {
        let layout = MineLayout::from_mine_coords((2, 2), &[(1, 1)]).unwrap();
        let mut preset = AfkPreset::v1();
        preset.timer.start_secs = 1;
        preset.timer.loss_continue_delay_secs = 2;
        let mut engine = AfkEngine::with_layout_for_tests(layout, preset, now());

        let outcome = engine.settle(now() + 2_000);
        assert!(outcome.changed);
        assert_eq!(engine.phase(), AfkRoundPhase::TimedOut);
        assert!(!engine.settle(now() + 2_500).needs_restart);
        assert!(engine.settle(now() + 4_000).needs_restart);
    }

    #[test]
    fn pause_freezes_active_timer_until_resume() {
        let layout = MineLayout::from_mine_coords((2, 2), &[(1, 1)]).unwrap();
        let mut engine = AfkEngine::with_layout_for_tests(layout, AfkPreset::v1(), now());

        engine.pause(now() + 500);
        assert!(engine.is_paused());
        assert_eq!(engine.display_timer_remaining_secs(now() + 4_500), 120);
        assert!(!engine.settle(now() + 4_500).changed);

        engine.resume(now() + 4_500);
        assert!(!engine.is_paused());
        assert_eq!(engine.settle(now() + 5_500).changed, true);
        assert_eq!(engine.timer_remaining_secs(), 119);
    }

    #[test]
    fn opening_move_starts_the_round_without_consuming_timer_bonus() {
        let mut engine = AfkEngine::new(1, AfkPreset::v1(), now());
        assert_eq!(engine.phase(), AfkRoundPhase::Countdown);
        let opened = engine
            .open_starting_cell((0, 0), now() + 500)
            .expect("opening move should succeed");
        assert!(opened);
        assert_eq!(engine.phase(), AfkRoundPhase::Active);
        assert_eq!(
            engine.timer_remaining_secs(),
            AfkPreset::v1().timer.start_secs as i32
        );
    }

    #[test]
    fn reveal_on_flagged_cell_is_rejected() {
        let layout = MineLayout::from_mine_coords((2, 2), &[(1, 1)]).unwrap();
        let mut engine = AfkEngine::with_layout_for_tests(layout, AfkPreset::v1(), now());

        engine
            .apply_action(AfkAction::SetFlag((0, 0)), now())
            .expect("flag should succeed");
        let outcome = engine
            .apply_action(AfkAction::Reveal((0, 0)), now())
            .expect("reveal on flag should succeed without panic");
        assert!(!outcome.changed);
        assert_eq!(engine.cell_state_at((0, 0)).unwrap(), AfkCellState::Flagged);
    }

    #[test]
    fn cratered_mines_keep_neighbor_counts_intact() {
        let layout = MineLayout::from_mine_coords((3, 2), &[(1, 0), (2, 1)]).unwrap();
        let preset = AfkPreset {
            config: GameConfig::new_unchecked((3, 2), 2),
            timer: AfkTimerProfile::v1(),
        };
        let mut engine = AfkEngine::with_layout_for_tests(layout, preset, now());

        let hit = engine
            .apply_action(AfkAction::Reveal((1, 0)), now())
            .expect("mine reveal should succeed");
        assert!(hit.mine_triggered);

        let reveal = engine
            .apply_action(AfkAction::Reveal((1, 1)), now())
            .expect("safe reveal should succeed");
        assert!(reveal.changed);
        assert_eq!(
            engine.cell_state_at((1, 1)).unwrap(),
            AfkCellState::Revealed(2)
        );
    }

    #[test]
    fn chord_flag_marks_hidden_neighbors_when_only_mines_remain() {
        let layout = MineLayout::from_mine_coords((4, 3), &[(0, 0), (2, 0)]).unwrap();
        let preset = AfkPreset {
            config: GameConfig::new_unchecked((4, 3), 2),
            timer: AfkTimerProfile::v1(),
        };
        let mut engine = AfkEngine::with_layout_for_tests(layout, preset, now());

        engine
            .apply_action(AfkAction::Reveal((1, 1)), now())
            .expect("safe reveal should succeed");
        engine
            .apply_action(AfkAction::Reveal((1, 0)), now())
            .expect("safe reveal should succeed");
        engine
            .apply_action(AfkAction::Reveal((0, 1)), now())
            .expect("safe reveal should succeed");
        engine
            .apply_action(AfkAction::Reveal((2, 1)), now())
            .expect("safe reveal should succeed");
        engine
            .apply_action(AfkAction::Reveal((0, 2)), now())
            .expect("safe reveal should succeed");
        engine
            .apply_action(AfkAction::Reveal((1, 2)), now())
            .expect("safe reveal should succeed");
        engine
            .apply_action(AfkAction::Reveal((2, 2)), now())
            .expect("safe reveal should succeed");
        engine
            .apply_action(AfkAction::Reveal((0, 0)), now())
            .expect("mine reveal should succeed");

        let outcome = engine
            .apply_action(AfkAction::ChordFlag((1, 1)), now())
            .expect("chord flag should succeed");

        assert!(outcome.changed);
        assert_eq!(engine.cell_state_at((2, 0)).unwrap(), AfkCellState::Flagged);
    }

    #[test]
    fn won_round_keeps_craters_visible_while_auto_flagging_other_mines() {
        let layout = MineLayout::from_mine_coords((3, 2), &[(0, 0), (2, 0)]).unwrap();
        let preset = AfkPreset {
            config: GameConfig::new_unchecked((3, 2), 2),
            timer: AfkTimerProfile::v1(),
        };
        let mut engine = AfkEngine::with_layout_for_tests(layout, preset, now());

        let hit = engine
            .apply_action(AfkAction::Reveal((0, 0)), now())
            .expect("mine reveal should succeed");
        assert!(hit.mine_triggered);

        for coords in [(1, 0), (0, 1), (1, 1), (2, 1)] {
            engine
                .apply_action(AfkAction::Reveal(coords), now())
                .expect("safe reveal should succeed");
        }

        assert_eq!(engine.phase(), AfkRoundPhase::Won);
        assert_eq!(engine.cell_state_at((0, 0)).unwrap(), AfkCellState::Crater);
        assert_eq!(engine.cell_state_at((2, 0)).unwrap(), AfkCellState::Flagged);
    }

    #[test]
    fn base_frontier_labels_stay_unchanged_without_endgame_unlock() {
        let mut engine = line_engine(6, &[0, 3, 4, 5]);

        engine
            .apply_action(AfkAction::Reveal((1, 0)), now())
            .expect("safe reveal should succeed");

        assert!(label_at(&engine, (0, 0)));
        assert!(label_at(&engine, (2, 0)));
        assert!(!label_at(&engine, (3, 0)));
        assert!(!label_at(&engine, (4, 0)));
        assert!(!label_at(&engine, (5, 0)));
    }

    #[test]
    fn locally_forced_flags_hide_their_labels() {
        let mut engine = line_engine(2, &[0]);
        engine
            .apply_action(AfkAction::Reveal((1, 0)), now())
            .expect("safe reveal should succeed");
        engine
            .apply_action(AfkAction::SetFlag((0, 0)), now())
            .expect("flag should succeed");

        assert!(!label_at(&engine, (0, 0)));
    }

    #[test]
    fn non_forced_flags_keep_their_labels() {
        let mut engine = line_engine(3, &[0]);
        engine
            .apply_action(AfkAction::Reveal((1, 0)), now())
            .expect("safe reveal should succeed");
        engine
            .apply_action(AfkAction::SetFlag((0, 0)), now())
            .expect("flag should succeed");

        assert!(label_at(&engine, (0, 0)));
    }

    #[test]
    fn safe_leaning_endgame_unlock_labels_back_cells() {
        let mut engine = line_engine(6, &[0, 2, 4, 5]);

        let mine_hit = engine
            .apply_action(AfkAction::Reveal((0, 0)), now())
            .expect("mine reveal should succeed");
        assert!(mine_hit.mine_triggered);
        engine
            .apply_action(AfkAction::Reveal((1, 0)), now())
            .expect("safe reveal should succeed");

        assert!(label_at(&engine, (3, 0)));
        assert!(label_at(&engine, (4, 0)));
        assert!(label_at(&engine, (5, 0)));
    }

    #[test]
    fn mine_leaning_endgame_unlock_uses_two_cell_cushion() {
        for (width, should_unlock) in [(6, true), (7, true), (8, true), (9, false)] {
            let mut engine = line_engine(width, &[0, width - 2, width - 1]);
            engine
                .apply_action(AfkAction::Reveal((1, 0)), now())
                .expect("safe reveal should succeed");

            assert_eq!(
                label_at(&engine, (3, 0)),
                should_unlock,
                "width {width} should {}unlock back cells",
                if should_unlock { "" } else { "not " }
            );
        }
    }

    #[test]
    fn unsupported_manual_flags_do_not_unlock_back_cells() {
        let mut engine = line_engine(6, &[0, 3, 4, 5]);
        engine
            .apply_action(AfkAction::Reveal((1, 0)), now())
            .expect("safe reveal should succeed");
        engine
            .apply_action(AfkAction::SetFlag((4, 0)), now())
            .expect("flag should succeed");
        engine
            .apply_action(AfkAction::SetFlag((5, 0)), now())
            .expect("flag should succeed");

        assert!(!label_at(&engine, (3, 0)));
        assert!(label_at(&engine, (4, 0)));
        assert!(label_at(&engine, (5, 0)));
    }
}
