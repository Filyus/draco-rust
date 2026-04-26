use crate::compression_config::EncodedGeometryType;
use crate::encoder_buffer::EncoderBuffer;
use crate::encoder_options::EncoderOptions;
use crate::geometry_attribute::GeometryAttributeType;
use crate::geometry_indices::PointIndex;
use crate::kd_tree_attributes_encoder::KdTreeAttributesEncoder;
use crate::mesh::Mesh;
use crate::point_cloud::PointCloud;
use crate::sequential_integer_attribute_encoder::SequentialIntegerAttributeEncoder;
use crate::sequential_normal_attribute_encoder::SequentialNormalAttributeEncoder;
use crate::status::{DracoError, Status};
use crate::version::{
    has_header_flags, uses_varint_encoding, uses_varint_unique_id,
    DEFAULT_POINT_CLOUD_KD_TREE_VERSION, DEFAULT_POINT_CLOUD_SEQUENTIAL_VERSION,
};

use crate::corner_table::CornerTable;

pub trait GeometryEncoder {
    fn point_cloud(&self) -> Option<&PointCloud>;
    fn mesh(&self) -> Option<&Mesh>;
    fn corner_table(&self) -> Option<&CornerTable>;
    fn options(&self) -> &EncoderOptions;
    fn get_geometry_type(&self) -> EncodedGeometryType;
    fn get_encoding_method(&self) -> Option<i32> {
        None
    }
    fn get_data_to_corner_map(&self) -> Option<&[u32]> {
        None
    }
    fn get_vertex_to_data_map(&self) -> Option<&[i32]> {
        None
    }
}

pub struct PointCloudEncoder {
    point_cloud: Option<PointCloud>,
    options: EncoderOptions,
}

impl GeometryEncoder for PointCloudEncoder {
    fn point_cloud(&self) -> Option<&PointCloud> {
        self.point_cloud.as_ref()
    }

    fn mesh(&self) -> Option<&Mesh> {
        None
    }

    fn corner_table(&self) -> Option<&CornerTable> {
        None
    }

    fn options(&self) -> &EncoderOptions {
        &self.options
    }

    fn get_geometry_type(&self) -> EncodedGeometryType {
        EncodedGeometryType::PointCloud
    }
}

impl Default for PointCloudEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl PointCloudEncoder {
    pub fn new() -> Self {
        Self {
            point_cloud: None,
            options: EncoderOptions::default(),
        }
    }

    pub fn point_cloud(&self) -> Option<&PointCloud> {
        self.point_cloud.as_ref()
    }

    pub fn set_point_cloud(&mut self, pc: PointCloud) {
        self.point_cloud = Some(pc);
    }

    pub fn encode(&mut self, options: &EncoderOptions, out_buffer: &mut EncoderBuffer) -> Status {
        self.options = options.clone();

        if self.point_cloud.is_none() {
            return Err(DracoError::DracoError("Point cloud not set".to_string()));
        }
        let pc = self.point_cloud.as_ref().unwrap();

        let method = self.options.get_encoding_method().unwrap_or(0);

        // 1. Encode Header
        self.encode_header(out_buffer, method);

        if method == 1 {
            // KD-Tree Encoding (Draco v2.3)

            // Encode Geometry Data (Num points)
            // Note: Draco point cloud encodes num_points as fixed u32 for both
            // sequential and KD-tree, NOT as varint (matching decoder).
            out_buffer.encode_u32(pc.num_points() as u32);

            // Generate Attributes Encoders
            // For now, we put all attributes into a single KdTreeAttributesEncoder
            let mut att_encoder = KdTreeAttributesEncoder::new(0);
            for i in 1..pc.num_attributes() {
                att_encoder.add_attribute_id(i);
            }

            // Encode number of attribute encoders
            out_buffer.encode_u8(1); // We have only 1 encoder

            // Init (Transform attributes to portable format)
            if !att_encoder.transform_attributes_to_portable_format(pc, &self.options) {
                return Err(DracoError::DracoError(
                    "Failed to transform attributes".to_string(),
                ));
            }

            // Note: KD-tree encoding does NOT write an encoder type identifier byte.
            // This is different from sequential encoding where each attribute has a decoder type.
            // The decoder knows to use KdTreeAttributesDecoder because the encoding method
            // in the header is 1 (KD-tree).

            // Encode Attributes Encoder Data (Metadata)
            if !att_encoder.encode_attributes_encoder_data(pc, out_buffer) {
                return Err(DracoError::DracoError(
                    "Failed to encode attribute metadata".to_string(),
                ));
            }

            // Encode Attributes (Portable Data)
            if !att_encoder.encode_attributes(pc, &self.options, out_buffer) {
                return Err(DracoError::DracoError(
                    "Failed to encode attributes".to_string(),
                ));
            }

            // Encode Attributes Transform Data
            if !att_encoder.encode_data_needed_by_portable_transforms(out_buffer) {
                return Err(DracoError::DracoError(
                    "Failed to encode attribute transform data".to_string(),
                ));
            }
        } else {
            // Sequential Encoding (Draco v1.3)
            //
            // C++ Structure:
            // 1. num_points (u32)
            // 2. num_attribute_encoders (u8)
            // 3. For each encoder: encoder_identifier (none for sequential - skipped in v1.3)
            // 4. For each encoder: EncodeAttributesEncoderData
            //    - num_attributes_in_encoder (varint for v2+, u32 for v1.x)
            //    - for each attribute: type, data_type, num_components, normalized, unique_id
            // 5. For each attribute: decoder_type (u8)
            // 6. For each attribute: encoded data

            let num_points = pc.num_points();
            let num_attributes = pc.num_attributes();
            let point_ids: Vec<PointIndex> =
                (0..num_points).map(|i| PointIndex(i as u32)).collect();

            // Draco bitstream < 2.0 encodes number of points as a fixed u32.
            out_buffer.encode_u32(num_points as u32);

            // Number of attribute encoders
            // For empty point clouds (0 attributes), we write 0 encoders
            if num_attributes == 0 {
                out_buffer.encode_u8(0);
                return Ok(());
            }

            // For non-empty point clouds, use 1 encoder for all attributes
            out_buffer.encode_u8(1);

            // Encode attributes encoder data:
            // Use the buffer's version (set in encode_header) for version checks
            let major = out_buffer.version_major();
            let minor = out_buffer.version_minor();
            if !uses_varint_encoding(major, minor) {
                out_buffer.encode_u32(num_attributes as u32);
            } else {
                out_buffer.encode_varint(num_attributes as u64);
            }

            // For each attribute, encode metadata
            for i in 0..num_attributes {
                let att = pc.attribute(i);
                out_buffer.encode_u8(att.attribute_type() as u8);
                out_buffer.encode_u8(att.data_type() as u8);
                out_buffer.encode_u8(att.num_components());
                out_buffer.encode_u8(if att.normalized() { 1 } else { 0 });

                if !uses_varint_unique_id(major, minor) {
                    out_buffer.encode_u16(att.unique_id() as u16);
                } else {
                    out_buffer.encode_varint(att.unique_id() as u64);
                }
            }

            // Encode decoder types for each attribute
            // 0 = SEQUENTIAL_ATTRIBUTE_ENCODER_GENERIC
            // 1 = SEQUENTIAL_ATTRIBUTE_ENCODER_INTEGER
            // 2 = SEQUENTIAL_ATTRIBUTE_ENCODER_QUANTIZATION
            // 3 = SEQUENTIAL_ATTRIBUTE_ENCODER_NORMALS
            for i in 0..num_attributes {
                let att = pc.attribute(i);
                if att.attribute_type() == GeometryAttributeType::Normal {
                    out_buffer.encode_u8(3); // NORMALS
                } else {
                    // Use QUANTIZATION if quantization is requested, otherwise GENERIC
                    let quant_bits = self.options.get_attribute_int(i, "quantization_bits", 0);
                    if quant_bits > 0 {
                        out_buffer.encode_u8(2); // QUANTIZATION
                    } else {
                        out_buffer.encode_u8(0); // GENERIC
                    }
                }
            }

            // Encoding follows C++ order:
            // 1. EncodePortableAttributes (encode_values for each attribute)
            // 2. EncodeDataNeededByPortableTransforms (transform params for each attribute)

            // Store encoders so we can call encode_data_needed_by_portable_transform later
            let mut integer_encoders: Vec<Option<SequentialIntegerAttributeEncoder>> =
                Vec::with_capacity(num_attributes as usize);
            let mut normal_encoders: Vec<Option<SequentialNormalAttributeEncoder>> =
                Vec::with_capacity(num_attributes as usize);

            // First pass: encode all values
            for i in 0..num_attributes {
                let att = pc.attribute(i);

                if att.attribute_type() == GeometryAttributeType::Normal {
                    let mut att_encoder = SequentialNormalAttributeEncoder::new();
                    if !att_encoder.init(pc, i, &self.options) {
                        return Err(DracoError::DracoError(format!(
                            "Failed to init normal attribute encoder {}",
                            i
                        )));
                    }

                    if !att_encoder.encode_values(pc, &point_ids, out_buffer, &self.options, self) {
                        return Err(DracoError::DracoError(format!(
                            "Failed to encode attribute {}",
                            i
                        )));
                    }

                    integer_encoders.push(None);
                    normal_encoders.push(Some(att_encoder));
                } else {
                    let quant_bits = self.options.get_attribute_int(i, "quantization_bits", 0);
                    let uses_quantization = quant_bits > 0
                        && (att.data_type() == crate::draco_types::DataType::Float32
                            || att.data_type() == crate::draco_types::DataType::Float64);

                    if uses_quantization {
                        let mut att_encoder = SequentialIntegerAttributeEncoder::new();
                        att_encoder.init(i);

                        if !att_encoder.encode_values(
                            pc,
                            &point_ids,
                            out_buffer,
                            &self.options,
                            self,
                            None,
                            false,
                        ) {
                            return Err(DracoError::DracoError(format!(
                                "Failed to encode attribute {}",
                                i
                            )));
                        }

                        integer_encoders.push(Some(att_encoder));
                    } else {
                        let entry_size = att.byte_stride() as usize;
                        let data = att.buffer().data();
                        for &point_id in &point_ids {
                            let value_index = att.mapped_index(point_id).0 as usize;
                            let offset = value_index.checked_mul(entry_size).ok_or_else(|| {
                                DracoError::DracoError(
                                    "Point cloud raw attribute offset overflow".to_string(),
                                )
                            })?;
                            let end = offset.checked_add(entry_size).ok_or_else(|| {
                                DracoError::DracoError(
                                    "Point cloud raw attribute byte range overflow".to_string(),
                                )
                            })?;
                            if end > data.len() {
                                return Err(DracoError::DracoError(
                                    "Point cloud raw attribute data out of bounds".to_string(),
                                ));
                            }
                            out_buffer.encode_data(&data[offset..end]);
                        }

                        integer_encoders.push(None);
                    }

                    normal_encoders.push(None);
                }
            }

            // Second pass: encode transform parameters (EncodeDataNeededByPortableTransforms)
            for i in 0..num_attributes as usize {
                let att = pc.attribute(i as i32);

                if att.attribute_type() == GeometryAttributeType::Normal {
                    if let Some(ref att_encoder) = normal_encoders[i] {
                        if !att_encoder.encode_data_needed_by_portable_transform(out_buffer) {
                            return Err(DracoError::DracoError(format!(
                                "Failed to encode normal attribute transform data {}",
                                i
                            )));
                        }
                    }
                } else if let Some(ref att_encoder) = integer_encoders[i] {
                    if !att_encoder.encode_data_needed_by_portable_transform(out_buffer) {
                        return Err(DracoError::DracoError(format!(
                            "Failed to encode quantization transform data {}",
                            i
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    fn encode_header(&self, buffer: &mut EncoderBuffer, method: i32) {
        buffer.encode_data(b"DRACO");

        let (mut major, mut minor) = self.options.get_version();
        if major == 0 && minor == 0 {
            if method == 1 {
                (major, minor) = DEFAULT_POINT_CLOUD_KD_TREE_VERSION;
            } else {
                (major, minor) = DEFAULT_POINT_CLOUD_SEQUENTIAL_VERSION;
            }
        }

        buffer.encode_u8(major);
        buffer.encode_u8(minor);
        buffer.set_version(major, minor);

        buffer.encode_u8(self.get_geometry_type() as u8);
        buffer.encode_u8(method as u8);

        if has_header_flags(major, minor) {
            buffer.encode_u16(0); // Flags
        }
    }

    pub fn get_geometry_type(&self) -> EncodedGeometryType {
        EncodedGeometryType::PointCloud
    }
}
