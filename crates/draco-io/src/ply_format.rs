/// PLY storage format.
///
/// The default is binary little-endian, the one format upstream Draco's
/// encoder writes: text is several times the size and slower to read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlyFormat {
    /// Text PLY format.
    Ascii,
    /// Binary little-endian PLY format.
    #[default]
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
