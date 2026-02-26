use crate::*;
pub use random::*;

mod random;

pub trait LayoutGenerator {
    fn generate(self, config: GameConfig) -> MineLayout;
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum FirstMovePolicy {
    Random,
    FirstMoveSafe,
    FirstMoveZero,
}
