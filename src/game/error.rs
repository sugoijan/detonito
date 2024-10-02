use thiserror::Error;

#[derive(Error, Debug)]
pub enum GameError {
    #[error("Invalid coordinates")]
    InvalidCoords,
    #[error("Too many mines")]
    TooManyMines,
    #[error("Game already ended, no new moves are accepted")]
    AlreadyEnded,
}

pub type Result<T> = std::result::Result<T, GameError>;
