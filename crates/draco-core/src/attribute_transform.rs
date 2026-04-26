use crate::attribute_transform_data::AttributeTransformData;
#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
use crate::draco_types::DataType;
#[cfg(feature = "encoder")]
use crate::encoder_buffer::EncoderBuffer;
use crate::geometry_attribute::PointAttribute;
use crate::geometry_indices::PointIndex;

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

pub trait AttributeTransform {
    fn transform_type(&self) -> AttributeTransformType;

    fn init_from_attribute(&mut self, attribute: &PointAttribute) -> bool;

    fn copy_to_attribute_transform_data(&self, out_data: &mut AttributeTransformData);

    fn transform_attribute(
        &self,
        attribute: &PointAttribute,
        point_ids: &[PointIndex],
        target_attribute: &mut PointAttribute,
    ) -> bool;

    fn inverse_transform_attribute(
        &self,
        attribute: &PointAttribute,
        target_attribute: &mut PointAttribute,
    ) -> bool;

    #[cfg(feature = "encoder")]
    fn encode_parameters(&self, encoder_buffer: &mut EncoderBuffer) -> bool;

    #[cfg(feature = "decoder")]
    fn decode_parameters(
        &mut self,
        attribute: &PointAttribute,
        decoder_buffer: &mut DecoderBuffer,
    ) -> bool;

    fn get_transformed_data_type(&self, attribute: &PointAttribute) -> DataType;
    fn get_transformed_num_components(&self, attribute: &PointAttribute) -> i32;
}
