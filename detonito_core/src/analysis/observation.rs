use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub size: Coord2,
    pub mine_count: Option<CellCount>,
    pub revealed: Array2<Option<u8>>,
    pub flags: Array2<bool>,
}

impl Observation {
    pub fn new(
        size: Coord2,
        mine_count: Option<CellCount>,
        revealed: Array2<Option<u8>>,
        flags: Array2<bool>,
    ) -> Result<Self> {
        let obs = Self {
            size,
            mine_count,
            revealed,
            flags,
        };
        obs.validate()?;
        Ok(obs)
    }

    pub fn from_engine(engine: &PlayEngine) -> Self {
        Self::from_engine_with_mine_count(engine, Some(engine.total_mines()))
    }

    pub fn from_engine_with_mine_count(engine: &PlayEngine, mine_count: Option<CellCount>) -> Self {
        let size = engine.size();
        let mut revealed = Array2::from_elem(size.to_nd_index(), None);
        let mut flags = Array2::from_elem(size.to_nd_index(), false);

        let (x_end, y_end) = size;
        for x in 0..x_end {
            for y in 0..y_end {
                let coords = (x, y);
                match engine.cell_at(coords) {
                    EngineCell::Hidden => {}
                    EngineCell::Revealed(count) => revealed[coords.to_nd_index()] = Some(count),
                    EngineCell::Flagged => flags[coords.to_nd_index()] = true,
                }
            }
        }

        Self {
            size,
            mine_count,
            revealed,
            flags,
        }
    }

    pub fn validate(&self) -> Result<()> {
        let expected = (self.size.0 as usize, self.size.1 as usize);
        if self.revealed.dim() != expected || self.flags.dim() != expected {
            return Err(GameError::InvalidBoardShape);
        }

        if let Some(mine_count) = self.mine_count {
            if mine_count > mult(self.size.0, self.size.1) {
                return Err(GameError::TooManyMines);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_engine_maps_revealed_and_flagged_cells() {
        let layout = MineLayout::from_mine_coords((2, 2), &[(0, 0)]).unwrap();
        let mut engine = PlayEngine::new(layout);

        engine.reveal((1, 1)).unwrap();
        engine.toggle_flag((0, 0)).unwrap();

        let obs = Observation::from_engine(&engine);

        assert_eq!(obs.mine_count, Some(1));
        assert_eq!(obs.revealed[(1, 1)], Some(1));
        assert!(obs.flags[(0, 0)]);
    }

    #[test]
    fn validate_rejects_shape_mismatch() {
        let obs = Observation {
            size: (2, 2),
            mine_count: Some(1),
            revealed: Array2::from_elem([2, 2], None),
            flags: Array2::from_elem([1, 2], false),
        };

        assert_eq!(obs.validate(), Err(GameError::InvalidBoardShape));
    }
}
