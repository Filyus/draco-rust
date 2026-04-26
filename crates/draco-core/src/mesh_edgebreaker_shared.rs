#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgebreakerSymbol {
    Center = 0,
    Split = 1,
    Left = 2,
    Right = 3,
    End = 4,
    Hole = 5, // Not used in standard stream, handled separately?
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
            _ => EdgebreakerSymbol::Center, // Default/Error
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
