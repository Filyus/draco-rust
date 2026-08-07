//! Attribute transform trait and transform-type enum.
//!
//! An attribute transform converts attribute values into a more compressible
//! "portable" integer representation before coding, and inverts it on decode
//! (e.g. quantization, octahedral normal mapping). [`AttributeTransform`] is the
//! shared interface; [`AttributeTransformType`] tags which transform was used.
//! Port of Draco's `attribute_transform.h`.

use crate::attribute_transform_data::AttributeTransformData;
#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
use crate::draco_types::DataType;
#[cfg(feature = "encoder")]
use crate::encoder_buffer::EncoderBuffer;
use crate::geometry_attribute::PointAttribute;
use crate::prediction_scheme::EntryToPointIdMap;
use crate::status::Status;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeTransformType {
    InvalidTransform = -1,
    NoTransform = 0,
    QuantizationTransform = 1,
    OctahedronTransform = 2,
}

impl TryFrom<u8> for AttributeTransformType {
    type Error = ();
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(AttributeTransformType::NoTransform),
            1 => Ok(AttributeTransformType::QuantizationTransform),
            2 => Ok(AttributeTransformType::OctahedronTransform),
            _ => Err(()),
        }
    }
}

/// A transform between an attribute's stored values and its portable integer
/// form.
///
/// Every fallible method returns [`Status`] and names what went wrong. They
/// returned a bare `bool` through 1.x, which collapsed "the source buffer is
/// truncated", "the parameters were never computed" and "this component count
/// is not supported" into one indistinguishable `false`.
pub trait AttributeTransform {
    fn transform_type(&self) -> AttributeTransformType;

    fn init_from_attribute(&mut self, attribute: &PointAttribute) -> Status;

    fn copy_to_attribute_transform_data(&self, out_data: &mut AttributeTransformData);

    fn transform_attribute(
        &self,
        attribute: &PointAttribute,
        point_ids: EntryToPointIdMap<'_>,
        target_attribute: &mut PointAttribute,
    ) -> Status;

    fn inverse_transform_attribute(
        &self,
        attribute: &PointAttribute,
        target_attribute: &mut PointAttribute,
    ) -> Status;

    #[cfg(feature = "encoder")]
    fn encode_parameters(&self, encoder_buffer: &mut EncoderBuffer) -> Status;

    #[cfg(feature = "decoder")]
    fn decode_parameters(
        &mut self,
        attribute: &PointAttribute,
        decoder_buffer: &mut DecoderBuffer,
    ) -> Status;

    fn get_transformed_data_type(&self, attribute: &PointAttribute) -> DataType;
    fn get_transformed_num_components(&self, attribute: &PointAttribute) -> i32;
}
