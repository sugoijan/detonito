use thiserror::Error;

#[derive(Error, Debug, Copy, Clone, PartialEq, Eq)]
pub enum GameError {
    #[error("Invalid coordinates")]
    InvalidCoords,
    #[error("Too many mines")]
    TooManyMines,
    #[error("Board shape does not match declared size")]
    InvalidBoardShape,
    #[error("Game already ended, no new moves are accepted")]
    AlreadyEnded,
}

pub type Result<T> = core::result::Result<T, GameError>;
