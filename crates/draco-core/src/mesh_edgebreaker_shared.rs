//! Shared EdgeBreaker symbols and constants.
//!
//! Defines [`EdgebreakerSymbol`] (the C/L/R/S/E traversal symbols) and other
//! constants shared by the EdgeBreaker encoder and decoder. Port of Draco's
//! `mesh_edgebreaker_shared.h`.

/// The traversal symbols, numbered as the valence coder counts them.
///
/// These are indices, not the bit patterns that reach the stream: upstream
/// writes C, S, L, R and E as `0`, `1`, `3`, `5` and `7`, and the encoder maps
/// to those where it emits them. `Hole` is the classic Edgebreaker H symbol,
/// which Draco's bitstream has no encoding for; nothing here emits it, and the
/// encoder gives it E's pattern where a match has to cover it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgebreakerSymbol {
    Center = 0,
    Split = 1,
    Left = 2,
    Right = 3,
    End = 4,
    Hole = 5,
}

impl From<u32> for EdgebreakerSymbol {
    fn from(v: u32) -> Self {
        match v {
            0 => EdgebreakerSymbol::Center,
            1 => EdgebreakerSymbol::Split,
            2 => EdgebreakerSymbol::Left,
            3 => EdgebreakerSymbol::Right,
            4 => EdgebreakerSymbol::End,
            5 => EdgebreakerSymbol::Hole,
            // Only symbols this encoder produced reach here, so the arm is
            // unreachable rather than lenient.
            _ => EdgebreakerSymbol::Center,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeFaceName {
    LeftFaceEdge = 0,
    RightFaceEdge = 1,
}

#[derive(Debug, Clone)]
pub struct TopologySplitEventData {
    pub split_symbol_id: u32,
    pub source_symbol_id: u32,
    pub source_edge: EdgeFaceName,
}
