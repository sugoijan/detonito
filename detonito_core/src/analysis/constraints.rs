use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

use ndarray::Array2;
use serde::{Deserialize, Serialize};

use super::Observation;
use crate::*;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlagSemantics {
    Soft,
    Strict,
}

impl Default for FlagSemantics {
    fn default() -> Self {
        Self::Soft
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MineCountUsage {
    UseIfKnown,
    Ignore,
}

impl Default for MineCountUsage {
    fn default() -> Self {
        Self::UseIfKnown
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AnalysisConfig {
    pub flag_semantics: FlagSemantics,
    pub mine_count_usage: MineCountUsage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintVariable {
    pub id: usize,
    pub coords: Coord2,
    pub flagged: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EquationKind {
    LocalClue { clue: Coord2 },
    GlobalMineCount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintEquation {
    pub id: usize,
    pub kind: EquationKind,
    pub variable_ids: Vec<usize>,
    pub target_mines: CellCount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintComponent {
    pub variable_ids: Vec<usize>,
    pub equation_ids: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintProblem {
    pub variables: Vec<ConstraintVariable>,
    pub equations: Vec<ConstraintEquation>,
    pub components: Vec<ConstraintComponent>,
    pub unconstrained_variable_ids: Vec<usize>,
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
    let mut variable_ids: Array2<Option<usize>> = Array2::from_elem(obs.size.to_nd_index(), None);

    let (x_end, y_end) = obs.size;
    for x in 0..x_end {
        for y in 0..y_end {
            let coords = (x, y);
            if obs.revealed[coords.to_nd_index()].is_none() {
                let id = variables.len();
                variables.push(ConstraintVariable {
                    id,
                    coords,
                    flagged: obs.flags[coords.to_nd_index()],
                });
                variable_ids[coords.to_nd_index()] = Some(id);
            }
        }
    }

    let mut equations = Vec::new();
    let mut local_equation_count = 0;

    for x in 0..x_end {
        for y in 0..y_end {
            let clue = (x, y);
            let Some(clue_mines) = obs.revealed[clue.to_nd_index()] else {
                continue;
            };

            let mut target_mines = i16::from(clue_mines);
            let mut equation_vars = Vec::new();

            for neighbor in obs.revealed.iter_neighbors(clue) {
                if obs.revealed[neighbor.to_nd_index()].is_some() {
                    continue;
                }

                let var_id =
                    variable_ids[neighbor.to_nd_index()].expect("variable id should exist");
                let flagged = obs.flags[neighbor.to_nd_index()];

                if matches!(cfg.flag_semantics, FlagSemantics::Strict) && flagged {
                    target_mines -= 1;
                } else {
                    equation_vars.push(var_id);
                }
            }

            if target_mines < 0 || (target_mines as usize) > equation_vars.len() {
                contradictions.push(Contradiction::LocalClueImpossible {
                    clue,
                    target_mines,
                    available_variables: equation_vars.len(),
                });
                continue;
            }

            equations.push(ConstraintEquation {
                id: equations.len(),
                kind: EquationKind::LocalClue { clue },
                variable_ids: equation_vars,
                target_mines: target_mines as CellCount,
            });
            local_equation_count += 1;
        }
    }

    let mut global_equation_count = 0;

    if matches!(cfg.mine_count_usage, MineCountUsage::UseIfKnown) {
        if let Some(total_mines) = obs.mine_count {
            let mut target_mines = i32::from(total_mines);
            let mut equation_vars = Vec::new();

            for var in &variables {
                if matches!(cfg.flag_semantics, FlagSemantics::Strict) && var.flagged {
                    target_mines -= 1;
                } else {
                    equation_vars.push(var.id);
                }
            }

            if target_mines < 0 || (target_mines as usize) > equation_vars.len() {
                contradictions.push(Contradiction::GlobalMineCountImpossible {
                    target_mines,
                    available_variables: equation_vars.len(),
                });
            } else {
                equations.push(ConstraintEquation {
                    id: equations.len(),
                    kind: EquationKind::GlobalMineCount,
                    variable_ids: equation_vars,
                    target_mines: target_mines as CellCount,
                });
                global_equation_count = 1;
            }
        }
    }

    let (components, unconstrained_variable_ids) = build_components(variables.len(), &equations);

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

fn build_components(
    variable_count: usize,
    equations: &[ConstraintEquation],
) -> (Vec<ConstraintComponent>, Vec<usize>) {
    let mut dsu = Dsu::new(variable_count);
    let mut touched = vec![false; variable_count];

    for equation in equations {
        if !matches!(equation.kind, EquationKind::LocalClue { .. }) {
            continue;
        }

        if let Some((&first, rest)) = equation.variable_ids.split_first() {
            touched[first] = true;
            for &var in rest {
                touched[var] = true;
                dsu.union(first, var);
            }
        }
    }

    let mut root_to_component = BTreeMap::new();
    let mut components = Vec::new();

    for var in 0..variable_count {
        if !touched[var] {
            continue;
        }

        let root = dsu.find(var);
        let component_idx = *root_to_component.entry(root).or_insert_with(|| {
            components.push(ConstraintComponent {
                variable_ids: Vec::new(),
                equation_ids: Vec::new(),
            });
            components.len() - 1
        });

        components[component_idx].variable_ids.push(var);
    }

    for equation in equations {
        if !matches!(equation.kind, EquationKind::LocalClue { .. }) {
            continue;
        }

        let mut roots = BTreeSet::new();
        for &var in &equation.variable_ids {
            if touched[var] {
                roots.insert(dsu.find(var));
            }
        }

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
    for (var_id, was_touched) in touched.into_iter().enumerate() {
        if !was_touched {
            unconstrained_variable_ids.push(var_id);
        }
    }

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
            .find(|eq| matches!(eq.kind, EquationKind::LocalClue { clue: (1, 1) }))
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
            },
        );
        let soft = build_constraints(
            &obs,
            AnalysisConfig {
                flag_semantics: FlagSemantics::Soft,
                mine_count_usage: MineCountUsage::Ignore,
            },
        );

        assert!(strict
            .contradictions
            .iter()
            .any(|c| matches!(c, Contradiction::LocalClueImpossible { clue: (1, 0), .. })));
        assert!(soft.contradictions.is_empty());
    }

    #[test]
    fn unknown_mine_count_skips_global_equation() {
        let revealed = Array2::from_shape_vec([2, 1], vec![None, Some(1)]).unwrap();
        let flags = Array2::from_shape_vec([2, 1], vec![false, false]).unwrap();
        let obs = Observation::new((2, 1), None, revealed, flags).unwrap();

        let out = build_constraints(&obs, AnalysisConfig::default());

        assert!(out
            .problem
            .equations
            .iter()
            .all(|eq| !matches!(eq.kind, EquationKind::GlobalMineCount)));
    }

    #[test]
    fn splits_independent_components() {
        let revealed =
            Array2::from_shape_vec([5, 1], vec![None, Some(1), None, None, Some(0)]).unwrap();
        let flags = Array2::from_elem([5, 1], false);
        let obs = Observation::new((5, 1), None, revealed, flags).unwrap();

        let out = build_constraints(
            &obs,
            AnalysisConfig {
                flag_semantics: FlagSemantics::Soft,
                mine_count_usage: MineCountUsage::Ignore,
            },
        );

        assert_eq!(out.problem.components.len(), 2);
        assert!(out
            .problem
            .components
            .iter()
            .any(|component| component.variable_ids == vec![0, 1]));
        assert!(out
            .problem
            .components
            .iter()
            .any(|component| component.variable_ids == vec![2]));
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
            },
        );

        assert!(out
            .contradictions
            .iter()
            .any(|c| matches!(c, Contradiction::GlobalMineCountImpossible { .. })));
    }
}
