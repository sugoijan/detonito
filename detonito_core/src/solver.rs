use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::analysis::{
    AnalysisConfig, CellVarId, ConstraintEquation, ConstraintProblem, ConstraintScope,
    ConstraintVariable, EquationId, build_constraints, reduce_equations_inplace,
};
use crate::analysis::{FlagSemantics, MineCountUsage};
use crate::*;

mod propagation;
mod sat_check;

// Public types

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuessStatus {
    /// At least one forced move or flag was found; the caller should apply them.
    ForcedMovesAvailable,
    /// No forced moves exist; a guess is required.
    GuessRequired,
    /// The board state is contradictory (should not happen on a valid board).
    Contradiction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SolverStats {
    /// Total SAT calls (phase2 + phase3 combined). Kept for backward compatibility.
    pub sat_calls: u64,
    pub elapsed_micros: u128,
    /// Cells forced safe by Phase 1 (constraint propagation).
    #[serde(default)]
    pub phase1_forced_safe: usize,
    /// Cells forced mine by Phase 1.
    #[serde(default)]
    pub phase1_forced_mines: usize,
    /// SAT calls made during Phase 2 (per-component 2-query SAT).
    #[serde(default)]
    pub phase2_sat_calls: u64,
    /// Cells forced safe by Phase 2.
    #[serde(default)]
    pub phase2_forced_safe: usize,
    /// Cells forced mine by Phase 2.
    #[serde(default)]
    pub phase2_forced_mines: usize,
    /// SAT calls made during Phase 3 (global fallback SAT).
    #[serde(default)]
    pub phase3_sat_calls: u64,
    /// Cells forced safe by Phase 3.
    #[serde(default)]
    pub phase3_forced_safe: usize,
    /// Cells forced mine by Phase 3.
    #[serde(default)]
    pub phase3_forced_mines: usize,
    /// Number of connected components evaluated in Phase 2.
    #[serde(default)]
    pub components_checked: usize,
    /// Components for which a cached solver was reused (cache hit).
    #[serde(default)]
    pub components_cache_hits: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolverOutput {
    pub forced_safe: Vec<Coord2>,
    pub forced_mines: Vec<Coord2>,
    pub guess_status: GuessStatus,
    pub stats: SolverStats,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolverConfig {
    pub flag_semantics: FlagSemantics,
    pub mine_count_usage: MineCountUsage,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            flag_semantics: FlagSemantics::Strict,
            mine_count_usage: MineCountUsage::UseIfKnown,
        }
    }
}

#[derive(Error, Clone, Debug, PartialEq, Eq)]
pub enum SolverError {
    #[error("SAT backend error: {0}")]
    Backend(String),
    #[error("Arithmetic overflow in {0}")]
    ArithmeticOverflow(&'static str),
}

// SolveDepth controls how much work solve() does

/// Controls the cost/completeness tradeoff for a `StatefulSolver::solve()` call.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SolveDepth {
    /// Phase 1 (BFS constraint propagation) only. Microseconds; no SAT calls.
    PropagateOnly,
    /// Phases 1 + 2 + 3 - the standard mode. Returns `ForcedMovesAvailable` or
    /// `GuessRequired` based on a single pass. Used for game-loop stepping.
    UntilGuessConclusion,
    /// Exhaustive: runs phases 1 + 2 + 3, then iteratively applies forced-mine
    /// assignments to the constraint equations and re-propagates until no new
    /// cells are found. Returns the maximal set of logically forced cells
    /// reachable without an actual board reveal. Used by TUI peek.
    Exhaustive,
}

// StatefulSolver reuses per-component SAT solvers across steps

/// Cached solver for a single connected component.
struct CachedComponent {
    handle: sat_check::ComponentSolverHandle,
}

/// A solver that caches per-component `BasicSolver` instances across consecutive
/// `solve()` calls on the same game. CDCL learned clauses are preserved, and
/// previously-resolved variables are passed as additional assumptions rather than
/// requiring a full CNF rebuild.
///
/// # Cache invalidation
/// After each call, cache entries are pruned to only those whose equation-ID key
/// still matches a component in the current problem. When a component's equations
/// change structurally (new equation added, or component merges/splits), its key
/// changes and the old solver is naturally evicted via key mismatch.
///
/// Variables that shrink from equations (cells revealed safe) are NOT evicted -
/// instead they are passed as SAT assumptions via `cumulative_assignments`, which
/// keeps the old CNF valid without a rebuild.
pub struct StatefulSolver {
    cfg: SolverConfig,
    /// Key: sorted `Vec<EquationId>` (component identity).
    component_cache: alloc::collections::BTreeMap<Vec<EquationId>, CachedComponent>,
    /// All variable assignments (safe/mine) accumulated across `solve()` calls.
    /// Used as cumulative assumptions when reusing a cached solver so that
    /// previously-resolved variables are correctly handled by the old CNF.
    cumulative_assignments: alloc::collections::BTreeMap<CellVarId, bool>,
    /// Cells proven to be mines in a previous `solve()` call. Injected into the
    /// constraint problem before Phase 1 on the next call to amplify propagation.
    certain_mines: alloc::collections::BTreeSet<CellVarId>,
    /// Cells proven safe in a previous `solve()` call. Injected similarly.
    certain_safe: alloc::collections::BTreeSet<CellVarId>,
}

impl StatefulSolver {
    pub fn new(cfg: SolverConfig) -> Self {
        Self {
            cfg,
            component_cache: alloc::collections::BTreeMap::new(),
            cumulative_assignments: alloc::collections::BTreeMap::new(),
            certain_mines: alloc::collections::BTreeSet::new(),
            certain_safe: alloc::collections::BTreeSet::new(),
        }
    }

    pub fn solve(
        &mut self,
        obs: &Observation,
        depth: SolveDepth,
    ) -> core::result::Result<SolverOutput, SolverError> {
        use alloc::collections::{BTreeMap, BTreeSet};
        use web_time::Instant;

        let start = Instant::now();

        let analysis_cfg = AnalysisConfig {
            flag_semantics: self.cfg.flag_semantics,
            mine_count_usage: self.cfg.mine_count_usage,
            scope: ConstraintScope::FullBoard,
            frontier_seed_clues: Vec::new(),
        };

        let build = build_constraints(obs, analysis_cfg);

        if !build.contradictions.is_empty() {
            return Ok(contradiction_output(start.elapsed().as_micros()));
        }

        // Invalidate certain entries for cells that are no longer hidden (already
        // revealed by a prior step). Flagged cells stay since they are still hidden.
        {
            let hidden_vars: BTreeSet<CellVarId> =
                build.problem.variables.iter().map(|v| v.id).collect();
            self.certain_mines.retain(|id| hidden_vars.contains(id));
            self.certain_safe.retain(|id| hidden_vars.contains(id));
        }

        // Prune cache entries whose equation-ID key no longer matches any current
        // component. When equations are added or components restructure, the key
        // changes and the old solver is evicted naturally here.
        // Variables that merely shrank from equations (revealed cells) are handled
        // via cumulative_assignments assumptions instead.
        self.prune_stale_cache(&build.problem);

        // Apply solver-proven assignments from previous calls to reduce the problem
        // before Phase 1 runs, enabling stronger cascade propagation.
        let initial_problem = if !self.certain_mines.is_empty() || !self.certain_safe.is_empty() {
            build
                .problem
                .apply_prior_assignments(&self.certain_mines, &self.certain_safe)
        } else {
            build.problem
        };

        let board_size = obs.size;

        // Phase 1 only (PropagateOnly depth)
        if matches!(depth, SolveDepth::PropagateOnly) {
            let prop = propagation::run_propagation(&initial_problem);
            if prop.contradiction {
                return Ok(contradiction_output(start.elapsed().as_micros()));
            }

            let p1_safe = prop.forced_safe.len();
            let p1_mines = prop.forced_mines.len();
            let mut forced_safe_ids = prop.forced_safe;
            let mut forced_mines_ids = prop.forced_mines;

            forced_safe_ids.sort_unstable();
            forced_safe_ids.dedup();
            forced_mines_ids.sort_unstable();
            forced_mines_ids.dedup();

            let forced_safe: Vec<Coord2> = forced_safe_ids
                .iter()
                .map(|id| id.to_coords(board_size))
                .collect();
            let forced_mines: Vec<Coord2> = forced_mines_ids
                .iter()
                .map(|id| id.to_coords(board_size))
                .collect();
            let has_moves = !forced_safe.is_empty() || !forced_mines.is_empty();

            return Ok(SolverOutput {
                forced_safe,
                forced_mines,
                guess_status: if has_moves {
                    GuessStatus::ForcedMovesAvailable
                } else {
                    GuessStatus::GuessRequired
                },
                stats: SolverStats {
                    sat_calls: 0,
                    elapsed_micros: start.elapsed().as_micros(),
                    phase1_forced_safe: p1_safe,
                    phase1_forced_mines: p1_mines,
                    ..Default::default()
                },
            });
        }

        // Phase 1: Constraint propagation
        let mut all_safe_ids: Vec<CellVarId> = Vec::new();
        let mut all_mines_ids: Vec<CellVarId> = Vec::new();

        let mut total_p1_safe: usize = 0;
        let mut total_p1_mines: usize = 0;
        let mut total_sat_calls: u64 = 0;
        let mut total_p2_sat_calls: u64 = 0;
        let mut total_p2_forced_safe: usize = 0;
        let mut total_p2_forced_mines: usize = 0;
        let mut total_components_checked: usize = 0;
        let mut total_components_cache_hits: usize = 0;

        let mut current_problem = initial_problem;

        {
            let prop = propagation::run_propagation(&current_problem);
            if prop.contradiction {
                return Ok(contradiction_output(start.elapsed().as_micros()));
            }
            total_p1_safe += prop.forced_safe.len();
            total_p1_mines += prop.forced_mines.len();
            let prop_assignments = prop.assignments;
            all_safe_ids.extend_from_slice(&prop.forced_safe);
            all_mines_ids.extend_from_slice(&prop.forced_mines);

            // Phase 2: 2-query SAT per connected component (with solver cache)
            {
                let var_to_dense = propagation::build_var_to_dense(&current_problem.variables);

                let eq_by_id: BTreeMap<EquationId, &ConstraintEquation> = current_problem
                    .equations
                    .iter()
                    .map(|eq| (eq.id, eq))
                    .collect();

                for component in &current_problem.components {
                    total_components_checked += 1;

                    let mut unresolved_vars: Vec<CellVarId> = Vec::new();
                    let mut phase1_priors: Vec<(CellVarId, bool)> = Vec::new();

                    for &var_id in &component.variable_ids {
                        let raw = var_id.0 as usize;
                        let dense = if raw < var_to_dense.len() {
                            var_to_dense[raw]
                        } else {
                            usize::MAX
                        };
                        match prop_assignments.get(dense).copied().flatten() {
                            Some(value) => phase1_priors.push((var_id, value)),
                            None => unresolved_vars.push(var_id),
                        }
                    }

                    if unresolved_vars.is_empty() {
                        continue;
                    }

                    let component_equations: Vec<&ConstraintEquation> = component
                        .equation_ids
                        .iter()
                        .filter_map(|&eq_id| eq_by_id.get(&eq_id).copied())
                        .collect();

                    let result = if let Some(cached) = self
                        .component_cache
                        .get_mut(component.equation_ids.as_slice())
                    {
                        total_components_cache_hits += 1;

                        let mut all_priors = phase1_priors.clone();
                        let unresolved_set: BTreeSet<CellVarId> =
                            unresolved_vars.iter().copied().collect();
                        let phase1_set: BTreeSet<CellVarId> =
                            phase1_priors.iter().map(|(id, _)| *id).collect();

                        for (&var_id, &is_mine) in &self.cumulative_assignments {
                            if cached.handle.all_var_ids.contains(&var_id)
                                && !unresolved_set.contains(&var_id)
                                && !phase1_set.contains(&var_id)
                            {
                                all_priors.push((var_id, is_mine));
                            }
                        }

                        cached.handle.query_forced(&unresolved_vars, &all_priors)?
                    } else {
                        let mut handle = sat_check::ComponentSolverHandle::new(
                            &component_equations,
                            &component.variable_ids,
                        )?;
                        let result = handle.query_forced(&unresolved_vars, &phase1_priors)?;
                        self.component_cache
                            .insert(component.equation_ids.clone(), CachedComponent { handle });
                        result
                    };

                    total_p2_sat_calls += result.sat_calls;
                    total_p2_forced_safe += result.forced_safe.len();
                    total_p2_forced_mines += result.forced_mines.len();
                    total_sat_calls += result.sat_calls;
                    all_safe_ids.extend_from_slice(&result.forced_safe);
                    all_mines_ids.extend_from_slice(&result.forced_mines);
                }
                // eq_by_id and var_to_dense borrows of current_problem end here.
            }
        }

        // Post-Phase-2 Phase 1 cascade
        //
        // If Phase 2 found mines, those mines may tighten constraints in adjacent
        // components, enabling Phase 1 to propagate further without any extra SAT
        // calls. Reduce the problem once and re-run a single Phase 1 pass.
        if !all_mines_ids.is_empty() || !all_safe_ids.is_empty() {
            let all_mines_set: BTreeSet<CellVarId> = all_mines_ids.iter().copied().collect();
            let all_safe_set: BTreeSet<CellVarId> = all_safe_ids.iter().copied().collect();
            current_problem =
                current_problem.apply_prior_assignments(&all_mines_set, &all_safe_set);

            if !current_problem.variables.is_empty() {
                let cascade = propagation::run_propagation(&current_problem);
                if !cascade.contradiction {
                    total_p1_safe += cascade.forced_safe.len();
                    total_p1_mines += cascade.forced_mines.len();
                    all_safe_ids.extend_from_slice(&cascade.forced_safe);
                    all_mines_ids.extend_from_slice(&cascade.forced_mines);

                    // Reduce for Phase 3 using only the new cascade results - the
                    // original Phase 1+2 cells are already absent from current_problem.
                    if !cascade.forced_safe.is_empty() || !cascade.forced_mines.is_empty() {
                        let cascade_mines: BTreeSet<CellVarId> =
                            cascade.forced_mines.iter().copied().collect();
                        let cascade_safe: BTreeSet<CellVarId> =
                            cascade.forced_safe.iter().copied().collect();
                        current_problem =
                            current_problem.apply_prior_assignments(&cascade_mines, &cascade_safe);
                    }
                }
            }
        }

        // Phase 3: Global fallback SAT (when no forced-safe cells found yet)
        let mut p3_sat_calls: u64 = 0;
        let mut p3_forced_safe: usize = 0;
        let mut p3_forced_mines: usize = 0;

        if all_safe_ids.is_empty()
            && current_problem
                .equations
                .iter()
                .any(|eq| matches!(eq.id, EquationId::GlobalMineCount))
        {
            // After the loop, current_problem has all Phase 1+2 discoveries removed.
            // Its variables are exactly the remaining unknowns; its equations are fully
            // reduced. No explicit prior list needed - the equations encode everything.
            let mut global_unresolved: Vec<CellVarId> =
                current_problem.variables.iter().map(|v| v.id).collect();
            global_unresolved.extend_from_slice(&current_problem.unconstrained_variable_ids);
            global_unresolved.sort_unstable();
            global_unresolved.dedup();

            if !global_unresolved.is_empty() {
                let all_eqs: Vec<&ConstraintEquation> = current_problem.equations.iter().collect();

                // current_problem.equations are already fully reduced (resolved cells
                // removed, targets decremented), so no explicit priors are required.
                let result =
                    sat_check::find_forced_in_component(&all_eqs, &global_unresolved, &[])?;
                p3_sat_calls = result.sat_calls;
                p3_forced_safe = result.forced_safe.len();
                p3_forced_mines = result.forced_mines.len();
                total_sat_calls += result.sat_calls;
                all_safe_ids.extend_from_slice(&result.forced_safe);
                all_mines_ids.extend_from_slice(&result.forced_mines);
            }
        }

        // Update cumulative assignments and certain sets.
        for &id in &all_safe_ids {
            self.cumulative_assignments.insert(id, false);
            self.certain_safe.insert(id);
        }
        for &id in &all_mines_ids {
            self.cumulative_assignments.insert(id, true);
            self.certain_mines.insert(id);
        }

        // Exhaustive loop: re-propagate after applying forced assignments
        if matches!(depth, SolveDepth::Exhaustive)
            && (!all_safe_ids.is_empty() || !all_mines_ids.is_empty())
        {
            // Use the original (pre-loop-reduction) problem as the equation base so
            // that the exhaustive loop works with the full local-clue set. The loop
            // internally reduces incrementally.
            // NOTE: current_problem is fully reduced; we need the original equations
            // for the exhaustive loop. Reconstruct by re-building from obs is expensive;
            // instead pass the accumulated safe/mines to exhaustive_propagation_loop
            // using initial_problem is no longer available. Use current_problem but
            // seed it with the already-found cells - the loop handles that gracefully.
            let (ex_safe, ex_mines) =
                exhaustive_propagation_loop(&current_problem, all_safe_ids, all_mines_ids);
            all_safe_ids = ex_safe;
            all_mines_ids = ex_mines;

            // Update cumulative assignments with exhaustive loop results too.
            for &id in &all_safe_ids {
                self.cumulative_assignments.insert(id, false);
            }
            for &id in &all_mines_ids {
                self.cumulative_assignments.insert(id, true);
            }
        }

        // Deduplicate and convert CellVarId -> Coord2
        all_safe_ids.sort_unstable();
        all_safe_ids.dedup();
        all_mines_ids.sort_unstable();
        all_mines_ids.dedup();

        let forced_safe: Vec<Coord2> = all_safe_ids
            .iter()
            .map(|id| id.to_coords(board_size))
            .collect();
        let forced_mines: Vec<Coord2> = all_mines_ids
            .iter()
            .map(|id| id.to_coords(board_size))
            .collect();

        let has_moves = !forced_safe.is_empty() || !forced_mines.is_empty();
        let guess_status = if has_moves {
            GuessStatus::ForcedMovesAvailable
        } else {
            GuessStatus::GuessRequired
        };

        Ok(SolverOutput {
            forced_safe,
            forced_mines,
            guess_status,
            stats: SolverStats {
                sat_calls: total_sat_calls,
                elapsed_micros: start.elapsed().as_micros(),
                phase1_forced_safe: total_p1_safe,
                phase1_forced_mines: total_p1_mines,
                phase2_sat_calls: total_p2_sat_calls,
                phase2_forced_safe: total_p2_forced_safe,
                phase2_forced_mines: total_p2_forced_mines,
                phase3_sat_calls: p3_sat_calls,
                phase3_forced_safe: p3_forced_safe,
                phase3_forced_mines: p3_forced_mines,
                components_checked: total_components_checked,
                components_cache_hits: total_components_cache_hits,
            },
        })
    }

    /// Remove cache entries that no longer correspond to any component in `problem`.
    /// An entry becomes stale when its equation-ID key is absent from the current
    /// component list (new equations were added, or the component restructured).
    fn prune_stale_cache(&mut self, problem: &ConstraintProblem) {
        use alloc::collections::BTreeSet;
        let valid_keys: BTreeSet<&Vec<EquationId>> =
            problem.components.iter().map(|c| &c.equation_ids).collect();
        self.component_cache
            .retain(|key, _| valid_keys.contains(key));
    }
}

// Clone + Debug for StatefulSolver

/// Cloning discards the component cache and cumulative assignments.
/// Returns a fresh solver with the same config.
impl Clone for StatefulSolver {
    fn clone(&self) -> Self {
        Self::new(self.cfg.clone())
    }
}

impl core::fmt::Debug for StatefulSolver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StatefulSolver")
            .field("cfg", &self.cfg)
            .field("cache_entries", &self.component_cache.len())
            .finish()
    }
}

// Exhaustive propagation loop (used by SolveDepth::Exhaustive)

/// After phases 1+2+3 have found initial forced assignments, iteratively apply
/// those assignments to a copy of the constraint equations and re-run Phase 1
/// propagation until no new cells are found.
///
/// Returns the extended (safe, mines) lists including all exhaustive discoveries.
fn exhaustive_propagation_loop(
    problem: &ConstraintProblem,
    initial_safe: Vec<CellVarId>,
    initial_mines: Vec<CellVarId>,
) -> (Vec<CellVarId>, Vec<CellVarId>) {
    use alloc::collections::BTreeSet;

    // Work on a mutable copy of the local equations (exclude GlobalMineCount
    // since it references unconstrained cells and complicates propagation).
    let mut equations: Vec<ConstraintEquation> = problem
        .equations
        .iter()
        .filter(|eq| !matches!(eq.id, EquationId::GlobalMineCount))
        .cloned()
        .collect();

    let mut all_safe = initial_safe;
    let mut all_mines = initial_mines;

    // Apply the initial forced assignments.
    reduce_equations(&mut equations, &all_safe, &all_mines);

    // Maintain the resolved set incrementally across iterations.
    let mut resolved: BTreeSet<CellVarId> =
        all_safe.iter().chain(all_mines.iter()).copied().collect();

    loop {
        let temp_vars: Vec<ConstraintVariable> = problem
            .variables
            .iter()
            .filter(|v| !resolved.contains(&v.id))
            .cloned()
            .collect();

        if temp_vars.is_empty() {
            break;
        }

        let temp_problem = ConstraintProblem {
            variables: temp_vars,
            equations: equations.clone(),
            components: Vec::new(),
            unconstrained_variable_ids: Vec::new(),
        };

        let prop = propagation::run_propagation(&temp_problem);
        if prop.contradiction || (prop.forced_safe.is_empty() && prop.forced_mines.is_empty()) {
            break;
        }

        resolved.extend(
            prop.forced_safe
                .iter()
                .chain(prop.forced_mines.iter())
                .copied(),
        );
        reduce_equations(&mut equations, &prop.forced_safe, &prop.forced_mines);
        all_safe.extend_from_slice(&prop.forced_safe);
        all_mines.extend_from_slice(&prop.forced_mines);
    }

    (all_safe, all_mines)
}

fn contradiction_output(elapsed_micros: u128) -> SolverOutput {
    SolverOutput {
        forced_safe: Vec::new(),
        forced_mines: Vec::new(),
        guess_status: GuessStatus::Contradiction,
        stats: SolverStats {
            sat_calls: 0,
            elapsed_micros,
            ..Default::default()
        },
    }
}

/// Remove `safe_ids` and `mine_ids` from each equation's variable list,
/// decrementing the target for each mine removed. Equations that become empty
/// are dropped. Delegates to [`reduce_equations_inplace`] after building sets.
fn reduce_equations(
    equations: &mut Vec<ConstraintEquation>,
    safe_ids: &[CellVarId],
    mine_ids: &[CellVarId],
) {
    use alloc::collections::BTreeSet;
    if safe_ids.is_empty() && mine_ids.is_empty() {
        return;
    }
    let safe_set: BTreeSet<CellVarId> = safe_ids.iter().copied().collect();
    let mine_set: BTreeSet<CellVarId> = mine_ids.iter().copied().collect();
    reduce_equations_inplace(equations, &safe_set, &mine_set);
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;

    /// Regression test for safe/mine argument order in `apply_prior_assignments`.
    ///
    /// In this 5x1 scenario, swapping safe/mine prior arguments would incorrectly
    /// force the mine at (3,0) to appear in `forced_safe` on step 2.
    #[test]
    fn stateful_solver_does_not_mark_mine_as_safe_across_steps() {
        let mut solver = StatefulSolver::new(SolverConfig::default());

        // Step 1: only (0,0) revealed, clue=0. Neighbors of (0,0) in 5x1 = {(1,0)}.
        // Phase 1 forces (1,0) safe; no mine identified yet.
        let rev1 = Array2::from_shape_vec([5, 1], vec![Some(0u8), None, None, None, None]).unwrap();
        let flags = Array2::from_elem([5, 1], false);
        let obs1 = Observation::new((5, 1), Some(1), rev1, flags.clone()).unwrap();

        let out1 = solver
            .solve(&obs1, SolveDepth::UntilGuessConclusion)
            .unwrap();
        assert!(
            out1.forced_safe.contains(&(1, 0)),
            "step 1 should force (1,0) safe from the clue=0 equation"
        );

        // Step 2: (2,0) is now also revealed (clue=1), but (1,0) is still hidden.
        // `certain_safe` from step 1 contains (1,0).  The solver must remove (1,0)
        // as a safe prior without decrementing the equation target.
        let rev2 =
            Array2::from_shape_vec([5, 1], vec![Some(0u8), None, Some(1u8), None, None]).unwrap();
        let obs2 = Observation::new((5, 1), Some(1), rev2, flags).unwrap();

        let out2 = solver
            .solve(&obs2, SolveDepth::UntilGuessConclusion)
            .unwrap();

        assert!(
            !out2.forced_safe.contains(&(3, 0)),
            "mine at (3,0) must not appear in forced_safe; swapped safe/mine args in \
             apply_prior_assignments would decrement the equation target for the safe \
             prior (1,0), reducing ((3,0))=1 to ((3,0))=0 and falsely forcing the \
             mine as safe"
        );
        assert!(
            out2.forced_mines.contains(&(3, 0)),
            "mine at (3,0) should be identified as forced_mine"
        );
    }

    #[test]
    fn phase3_deduplicates_unresolved_variables_before_sat_queries() {
        // 2x1 board, no revealed clues, known total mines = 1.
        // This produces only a global equation {x0, x1} = 1 and no local equations,
        // so both variables are unconstrained and phase 3 runs.
        let revealed = Array2::from_shape_vec([2, 1], vec![None, None]).unwrap();
        let flags = Array2::from_elem([2, 1], false);
        let obs = Observation::new((2, 1), Some(1), revealed, flags).unwrap();

        let mut solver = StatefulSolver::new(SolverConfig::default());
        let out = solver
            .solve(&obs, SolveDepth::UntilGuessConclusion)
            .unwrap();

        assert_eq!(out.guess_status, GuessStatus::GuessRequired);
        assert_eq!(
            out.stats.phase3_sat_calls, 4,
            "phase 3 should query each unresolved variable once (2 vars x 2 SAT queries)"
        );
        assert_eq!(out.stats.sat_calls, 4);
    }
}
