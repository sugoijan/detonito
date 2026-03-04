use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec;
use alloc::vec::Vec;

use smallvec::SmallVec;

use ndarray::Array2;
use serde::{Deserialize, Serialize};

use super::Observation;
use crate::*;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FlagSemantics {
    #[default]
    Soft,
    Strict,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MineCountUsage {
    #[default]
    UseIfKnown,
    Ignore,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ConstraintScope {
    #[default]
    FullBoard,
    FrontierMaximal,
    FrontierSeeded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AnalysisConfig {
    pub flag_semantics: FlagSemantics,
    pub mine_count_usage: MineCountUsage,
    pub scope: ConstraintScope,
    pub frontier_seed_clues: Vec<Coord2>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CellVarId(pub u16);

impl CellVarId {
    pub fn from_coords(size: Coord2, coords: Coord2) -> Self {
        let width = u16::from(size.0);
        let x = u16::from(coords.0);
        let y = u16::from(coords.1);
        Self(y.saturating_mul(width).saturating_add(x))
    }

    pub fn to_coords(self, size: Coord2) -> Coord2 {
        let width = u16::from(size.0);
        let x = self.0 % width;
        let y = self.0 / width;
        (
            Coord::try_from(x).expect("x coordinate should fit in Coord"),
            Coord::try_from(y).expect("y coordinate should fit in Coord"),
        )
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EquationId {
    LocalClue(Coord2),
    GlobalMineCount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintVariable {
    pub id: CellVarId,
    pub coords: Coord2,
    pub flagged: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintEquation {
    pub id: EquationId,
    pub variable_ids: SmallVec<[CellVarId; 8]>,
    pub target_mines: CellCount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintComponent {
    pub variable_ids: Vec<CellVarId>,
    pub equation_ids: SmallVec<[EquationId; 8]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintProblem {
    pub variables: Vec<ConstraintVariable>,
    pub equations: Vec<ConstraintEquation>,
    pub components: Vec<ConstraintComponent>,
    pub unconstrained_variable_ids: Vec<CellVarId>,
}

impl ConstraintProblem {
    pub fn variable_by_id(&self, id: CellVarId) -> Option<&ConstraintVariable> {
        self.variables.iter().find(|var| var.id == id)
    }

    pub fn equation_by_id(&self, id: EquationId) -> Option<&ConstraintEquation> {
        self.equations.iter().find(|eq| eq.id == id)
    }

    /// Remove `prior_mines` and `prior_safe` from the problem, decrement equation
    /// targets for mines, drop now-empty equations, and rebuild connected components.
    ///
    /// Both slices must be sorted. Duplicates are tolerated (the retain is idempotent).
    ///
    /// Used by the Phase 1+2 fixpoint loop and cross-step certain-cell injection.
    pub(crate) fn apply_prior_assignments(
        mut self,
        prior_mines: &[CellVarId],
        prior_safe: &[CellVarId],
    ) -> Self {
        if prior_mines.is_empty() && prior_safe.is_empty() {
            return self;
        }

        self.variables.retain(|v| {
            prior_mines.binary_search(&v.id).is_err()
                && prior_safe.binary_search(&v.id).is_err()
        });

        reduce_equations_inplace(&mut self.equations, prior_safe, prior_mines);

        let (components, unconstrained) = build_components(&self.variables, &self.equations);
        self.components = components;
        self.unconstrained_variable_ids = unconstrained;
        self
    }
}

/// Remove variables in `safe` and `mines` from each equation's variable list,
/// decrement the target for each mine removed, and drop now-empty equations.
///
/// Both slices must be sorted. Duplicates are tolerated.
///
/// Shared by [`ConstraintProblem::apply_prior_assignments`] and the solver's
/// exhaustive propagation loop.
pub(crate) fn reduce_equations_inplace(
    equations: &mut Vec<ConstraintEquation>,
    safe: &[CellVarId],
    mines: &[CellVarId],
) {
    if safe.is_empty() && mines.is_empty() {
        return;
    }
    equations.retain_mut(|eq| {
        let mines_removed = eq
            .variable_ids
            .iter()
            .filter(|id| mines.binary_search(id).is_ok())
            .count() as u16;
        eq.variable_ids
            .retain(|id| mines.binary_search(id).is_err() && safe.binary_search(id).is_err());
        eq.target_mines = eq.target_mines.saturating_sub(mines_removed);
        !eq.variable_ids.is_empty()
    });
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Contradiction {
    InvalidObservationShape,
    InvalidMineCount {
        mine_count: CellCount,
        max_cells: CellCount,
    },
    LocalClueImpossible {
        clue: Coord2,
        target_mines: i16,
        available_variables: usize,
    },
    GlobalMineCountImpossible {
        target_mines: i32,
        available_variables: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintStats {
    pub variable_count: usize,
    pub local_equation_count: usize,
    pub global_equation_count: usize,
    pub component_count: usize,
    pub max_component_variables: usize,
    pub contradiction_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintBuildOutput {
    pub problem: ConstraintProblem,
    pub contradictions: Vec<Contradiction>,
    pub stats: ConstraintStats,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationDelta {
    pub size: Coord2,
    pub changed_revealed: Vec<(Coord2, Option<u8>, Option<u8>)>,
    pub changed_flags: Vec<(Coord2, bool, bool)>,
    pub mine_count_before: Option<CellCount>,
    pub mine_count_after: Option<CellCount>,
}

impl ObservationDelta {
    pub fn between(prev: &Observation, next: &Observation) -> Result<Self> {
        if prev.size != next.size {
            return Err(GameError::InvalidBoardShape);
        }

        let expected = prev.size.to_nd_index();
        if prev.revealed.dim() != (expected[0], expected[1])
            || prev.flags.dim() != (expected[0], expected[1])
            || next.revealed.dim() != (expected[0], expected[1])
            || next.flags.dim() != (expected[0], expected[1])
        {
            return Err(GameError::InvalidBoardShape);
        }

        let mut changed_revealed = Vec::new();
        let mut changed_flags = Vec::new();

        let (x_end, y_end) = prev.size;
        for x in 0..x_end {
            for y in 0..y_end {
                let coords = (x, y);
                let prev_revealed = prev.revealed[coords.to_nd_index()];
                let next_revealed = next.revealed[coords.to_nd_index()];
                if prev_revealed != next_revealed {
                    changed_revealed.push((coords, prev_revealed, next_revealed));
                }

                let prev_flag = prev.flags[coords.to_nd_index()];
                let next_flag = next.flags[coords.to_nd_index()];
                if prev_flag != next_flag {
                    changed_flags.push((coords, prev_flag, next_flag));
                }
            }
        }

        Ok(Self {
            size: prev.size,
            changed_revealed,
            changed_flags,
            mine_count_before: prev.mine_count,
            mine_count_after: next.mine_count,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.changed_revealed.is_empty()
            && self.changed_flags.is_empty()
            && self.mine_count_before == self.mine_count_after
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintDelta {
    pub added_variable_ids: Vec<CellVarId>,
    pub removed_variable_ids: Vec<CellVarId>,
    pub added_equation_ids: Vec<EquationId>,
    pub removed_equation_ids: Vec<EquationId>,
    pub updated_equation_ids: Vec<EquationId>,
    pub affected_variable_ids: Vec<CellVarId>,
}

impl ConstraintDelta {
    pub fn from_problems(before: &ConstraintProblem, after: &ConstraintProblem) -> Self {
        let before_vars: BTreeSet<_> = before.variables.iter().map(|var| var.id).collect();
        let after_vars: BTreeSet<_> = after.variables.iter().map(|var| var.id).collect();

        let mut added_variable_ids: Vec<_> = after_vars.difference(&before_vars).copied().collect();
        let mut removed_variable_ids: Vec<_> =
            before_vars.difference(&after_vars).copied().collect();

        let before_eqs: BTreeMap<_, _> = before.equations.iter().map(|eq| (eq.id, eq)).collect();
        let after_eqs: BTreeMap<_, _> = after.equations.iter().map(|eq| (eq.id, eq)).collect();

        let before_eq_ids: BTreeSet<_> = before_eqs.keys().copied().collect();
        let after_eq_ids: BTreeSet<_> = after_eqs.keys().copied().collect();

        let mut added_equation_ids: Vec<_> =
            after_eq_ids.difference(&before_eq_ids).copied().collect();
        let mut removed_equation_ids: Vec<_> =
            before_eq_ids.difference(&after_eq_ids).copied().collect();

        let mut updated_equation_ids = Vec::new();
        for eq_id in before_eq_ids.intersection(&after_eq_ids) {
            let before_eq = before_eqs.get(eq_id).expect("equation must exist");
            let after_eq = after_eqs.get(eq_id).expect("equation must exist");
            if before_eq.variable_ids != after_eq.variable_ids
                || before_eq.target_mines != after_eq.target_mines
            {
                updated_equation_ids.push(*eq_id);
            }
        }

        added_variable_ids.sort_unstable();
        removed_variable_ids.sort_unstable();
        added_equation_ids.sort_unstable();
        removed_equation_ids.sort_unstable();
        updated_equation_ids.sort_unstable();

        let mut affected_variables = BTreeSet::new();
        affected_variables.extend(added_variable_ids.iter().copied());
        affected_variables.extend(removed_variable_ids.iter().copied());

        for eq_id in added_equation_ids
            .iter()
            .chain(removed_equation_ids.iter())
            .chain(updated_equation_ids.iter())
        {
            if let Some(eq) = after_eqs
                .get(eq_id)
                .copied()
                .or_else(|| before_eqs.get(eq_id).copied())
            {
                for &var_id in &eq.variable_ids {
                    affected_variables.insert(var_id);
                }
            }
        }

        Self {
            added_variable_ids,
            removed_variable_ids,
            added_equation_ids,
            removed_equation_ids,
            updated_equation_ids,
            affected_variable_ids: affected_variables.into_iter().collect(),
        }
    }

    pub fn from_observation_delta(
        prev: &Observation,
        next: &Observation,
        cfg: AnalysisConfig,
    ) -> Result<Self> {
        if prev.size != next.size {
            return Err(GameError::InvalidBoardShape);
        }
        let before = build_constraints(prev, cfg.clone());
        let after = build_constraints(next, cfg);
        Ok(Self::from_problems(&before.problem, &after.problem))
    }
}

pub fn build_constraints(obs: &Observation, cfg: AnalysisConfig) -> ConstraintBuildOutput {
    let mut contradictions = Vec::new();

    let expected = (obs.size.0 as usize, obs.size.1 as usize);
    if obs.revealed.dim() != expected || obs.flags.dim() != expected {
        contradictions.push(Contradiction::InvalidObservationShape);
        return empty_output(contradictions);
    }

    if let Some(mine_count) = obs.mine_count {
        let max_cells = mult(obs.size.0, obs.size.1);
        if mine_count > max_cells {
            contradictions.push(Contradiction::InvalidMineCount {
                mine_count,
                max_cells,
            });
            return empty_output(contradictions);
        }
    }

    let mut variables = Vec::new();
    let mut variable_ids: Array2<Option<CellVarId>> =
        Array2::from_elem(obs.size.to_nd_index(), None);

    let (x_end, y_end) = obs.size;
    for x in 0..x_end {
        for y in 0..y_end {
            let coords = (x, y);
            if obs.revealed[coords.to_nd_index()].is_none() {
                let id = CellVarId::from_coords(obs.size, coords);
                variables.push(ConstraintVariable {
                    id,
                    coords,
                    flagged: obs.flags[coords.to_nd_index()],
                });
                variable_ids[coords.to_nd_index()] = Some(id);
            }
        }
    }

    let included_clues = select_local_clues(obs, &cfg, &variable_ids);

    let mut equations = Vec::new();
    let mut local_equation_count = 0;

    for clue in included_clues {
        let clue_mines =
            obs.revealed[clue.to_nd_index()].expect("included_clues contains only revealed cells");

        let mut target_mines = i16::from(clue_mines);
        let mut equation_vars: SmallVec<[CellVarId; 8]> = SmallVec::new();

        for neighbor in obs.revealed.iter_neighbors(clue) {
            if obs.revealed[neighbor.to_nd_index()].is_some() {
                continue;
            }

            let var_id = variable_ids[neighbor.to_nd_index()]
                .expect("variable id should exist for hidden cells");
            let flagged = obs.flags[neighbor.to_nd_index()];

            if matches!(cfg.flag_semantics, FlagSemantics::Strict) && flagged {
                target_mines -= 1;
            } else {
                equation_vars.push(var_id);
            }
        }

        equation_vars.sort_unstable();
        equation_vars.dedup();

        if target_mines < 0 || (target_mines as usize) > equation_vars.len() {
            contradictions.push(Contradiction::LocalClueImpossible {
                clue,
                target_mines,
                available_variables: equation_vars.len(),
            });
            continue;
        }

        equations.push(ConstraintEquation {
            id: EquationId::LocalClue(clue),
            variable_ids: equation_vars,
            target_mines: target_mines as CellCount,
        });
        local_equation_count += 1;
    }

    let mut global_equation_count = 0;

    if matches!(cfg.mine_count_usage, MineCountUsage::UseIfKnown)
        && let Some(total_mines) = obs.mine_count
    {
        let mut target_mines = i32::from(total_mines);
        let mut equation_vars: SmallVec<[CellVarId; 8]> = SmallVec::new();

        for var in &variables {
            if matches!(cfg.flag_semantics, FlagSemantics::Strict) && var.flagged {
                target_mines -= 1;
            } else {
                equation_vars.push(var.id);
            }
        }

        equation_vars.sort_unstable();
        equation_vars.dedup();

        if target_mines < 0 || (target_mines as usize) > equation_vars.len() {
            contradictions.push(Contradiction::GlobalMineCountImpossible {
                target_mines,
                available_variables: equation_vars.len(),
            });
        } else {
            equations.push(ConstraintEquation {
                id: EquationId::GlobalMineCount,
                variable_ids: equation_vars,
                target_mines: target_mines as CellCount,
            });
            global_equation_count = 1;
        }
    }

    let (components, unconstrained_variable_ids) = build_components(&variables, &equations);

    let max_component_variables = components
        .iter()
        .map(|component| component.variable_ids.len())
        .max()
        .unwrap_or(0);

    let problem = ConstraintProblem {
        variables,
        equations,
        components,
        unconstrained_variable_ids,
    };

    let stats = ConstraintStats {
        variable_count: problem.variables.len(),
        local_equation_count,
        global_equation_count,
        component_count: problem.components.len(),
        max_component_variables,
        contradiction_count: contradictions.len(),
    };

    ConstraintBuildOutput {
        problem,
        contradictions,
        stats,
    }
}

fn select_local_clues(
    obs: &Observation,
    cfg: &AnalysisConfig,
    variable_ids: &Array2<Option<CellVarId>>,
) -> BTreeSet<Coord2> {
    let mut frontier_clues = BTreeSet::new();
    let (x_end, y_end) = obs.size;

    for x in 0..x_end {
        for y in 0..y_end {
            let clue = (x, y);
            if obs.revealed[clue.to_nd_index()].is_none() {
                continue;
            }

            match cfg.scope {
                ConstraintScope::FullBoard => {
                    frontier_clues.insert(clue);
                }
                ConstraintScope::FrontierMaximal | ConstraintScope::FrontierSeeded => {
                    let has_hidden_neighbor = obs
                        .revealed
                        .iter_neighbors(clue)
                        .any(|neighbor| variable_ids[neighbor.to_nd_index()].is_some());
                    if has_hidden_neighbor {
                        frontier_clues.insert(clue);
                    }
                }
            }
        }
    }

    if !matches!(cfg.scope, ConstraintScope::FrontierSeeded) || cfg.frontier_seed_clues.is_empty() {
        return frontier_clues;
    }

    let mut valid_seeds = BTreeSet::new();
    for &seed in &cfg.frontier_seed_clues {
        if frontier_clues.contains(&seed) {
            valid_seeds.insert(seed);
        }
    }

    if valid_seeds.is_empty() {
        return frontier_clues;
    }

    let mut clue_to_vars: BTreeMap<Coord2, Vec<CellVarId>> = BTreeMap::new();
    let mut var_to_clues: BTreeMap<CellVarId, Vec<Coord2>> = BTreeMap::new();

    for &clue in &frontier_clues {
        let mut vars = Vec::new();
        for neighbor in obs.revealed.iter_neighbors(clue) {
            if let Some(var_id) = variable_ids[neighbor.to_nd_index()] {
                vars.push(var_id);
                var_to_clues.entry(var_id).or_default().push(clue);
            }
        }
        vars.sort_unstable();
        vars.dedup();
        clue_to_vars.insert(clue, vars);
    }

    let mut included = BTreeSet::new();
    let mut queue: VecDeque<Coord2> = valid_seeds.into_iter().collect();

    while let Some(clue) = queue.pop_front() {
        if !included.insert(clue) {
            continue;
        }

        if let Some(vars) = clue_to_vars.get(&clue) {
            for &var_id in vars {
                if let Some(next_clues) = var_to_clues.get(&var_id) {
                    for &next_clue in next_clues {
                        if !included.contains(&next_clue) {
                            queue.push_back(next_clue);
                        }
                    }
                }
            }
        }
    }

    included
}

fn empty_output(contradictions: Vec<Contradiction>) -> ConstraintBuildOutput {
    let contradiction_count = contradictions.len();
    ConstraintBuildOutput {
        problem: ConstraintProblem {
            variables: Vec::new(),
            equations: Vec::new(),
            components: Vec::new(),
            unconstrained_variable_ids: Vec::new(),
        },
        contradictions,
        stats: ConstraintStats {
            variable_count: 0,
            local_equation_count: 0,
            global_equation_count: 0,
            component_count: 0,
            max_component_variables: 0,
            contradiction_count,
        },
    }
}

pub(crate) fn build_components(
    variables: &[ConstraintVariable],
    equations: &[ConstraintEquation],
) -> (Vec<ConstraintComponent>, Vec<CellVarId>) {
    let mut touched_vars: Vec<CellVarId> = Vec::new();
    for equation in equations {
        if !matches!(equation.id, EquationId::LocalClue(_)) {
            continue;
        }
        touched_vars.extend(equation.variable_ids.iter().copied());
    }
    touched_vars.sort_unstable();
    touched_vars.dedup();

    let touched_list = &touched_vars;
    // Build O(1)-lookup dense index using CellVarId as a direct array offset.
    // CellVarId values are bounded by board size (≤600 for the largest supported board).
    let dense_len = touched_list.last().map_or(0, |id| id.0 as usize + 1);
    let mut dense_index = alloc::vec![usize::MAX; dense_len];
    for (idx, var_id) in touched_list.iter().copied().enumerate() {
        dense_index[var_id.0 as usize] = idx;
    }

    let mut dsu = Dsu::new(touched_list.len());

    for equation in equations {
        if !matches!(equation.id, EquationId::LocalClue(_)) {
            continue;
        }

        if let Some((&first, rest)) = equation.variable_ids.split_first() {
            let first_dense = dense_index[first.0 as usize];
            for &var in rest {
                dsu.union(first_dense, dense_index[var.0 as usize]);
            }
        }
    }

    let mut root_to_component = BTreeMap::new();
    let mut components = Vec::new();

    for &var_id in touched_list {
        let root = dsu.find(dense_index[var_id.0 as usize]);
        let component_idx = *root_to_component.entry(root).or_insert_with(|| {
            components.push(ConstraintComponent {
                variable_ids: Vec::new(),
                equation_ids: SmallVec::new(),
            });
            components.len() - 1
        });

        components[component_idx].variable_ids.push(var_id);
    }

    for equation in equations {
        if !matches!(equation.id, EquationId::LocalClue(_)) {
            continue;
        }

        let mut roots: SmallVec<[usize; 4]> = SmallVec::new();
        for &var in &equation.variable_ids {
            let dense = dense_index[var.0 as usize];
            if dense != usize::MAX {
                roots.push(dsu.find(dense));
            }
        }
        roots.sort_unstable();
        roots.dedup();

        for root in roots {
            if let Some(&component_idx) = root_to_component.get(&root) {
                components[component_idx].equation_ids.push(equation.id);
            }
        }
    }

    for component in &mut components {
        component.variable_ids.sort_unstable();
        component.equation_ids.sort_unstable();
        component.equation_ids.dedup();
    }

    let mut unconstrained_variable_ids = Vec::new();
    for var in variables {
        if touched_list.binary_search(&var.id).is_err() {
            unconstrained_variable_ids.push(var.id);
        }
    }

    unconstrained_variable_ids.sort_unstable();

    (components, unconstrained_variable_ids)
}

#[derive(Clone, Debug)]
struct Dsu {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl Dsu {
    fn new(size: usize) -> Self {
        Self {
            parent: (0..size).collect(),
            rank: vec![0; size],
        }
    }

    fn find(&mut self, value: usize) -> usize {
        if self.parent[value] != value {
            let root = self.find(self.parent[value]);
            self.parent[value] = root;
        }
        self.parent[value]
    }

    fn union(&mut self, left: usize, right: usize) {
        let mut left_root = self.find(left);
        let mut right_root = self.find(right);

        if left_root == right_root {
            return;
        }

        if self.rank[left_root] < self.rank[right_root] {
            core::mem::swap(&mut left_root, &mut right_root);
        }

        self.parent[right_root] = left_root;
        if self.rank[left_root] == self.rank[right_root] {
            self.rank[left_root] += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::smallvec;

    fn var_id(size: Coord2, coords: Coord2) -> CellVarId {
        CellVarId::from_coords(size, coords)
    }

    #[test]
    fn builds_local_clue_equation() {
        let layout = MineLayout::from_mine_coords((2, 2), &[(0, 0)]).unwrap();
        let mut engine = PlayEngine::new(layout);
        engine.reveal((1, 1)).unwrap();
        let obs = Observation::from_engine(&engine);

        let out = build_constraints(&obs, AnalysisConfig::default());

        let local = out
            .problem
            .equations
            .iter()
            .find(|eq| matches!(eq.id, EquationId::LocalClue((1, 1))))
            .expect("local equation should exist");
        assert_eq!(local.target_mines, 1);
        assert_eq!(local.variable_ids.len(), 3);
    }

    #[test]
    fn strict_flags_can_create_clue_contradiction() {
        let revealed = Array2::from_shape_vec([2, 1], vec![None, Some(0)]).unwrap();
        let flags = Array2::from_shape_vec([2, 1], vec![true, false]).unwrap();
        let obs = Observation::new((2, 1), None, revealed, flags).unwrap();

        let strict = build_constraints(
            &obs,
            AnalysisConfig {
                flag_semantics: FlagSemantics::Strict,
                mine_count_usage: MineCountUsage::Ignore,
                ..AnalysisConfig::default()
            },
        );
        let soft = build_constraints(
            &obs,
            AnalysisConfig {
                flag_semantics: FlagSemantics::Soft,
                mine_count_usage: MineCountUsage::Ignore,
                ..AnalysisConfig::default()
            },
        );

        assert!(
            strict
                .contradictions
                .iter()
                .any(|c| matches!(c, Contradiction::LocalClueImpossible { clue: (1, 0), .. }))
        );
        assert!(soft.contradictions.is_empty());
    }

    #[test]
    fn unknown_mine_count_skips_global_equation() {
        let revealed = Array2::from_shape_vec([2, 1], vec![None, Some(1)]).unwrap();
        let flags = Array2::from_shape_vec([2, 1], vec![false, false]).unwrap();
        let obs = Observation::new((2, 1), None, revealed, flags).unwrap();

        let out = build_constraints(&obs, AnalysisConfig::default());

        assert!(
            out.problem
                .equations
                .iter()
                .all(|eq| !matches!(eq.id, EquationId::GlobalMineCount))
        );
    }

    #[test]
    fn splits_independent_components() {
        let size = (5, 1);
        let revealed =
            Array2::from_shape_vec([5, 1], vec![None, Some(1), None, None, Some(0)]).unwrap();
        let flags = Array2::from_elem([5, 1], false);
        let obs = Observation::new(size, None, revealed, flags).unwrap();

        let out = build_constraints(
            &obs,
            AnalysisConfig {
                flag_semantics: FlagSemantics::Soft,
                mine_count_usage: MineCountUsage::Ignore,
                ..AnalysisConfig::default()
            },
        );

        assert_eq!(out.problem.components.len(), 2);
        assert!(out.problem.components.iter().any(|component| {
            component.variable_ids == vec![var_id(size, (0, 0)), var_id(size, (2, 0))]
        }));
        assert!(
            out.problem
                .components
                .iter()
                .any(|component| component.variable_ids == vec![var_id(size, (3, 0))])
        );
    }

    #[test]
    fn strict_global_equation_reports_impossible_target() {
        let revealed = Array2::from_shape_vec([2, 1], vec![None, None]).unwrap();
        let flags = Array2::from_shape_vec([2, 1], vec![true, false]).unwrap();
        let obs = Observation::new((2, 1), Some(0), revealed, flags).unwrap();

        let out = build_constraints(
            &obs,
            AnalysisConfig {
                flag_semantics: FlagSemantics::Strict,
                mine_count_usage: MineCountUsage::UseIfKnown,
                ..AnalysisConfig::default()
            },
        );

        assert!(
            out.contradictions
                .iter()
                .any(|c| matches!(c, Contradiction::GlobalMineCountImpossible { .. }))
        );
    }

    #[test]
    fn variable_ids_are_stable_from_board_coords() {
        let layout = MineLayout::from_mine_coords((3, 1), &[(0, 0)]).unwrap();
        let mut engine = PlayEngine::new(layout);
        let obs0 = Observation::from_engine(&engine);
        let out0 = build_constraints(&obs0, AnalysisConfig::default());

        engine.reveal((2, 0)).unwrap();
        let obs1 = Observation::from_engine(&engine);
        let out1 = build_constraints(&obs1, AnalysisConfig::default());

        let id_00 = CellVarId::from_coords((3, 1), (0, 0));

        assert!(out0.problem.variable_by_id(id_00).is_some());
        assert!(out1.problem.variable_by_id(id_00).is_some());
    }

    #[test]
    fn frontier_maximal_ignores_fully_resolved_clues() {
        let revealed = Array2::from_shape_vec([3, 1], vec![Some(0), Some(0), None]).unwrap();
        let flags = Array2::from_elem([3, 1], false);
        let obs = Observation::new((3, 1), None, revealed, flags).unwrap();

        let full = build_constraints(
            &obs,
            AnalysisConfig {
                scope: ConstraintScope::FullBoard,
                mine_count_usage: MineCountUsage::Ignore,
                ..AnalysisConfig::default()
            },
        );

        let frontier = build_constraints(
            &obs,
            AnalysisConfig {
                scope: ConstraintScope::FrontierMaximal,
                mine_count_usage: MineCountUsage::Ignore,
                ..AnalysisConfig::default()
            },
        );

        assert!(full.stats.local_equation_count > frontier.stats.local_equation_count);
    }

    #[test]
    fn observation_delta_detects_reveal_and_flag_changes() {
        let revealed_prev = Array2::from_shape_vec([2, 1], vec![None, Some(1)]).unwrap();
        let flags_prev = Array2::from_shape_vec([2, 1], vec![false, false]).unwrap();
        let prev = Observation::new((2, 1), Some(1), revealed_prev, flags_prev).unwrap();

        let revealed_next = Array2::from_shape_vec([2, 1], vec![Some(0), Some(1)]).unwrap();
        let flags_next = Array2::from_shape_vec([2, 1], vec![true, false]).unwrap();
        let next = Observation::new((2, 1), Some(1), revealed_next, flags_next).unwrap();

        let delta = ObservationDelta::between(&prev, &next).unwrap();

        assert_eq!(delta.changed_revealed.len(), 1);
        assert_eq!(delta.changed_flags.len(), 1);
        assert!(!delta.is_empty());
    }

    #[test]
    fn constraint_delta_reports_updated_and_removed_items() {
        let revealed_a = Array2::from_shape_vec([2, 1], vec![None, Some(1)]).unwrap();
        let flags_a = Array2::from_shape_vec([2, 1], vec![false, false]).unwrap();
        let obs_a = Observation::new((2, 1), Some(1), revealed_a, flags_a).unwrap();

        let revealed_b = Array2::from_shape_vec([2, 1], vec![Some(0), Some(1)]).unwrap();
        let flags_b = Array2::from_shape_vec([2, 1], vec![false, false]).unwrap();
        let obs_b = Observation::new((2, 1), Some(1), revealed_b, flags_b).unwrap();

        let cfg = AnalysisConfig::default();
        let out_a = build_constraints(&obs_a, cfg.clone());
        let out_b = build_constraints(&obs_b, cfg);

        let delta = ConstraintDelta::from_problems(&out_a.problem, &out_b.problem);

        assert!(!delta.removed_variable_ids.is_empty() || !delta.removed_equation_ids.is_empty());
    }

    /// `apply_prior_assignments` must decrement `target_mines` for removed mines but
    /// NOT for removed safe cells.  A past bug swapped the `safe`/`mines` arguments
    /// when delegating to `reduce_equations_inplace`, causing equation targets to be
    /// decremented for safe-cell removals instead of mine removals.  This test catches
    /// that argument-order mistake directly.
    #[test]
    fn apply_prior_assignments_decrements_target_only_for_mines() {
        // Board: 3x1, mine at (0,0), clue at (1,0) with neighbors (0,0) and (2,0).
        // Build a minimal ConstraintProblem directly.
        let a = CellVarId::from_coords((3, 1), (0, 0)); // mine
        let c = CellVarId::from_coords((3, 1), (2, 0)); // safe hidden

        // Equation: {a, c} = 1  (clue at (1,0) sees hidden cells a and c, one is a mine)
        let eq = ConstraintEquation {
            id: EquationId::LocalClue((1, 0)),
            variable_ids: smallvec![a, c],
            target_mines: 1,
        };

        let problem = ConstraintProblem {
            variables: vec![
                ConstraintVariable {
                    id: a,
                    coords: (0, 0),
                    flagged: false,
                },
                ConstraintVariable {
                    id: c,
                    coords: (2, 0),
                    flagged: false,
                },
            ],
            equations: vec![eq],
            components: vec![],
            unconstrained_variable_ids: vec![],
        };

        // We know `a` is a mine.  Applying this prior must produce equation {c} = 0.
        let prior_mines = [a];
        let prior_safe: &[CellVarId] = &[];

        let reduced = problem.apply_prior_assignments(&prior_mines, prior_safe);

        assert_eq!(
            reduced.equations.len(),
            1,
            "equation with one unresolved variable should remain"
        );
        assert_eq!(
            reduced.equations[0].target_mines, 0,
            "removing a mine must decrement target_mines; if safe/mines args are \
             swapped the target stays at 1 and the equation wrongly forces c to be a mine"
        );
        assert_eq!(
            reduced.equations[0].variable_ids.as_slice(),
            &[c],
            "only the safe variable c should remain in the equation"
        );

        // Symmetrical check: removing a safe cell must NOT decrement the target.
        let prior_mines2: &[CellVarId] = &[];
        let prior_safe2 = [c];

        let eq2 = ConstraintEquation {
            id: EquationId::LocalClue((1, 0)),
            variable_ids: smallvec![a, c],
            target_mines: 1,
        };
        let problem2 = ConstraintProblem {
            variables: vec![
                ConstraintVariable {
                    id: a,
                    coords: (0, 0),
                    flagged: false,
                },
                ConstraintVariable {
                    id: c,
                    coords: (2, 0),
                    flagged: false,
                },
            ],
            equations: vec![eq2],
            components: vec![],
            unconstrained_variable_ids: vec![],
        };

        let reduced2 = problem2.apply_prior_assignments(prior_mines2, &prior_safe2);

        assert_eq!(
            reduced2.equations.len(),
            1,
            "equation with one unresolved variable should remain"
        );
        assert_eq!(
            reduced2.equations[0].target_mines, 1,
            "removing a safe cell must not decrement target_mines; if args are swapped \
             the target becomes 0 and the equation wrongly forces a to be safe"
        );
        assert_eq!(reduced2.equations[0].variable_ids.as_slice(), &[a]);
    }
}
