use crate::*;
pub use random::*;

mod random;

pub trait MinefieldGenerator {
    fn generate(self, config: GameConfig) -> Minefield;
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum StartTile {
    Random,
    SimpleSafe,
    AlwaysZero,
}
