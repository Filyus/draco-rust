/// PLY storage format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlyFormat {
    #[default]
    Ascii,
    BinaryLittleEndian,
    BinaryBigEndian,
}

impl PlyFormat {
    pub fn as_ply_token(self) -> &'static str {
        match self {
            PlyFormat::Ascii => "ascii",
            PlyFormat::BinaryLittleEndian => "binary_little_endian",
            PlyFormat::BinaryBigEndian => "binary_big_endian",
        }
    }
}
