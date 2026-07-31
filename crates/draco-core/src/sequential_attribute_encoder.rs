//! Base sequential attribute encoder.
//!
//! [`SequentialAttributeEncoder`] is the generic encode path that writes
//! attribute values in point order with no prediction or transform. Encode-side
//! base reused by the integer and normal encoders. Port of Draco's
//! `sequential_attribute_encoder.h`.

use crate::draco_types::DataType;
use crate::encoder_buffer::EncoderBuffer;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::PointIndex;
use crate::point_cloud::PointCloud;

/// Which sequential encoder handles an attribute. The value is what the
/// bitstream carries: one identifier byte per attribute, which the decoder
/// turns back into a decoder in `SequentialAttributeDecodersController::
/// CreateSequentialDecoder`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SequentialAttributeEncoderType {
    /// Raw bytes, in point order, no transform.
    Generic = 0,
    /// Entropy-coded integers, optionally predicted.
    Integer = 1,
    /// Floats quantized to integers, then encoded as integers.
    Quantization = 2,
    /// Normals mapped onto the octahedron, then encoded as integers.
    Normals = 3,
}

/// Picks the encoder for `attribute`, as
/// `SequentialAttributeEncodersController::CreateSequentialEncoder` does.
///
/// The data type decides first and quantization only breaks the tie within
/// `Float32`: an integer-typed attribute is encoded as an integer whatever
/// quantization was requested for it, and everything upstream's switch does not
/// name -- `Float64`, the 64-bit integers, `Bool` -- falls through to the
/// generic encoder rather than being treated as an integer. `quantization_bits`
/// is the value read from the encoder options for this attribute; any
/// non-positive value means unquantized.
pub fn select_sequential_encoder(
    attribute: &PointAttribute,
    quantization_bits: i32,
) -> SequentialAttributeEncoderType {
    match attribute.data_type() {
        DataType::Uint8
        | DataType::Int8
        | DataType::Uint16
        | DataType::Int16
        | DataType::Uint32
        | DataType::Int32 => SequentialAttributeEncoderType::Integer,
        DataType::Float32 if quantization_bits > 0 => {
            if attribute.attribute_type() == GeometryAttributeType::Normal {
                SequentialAttributeEncoderType::Normals
            } else {
                SequentialAttributeEncoderType::Quantization
            }
        }
        _ => SequentialAttributeEncoderType::Generic,
    }
}

pub struct SequentialAttributeEncoder {
    attribute_id: i32,
}

impl Default for SequentialAttributeEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SequentialAttributeEncoder {
    pub fn new() -> Self {
        Self { attribute_id: -1 }
    }

    pub fn attribute_id(&self) -> i32 {
        self.attribute_id
    }

    pub fn init(&mut self, attribute_id: i32) -> bool {
        self.attribute_id = attribute_id;
        true
    }

    pub fn initialize_standalone(&mut self, _attribute: &PointAttribute) -> bool {
        true
    }

    pub fn transform_attribute_to_portable_format(&mut self, _point_ids: &[PointIndex]) -> bool {
        true
    }

    pub fn encode_values(
        &mut self,
        point_cloud: &PointCloud,
        point_ids: &[PointIndex],
        out_buffer: &mut EncoderBuffer,
    ) -> bool {
        let att = point_cloud.attribute(self.attribute_id);
        let entry_size = att.byte_stride() as usize;
        let buffer_data = att.buffer().data();

        for &p_id in point_ids {
            let mapped_index = att.mapped_index(p_id).0 as usize;
            let offset = mapped_index * entry_size;
            if offset + entry_size > buffer_data.len() {
                return false;
            }
            let bytes = &buffer_data[offset..offset + entry_size];
            out_buffer.encode_data(bytes);
        }
        true
    }
}
