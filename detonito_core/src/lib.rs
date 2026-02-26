#![no_std]

extern crate alloc;

use core::ops::{BitOr, Index, IndexMut};
use ndarray::Array2;
use serde::{Deserialize, Serialize};

pub use analysis::*;
pub use engine::*;
pub use error::*;
pub use generator::*;
pub use tile::*;
pub use types::*;

mod analysis;
mod engine;
mod error;
mod generator;
mod solver;
mod tile;
mod types;

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameConfig {
    pub size: Coord2,
    pub mines: CellCount,
}

impl GameConfig {
    pub const fn new_unchecked(size: Coord2, mines: CellCount) -> Self {
        Self { size, mines }
    }

    pub fn new((size_x, size_y): Coord2, mines: CellCount) -> Self {
        let size_x = size_x.clamp(1, Coord::MAX);
        let size_y = size_y.clamp(1, Coord::MAX);
        let mines = mines.clamp(1, mult(size_x, size_y));
        Self::new_unchecked((size_x, size_y), mines)
    }

    pub const fn total_cells(&self) -> CellCount {
        mult(self.size.0, self.size.1)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MineLayout {
    mine_mask: Array2<bool>,
    mine_count: CellCount,
}

impl MineLayout {
    pub fn from_mine_mask(mine_mask: Array2<bool>) -> Self {
        let mine_count = mine_mask
            .iter()
            .filter(|&&is_mine| is_mine)
            .count()
            .try_into()
            .unwrap();
        Self {
            mine_mask,
            mine_count,
        }
    }

    pub fn from_mine_coords(size: Coord2, mine_coords: &[Coord2]) -> Result<Self> {
        let mut mine_mask: Array2<bool> = Array2::default(size.to_nd_index());

        for &coords in mine_coords {
            if coords.0 >= size.0 || coords.1 >= size.1 {
                return Err(GameError::InvalidCoords);
            }
            mine_mask[coords.to_nd_index()] = true;
        }

        Ok(Self::from_mine_mask(mine_mask))
    }

    pub fn game_config(&self) -> GameConfig {
        GameConfig {
            size: self.size(),
            mines: self.mine_count,
        }
    }

    pub fn validate_coords(&self, coords: Coord2) -> Result<Coord2> {
        let size = self.size();
        if coords.0 < size.0 && coords.1 < size.1 {
            Ok(coords)
        } else {
            Err(GameError::InvalidCoords)
        }
    }

    pub fn size(&self) -> Coord2 {
        let dim = self.mine_mask.dim();
        (dim.0.try_into().unwrap(), dim.1.try_into().unwrap())
    }

    pub fn safe_cell_count(&self) -> CellCount {
        self.total_cells() - self.mine_count
    }

    pub fn total_cells(&self) -> CellCount {
        self.mine_mask.len().try_into().unwrap()
    }

    pub fn mine_count(&self) -> CellCount {
        self.mine_count
    }

    pub fn contains_mine(&self, coords: Coord2) -> bool {
        self[coords]
    }

    pub fn adjacent_mine_count(&self, coords: Coord2) -> u8 {
        self.mine_mask
            .iter_neighbors(coords)
            .filter(|&pos| self[pos])
            .count()
            .try_into()
            .unwrap()
    }

    pub(crate) fn iter_neighbors(&self, coords: Coord2) -> NeighborIter {
        self.mine_mask.iter_neighbors(coords)
    }
}

impl Index<Coord2> for MineLayout {
    type Output = bool;

    fn index(&self, (x, y): Coord2) -> &Self::Output {
        &self.mine_mask[(x as usize, y as usize)]
    }
}

impl IndexMut<Coord2> for MineLayout {
    fn index_mut(&mut self, (x, y): Coord2) -> &mut Self::Output {
        &mut self.mine_mask[(x as usize, y as usize)]
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum MarkOutcome {
    NoChange,
    Changed,
}

impl MarkOutcome {
    pub const fn has_update(self) -> bool {
        match self {
            Self::NoChange => false,
            Self::Changed => true,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum RevealOutcome {
    NoChange,
    Revealed,
    HitMine,
    Won,
}

impl RevealOutcome {
    pub const fn has_update(self) -> bool {
        use RevealOutcome::*;
        match self {
            NoChange => false,
            Revealed => true,
            HitMine => true,
            Won => true,
        }
    }
}

impl BitOr for RevealOutcome {
    type Output = RevealOutcome;

    fn bitor(self, rhs: Self) -> Self::Output {
        use RevealOutcome::*;
        match (self, rhs) {
            (HitMine, _) => HitMine,
            (_, HitMine) => HitMine,
            (Won, _) => Won,
            (_, Won) => Won,
            (Revealed, _) => Revealed,
            (_, Revealed) => Revealed,
            (NoChange, NoChange) => NoChange,
        }
    }
}
