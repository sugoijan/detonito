use super::*;

/// Generation strategy that can optionally try to make the starting tile zero or at least safe, but other than that is
/// purely random.
#[derive(Clone, Debug, PartialEq)]
pub struct RandomMinefieldGenerator {
    seed: u64,
    start: Ix2,
    start_tile: StartTile,
}

impl RandomMinefieldGenerator {
    pub fn new(seed: u64, start: Ix2, start_tile: StartTile) -> Self {
        Self {
            seed,
            start,
            start_tile,
        }
    }
}

impl MinefieldGenerator for RandomMinefieldGenerator {
    fn generate(self, config: GameConfig) -> Minefield {
        use rand::prelude::*;
        use StartTile::*;

        let total_tiles = config.total_tiles();

        // optimize for full boards
        if config.mines >= total_tiles {
            if config.mines > total_tiles {
                log::warn!(
                    "Minefield already full, generated anyway, requested {} but only fits {}",
                    config.mines,
                    total_tiles
                );
            }
            return Minefield {
                mines: Array2::from_elem(config.size.convert(), true),
                count: config.mines,
            };
        }

        let actual_start_tile = match self.start_tile {
            Random => Random,
            SimpleSafe | AlwaysZero if config.mines + 1 > total_tiles => {
                log::warn!("Cannot make start tile safe, fallback to random");
                Random
            }
            SimpleSafe => SimpleSafe,
            AlwaysZero if config.mines + 9 > total_tiles => {
                log::warn!("Cannot make start tile zero, fallback to simple safe");
                SimpleSafe
            }
            AlwaysZero => AlwaysZero,
        };
        let mut mines: Array2<bool> = Array2::default(config.size.convert());
        let mut free_tiles = match actual_start_tile {
            Random => total_tiles,
            SimpleSafe => {
                mines[self.start.convert()] = true;
                total_tiles - 1
            }
            AlwaysZero => {
                mines[self.start.convert()] = true;
                for coord in mines.iter_adjacent(self.start) {
                    mines[coord.convert()] = true;
                }
                total_tiles - 9
            }
        };
        let mut mines_placed = 0;

        let mut rng = SmallRng::seed_from_u64(self.seed);
        {
            let tiles = mines.as_slice_mut().expect("layout should be standard");
            while mines_placed < config.mines {
                if free_tiles == 0 {
                    break;
                }
                let mut place: Ax = rng.random_range(0..free_tiles);
                for (i, tile) in tiles.iter_mut().enumerate() {
                    let i = i as Ax;
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
                mines[self.start.convert()] = false;
            }
            AlwaysZero => {
                mines[self.start.convert()] = false;
                for coord in mines.iter_adjacent(self.start) {
                    mines[coord.convert()] = false;
                }
            }
        }

        // double check mine count
        let count = mines.iter().filter(|&&tile| tile).count() as Ax;
        if count != config.mines {
            log::warn!(
                "Generated minefield count mismatch, actual: {}, requested: {}",
                count,
                config.mines
            );
        }
        Minefield { mines, count }
    }
}
