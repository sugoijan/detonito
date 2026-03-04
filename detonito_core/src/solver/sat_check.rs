use alloc::string::ToString;
use alloc::vec::Vec;

use rustsat::{
    encodings::card::{BoundBoth, Totalizer},
    instances::{BasicVarManager, Cnf, ManageVars},
    solvers::{Solve, SolveIncremental, SolverResult},
    types::{Lit, Var, constraints::CardConstraint},
};

use crate::analysis::{CellVarId, ConstraintEquation};
use crate::solver::SolverError;

pub struct ComponentForced {
    pub forced_safe: Vec<CellVarId>,
    pub forced_mines: Vec<CellVarId>,
    pub sat_calls: u64,
}

/// A pre-built SAT solver for a connected component, with its CNF already encoded.
/// Keeps CDCL learned clauses across multiple `query_forced` calls, enabling reuse
/// across consecutive `solve()` invocations by `StatefulSolver`.
pub struct ComponentSolverHandle {
    pub(super) solver: rustsat_batsat::BasicSolver,
    /// All variable IDs present in the component when this solver was built, sorted.
    /// Used by `StatefulSolver` to determine which cumulative assignments to pass
    /// as assumptions on reuse.
    pub all_var_ids: Vec<CellVarId>,
}

impl ComponentSolverHandle {
    /// Encode `equations` into a new `BasicSolver` and return the handle.
    ///
    /// `all_var_ids` must include every variable that appears (or may appear) in
    /// the equations — both currently unresolved and already-resolved ones — so
    /// that the Totalizer auxiliary variable range does not overlap cell literals.
    pub fn new(
        equations: &[&ConstraintEquation],
        all_var_ids: &[CellVarId],
    ) -> Result<Self, SolverError> {
        let mut cnf = Cnf::new();
        let mut var_manager = BasicVarManager::default();

        // Reserve the var-ID space for all cell variables so Totalizer auxiliaries
        // are allocated above the highest cell literal.
        let max_var = equations
            .iter()
            .flat_map(|eq| eq.variable_ids.iter())
            .chain(all_var_ids.iter())
            .map(|id| u32::from(id.0))
            .max();
        if let Some(max_var) = max_var {
            var_manager.increase_next_free(Var::new(max_var.saturating_add(1)));
        }

        for equation in equations {
            let lits: Vec<Lit> = equation
                .variable_ids
                .iter()
                .map(|var_id| Lit::positive(u32::from(var_id.0)))
                .collect();

            let constraint = CardConstraint::new_eq(lits, usize::from(equation.target_mines));
            Totalizer::encode_constr(constraint, &mut cnf, &mut var_manager)
                .map_err(|err| SolverError::Backend(err.to_string()))?;
        }

        let mut solver = rustsat_batsat::BasicSolver::default();
        solver
            .add_cnf(cnf)
            .map_err(|err| SolverError::Backend(err.to_string()))?;

        let mut sorted_var_ids: Vec<CellVarId> = all_var_ids.iter().copied().collect();
        sorted_var_ids.sort_unstable();

        Ok(Self {
            solver,
            all_var_ids: sorted_var_ids,
        })
    }

    /// Run 2-query SAT per variable in `var_ids` to find forced assignments.
    ///
    /// `prior_assignments` are added as base assumptions for every query, giving
    /// the solver the full constraint context including Phase 1 results and
    /// any previously resolved variables.
    pub fn query_forced(
        &mut self,
        var_ids: &[CellVarId],
        prior_assignments: &[(CellVarId, bool)],
    ) -> Result<ComponentForced, SolverError> {
        if var_ids.is_empty() {
            return Ok(ComponentForced {
                forced_safe: Vec::new(),
                forced_mines: Vec::new(),
                sat_calls: 0,
            });
        }

        let mut base_assumps: Vec<Lit> = prior_assignments
            .iter()
            .map(|&(var_id, is_mine)| {
                let lit = Lit::positive(u32::from(var_id.0));
                if is_mine { lit } else { !lit }
            })
            .collect();

        let mut forced_safe = Vec::new();
        let mut forced_mines = Vec::new();
        let mut sat_calls: u64 = 0;

        for &var_id in var_ids {
            let lit = Lit::positive(u32::from(var_id.0));
            let base_len = base_assumps.len();

            // Query 1: assume var_id = mine (positive literal). UNSAT → forced safe.
            base_assumps.push(lit);
            let res = self
                .solver
                .solve_assumps(&base_assumps)
                .map_err(|err| SolverError::Backend(err.to_string()))?;
            base_assumps.truncate(base_len);
            sat_calls += 1;
            if matches!(res, SolverResult::Interrupted) {
                return Err(SolverError::Backend(
                    "solver interrupted during forced-safe check".to_string(),
                ));
            }
            if matches!(res, SolverResult::Unsat) {
                forced_safe.push(var_id);
                // Propagate this fact to all subsequent queries in this call.
                base_assumps.push(!lit);
                continue;
            }

            // Query 2: assume var_id = safe (negative literal). UNSAT → forced mine.
            base_assumps.push(!lit);
            let res = self
                .solver
                .solve_assumps(&base_assumps)
                .map_err(|err| SolverError::Backend(err.to_string()))?;
            base_assumps.truncate(base_len);
            sat_calls += 1;
            if matches!(res, SolverResult::Interrupted) {
                return Err(SolverError::Backend(
                    "solver interrupted during forced-mine check".to_string(),
                ));
            }
            if matches!(res, SolverResult::Unsat) {
                forced_mines.push(var_id);
                // Propagate this fact to all subsequent queries in this call.
                base_assumps.push(lit);
            }
        }

        Ok(ComponentForced {
            forced_safe,
            forced_mines,
            sat_calls,
        })
    }
}

/// For a single connected component, run 2 SAT queries per unresolved variable
/// to detect forced-safe and forced-mine assignments.
///
/// Convenience one-shot wrapper: builds a fresh `ComponentSolverHandle` and queries it.
/// Callers that benefit from reuse should use `ComponentSolverHandle` directly.
///
/// `equations`         — the equations that apply to this component (local clues only).
/// `var_ids`           — the unresolved frontier variable IDs to query.
/// `prior_assignments` — Phase 1 assignments for variables already determined;
///                       added as unit clauses so Phase 2 has full context.
pub fn find_forced_in_component(
    equations: &[&ConstraintEquation],
    var_ids: &[CellVarId],
    prior_assignments: &[(CellVarId, bool)],
) -> Result<ComponentForced, SolverError> {
    if var_ids.is_empty() {
        return Ok(ComponentForced {
            forced_safe: Vec::new(),
            forced_mines: Vec::new(),
            sat_calls: 0,
        });
    }

    // Merge all variable IDs so the var_manager is sized correctly.
    let all_var_ids: Vec<CellVarId> = var_ids
        .iter()
        .chain(prior_assignments.iter().map(|(id, _)| id))
        .copied()
        .collect();

    let mut handle = ComponentSolverHandle::new(equations, &all_var_ids)?;
    handle.query_forced(var_ids, prior_assignments)
}
