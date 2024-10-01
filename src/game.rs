use chrono::prelude::*;
use ndarray::Array2;
use serde::{Deserialize, Serialize};
use std::ops::{BitOr, Index, IndexMut};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GameError {
    #[error("Invalid coordinates")]
    InvalidCoords,
    #[error("Too many mines")]
    TooManyMines,
    #[error("Game already ended, no new moves are accepted")]
    AlreadyEnded,
}

pub type Result<T> = std::result::Result<T, GameError>;

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Difficulty {
    pub size: (usize, usize),
    pub mines: usize,
}

impl Difficulty {
    pub const MAX_SIZE: usize = 100;

    pub(crate) const fn new_unchecked(size: (usize, usize), mines: usize) -> Self {
        Self { size, mines }
    }

    pub fn new((size_x, size_y): (usize, usize), mines: usize) -> Self {
        let size_x = size_x.clamp(1, Self::MAX_SIZE);
        let size_y = size_y.clamp(1, Self::MAX_SIZE);
        let mines = mines.clamp(1, size_x * size_y);
        Self::new_unchecked((size_x, size_y), mines)
    }

    pub const fn total_tiles(&self) -> usize {
        self.size.0 * self.size.1
    }
}

pub trait MinefieldGenerator {
    fn generate(self, difficulty: Difficulty) -> Minefield;
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum StartTile {
    Random,
    SimpleSafe,
    AlwaysZero,
}

/// Generation strategy that can optionally try to make the starting tile zero or at least safe, but other than that is
/// purely random.
#[derive(Clone, Debug, PartialEq)]
pub struct RandomMinefieldGenerator {
    seed: u64,
    start: (usize, usize),
    start_tile: StartTile,
}

impl RandomMinefieldGenerator {
    pub fn new(seed: u64, start: (usize, usize), start_tile: StartTile) -> Self {
        Self {
            seed,
            start,
            start_tile,
        }
    }
}

impl MinefieldGenerator for RandomMinefieldGenerator {
    fn generate(self, diff: Difficulty) -> Minefield {
        use rand::prelude::*;
        use StartTile::*;

        let total_tiles = diff.total_tiles();

        // optimize for full boards
        if diff.mines >= total_tiles {
            if diff.mines > total_tiles {
                log::warn!(
                    "Minefield already full, generated anyway, requested {} but only fits {}",
                    diff.mines,
                    total_tiles
                );
            }
            return Minefield {
                mines: Array2::from_elem(diff.size, true),
                count: diff.mines,
            };
        }

        let actual_start_tile = match self.start_tile {
            Random => Random,
            SimpleSafe | AlwaysZero if diff.mines + 1 > total_tiles => {
                log::warn!("Cannot make start tile safe, fallback to random");
                Random
            }
            SimpleSafe => SimpleSafe,
            AlwaysZero if diff.mines + 9 > total_tiles => {
                log::warn!("Cannot make start tile zero, fallback to simple safe");
                SimpleSafe
            }
            AlwaysZero => AlwaysZero,
        };
        let mut mines: Array2<bool> = Array2::default(diff.size);
        let mut free_tiles = match actual_start_tile {
            Random => total_tiles,
            SimpleSafe => {
                mines[self.start] = true;
                total_tiles - 1
            }
            AlwaysZero => {
                mines[self.start] = true;
                for coord in IterNeighbors::new(self.start, diff.size) {
                    mines[coord] = true;
                }
                total_tiles - 9
            }
        };
        let mut mines_placed = 0;

        let mut rng = SmallRng::seed_from_u64(self.seed);
        {
            let tiles = mines.as_slice_mut().expect("layout should be standard");
            while mines_placed < diff.mines {
                if free_tiles == 0 {
                    break;
                }
                let mut place = rng.gen_range(0..free_tiles);
                for (i, tile) in tiles.iter_mut().enumerate() {
                    if *tile {
                        place += 1;
                    }
                    if i == place {
                        *tile = true;
                        mines_placed += 1;
                        free_tiles -= 1;
                        break;
                    }
                }
            }
        }

        // undo to make safe tiles
        match actual_start_tile {
            Random => {}
            SimpleSafe => {
                mines[self.start] = false;
            }
            AlwaysZero => {
                mines[self.start] = false;
                for coord in IterNeighbors::new(self.start, diff.size) {
                    mines[coord] = false;
                }
            }
        }

        // double check mine count
        let count = mines.iter().filter(|&&tile| tile).count();
        if count != diff.mines {
            log::warn!(
                "Generated minefield count mismatch, actual: {}, requested: {}",
                count,
                diff.mines
            );
        }
        Minefield { mines, count }
    }
}

// Define a displacement mapping for each direction
const DISPLACEMENTS: [(isize, isize); 8] = [
    (-1, -1), // Top-Left
    (0, -1),  // Top
    (1, -1),  // Top-Right
    (-1, 0),  // Left
    (1, 0),   // Right
    (-1, 1),  // Bottom-Left
    (0, 1),   // Bottom
    (1, 1),   // Bottom-Right
];

/// Will make coords + delta and return the result if it is withing bounds
fn apply_delta(
    coords: (usize, usize),
    delta: (isize, isize),
    bounds: (usize, usize),
) -> Option<(usize, usize)> {
    let (x, y) = coords;
    let (dx, dy) = delta;
    let (bx, by) = bounds;
    let nx = x.checked_add_signed(dx)?;
    if nx >= bx {
        return None;
    }
    let ny = y.checked_add_signed(dy)?;
    if ny >= by {
        return None;
    }
    Some((nx, ny))
}

#[derive(Debug)]
struct IterNeighbors {
    center: (usize, usize),
    bounds: (usize, usize),
    index: usize,
}

impl IterNeighbors {
    fn new(center: (usize, usize), bounds: (usize, usize)) -> Self {
        IterNeighbors {
            center,
            bounds,
            index: 0,
        }
    }
}

impl Iterator for IterNeighbors {
    type Item = (usize, usize);
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.index >= DISPLACEMENTS.len() {
                return None;
            }
            let next_item = apply_delta(self.center, DISPLACEMENTS[self.index], self.bounds);
            self.index += 1;
            if next_item.is_some() {
                return next_item;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Minefield {
    mines: Array2<bool>,
    count: usize,
}

impl Minefield {
    pub fn difficulty(&self) -> Difficulty {
        Difficulty {
            size: self.size(),
            mines: self.count,
        }
    }

    pub fn validate_coords(&self, coords: (usize, usize)) -> Result<(usize, usize)> {
        let size = self.size();
        if coords.0 < size.0 && coords.1 < size.1 {
            Ok(coords)
        } else {
            Err(GameError::InvalidCoords)
        }
    }

    pub fn size(&self) -> (usize, usize) {
        self.mines.dim()
    }

    pub fn safe_count(&self) -> usize {
        self.total_tiles() - self.count
    }

    pub fn total_tiles(&self) -> usize {
        self.mines.len()
    }

    pub fn iter_neighbors(&self, coords: (usize, usize)) -> impl Iterator<Item = (usize, usize)> {
        IterNeighbors::new(coords, self.size())
    }

    pub fn get_count(&self, coords: (usize, usize)) -> usize {
        self.iter_neighbors(coords).filter(|&pos| self[pos]).count()
    }
}

impl Index<(usize, usize)> for Minefield {
    type Output = bool;

    fn index(&self, index: (usize, usize)) -> &Self::Output {
        &self.mines[index]
    }
}

impl IndexMut<(usize, usize)> for Minefield {
    fn index_mut(&mut self, index: (usize, usize)) -> &mut Self::Output {
        &mut self.mines[index]
    }
}

// Define your enum for tile state and make it JS-compatible
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Tile {
    Closed,
    Open(usize),
    Flag,
    Question,
    Exploded,
    // these are only used to show result after the game finishes:
    Mine,
    AutoFlag,
    IncorrectFlag,
}

impl Tile {
    // whether the tile is visually closed
    pub fn is_closed(self) -> bool {
        use Tile::*;
        match self {
            Closed => true,
            Open(_) => false,
            Flag => true,
            Question => true,
            Exploded => false,
            Mine => false,
            AutoFlag => true,
            IncorrectFlag => true,
        }
    }
}

impl Default for Tile {
    fn default() -> Self {
        Self::Closed
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
    grid: Array2<Tile>,
    open_count: usize,
    flag_count: usize,
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
            grid: Array2::default(size),
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

    pub fn size(&self) -> (usize, usize) {
        self.minefield.size()
    }

    pub fn total_mines(&self) -> usize {
        self.minefield.count
    }

    pub fn tile_at(&self, coords: (usize, usize)) -> Tile {
        self.grid[coords]
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
    pub fn flag(&mut self, coords: (usize, usize)) -> Result<FlagOutcome> {
        self.do_flag_question(coords, false)
    }

    /// Flag or question a tile
    pub fn flag_question(&mut self, coords: (usize, usize)) -> Result<FlagOutcome> {
        self.do_flag_question(coords, true)
    }

    pub fn do_flag_question(
        &mut self,
        coords: (usize, usize),
        use_question: bool,
    ) -> Result<FlagOutcome> {
        use FlagOutcome::*;
        use Tile::*;

        let coords = self.minefield.validate_coords(coords)?;

        self.check_in_progress()?;

        Ok(match self.grid[coords] {
            Closed => {
                self.grid[coords] = Flag;
                self.flag_count += 1;
                MarkChanged
            }
            Flag => {
                self.grid[coords] = if use_question { Question } else { Closed };
                self.flag_count -= 1;
                MarkChanged
            }
            Question => {
                self.grid[coords] = Closed;
                MarkChanged
            }
            _ => NoChange,
        })
    }

    fn count_flagged(&self, coords: (usize, usize)) -> usize {
        self.minefield
            .iter_neighbors(coords)
            .filter(|&pos| self.grid[pos] == Tile::Flag)
            .count()
    }

    fn has_question_neighbor(&self, coords: (usize, usize)) -> bool {
        self.minefield
            .iter_neighbors(coords)
            .map(|pos| self.grid[pos])
            .any(|tile| tile == Tile::Question)
    }

    /// Open a closed tile, do not open neighbor tiles
    pub fn open(&mut self, coords: (usize, usize)) -> Result<OpenOutcome> {
        if matches!(self.grid[coords], Tile::Closed) {
            self.open_with_chords(coords)
        } else {
            Ok(OpenOutcome::NoChange)
        }
    }

    pub fn is_chordable(&self, coords: (usize, usize)) -> bool {
        match self.grid[coords] {
            Tile::Open(count)
                if count == self.count_flagged(coords) && !self.has_question_neighbor(coords) =>
            {
                true
            }
            _ => false,
        }
    }

    /// Open a tile, or try to open neighbor tiles
    pub fn open_with_chords(&mut self, coords: (usize, usize)) -> Result<OpenOutcome> {
        use OpenOutcome::*;

        let coords = self.minefield.validate_coords(coords)?;

        self.check_final()?;

        Ok(match self.grid[coords] {
            Tile::Open(count)
                if count == self.count_flagged(coords) && !self.has_question_neighbor(coords) =>
            {
                self.check_in_progress()?;
                // Perform opening of all closed neighbors when flagged count matches
                self.minefield
                    .iter_neighbors(coords)
                    .map(|neighbor_coords| self.open_tile(neighbor_coords))
                    .reduce(BitOr::bitor)
                    .unwrap_or(NoChange)
            }
            _ => self.open_tile(coords),
        })
    }

    /// Helper function to open a single tile and perform flood-fill if necessary
    fn open_tile(&mut self, coords: (usize, usize)) -> OpenOutcome {
        use std::collections::{HashSet, VecDeque};
        use OpenOutcome::*;
        use Tile::*;

        let tile = self.grid[coords];
        let mine = self.minefield[coords];

        match (tile, mine) {
            (Closed, true) => {
                self.grid[coords] = Exploded;
                self.mark_ended(false);
                Explode
            }
            (Closed, false) => {
                let count = self.minefield.get_count(coords);
                self.grid[coords] = Open(count);
                self.open_count += 1;
                log::debug!("Open tile at {:?}, mine count: {}", coords, count);

                if count == 0 {
                    let mut visited = HashSet::from([coords]);
                    let mut to_visit: VecDeque<_> = self
                        .minefield
                        .iter_neighbors(coords)
                        .filter(|&pos| matches!(self.grid[pos], Closed))
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
                        if matches!(self.grid[visit_coords], Open(_) | Flag) {
                            log::trace!("Skipping tile at {:?}", visit_coords);
                            continue;
                        }

                        // open visited tiles
                        let visit_count = self.minefield.get_count(visit_coords);
                        self.grid[visit_coords] = Open(visit_count);
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
                                    .iter_neighbors(visit_coords)
                                    .filter(|&pos| matches!(self.grid[pos], Closed))
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
        use Tile::*;
        let (x_end, y_end) = self.minefield.size();
        for x in 0..x_end {
            for y in 0..y_end {
                let coords = (x, y);
                let tile = self.grid[coords];
                let mine = self.minefield[coords];
                if mine {
                    if tile == Closed || tile == Question {
                        if won {
                            self.grid[coords] = AutoFlag;
                            self.flag_count += 1;
                        } else {
                            self.grid[coords] = Mine;
                        }
                    }
                } else {
                    if tile == Flag {
                        self.grid[coords] = IncorrectFlag;
                    }
                }
            }
        }
    }
}
