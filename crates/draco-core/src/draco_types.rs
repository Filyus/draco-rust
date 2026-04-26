use crate::status::DracoError;
use std::convert::TryFrom;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Invalid = 0,
    Int8,
    Uint8,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Int64,
    Uint64,
    Float32,
    Float64,
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
    pub fn byte_length(&self) -> usize {
        match self {
            DataType::Invalid => 0,
            DataType::Int8 | DataType::Uint8 | DataType::Bool => 1,
            DataType::Int16 | DataType::Uint16 => 2,
            DataType::Int32 | DataType::Uint32 | DataType::Float32 => 4,
            DataType::Int64 | DataType::Uint64 | DataType::Float64 => 8,
        }
    }

    pub fn is_integral(&self) -> bool {
        !matches!(
            self,
            DataType::Float32 | DataType::Float64 | DataType::Invalid
        )
    }
}
