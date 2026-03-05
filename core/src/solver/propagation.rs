use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use smallvec::SmallVec;

use crate::analysis::{CellVarId, ConstraintProblem, ConstraintVariable};

/// Runtime state for a single equation during propagation.
#[derive(Copy, Clone, Debug)]
pub struct EquationRuntimeState {
    /// Remaining mines needed (target minus already-assigned mines).
    pub target: i32,
    /// Number of variables in this equation already assigned as mines.
    pub assigned_mines: i32,
    /// Number of variables in this equation not yet assigned.
    pub unassigned: i32,
}

impl EquationRuntimeState {
    pub fn is_feasible(self) -> bool {
        self.assigned_mines <= self.target && self.assigned_mines + self.unassigned >= self.target
    }
}

/// Output from Phase 1 propagation.
pub struct PropagationResult {
    pub forced_safe: Vec<CellVarId>,
    pub forced_mines: Vec<CellVarId>,
    pub contradiction: bool,
    /// `assignments[i]` is the assigned value for `problem.variables[i]`,
    /// None if unresolved.
    pub assignments: Vec<Option<bool>>,
}

/// Build a flat lookup table mapping `CellVarId.0` → dense index into `variables`
/// (or `usize::MAX` if the variable is absent). O(1) per lookup.
pub fn build_var_to_dense(variables: &[ConstraintVariable]) -> Vec<usize> {
    let max_id = variables.iter().map(|v| v.id.0 as usize).max().unwrap_or(0);
    let mut var_to_dense = alloc::vec![usize::MAX; max_id + 1];
    for (i, var) in variables.iter().enumerate() {
        var_to_dense[var.id.0 as usize] = i;
    }
    var_to_dense
}

/// Build auxiliary indices:
/// - `var_to_equations[dense_idx]` → equation indices the variable participates in.
/// - `var_to_dense[var_id.0]` → dense index (or `usize::MAX` if absent).
///
/// Uses a flat Vec indexed by `CellVarId.0` for O(1) lookups, consistent with
/// the approach in `build_components`.
fn build_var_to_equations(problem: &ConstraintProblem) -> (Vec<SmallVec<[usize; 6]>>, Vec<usize>) {
    let var_to_dense = build_var_to_dense(&problem.variables);

    let mut var_to_equations: Vec<SmallVec<[usize; 6]>> =
        vec![SmallVec::new(); problem.variables.len()];

    for (eq_idx, equation) in problem.equations.iter().enumerate() {
        for &var_id in &equation.variable_ids {
            let raw = var_id.0 as usize;
            if raw < var_to_dense.len() {
                let dense = var_to_dense[raw];
                if dense != usize::MAX {
                    var_to_equations[dense].push(eq_idx);
                }
            }
        }
    }

    (var_to_equations, var_to_dense)
}

/// Run BFS constraint propagation to fixpoint.
/// Returns forced assignments and whether a contradiction was found.
pub fn run_propagation(problem: &ConstraintProblem) -> PropagationResult {
    let (var_to_equations, var_to_dense) = build_var_to_equations(problem);
    let n_vars = problem.variables.len();
    let n_eqs = problem.equations.len();

    let mut assignments: Vec<Option<bool>> = vec![None; n_vars];
    let mut equation_states: Vec<EquationRuntimeState> = problem
        .equations
        .iter()
        .map(|eq| EquationRuntimeState {
            target: i32::from(eq.target_mines),
            assigned_mines: 0,
            unassigned: eq.variable_ids.len() as i32,
        })
        .collect();

    let mut forced_safe: Vec<CellVarId> = Vec::new();
    let mut forced_mines: Vec<CellVarId> = Vec::new();

    // BFS queue of equation indices to re-examine.
    let mut in_queue = vec![true; n_eqs];
    let mut queue: VecDeque<usize> = (0..n_eqs).collect();

    let mut contradiction = false;

    'outer: while let Some(eq_idx) = queue.pop_front() {
        in_queue[eq_idx] = false;

        let state = equation_states[eq_idx];
        if !state.is_feasible() {
            contradiction = true;
            break 'outer;
        }

        if state.unassigned == 0 {
            continue;
        }

        let remaining = state.target - state.assigned_mines;
        let assign_all_mines = remaining == state.unassigned;
        let assign_all_safe = remaining == 0;

        if !assign_all_mines && !assign_all_safe {
            continue;
        }

        let assign_value = assign_all_mines;

        for &var_id in &problem.equations[eq_idx].variable_ids {
            let raw = var_id.0 as usize;
            let dense = if raw < var_to_dense.len() {
                var_to_dense[raw]
            } else {
                usize::MAX
            };
            if dense == usize::MAX {
                continue;
            }

            if assignments[dense].is_some() {
                continue;
            }

            assignments[dense] = Some(assign_value);
            if assign_value {
                forced_mines.push(var_id);
            } else {
                forced_safe.push(var_id);
            }

            // Update all equations that include this variable.
            for &eq_idx2 in &var_to_equations[dense] {
                let st = &mut equation_states[eq_idx2];
                st.unassigned -= 1;
                if assign_value {
                    st.assigned_mines += 1;
                }

                if !st.is_feasible() {
                    contradiction = true;
                    break 'outer;
                }

                if !in_queue[eq_idx2] {
                    in_queue[eq_idx2] = true;
                    queue.push_back(eq_idx2);
                }
            }
        }
    }

    PropagationResult {
        forced_safe,
        forced_mines,
        contradiction,
        assignments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    #[test]
    fn propagation_forces_safe_when_target_zero() {
        // 1x3 board: revealed (clue=0) at x=1; hidden at x=0 and x=2.
        // Clue 0 at (1,0) says 0 mines among neighbors → both (0,0) and (2,0) are safe.
        use ndarray::Array2;
        let revealed = Array2::from_shape_vec([3, 1], vec![None, Some(0), None]).unwrap();
        let flags = Array2::from_elem([3, 1], false);
        let obs = Observation::new((3, 1), None, revealed, flags).unwrap();
        let cfg = AnalysisConfig::default();
        let build = build_constraints(&obs, cfg);

        let result = run_propagation(&build.problem);
        assert!(!result.contradiction);
        assert_eq!(result.forced_mines.len(), 0);
        assert_eq!(result.forced_safe.len(), 2);
    }

    #[test]
    fn propagation_forces_mines_when_target_equals_count() {
        // 1x3 board: revealed (clue=2) at x=1; hidden at x=0 and x=2.
        // Clue 2 → both (0,0) and (2,0) are mines.
        use ndarray::Array2;
        let revealed = Array2::from_shape_vec([3, 1], vec![None, Some(2), None]).unwrap();
        let flags = Array2::from_elem([3, 1], false);
        let obs = Observation::new((3, 1), None, revealed, flags).unwrap();
        let cfg = AnalysisConfig::default();
        let build = build_constraints(&obs, cfg);

        let result = run_propagation(&build.problem);
        assert!(!result.contradiction);
        assert_eq!(result.forced_safe.len(), 0);
        assert_eq!(result.forced_mines.len(), 2);
    }
}
