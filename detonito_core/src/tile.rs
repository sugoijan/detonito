use serde::{Deserialize, Serialize};

/// Canonical player-visible state stored by the gameplay engine.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub enum EngineCell {
    #[default]
    Hidden,
    Revealed(u8),
    Flagged,
}

impl EngineCell {
    pub const fn is_unrevealed(self) -> bool {
        matches!(self, Self::Hidden | Self::Flagged)
    }
}
