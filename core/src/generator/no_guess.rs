use super::*;
use ndarray::Array2;
use rand::prelude::*;
use web_time::Instant;

use crate::{
    EngineCell, EngineState, Observation, PlayEngine, RevealOutcome, SolveDepth, SolverConfig,
    StatefulSolver,
};

/// Layout generator that uses the SAT solver to guarantee no-guess solvability.
///
/// Uses DFS with committed mines: each time the solver finds forced mines before
/// hitting a guess point, those mines become fixed constraints for the next DFS
/// level. Backtracks when a level yields no progress after `max_retries_per_level`
/// attempts.
#[derive(Clone, Debug)]
pub struct NoGuessLayoutGenerator {
    seed: u64,
    first_move: Coord2,
    max_retries_per_level: usize,
    max_total_attempts: usize,
}

/// Statistics from a `NoGuessLayoutGenerator::generate_with_stats` call.
#[derive(Clone, Debug, Default)]
pub struct NoGuessGenStats {
    /// Total layouts generated and tested.
    pub attempts: usize,
    /// Number of DFS backtrack operations.
    pub backtracks: usize,
    /// Maximum DFS stack depth reached.
    pub max_depth_reached: usize,
    /// Wall-clock time for the entire generation.
    pub elapsed_micros: u128,
    /// Whether a no-guess layout was found (false = fell back to random).
    pub succeeded: bool,
}

impl NoGuessLayoutGenerator {
    pub fn new(seed: u64, first_move: Coord2) -> Self {
        Self {
            seed,
            first_move,
            max_retries_per_level: 10,
            max_total_attempts: 200,
        }
    }

    pub fn with_retries(self, per_level: usize, total: usize) -> Self {
        Self {
            max_retries_per_level: per_level,
            max_total_attempts: total,
            ..self
        }
    }

    pub fn generate_with_stats(self, config: GameConfig) -> (MineLayout, NoGuessGenStats) {
        let start = Instant::now();
        let mut stats = NoGuessGenStats::default();
        let mut rng = SmallRng::seed_from_u64(self.seed);

        let initial_safe = first_move_zero_zone(config.size, self.first_move);
        let mut stack: Vec<StackFrame> = vec![StackFrame {
            fixed_mines: Vec::new(),
            fixed_safe: initial_safe,
            retries_used: 0,
        }];

        loop {
            if stats.attempts >= self.max_total_attempts || stack.is_empty() {
                let fallback = generate_constrained(config, &[], &[], self.first_move, &mut rng);
                stats.elapsed_micros = start.elapsed().as_micros();
                stats.succeeded = false;
                return (fallback, stats);
            }

            stats.max_depth_reached = stats.max_depth_reached.max(stack.len());

            let (fixed_mines, fixed_safe) = {
                let frame = stack.last().unwrap();
                (frame.fixed_mines.clone(), frame.fixed_safe.clone())
            };

            let layout =
                generate_constrained(config, &fixed_mines, &fixed_safe, self.first_move, &mut rng);
            stats.attempts += 1;

            match simulate_to_guess(&layout, self.first_move) {
                SimResult::Won => {
                    stats.elapsed_micros = start.elapsed().as_micros();
                    stats.succeeded = true;
                    return (layout, stats);
                }
                SimResult::HitMine => {
                    let frame = stack.last_mut().unwrap();
                    frame.retries_used += 1;
                    if frame.retries_used >= self.max_retries_per_level {
                        stack.pop();
                        stats.backtracks += 1;
                    }
                }
                SimResult::GuessRequired {
                    accumulated_mines,
                    revealed_cells,
                } => {
                    let current_fixed_count = stack.last().unwrap().fixed_mines.len();
                    if accumulated_mines.len() > current_fixed_count {
                        stack.push(StackFrame {
                            fixed_mines: accumulated_mines,
                            fixed_safe: revealed_cells,
                            retries_used: 0,
                        });
                    } else {
                        let frame = stack.last_mut().unwrap();
                        frame.retries_used += 1;
                        if frame.retries_used >= self.max_retries_per_level {
                            stack.pop();
                            stats.backtracks += 1;
                        }
                    }
                }
            }
        }
    }
}

impl LayoutGenerator for NoGuessLayoutGenerator {
    fn generate(self, config: GameConfig) -> MineLayout {
        self.generate_with_stats(config).0
    }
}

// Private types

struct StackFrame {
    fixed_mines: Vec<Coord2>,
    fixed_safe: Vec<Coord2>,
    retries_used: usize,
}

enum SimResult {
    HitMine,
    Won,
    GuessRequired {
        accumulated_mines: Vec<Coord2>,
        revealed_cells: Vec<Coord2>,
    },
}

// Private helpers

/// Returns first_move and all in-bounds neighbors (FirstMoveZero zone).
fn first_move_zero_zone(size: Coord2, first_move: Coord2) -> Vec<Coord2> {
    let (w, h) = size;
    let (fx, fy) = first_move;
    let mut zone = vec![first_move];
    for dx in -1i32..=1 {
        for dy in -1i32..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let nx = fx as i32 + dx;
            let ny = fy as i32 + dy;
            if nx >= 0 && nx < w as i32 && ny >= 0 && ny < h as i32 {
                zone.push((nx as u8, ny as u8));
            }
        }
    }
    zone
}

/// Generate a random layout respecting fixed constraints.
///
/// - `fixed_mines`: cells that must be mines.
/// - `fixed_safe`: cells that must not be mines (revealed cells).
/// - first-move zero zone is also reserved (never a mine).
fn generate_constrained(
    config: GameConfig,
    fixed_mines: &[Coord2],
    fixed_safe: &[Coord2],
    first_move: Coord2,
    rng: &mut impl Rng,
) -> MineLayout {
    let total_cells = config.total_cells();

    let mut mine_mask: Array2<bool> = Array2::default(config.size.to_nd_index());

    // Mark fixed_mines as reserved (they will remain as mines).
    for &coord in fixed_mines {
        mine_mask[coord.to_nd_index()] = true;
    }
    let fixed_mines_count = fixed_mines.len() as CellCount;

    // Mark fixed_safe as reserved (not mines; cleared after placement).
    for &coord in fixed_safe {
        mine_mask[coord.to_nd_index()] = true;
    }

    // Mark first-move zero zone as reserved (cleared after placement).
    let zone = first_move_zero_zone(config.size, first_move);
    for &coord in &zone {
        mine_mask[coord.to_nd_index()] = true;
    }

    // Count non-reserved cells available for random placement.
    let reserved_count = mine_mask.iter().filter(|&&b| b).count() as CellCount;
    let available_cells = total_cells.saturating_sub(reserved_count);

    let remaining_mines = config
        .mines
        .saturating_sub(fixed_mines_count)
        .min(available_cells);

    // Fisher-Yates-style placement: skip reserved cells.
    let mut placed: CellCount = 0;
    let mut available = available_cells;
    {
        let cells = mine_mask.as_slice_mut().expect("layout should be standard");
        while placed < remaining_mines {
            if available == 0 {
                break;
            }
            let mut place: CellCount = rng.random_range(0..available);
            for (i, is_reserved) in cells.iter_mut().enumerate() {
                let i = i as CellCount;
                if *is_reserved {
                    place += 1;
                }
                if i == place {
                    *is_reserved = true;
                    placed += 1;
                    available -= 1;
                    break;
                }
            }
        }
    }

    // Clear reserved-non-mine cells (fixed_safe + zone).
    for &coord in fixed_safe {
        mine_mask[coord.to_nd_index()] = false;
    }
    for &coord in &zone {
        mine_mask[coord.to_nd_index()] = false;
    }

    // Re-apply fixed_mines (in case any were in the zone/safe sets).
    for &coord in fixed_mines {
        mine_mask[coord.to_nd_index()] = true;
    }

    let mine_count = mine_mask.iter().filter(|&&b| b).count() as CellCount;

    MineLayout {
        mine_mask,
        mine_count,
    }
}

/// Simulate a game with the SAT solver until Won or GuessRequired.
fn simulate_to_guess(layout: &MineLayout, first_move: Coord2) -> SimResult {
    let cfg = SolverConfig::default();
    let mut engine = PlayEngine::new(layout.clone());
    let mine_count = layout.mine_count();

    let outcome = engine.reveal(first_move).expect("reveal should not fail");
    if matches!(outcome, RevealOutcome::HitMine) {
        return SimResult::HitMine;
    }
    if matches!(outcome, RevealOutcome::Won) {
        return SimResult::Won;
    }

    let step_budget = (layout.total_cells() as usize).saturating_mul(8);
    let mut solver = StatefulSolver::new(cfg);

    for _ in 0..step_budget {
        match engine.state() {
            EngineState::Won => return SimResult::Won,
            EngineState::Lost => return SimResult::HitMine,
            EngineState::Ready | EngineState::Active => {}
        }

        let obs = Observation::from_engine_with_mine_count(&engine, Some(mine_count));
        let out = solver
            .solve(&obs, SolveDepth::UntilGuessConclusion)
            .expect("solver should not fail on a valid board");

        if out.forced_safe.is_empty() && out.forced_mines.is_empty() {
            return collect_guess_required(&engine);
        }

        let mut progressed = false;

        for &coords in &out.forced_mines {
            if matches!(engine.cell_at(coords), EngineCell::Hidden) {
                engine.toggle_flag(coords).expect("flag should succeed");
                progressed = true;
            }
        }

        for &coords in &out.forced_safe {
            match engine.cell_at(coords) {
                EngineCell::Hidden => {
                    let result = engine.reveal(coords).expect("reveal should succeed");
                    if matches!(result, RevealOutcome::HitMine) {
                        return SimResult::HitMine;
                    }
                    progressed = true;
                }
                _ => {}
            }
        }

        if !progressed {
            return collect_guess_required(&engine);
        }
    }

    collect_guess_required(&engine)
}

fn collect_guess_required(engine: &PlayEngine) -> SimResult {
    let (w, h) = engine.size();
    let mut accumulated_mines = Vec::new();
    let mut revealed_cells = Vec::new();

    for x in 0..w {
        for y in 0..h {
            let coord = (x, y);
            match engine.cell_at(coord) {
                EngineCell::Flagged => accumulated_mines.push(coord),
                EngineCell::Revealed(_) => revealed_cells.push(coord),
                EngineCell::Hidden => {}
            }
        }
    }

    SimResult::GuessRequired {
        accumulated_mines,
        revealed_cells,
    }
}
