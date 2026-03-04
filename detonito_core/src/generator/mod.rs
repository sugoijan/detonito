use crate::*;
pub use no_guess::*;
pub use random::*;

mod no_guess;
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
