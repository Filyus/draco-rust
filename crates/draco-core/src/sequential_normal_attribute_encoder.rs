//! Normal sequential attribute encoder.
//!
//! [`SequentialNormalAttributeEncoder`] octahedron-encodes unit normals into a
//! quantized 2D integer pair and encodes them with the normal-specialized
//! prediction transform. Encode-side counterpart of
//! `SequentialNormalAttributeDecoder`. Port of Draco's
//! `sequential_normal_attribute_encoder.h`.

use crate::attribute_octahedron_transform::AttributeOctahedronTransform;
use crate::draco_types::DataType;
use crate::encoder_buffer::EncoderBuffer;
use crate::encoder_options::EncoderOptions;
use crate::geometry_indices::PointIndex;
use crate::point_cloud::PointCloud;
use crate::point_cloud_encoder::GeometryEncoder;
use crate::sequential_integer_attribute_encoder::{
    IntPredictionTransformFamily, SequentialIntegerAttributeEncoder,
};
use crate::status::{DracoError, Status};

pub struct SequentialNormalAttributeEncoder {
    base: SequentialIntegerAttributeEncoder,
    attribute_octahedron_transform: AttributeOctahedronTransform,
    portable_attribute: crate::geometry_attribute::PointAttribute,
}

impl Default for SequentialNormalAttributeEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SequentialNormalAttributeEncoder {
    pub fn new() -> Self {
        Self {
            base: SequentialIntegerAttributeEncoder::new(),
            attribute_octahedron_transform: AttributeOctahedronTransform::new(-1),
            portable_attribute: crate::geometry_attribute::PointAttribute::default(),
        }
    }

    /// The prediction scheme and transform the octahedron-folded values were
    /// encoded with. Delegated: a normal is encoded as integers once folded.
    pub fn selected_prediction(
        &self,
    ) -> Option<(
        crate::prediction_scheme::PredictionSchemeMethod,
        crate::prediction_scheme::PredictionSchemeTransformType,
    )> {
        self.base.selected_prediction()
    }

    pub fn init(
        &mut self,
        point_cloud: &PointCloud,
        attribute_id: i32,
        options: &EncoderOptions,
    ) -> Status {
        if !self.base.init(attribute_id) {
            return Err(DracoError::invalid_parameter(format!(
                "Attribute {attribute_id} is not a valid encoder attribute"
            )));
        }

        let attribute = point_cloud.attribute(attribute_id);
        if attribute.num_components() != 3 {
            return Err(DracoError::invalid_parameter(format!(
                "Normal encoding needs 3 components, attribute {attribute_id} has {}",
                attribute.num_components()
            )));
        }

        let quantization_bits = options.get_attribute_int(attribute_id, "quantization_bits", -1);
        // The octahedron transform accepts 2..=30, narrower than the `>= 1`
        // this checked. Its own answer used to be discarded, so a bit count of
        // 1 or above 30 left the transform uninitialized while `init` reported
        // success -- the encode then failed later, at the folding step, with
        // nothing naming the bit count as the cause.
        self.attribute_octahedron_transform
            .set_parameters(quantization_bits)
    }

    pub fn encode_data_needed_by_portable_transform(&self, out_buffer: &mut EncoderBuffer) -> bool {
        // The one byte AttributeOctahedronTransform::EncodeParameters writes.
        out_buffer.encode(self.attribute_octahedron_transform.quantization_bits() as u8);
        true
    }

    pub fn encode_values(
        &mut self,
        point_cloud: &PointCloud,
        point_ids: &[PointIndex],
        out_buffer: &mut EncoderBuffer,
        options: &EncoderOptions,
        encoder: &dyn GeometryEncoder,
    ) -> Status {
        let attribute_id = self.base.base.attribute_id();
        let attribute = point_cloud.attribute(attribute_id);

        // Prepare values (transform to octahedral coordinates)
        self.portable_attribute = crate::geometry_attribute::PointAttribute::new();
        self.portable_attribute.init(
            crate::geometry_attribute::GeometryAttributeType::Generic,
            2,
            DataType::Uint32,
            false,
            point_ids.len(),
        );

        self.attribute_octahedron_transform
            .generate_portable_attribute(
                attribute,
                point_ids,
                point_ids.len(),
                &mut self.portable_attribute,
            )?;

        let quantization_bits = self.attribute_octahedron_transform.quantization_bits();
        // quantization_bits can be 31; avoid signed shift overflow.
        let max_value: i32 = ((1u64 << (quantization_bits as u32)) - 1) as i32;

        let (major, minor) = options.get_version();
        let bitstream_version = crate::version::bitstream_version(major, minor);
        let canonicalized = !(cfg!(feature = "legacy_bitstream_encode")
            && bitstream_version != 0
            && bitstream_version < 0x0102);

        // Name the transform family and let the base select the method, exactly
        // as SequentialNormalAttributeEncoder::CreateIntPredictionScheme does
        // upstream: the override decides the transform, the shared code decides
        // between geometric normal and difference. Installing a delta scheme
        // here instead would short-circuit that choice, which is why normals
        // were never predicted geometrically.
        self.base
            .set_transform_family(IntPredictionTransformFamily::NormalOctahedron {
                max_quantized_value: max_value,
                canonicalized,
            });

        self.base.encode_values(
            point_cloud,
            point_ids,
            out_buffer,
            options,
            encoder,
            Some(&self.portable_attribute),
            true,
        )
    }
}
