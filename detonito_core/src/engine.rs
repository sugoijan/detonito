use alloc::collections::{BTreeSet, VecDeque};
use core::num::Saturating;
use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::*;

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EngineState {
    Ready,
    Active,
    Won,
    Lost,
}

impl EngineState {
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }

    pub const fn is_finished(self) -> bool {
        matches!(self, Self::Won | Self::Lost)
    }
}

impl Default for EngineState {
    fn default() -> Self {
        Self::Ready
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayEngine {
    mine_layout: MineLayout,
    board: Array2<EngineCell>,
    revealed_count: Saturating<CellCount>,
    flagged_count: Saturating<CellCount>,
    state: EngineState,
    triggered_mine: Option<Coord2>,
}

impl PlayEngine {
    pub fn new(mine_layout: MineLayout) -> Self {
        let size = mine_layout.size();
        Self {
            mine_layout,
            board: Array2::default(size.to_nd_index()),
            revealed_count: Saturating(0),
            flagged_count: Saturating(0),
            state: Default::default(),
            triggered_mine: None,
        }
    }

    pub fn state(&self) -> EngineState {
        self.state
    }

    pub fn is_finished(&self) -> bool {
        self.state.is_finished()
    }

    pub fn size(&self) -> Coord2 {
        self.mine_layout.size()
    }

    pub fn total_mines(&self) -> CellCount {
        self.mine_layout.mine_count()
    }

    pub fn mines_left(&self) -> isize {
        (self.mine_layout.mine_count() as isize) - (self.flagged_count.0 as isize)
    }

    pub fn cell_at(&self, coords: Coord2) -> EngineCell {
        self.board[coords.to_nd_index()]
    }

    pub fn triggered_mine(&self) -> Option<Coord2> {
        self.triggered_mine
    }

    pub fn has_mine_at(&self, coords: Coord2) -> bool {
        self.mine_layout.contains_mine(coords)
    }

    pub fn can_interact_at(&self, coords: Coord2) -> bool {
        use EngineCell::*;

        if self.state.is_finished() {
            return false;
        }

        match self.cell_at(coords) {
            Hidden => true,
            Revealed(count) if count == 0 => false,
            Revealed(count) => {
                let mut adjacent_flag_count = 0;
                for pos in self.mine_layout_iter_neighbors(coords) {
                    let adjacent_cell = self.board[pos.to_nd_index()];
                    match adjacent_cell {
                        Flagged => adjacent_flag_count += 1,
                        Revealed(_) => continue,
                        Hidden => return true,
                    }
                }
                adjacent_flag_count != count
            }
            Flagged => true,
        }
    }

    pub fn can_chord_reveal_at(&self, coords: Coord2) -> bool {
        if self.state.is_finished() {
            return false;
        }

        if let EngineCell::Revealed(count) = self.board[coords.to_nd_index()] {
            count == self.count_flagged_neighbors(coords)
        } else {
            false
        }
    }

    pub fn toggle_flag(&mut self, coords: Coord2) -> Result<MarkOutcome> {
        use EngineCell::*;
        use MarkOutcome::*;

        let coords = self.mine_layout.validate_coords(coords)?;
        self.check_active()?;

        Ok(match self.board[coords.to_nd_index()] {
            Hidden => {
                self.board[coords.to_nd_index()] = Flagged;
                self.flagged_count += 1;
                Changed
            }
            Flagged => {
                self.board[coords.to_nd_index()] = Hidden;
                self.flagged_count -= 1;
                Changed
            }
            Revealed(_) => NoChange,
        })
    }

    pub fn chord_flag(&mut self, coords: Coord2) -> Result<MarkOutcome> {
        use EngineCell::*;
        use MarkOutcome::*;

        let coords = self.mine_layout.validate_coords(coords)?;
        self.check_active()?;

        let Revealed(count) = self.board[coords.to_nd_index()] else {
            return Ok(NoChange);
        };

        if count != self.count_unrevealed_neighbors(coords) {
            return Ok(NoChange);
        }

        let mut updated = false;
        for pos in self.mine_layout_iter_neighbors(coords) {
            if matches!(self.board[pos.to_nd_index()], Hidden) {
                self.board[pos.to_nd_index()] = Flagged;
                self.flagged_count += 1;
                updated = true;
            }
        }

        Ok(if updated { Changed } else { NoChange })
    }

    pub fn reveal(&mut self, coords: Coord2) -> Result<RevealOutcome> {
        use EngineCell::*;
        use RevealOutcome::*;

        let coords = self.mine_layout.validate_coords(coords)?;

        if matches!(self.board[coords.to_nd_index()], Hidden) {
            self.check_not_finished()?;
            Ok(self.reveal_single_cell(coords))
        } else {
            Ok(NoChange)
        }
    }

    pub fn chord_reveal(&mut self, coords: Coord2) -> Result<RevealOutcome> {
        let coords = self.mine_layout.validate_coords(coords)?;
        self.check_not_finished()?;

        Ok(match self.board[coords.to_nd_index()] {
            EngineCell::Revealed(count) if count == self.count_flagged_neighbors(coords) => {
                self.check_active()?;
                self.mine_layout_iter_neighbors(coords)
                    .map(|neighbor_coords| self.reveal_single_cell(neighbor_coords))
                    .reduce(core::ops::BitOr::bitor)
                    .unwrap_or(RevealOutcome::NoChange)
            }
            _ => self.reveal_single_cell(coords),
        })
    }

    fn reveal_single_cell(&mut self, coords: Coord2) -> RevealOutcome {
        let cell_state = self.board[coords.to_nd_index()];
        let has_mine = self.mine_layout[coords];

        match (cell_state, has_mine) {
            (EngineCell::Hidden, true) => {
                self.triggered_mine = Some(coords);
                self.end_game(false);
                RevealOutcome::HitMine
            }
            (EngineCell::Hidden, false) => {
                let adjacent_mines = self.mine_layout.adjacent_mine_count(coords);
                self.board[coords.to_nd_index()] = EngineCell::Revealed(adjacent_mines);
                self.revealed_count += 1;

                if adjacent_mines == 0 {
                    let mut visited = BTreeSet::from([coords]);
                    let mut to_visit: VecDeque<_> = self
                        .mine_layout_iter_neighbors(coords)
                        .filter(|&pos| matches!(self.board[pos.to_nd_index()], EngineCell::Hidden))
                        .collect();

                    while let Some(visit_coords) = to_visit.pop_front() {
                        if !visited.insert(visit_coords) {
                            continue;
                        }

                        if matches!(
                            self.board[visit_coords.to_nd_index()],
                            EngineCell::Revealed(_) | EngineCell::Flagged
                        ) {
                            continue;
                        }

                        let visit_adjacent_mines =
                            self.mine_layout.adjacent_mine_count(visit_coords);
                        self.board[visit_coords.to_nd_index()] =
                            EngineCell::Revealed(visit_adjacent_mines);
                        self.revealed_count += 1;

                        if visit_adjacent_mines == 0 {
                            to_visit.extend(
                                self.mine_layout_iter_neighbors(visit_coords)
                                    .filter(|&pos| {
                                        matches!(self.board[pos.to_nd_index()], EngineCell::Hidden)
                                    })
                                    .filter(|pos| !visited.contains(pos)),
                            );
                        }
                    }
                }

                if self.revealed_count == Saturating(self.mine_layout.safe_cell_count()) {
                    self.end_game(true);
                    RevealOutcome::Won
                } else {
                    self.mark_started();
                    RevealOutcome::Revealed
                }
            }
            _ => RevealOutcome::NoChange,
        }
    }

    fn mark_started(&mut self) {
        if matches!(self.state, EngineState::Ready) {
            self.state = EngineState::Active;
        }
    }

    fn end_game(&mut self, won: bool) {
        if self.state.is_finished() {
            return;
        }

        self.state = if won {
            EngineState::Won
        } else {
            EngineState::Lost
        };
        if won {
            self.triggered_mine = None;
        }
    }

    fn count_flagged_neighbors(&self, coords: Coord2) -> u8 {
        self.mine_layout_iter_neighbors(coords)
            .filter(|&pos| self.board[pos.to_nd_index()] == EngineCell::Flagged)
            .count()
            .try_into()
            .unwrap()
    }

    fn count_unrevealed_neighbors(&self, coords: Coord2) -> u8 {
        self.mine_layout_iter_neighbors(coords)
            .filter(|&pos| self.board[pos.to_nd_index()].is_unrevealed())
            .count()
            .try_into()
            .unwrap()
    }

    fn check_active(&self) -> Result<()> {
        if matches!(self.state, EngineState::Active) {
            Ok(())
        } else {
            Err(GameError::AlreadyEnded)
        }
    }

    fn check_not_finished(&self) -> Result<()> {
        if self.state.is_finished() {
            Err(GameError::AlreadyEnded)
        } else {
            Ok(())
        }
    }

    fn mine_layout_iter_neighbors(&self, coords: Coord2) -> impl Iterator<Item = Coord2> + use<> {
        self.mine_layout.iter_neighbors(coords)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(size: Coord2, mines: &[Coord2]) -> MineLayout {
        MineLayout::from_mine_coords(size, mines).unwrap()
    }

    #[test]
    fn reveal_hits_mine_and_sets_triggered_cell() {
        let mut engine = PlayEngine::new(layout((2, 2), &[(0, 0)]));

        let outcome = engine.reveal((0, 0)).unwrap();

        assert_eq!(outcome, RevealOutcome::HitMine);
        assert_eq!(engine.state(), EngineState::Lost);
        assert_eq!(engine.triggered_mine(), Some((0, 0)));
    }

    #[test]
    fn reveal_flood_fill_opens_zero_region() {
        let mut engine = PlayEngine::new(layout((3, 3), &[(2, 2)]));

        let outcome = engine.reveal((0, 0)).unwrap();

        assert_eq!(outcome, RevealOutcome::Won);
        assert_eq!(engine.cell_at((0, 0)), EngineCell::Revealed(0));
        assert_eq!(engine.cell_at((1, 1)), EngineCell::Revealed(1));
        assert_eq!(engine.cell_at((2, 2)), EngineCell::Hidden);
    }

    #[test]
    fn chord_reveal_uses_flagged_neighbors() {
        let mines = &[(0, 1), (2, 1)];
        let mut engine = PlayEngine::new(layout((3, 3), mines));

        engine.reveal((1, 1)).unwrap();
        engine.toggle_flag((0, 1)).unwrap();
        engine.toggle_flag((2, 1)).unwrap();

        let outcome = engine.chord_reveal((1, 1)).unwrap();

        assert_eq!(outcome, RevealOutcome::Won);
        assert_eq!(engine.cell_at((1, 0)), EngineCell::Revealed(2));
        assert_eq!(engine.cell_at((1, 2)), EngineCell::Revealed(2));
    }

    #[test]
    fn chord_flag_marks_all_unrevealed_neighbors_when_count_matches() {
        let mines = &[(0, 0), (2, 0)];
        let mut engine = PlayEngine::new(layout((4, 1), mines));

        assert_eq!(engine.reveal((1, 0)).unwrap(), RevealOutcome::Revealed);
        let outcome = engine.chord_flag((1, 0)).unwrap();

        assert_eq!(outcome, MarkOutcome::Changed);
        assert_eq!(engine.cell_at((0, 0)), EngineCell::Flagged);
        assert_eq!(engine.cell_at((2, 0)), EngineCell::Flagged);
    }

    #[test]
    fn winning_board_transitions_to_won_state() {
        let mut engine = PlayEngine::new(layout((2, 1), &[(0, 0)]));

        assert_eq!(engine.reveal((1, 0)).unwrap(), RevealOutcome::Won);
        assert_eq!(engine.state(), EngineState::Won);
        assert!(engine.is_finished());
    }
}
