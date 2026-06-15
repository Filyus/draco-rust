use crate::status::DracoError;
use std::convert::TryFrom;

/// Scalar data type stored in a Draco geometry attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    /// Invalid or unset data type.
    Invalid = 0,
    /// Signed 8-bit integer.
    Int8,
    /// Unsigned 8-bit integer.
    Uint8,
    /// Signed 16-bit integer.
    Int16,
    /// Unsigned 16-bit integer.
    Uint16,
    /// Signed 32-bit integer.
    Int32,
    /// Unsigned 32-bit integer.
    Uint32,
    /// Signed 64-bit integer.
    Int64,
    /// Unsigned 64-bit integer.
    Uint64,
    /// 32-bit floating point value.
    Float32,
    /// 64-bit floating point value.
    Float64,
    /// Boolean value.
    Bool,
}

impl TryFrom<u8> for DataType {
    type Error = DracoError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Int8),
            2 => Ok(Self::Uint8),
            3 => Ok(Self::Int16),
            4 => Ok(Self::Uint16),
            5 => Ok(Self::Int32),
            6 => Ok(Self::Uint32),
            7 => Ok(Self::Int64),
            8 => Ok(Self::Uint64),
            9 => Ok(Self::Float32),
            10 => Ok(Self::Float64),
            11 => Ok(Self::Bool),
            _ => Err(DracoError::DracoError(format!(
                "Invalid attribute data type: {value}"
            ))),
        }
    }
}

impl DataType {
    /// Returns the byte width of one scalar value.
    pub fn byte_length(&self) -> usize {
        match self {
            DataType::Invalid => 0,
            DataType::Int8 | DataType::Uint8 | DataType::Bool => 1,
            DataType::Int16 | DataType::Uint16 => 2,
            DataType::Int32 | DataType::Uint32 | DataType::Float32 => 4,
            DataType::Int64 | DataType::Uint64 | DataType::Float64 => 8,
        }
    }

    /// Returns true for integer-like attribute storage types.
    pub fn is_integral(&self) -> bool {
        !matches!(
            self,
            DataType::Float32 | DataType::Float64 | DataType::Invalid
        )
    }
}
