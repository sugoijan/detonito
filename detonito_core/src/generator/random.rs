use super::*;

/// Random board generation with optional first-move constraints.
#[derive(Clone, Debug, PartialEq)]
pub struct RandomLayoutGenerator {
    seed: u64,
    first_move: Coord2,
    first_move_policy: FirstMovePolicy,
}

impl RandomLayoutGenerator {
    pub fn new(seed: u64, first_move: Coord2, first_move_policy: FirstMovePolicy) -> Self {
        Self {
            seed,
            first_move,
            first_move_policy,
        }
    }
}

impl LayoutGenerator for RandomLayoutGenerator {
    fn generate(self, config: GameConfig) -> MineLayout {
        use rand::prelude::*;
        use FirstMovePolicy::*;

        let total_cells = config.total_cells();

        if config.mines >= total_cells {
            if config.mines > total_cells {
                log::warn!(
                    "Layout already full, generated anyway, requested {} but only fits {}",
                    config.mines,
                    total_cells
                );
            }
            return MineLayout {
                mine_mask: Array2::from_elem(config.size.to_nd_index(), true),
                mine_count: config.mines,
            };
        }

        let effective_policy = match self.first_move_policy {
            Random => Random,
            FirstMoveSafe | FirstMoveZero if config.mines + 1 > total_cells => {
                log::warn!("Cannot make first move safe, fallback to random");
                Random
            }
            FirstMoveSafe => FirstMoveSafe,
            FirstMoveZero if config.mines + 9 > total_cells => {
                log::warn!("Cannot make first move zero, fallback to first-move-safe");
                FirstMoveSafe
            }
            FirstMoveZero => FirstMoveZero,
        };

        let mut mine_mask: Array2<bool> = Array2::default(config.size.to_nd_index());
        let mut available_cells = match effective_policy {
            Random => total_cells,
            FirstMoveSafe => {
                mine_mask[self.first_move.to_nd_index()] = true;
                total_cells - 1
            }
            FirstMoveZero => {
                mine_mask[self.first_move.to_nd_index()] = true;
                for coord in mine_mask.iter_neighbors(self.first_move) {
                    mine_mask[coord.to_nd_index()] = true;
                }
                total_cells - 9
            }
        };
        let mut placed_mines = 0;

        let mut rng = SmallRng::seed_from_u64(self.seed);
        {
            let cells = mine_mask.as_slice_mut().expect("layout should be standard");
            while placed_mines < config.mines {
                if available_cells == 0 {
                    break;
                }

                let mut place: CellCount = rng.random_range(0..available_cells);
                for (i, is_reserved) in cells.iter_mut().enumerate() {
                    let i = i as CellCount;
                    if *is_reserved {
                        place += 1;
                    }
                    if i == place {
                        *is_reserved = true;
                        placed_mines += 1;
                        available_cells -= 1;
                        break;
                    }
                }
            }
        }

        match effective_policy {
            Random => {}
            FirstMoveSafe => {
                mine_mask[self.first_move.to_nd_index()] = false;
            }
            FirstMoveZero => {
                mine_mask[self.first_move.to_nd_index()] = false;
                for coord in mine_mask.iter_neighbors(self.first_move) {
                    mine_mask[coord.to_nd_index()] = false;
                }
            }
        }

        let mine_count = mine_mask.iter().filter(|&&cell| cell).count() as CellCount;
        if mine_count != config.mines {
            log::warn!(
                "Generated mine count mismatch, actual: {}, requested: {}",
                mine_count,
                config.mines
            );
        }

        MineLayout {
            mine_mask,
            mine_count,
        }
    }
}
