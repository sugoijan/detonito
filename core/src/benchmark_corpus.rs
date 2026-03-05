use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::*;

pub const SOLVER_TIERS_CORPUS_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolverTiersCorpus {
    pub version: u32,
    pub scenarios: Vec<SolverTiersScenario>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolverTiersScenario {
    pub name: String,
    pub size: Coord2,
    pub mines: CellCount,
    pub layouts: Vec<SolverTiersLayout>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolverTiersLayout {
    pub mine_coords: Vec<Coord2>,
}

impl SolverTiersLayout {
    pub fn from_layout(layout: &MineLayout) -> Self {
        let size = layout.size();
        let mut mine_coords = Vec::new();
        for x in 0..size.0 {
            for y in 0..size.1 {
                let coords = (x, y);
                if layout.contains_mine(coords) {
                    mine_coords.push(coords);
                }
            }
        }
        Self { mine_coords }
    }

    pub fn to_layout(&self, size: Coord2) -> Result<MineLayout> {
        MineLayout::from_mine_coords(size, &self.mine_coords)
    }
}

pub fn parse_solver_tiers_corpus_json(
    input: &str,
) -> core::result::Result<SolverTiersCorpus, String> {
    serde_json::from_str(input).map_err(|err| err.to_string())
}

pub fn render_solver_tiers_corpus_json_pretty(
    corpus: &SolverTiersCorpus,
) -> core::result::Result<String, String> {
    serde_json::to_string_pretty(corpus).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_roundtrip_preserves_mine_positions() {
        let layout = MineLayout::from_mine_coords((4, 3), &[(0, 0), (2, 1), (3, 2)]).unwrap();
        let stored = SolverTiersLayout::from_layout(&layout);
        let rebuilt = stored.to_layout((4, 3)).unwrap();

        assert_eq!(layout, rebuilt);
    }
}
