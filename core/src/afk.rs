use alloc::collections::VecDeque;
use core::num::Saturating;

use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::{
    Coord2, FirstMovePolicy, GameConfig, GameError, LayoutGenerator, MineLayout, NeighborIterExt,
    RandomLayoutGenerator, Result, ToNdIndex,
};

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
            start_delay_secs: 5,
            win_continue_delay_secs: 30,
            loss_continue_delay_secs: 60,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AfkPreset {
    pub config: GameConfig,
    pub timer: AfkTimerProfile,
}

impl AfkPreset {
    pub const INITIAL_MINES: u16 = 50;
    pub const MAX_MINES: u16 = 200;
    pub const MINE_INCREMENT: u16 = 10;

    pub const fn for_mines(mines: u16) -> Self {
        Self {
            config: GameConfig::new_unchecked((30, 20), mines),
            timer: AfkTimerProfile::v1(),
        }
    }

    pub const fn v1() -> Self {
        Self::for_mines(Self::INITIAL_MINES)
    }

    pub const fn next_mine_count(current: u16) -> u16 {
        if current >= Self::MAX_MINES - Self::MINE_INCREMENT {
            Self::MAX_MINES
        } else {
            current + Self::MINE_INCREMENT
        }
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

    pub fn live_mines_left_for_display(&self) -> i32 {
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
        let mut timer_delta_secs =
            i32::from(safe_reveals) * self.preset.timer.safe_reveal_bonus_secs as i32;
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

    fn has_mine_at(&self, coords: Coord2) -> Result<bool> {
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
        if matches!(self.phase, AfkRoundPhase::Countdown) {
            return Ok(false);
        }
        match self.cell_state_at(coords)? {
            AfkCellState::Flagged => Ok(true),
            AfkCellState::Hidden => Ok(self.board.iter_neighbors(coords).any(|neighbor| {
                matches!(self.cell_state_at(neighbor), Ok(AfkCellState::Crater))
                    || matches!(
                        self.cell_state_at(neighbor),
                        Ok(AfkCellState::Revealed(count)) if count > 0
                    )
            })),
            AfkCellState::Revealed(_)
            | AfkCellState::Mine
            | AfkCellState::Misflagged
            | AfkCellState::Crater => Ok(false),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> i64 {
        1_000
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
}
