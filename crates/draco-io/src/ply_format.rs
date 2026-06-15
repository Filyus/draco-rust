/// PLY storage format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlyFormat {
    /// Text PLY format.
    #[default]
    Ascii,
    /// Binary little-endian PLY format.
    BinaryLittleEndian,
    /// Binary big-endian PLY format.
    BinaryBigEndian,
}

impl PlyFormat {
    /// Return the token used in a PLY header for this format.
    pub fn as_ply_token(self) -> &'static str {
        match self {
            PlyFormat::Ascii => "ascii",
            PlyFormat::BinaryLittleEndian => "binary_little_endian",
            PlyFormat::BinaryBigEndian => "binary_big_endian",
        }
    }
}
