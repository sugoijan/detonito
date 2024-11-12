use serde::{Deserialize, Serialize};

// Define your enum for tile state and make it JS-compatible
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AnyTile {
    Closed,
    Open(u8),
    Flag,
    Question,
    Exploded,
    Mine,
    IncorrectFlag,
}

impl AnyTile {
    // whether the tile is visually closed
    pub fn is_closed(self) -> bool {
        use AnyTile::*;
        match self {
            Closed => true,
            Open(_) => false,
            Flag => true,
            Question => true,
            Exploded => false,
            Mine => false,
            IncorrectFlag => true,
        }
    }
}

impl Default for AnyTile {
    fn default() -> Self {
        Self::Closed
    }
}

pub enum PlayTile {
    Closed,
    Open(u8),
    Flag,
    Question,
}

impl From<PlayTile> for AnyTile {
    fn from(other: PlayTile) -> Self {
        match other {
            PlayTile::Closed => AnyTile::Closed,
            PlayTile::Open(i) => AnyTile::Open(i),
            PlayTile::Flag => AnyTile::Flag,
            PlayTile::Question => AnyTile::Question,
        }
    }
}

pub enum WinTile {
    Open(u8),
    Flag,
}

impl From<WinTile> for AnyTile {
    fn from(other: WinTile) -> Self {
        match other {
            WinTile::Open(i) => AnyTile::Open(i),
            WinTile::Flag => AnyTile::Flag,
        }
    }
}
