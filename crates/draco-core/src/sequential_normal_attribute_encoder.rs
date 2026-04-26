use crate::attribute_octahedron_transform::AttributeOctahedronTransform;
use crate::draco_types::DataType;
use crate::encoder_buffer::EncoderBuffer;
use crate::encoder_options::EncoderOptions;
use crate::geometry_indices::PointIndex;
use crate::point_cloud::PointCloud;
use crate::point_cloud_encoder::GeometryEncoder;
use crate::prediction_scheme_normal_octahedron_canonicalized_encoding_transform::PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform;
use crate::sequential_integer_attribute_encoder::SequentialIntegerAttributeEncoder;

use crate::prediction_scheme_delta::PredictionSchemeDeltaEncoder;

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

    pub fn init(
        &mut self,
        point_cloud: &PointCloud,
        attribute_id: i32,
        options: &EncoderOptions,
    ) -> bool {
        if !self.base.init(attribute_id) {
            return false;
        }

        let attribute = point_cloud.attribute(attribute_id);
        if attribute.num_components() != 3 {
            return false;
        }

        let quantization_bits = options.get_attribute_int(attribute_id, "quantization_bits", -1);
        if quantization_bits < 1 {
            return false;
        }
        self.attribute_octahedron_transform
            .set_parameters(quantization_bits);
        true
    }

    pub fn encode_data_needed_by_portable_transform(&self, out_buffer: &mut EncoderBuffer) -> bool {
        // attribute_octahedron_transform_.EncodeParameters(out_buffer)
        // Wait, AttributeOctahedronTransform doesn't have EncodeParameters in Rust?
        // Let's check AttributeOctahedronTransform.
        // It has generate_portable_attribute.
        // But EncodeParameters is needed.
        // In C++, AttributeOctahedronTransform::EncodeParameters writes quantization_bits.
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
    ) -> bool {
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

        if self
            .attribute_octahedron_transform
            .generate_portable_attribute(
                attribute,
                point_ids,
                point_ids.len(),
                &mut self.portable_attribute,
            )
            .is_err()
        {
            return false;
        }

        let quantization_bits = self.attribute_octahedron_transform.quantization_bits();
        // quantization_bits can be 31; avoid signed shift overflow.
        let max_value: i32 = ((1u64 << (quantization_bits as u32)) - 1) as i32;

        let transform =
            PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform::new(max_value);

        let prediction_scheme = Box::new(PredictionSchemeDeltaEncoder::new(transform));
        self.base.set_prediction_scheme(prediction_scheme);

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
