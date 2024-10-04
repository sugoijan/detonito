use chrono::prelude::*;
use ndarray::Array2;
use serde::{Deserialize, Serialize};
use std::ops::{BitOr, Index, IndexMut};

pub use error::*;
pub use generator::*;
pub use tile::*;
pub use types::*;

mod error;
mod generator;
mod tile;
mod types;

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameConfig {
    pub size: Ix2,
    pub mines: Ax,
}

impl GameConfig {
    pub(crate) const fn new_unchecked(size: Ix2, mines: Ax) -> Self {
        Self { size, mines }
    }

    pub fn new((size_x, size_y): Ix2, mines: Ax) -> Self {
        let size_x = size_x.clamp(1, Ix::MAX);
        let size_y = size_y.clamp(1, Ix::MAX);
        let mines = mines.clamp(1, mult(size_x, size_y));
        Self::new_unchecked((size_x, size_y), mines)
    }

    pub const fn total_tiles(&self) -> Ax {
        mult(self.size.0, self.size.1)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Minefield {
    mines: Array2<bool>,
    count: Ax,
}

impl Minefield {
    pub fn game_config(&self) -> GameConfig {
        GameConfig {
            size: self.size(),
            mines: self.count,
        }
    }

    pub fn validate_coords(&self, coords: Ix2) -> Result<Ix2> {
        let size = self.size();
        if coords.0 < size.0 && coords.1 < size.1 {
            Ok(coords)
        } else {
            Err(GameError::InvalidCoords)
        }
    }

    pub fn size(&self) -> Ix2 {
        let dim = self.mines.dim();
        (dim.0.try_into().unwrap(), dim.1.try_into().unwrap())
    }

    pub fn safe_count(&self) -> Ax {
        self.total_tiles() - self.count
    }

    pub fn total_tiles(&self) -> Ax {
        self.mines.len().try_into().unwrap()
    }

    pub fn get_count(&self, coords: Ix2) -> u8 {
        self.mines.iter_adjacent(coords).filter(|&pos| self[pos]).count().try_into().unwrap()
    }
}

impl Index<Ix2> for Minefield {
    type Output = bool;

    fn index(&self, (ix, iy): Ix2) -> &Self::Output {
        &self.mines[(ix as usize, iy as usize)]
    }
}

impl IndexMut<Ix2> for Minefield {
    fn index_mut(&mut self, (ix, iy): Ix2) -> &mut Self::Output {
        &mut self.mines[(ix as usize, iy as usize)]
    }
}

/// Outcome of opening a tile
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum FlagOutcome {
    NoChange,
    MarkChanged,
}

impl FlagOutcome {
    /// Whether this outcome could have caused an update to the game
    pub const fn has_update(self) -> bool {
        match self {
            Self::NoChange => false,
            Self::MarkChanged => true,
        }
    }
}

/// Outcome of opening a tile
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum OpenOutcome {
    NoChange,
    Safe,
    Explode,
    Win,
}

impl OpenOutcome {
    /// Whether this outcome could have caused an update to the game
    pub const fn has_update(self) -> bool {
        use OpenOutcome::*;
        match self {
            NoChange => false,
            Safe => true,
            Explode => true,
            Win => true,
        }
    }
}

/// Used to merge outcomes when multi-opening
impl BitOr for OpenOutcome {
    type Output = OpenOutcome;

    // rhs is the "right-hand side" of the expression `a | b`
    fn bitor(self, rhs: Self) -> Self::Output {
        use OpenOutcome::*;
        match (self, rhs) {
            // explode has priority
            (Explode, _) => Explode,
            (_, Explode) => Explode,
            // then win
            (Win, _) => Win,
            (_, Win) => Win,
            // then safe
            (Safe, _) => Safe,
            (_, Safe) => Safe,
            // and no-change only with both
            (NoChange, NoChange) => NoChange,
        }
    }
}

/// Valid transitions:
/// - NotStarted -> InstantWin
/// - NotStarted -> InstantLoss
/// - NotStarted -> InProgress
/// - InProgress -> Win
/// - InProgress -> Loss
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GameState {
    /// Initial state
    NotStarted,
    /// Game started
    InProgress,
    /// Game ended and player won
    Win,
    /// Game ended and player lost
    Lose,
    /// Game ended and player won on the first move
    InstantWin,
    /// Game ended and player lost on the first move
    InstantLoss,
}

impl GameState {
    /// Indicates the game has not started yet
    pub const fn is_initial(self) -> bool {
        use GameState::*;
        match self {
            NotStarted => true,
            InProgress => false,
            Win => false,
            Lose => false,
            InstantWin => false,
            InstantLoss => false,
        }
    }

    /// Indicates the game has ended and no moves can be made anymore
    pub const fn is_final(self) -> bool {
        use GameState::*;
        match self {
            NotStarted => false,
            InProgress => false,
            Win => true,
            Lose => true,
            InstantWin => true,
            InstantLoss => true,
        }
    }
}

impl Default for GameState {
    fn default() -> Self {
        Self::NotStarted
    }
}

/// Represents a game from start to finish
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Game {
    minefield: Minefield,
    grid: Array2<AnyTile>,
    open_count: Ax,
    flag_count: Ax,
    state: GameState,
    started_at: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
}

impl Game {
    // Initialize the grid
    pub fn new(minefield: Minefield) -> Game {
        let size = minefield.size();
        Self {
            minefield,
            grid: Array2::default(size.convert()),
            open_count: 0,
            flag_count: 0,
            state: Default::default(),
            started_at: None,
            ended_at: None,
        }
    }

    pub fn cur_state(&self) -> GameState {
        self.state
    }

    pub fn ended(&self) -> bool {
        self.state.is_final()
    }

    pub fn size(&self) -> Ix2 {
        self.minefield.size()
    }

    pub fn total_mines(&self) -> Ax {
        self.minefield.count
    }

    pub fn tile_at(&self, coords: Ix2) -> AnyTile {
        self.grid[coords.convert()]
    }

    fn check_in_progress(&self) -> Result<()> {
        if matches!(self.state, GameState::InProgress) {
            Ok(())
        } else {
            Err(GameError::AlreadyEnded)
        }
    }

    fn check_final(&self) -> Result<()> {
        if self.state.is_final() {
            Err(GameError::AlreadyEnded)
        } else {
            Ok(())
        }
    }

    /// How many seconds have passed since game started, 0 if it hasn't started
    pub fn elapsed_secs(&self) -> u32 {
        if let Some(started_at) = self.started_at {
            (self.ended_at.unwrap_or_else(Utc::now) - started_at)
                .num_seconds()
                .max(0) as u32
        } else {
            0
        }
    }

    /// How many mines have not been flagged yet
    pub fn mines_left(&self) -> isize {
        (self.minefield.count as isize) - (self.flag_count as isize)
    }

    /// Flag a tile, do not consider question marker (unmark question if tile has one)
    pub fn flag(&mut self, coords: Ix2) -> Result<FlagOutcome> {
        self.do_flag_question(coords, false)
    }

    /// Flag or question a tile
    pub fn flag_question(&mut self, coords: Ix2) -> Result<FlagOutcome> {
        self.do_flag_question(coords, true)
    }

    pub fn do_flag_question(&mut self, coords: Ix2, use_question: bool) -> Result<FlagOutcome> {
        use AnyTile::*;
        use FlagOutcome::*;

        let coords = self.minefield.validate_coords(coords)?;

        self.check_in_progress()?;

        Ok(match self.grid[coords.convert()] {
            Closed => {
                self.grid[coords.convert()] = Flag;
                self.flag_count += 1;
                MarkChanged
            }
            Flag => {
                self.grid[coords.convert()] = if use_question { Question } else { Closed };
                self.flag_count -= 1;
                MarkChanged
            }
            Question => {
                self.grid[coords.convert()] = Closed;
                MarkChanged
            }
            _ => NoChange,
        })
    }

    fn count_flagged(&self, coords: Ix2) -> u8 {
        self.minefield.mines.iter_adjacent(coords)
            .filter(|&pos| self.grid[pos.convert()] == AnyTile::Flag)
            .count()
            .try_into()
            .unwrap()
    }

    fn has_question_neighbor(&self, coords: Ix2) -> bool {
        self.minefield.mines.iter_adjacent(coords)
            .map(|pos| self.grid[pos.convert()])
            .any(|tile| tile == AnyTile::Question)
    }

    /// Open a closed tile, do not open neighbor tiles
    pub fn open(&mut self, coords: Ix2) -> Result<OpenOutcome> {
        if matches!(self.grid[coords.convert()], AnyTile::Closed) {
            self.open_with_chords(coords)
        } else {
            Ok(OpenOutcome::NoChange)
        }
    }

    pub fn is_chordable(&self, coords: Ix2) -> bool {
        if let AnyTile::Open(count) = self.grid[coords.convert()] {
            count == self.count_flagged(coords) && !self.has_question_neighbor(coords)
        } else {
            false
        }
    }

    /// Open a tile, or try to open neighbor tiles
    pub fn open_with_chords(&mut self, coords: Ix2) -> Result<OpenOutcome> {
        use OpenOutcome::*;

        let coords = self.minefield.validate_coords(coords)?;

        self.check_final()?;

        Ok(match self.grid[coords.convert()] {
            AnyTile::Open(count)
                if count == self.count_flagged(coords) && !self.has_question_neighbor(coords) =>
            {
                self.check_in_progress()?;
                // Perform opening of all closed neighbors when flagged count matches
                self.minefield.mines.iter_adjacent(coords)
                    .map(|neighbor_coords| self.open_tile(neighbor_coords))
                    .reduce(BitOr::bitor)
                    .unwrap_or(NoChange)
            }
            _ => self.open_tile(coords),
        })
    }

    /// Helper function to open a single tile and perform flood-fill if necessary
    fn open_tile(&mut self, coords: Ix2) -> OpenOutcome {
        use std::collections::{HashSet, VecDeque};
        use AnyTile::*;
        use OpenOutcome::*;

        let tile = self.grid[coords.convert()];
        let mine = self.minefield[coords];

        match (tile, mine) {
            (Closed, true) => {
                self.grid[coords.convert()] = Exploded;
                self.mark_ended(false);
                Explode
            }
            (Closed, false) => {
                let count = self.minefield.get_count(coords);
                self.grid[coords.convert()] = Open(count);
                self.open_count += 1;
                log::debug!("Open tile at {:?}, mine count: {}", coords, count);

                if count == 0 {
                    let mut visited = HashSet::from([coords]);
                    let mut to_visit: VecDeque<_> = self
                        .minefield.mines.iter_adjacent(coords)
                        .filter(|&pos| matches!(self.grid[pos.convert()], Closed))
                        .collect();
                    log::trace!(
                        "Starting flood-fill from {:?}, initial neighbors: {:?}",
                        coords,
                        to_visit
                    );

                    while let Some(visit_coords) = to_visit.pop_front() {
                        if !visited.insert(visit_coords) {
                            continue;
                        }

                        // skip flagged or already opened tiles
                        if matches!(self.grid[visit_coords.convert()], Open(_) | Flag) {
                            log::trace!("Skipping tile at {:?}", visit_coords);
                            continue;
                        }

                        // open visited tiles
                        let visit_count = self.minefield.get_count(visit_coords);
                        self.grid[visit_coords.convert()] = Open(visit_count);
                        self.open_count += 1;
                        log::trace!(
                            "Flood opened tile at {:?}, mine count: {}",
                            visit_coords,
                            visit_count
                        );

                        // if this is also zero we visit the neighbors
                        if visit_count == 0 {
                            to_visit.extend(
                                self.minefield
                                    .mines.iter_adjacent(visit_coords)
                                    .filter(|&pos| matches!(self.grid[pos.convert()], Closed))
                                    .filter(|pos| !visited.contains(pos)),
                            );
                        }
                    }
                }

                if self.open_count == self.minefield.safe_count() {
                    self.mark_ended(true);
                    Win
                } else {
                    self.mark_started();
                    Safe
                }
            }
            _ => NoChange,
        }
    }

    /// Checks if the state is initial and changes to in-progress recording the start time
    fn mark_started(&mut self) {
        if matches!(self.state, GameState::NotStarted) {
            let now = Utc::now();
            log::debug!("started at {}", now);
            self.started_at.replace(now);
            self.state = GameState::InProgress;
        }
    }

    /// Checks for wrong flags and unflagged mines after game ends
    fn mark_ended(&mut self, won: bool) {
        use GameState::*;
        match (self.state, won) {
            (Win, false) => {
                self.state = Lose;
                self.reveal_mines(false);
                return;
            }
            (InstantWin, false) => {
                self.state = InstantLoss;
                self.reveal_mines(false);
                return;
            }
            (Win, _) => return,
            (Lose, _) => return,
            (InstantWin, _) => return,
            (InstantLoss, _) => return,
            (NotStarted, false) => {
                self.state = InstantLoss;
            }
            (InProgress, false) => {
                self.state = Lose;
            }
            (NotStarted, true) => {
                self.state = InstantWin;
            }
            (InProgress, true) => {
                self.state = Win;
            }
        }
        let now = Utc::now();
        self.ended_at.replace(now);
        log::debug!("ended at {}", now);
        if matches!(self.state, InstantWin | InstantLoss) {
            log::debug!("started at {}", now);
            self.started_at.replace(now);
        }
        self.reveal_mines(won);
    }

    fn reveal_mines(&mut self, won: bool) {
        use AnyTile::*;
        let (x_end, y_end) = self.minefield.size();
        for x in 0..x_end {
            for y in 0..y_end {
                let coords = (x, y);
                let tile = self.grid[coords.convert()];
                let mine = self.minefield[coords];
                if mine {
                    if tile == Closed || tile == Question {
                        if won {
                            self.grid[coords.convert()] = Flag;
                            self.flag_count += 1;
                        } else {
                            self.grid[coords.convert()] = Mine;
                        }
                    }
                } else {
                    if tile == Flag {
                        self.grid[coords.convert()] = IncorrectFlag;
                    }
                }
            }
        }
    }
}
