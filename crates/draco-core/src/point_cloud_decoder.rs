use crate::compression_config::EncodedGeometryType;
use crate::corner_table::CornerTable;
#[cfg(feature = "point_cloud_decode")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "point_cloud_decode")]
use crate::draco_types::DataType;
#[cfg(feature = "point_cloud_decode")]
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
#[cfg(feature = "point_cloud_decode")]
use crate::geometry_indices::PointIndex;
#[cfg(feature = "point_cloud_decode")]
use crate::kd_tree_attributes_decoder::KdTreeAttributesDecoder;
use crate::mesh::Mesh;
use crate::point_cloud::PointCloud;
#[cfg(feature = "point_cloud_decode")]
use crate::sequential_integer_attribute_decoder::SequentialIntegerAttributeDecoder;
#[cfg(feature = "point_cloud_decode")]
use crate::status::{DracoError, Status};

#[cfg(feature = "point_cloud_decode")]
use crate::attribute_octahedron_transform::AttributeOctahedronTransform;
#[cfg(feature = "point_cloud_decode")]
use crate::attribute_quantization_transform::AttributeQuantizationTransform;
#[cfg(feature = "point_cloud_decode")]
use crate::attribute_transform::AttributeTransform;

pub trait GeometryDecoder {
    fn point_cloud(&self) -> Option<&PointCloud>;
    fn mesh(&self) -> Option<&Mesh>;
    fn corner_table(&self) -> Option<&CornerTable>;
    fn get_geometry_type(&self) -> EncodedGeometryType;
    fn get_attribute_encoding_method(&self, _att_id: i32) -> Option<i32> {
        None
    }
}

pub struct PointCloudDecoder {
    geometry_type: EncodedGeometryType,
    #[cfg(feature = "point_cloud_decode")]
    method: u8,
    #[cfg(feature = "point_cloud_decode")]
    version_major: u8,
    #[cfg(feature = "point_cloud_decode")]
    version_minor: u8,
}

impl GeometryDecoder for PointCloudDecoder {
    fn point_cloud(&self) -> Option<&PointCloud> {
        None // PointCloudDecoder constructs PointCloud, doesn't hold it?
             // Actually decode takes &mut PointCloud.
             // So we can't return it here easily unless we store it.
             // But GeometryDecoder is usually passed to attribute decoders.
             // Attribute decoders take PointCloud as argument.
    }

    fn mesh(&self) -> Option<&Mesh> {
        None
    }

    fn corner_table(&self) -> Option<&CornerTable> {
        None
    }

    fn get_geometry_type(&self) -> EncodedGeometryType {
        self.geometry_type
    }
}

impl Default for PointCloudDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "point_cloud_decode")]
fn make_point_ids(num_points: usize) -> Result<Vec<PointIndex>, DracoError> {
    let mut point_ids = Vec::new();
    point_ids
        .try_reserve_exact(num_points)
        .map_err(|_| DracoError::DracoError("Failed to allocate point ids".to_string()))?;
    for i in 0..num_points {
        point_ids.push(PointIndex(i as u32));
    }
    Ok(point_ids)
}

#[cfg(feature = "point_cloud_decode")]
fn validate_num_attributes_in_decoder(
    num_attributes_in_decoder: usize,
    remaining_bytes: usize,
) -> Result<(), DracoError> {
    // Each attribute must have at least type, data type, component count,
    // normalized flag, unique id, and a decoder type byte. Reject impossible
    // counts before reserving vectors from untrusted input.
    const MIN_ATTRIBUTE_BYTES: usize = 6;
    if num_attributes_in_decoder == 0
        || num_attributes_in_decoder > remaining_bytes / MIN_ATTRIBUTE_BYTES
    {
        return Err(DracoError::DracoError(
            "Invalid number of attributes".to_string(),
        ));
    }
    Ok(())
}

#[cfg(feature = "point_cloud_decode")]
fn validate_num_components(num_components: u8) -> Result<(), DracoError> {
    if num_components == 0 {
        return Err(DracoError::DracoError(
            "Invalid attribute component count".to_string(),
        ));
    }
    Ok(())
}

#[cfg(feature = "point_cloud_decode")]
fn decode_raw_attribute_values(
    buffer: &mut DecoderBuffer<'_>,
    attribute: &mut PointAttribute,
    num_points: usize,
) -> Result<(), DracoError> {
    let entry_size = attribute.byte_stride() as usize;
    if entry_size == 0 {
        return Err(DracoError::DracoError(
            "Invalid point cloud attribute entry size".to_string(),
        ));
    }
    let required_size = entry_size.checked_mul(num_points).ok_or_else(|| {
        DracoError::DracoError("Point cloud raw attribute byte count overflow".to_string())
    })?;

    let dst = attribute.buffer_mut().data_mut();
    if dst.len() < required_size {
        return Err(DracoError::DracoError(
            "Point cloud attribute buffer too small".to_string(),
        ));
    }

    for chunk in dst[..required_size].chunks_exact_mut(entry_size) {
        buffer.decode_bytes(chunk).map_err(|_| {
            DracoError::DracoError("Failed to decode raw point cloud attribute values".to_string())
        })?;
    }

    Ok(())
}

impl PointCloudDecoder {
    pub fn new() -> Self {
        Self {
            geometry_type: EncodedGeometryType::PointCloud,
            #[cfg(feature = "point_cloud_decode")]
            method: 0,
            #[cfg(feature = "point_cloud_decode")]
            version_major: 0,
            #[cfg(feature = "point_cloud_decode")]
            version_minor: 0,
        }
    }

    #[cfg(feature = "point_cloud_decode")]
    pub fn decode(&mut self, in_buffer: &mut DecoderBuffer, out_pc: &mut PointCloud) -> Status {
        // 1. Decode Header
        self.decode_header(in_buffer)?;

        // 2. Decode Geometry Data
        self.decode_geometry_data(in_buffer, out_pc)
    }

    /// Decode point cloud data when header + metadata have already been parsed.
    /// Used by MeshDecoder to delegate point cloud streams.
    #[cfg(feature = "point_cloud_decode")]
    pub fn decode_after_header(
        &mut self,
        version_major: u8,
        version_minor: u8,
        method: u8,
        buffer: &mut DecoderBuffer,
        out_pc: &mut PointCloud,
    ) -> Status {
        self.version_major = version_major;
        self.version_minor = version_minor;
        self.method = method;
        self.geometry_type = EncodedGeometryType::PointCloud;
        self.decode_geometry_data(buffer, out_pc)
    }

    #[cfg(feature = "point_cloud_decode")]
    fn decode_header(&mut self, buffer: &mut DecoderBuffer) -> Status {
        let mut magic = [0u8; 5];
        buffer.decode_bytes(&mut magic)?;
        if &magic != b"DRACO" {
            return Err(DracoError::DracoError("Invalid magic".to_string()));
        }

        self.version_major = buffer.decode_u8()?;
        self.version_minor = buffer.decode_u8()?;
        buffer.set_version(self.version_major, self.version_minor);

        let g_type = buffer.decode_u8()?;
        self.geometry_type = match g_type {
            0 => EncodedGeometryType::PointCloud,
            1 => EncodedGeometryType::TriangularMesh,
            _ => return Err(DracoError::DracoError("Invalid geometry type".to_string())),
        };

        self.method = buffer.decode_u8()?;

        // Flags field is always present in the binary header (C++ reads unconditionally).
        let _flags = buffer
            .decode_u16()
            .map_err(|_| DracoError::DracoError("Failed to decode flags".to_string()))?;

        Ok(())
    }

    #[cfg(feature = "point_cloud_decode")]
    fn decode_geometry_data(&mut self, buffer: &mut DecoderBuffer, pc: &mut PointCloud) -> Status {
        let bitstream_version: u16 =
            ((self.version_major as u16) << 8) | (self.version_minor as u16);
        // Note: Draco point cloud bitstreams encode the number of points as a
        // fixed-width int32 for both sequential (method=0) and KD-tree
        // (method=1) encodings (see C++ PointCloudSequentialDecoder and
        // PointCloudKdTreeDecoder). It is NOT varint encoded, even for v2.x.
        let num_points: usize = buffer.decode_u32()? as usize;
        pc.set_num_points(num_points);

        let num_attributes_decoders = buffer.decode_u8()? as usize;

        if self.method == 1 {
            // KD-tree encoding.
            for _ in 0..num_attributes_decoders {
                let mut att_decoder = KdTreeAttributesDecoder::new(0);
                if !att_decoder.decode_attributes_decoder_data(pc, buffer) {
                    return Err(DracoError::DracoError(
                        "Failed to decode attribute metadata".to_string(),
                    ));
                }
                if !att_decoder.decode_attributes(pc, buffer) {
                    return Err(DracoError::DracoError(
                        "Failed to decode attributes".to_string(),
                    ));
                }
            }
        } else {
            // Sequential encoding.
            struct PendingQuant {
                att_id: i32,
                portable: PointAttribute,
                transform: AttributeQuantizationTransform,
            }

            struct PendingNormal {
                att_id: i32,
                portable: PointAttribute,
                quantization_bits: u8,
            }

            struct AttributeSpec {
                att_type: GeometryAttributeType,
                data_type: DataType,
                num_components: u8,
                normalized: bool,
                unique_id: u32,
            }

            for _ in 0..num_attributes_decoders {
                let num_attributes_in_decoder: usize = if bitstream_version < 0x0200 {
                    buffer.decode_u32()? as usize
                } else {
                    buffer.decode_varint()? as usize
                };
                if num_attributes_in_decoder == 0 {
                    return Err(DracoError::DracoError(
                        "Invalid number of attributes".to_string(),
                    ));
                }
                validate_num_attributes_in_decoder(
                    num_attributes_in_decoder,
                    buffer.remaining_size(),
                )?;

                let mut attribute_specs: Vec<AttributeSpec> =
                    Vec::with_capacity(num_attributes_in_decoder);
                let mut att_ids: Vec<i32> = Vec::with_capacity(num_attributes_in_decoder);
                let mut decoder_types: Vec<u8> = Vec::with_capacity(num_attributes_in_decoder);
                let mut pending_quant: Vec<PendingQuant> = Vec::new();
                let mut pending_normals: Vec<PendingNormal> = Vec::new();

                for _ in 0..num_attributes_in_decoder {
                    let att_type_val = buffer.decode_u8()?;
                    let att_type = GeometryAttributeType::try_from(att_type_val)?;

                    let data_type_val = buffer.decode_u8()?;
                    let data_type = DataType::try_from(data_type_val)?;

                    let num_components = buffer.decode_u8()?;
                    validate_num_components(num_components)?;
                    let normalized = buffer.decode_u8()? != 0;
                    let unique_id: u32 = if bitstream_version < 0x0103 {
                        buffer.decode_u16()? as u32
                    } else {
                        buffer.decode_varint()? as u32
                    };

                    attribute_specs.push(AttributeSpec {
                        att_type,
                        data_type,
                        num_components,
                        normalized,
                        unique_id,
                    });
                }

                for _ in 0..num_attributes_in_decoder {
                    decoder_types.push(buffer.decode_u8()?);
                }

                for (local_i, spec) in attribute_specs.iter().enumerate() {
                    if decoder_types[local_i] == 0 {
                        let entry_size =
                            spec.num_components as usize * spec.data_type.byte_length();
                        let bytes_needed = entry_size.checked_mul(num_points).ok_or_else(|| {
                            DracoError::DracoError(
                                "Raw point cloud attribute byte count overflow".to_string(),
                            )
                        })?;
                        if buffer.remaining_size() < bytes_needed {
                            return Err(DracoError::DracoError(
                                "Not enough data for raw point cloud attribute values".to_string(),
                            ));
                        }
                    }

                    let mut att = PointAttribute::new();
                    att.try_init(
                        spec.att_type,
                        spec.num_components,
                        spec.data_type,
                        spec.normalized,
                        num_points,
                    )?;
                    att.set_unique_id(spec.unique_id);
                    let att_id = pc.add_attribute(att);
                    att_ids.push(att_id);
                }

                let point_ids = if decoder_types.iter().any(|&decoder_type| decoder_type != 0) {
                    Some(make_point_ids(num_points)?)
                } else {
                    None
                };

                for (local_i, &att_id) in att_ids.iter().enumerate() {
                    let decoder_type = decoder_types[local_i];
                    match decoder_type {
                        1 => {
                            let point_ids = point_ids.as_ref().ok_or_else(|| {
                                DracoError::DracoError(
                                    "Point ids missing for integer attribute decoder".to_string(),
                                )
                            })?;
                            let mut att_decoder = SequentialIntegerAttributeDecoder::new();
                            att_decoder.init(self, att_id);
                            if !att_decoder.decode_values(
                                pc, point_ids, buffer, None, None, None, None, None, None,
                            ) {
                                return Err(DracoError::DracoError(
                                    "Failed to decode integer attribute".to_string(),
                                ));
                            }
                        }
                        2 => {
                            let original = pc.try_attribute(att_id)?;
                            let (original_type, original_num_components) =
                                (original.attribute_type(), original.num_components());
                            let mut portable = PointAttribute::default();
                            portable.try_init(
                                original_type,
                                original_num_components,
                                DataType::Uint32,
                                false,
                                num_points,
                            )?;
                            let mut transform = AttributeQuantizationTransform::new();

                            // Legacy compatibility shim: C++ bitstreams with version <= 1.1
                            // store quantization params before integer values in the stream.
                            // v1.2+ (including Rust-generated v1.3) stores them after.
                            let quant_skip_bytes = if bitstream_version < 0x0102 {
                                let saved_pos = buffer.position();
                                let method_byte = buffer.decode_u8().map_err(|_| {
                                    DracoError::DracoError("read pred method".to_string())
                                })?;
                                if method_byte != 0xFF {
                                    let _transform_byte = buffer.decode_u8().map_err(|_| {
                                        DracoError::DracoError("read transform".to_string())
                                    })?;
                                }
                                let original = pc.try_attribute(att_id)?;
                                if !transform.decode_parameters(original, buffer) {
                                    return Err(DracoError::DracoError(
                                        "Failed to decode quantization parameters (v<2.0)"
                                            .to_string(),
                                    ));
                                }
                                let bytes_consumed = buffer.position() - saved_pos;
                                let pred_header_bytes = if method_byte != 0xFF { 2 } else { 1 };
                                let skip = bytes_consumed - pred_header_bytes;
                                buffer
                                    .set_position(saved_pos)
                                    .map_err(|_| DracoError::DracoError("buf reset".to_string()))?;
                                skip
                            } else {
                                0
                            };
                            let mut att_decoder = SequentialIntegerAttributeDecoder::new();
                            att_decoder.init(self, att_id);
                            let mut skip_fn =
                                move |buf: &mut crate::decoder_buffer::DecoderBuffer<'_>| -> bool {
                                    if quant_skip_bytes > 0 {
                                        if buf.try_advance(quant_skip_bytes).is_err() {
                                            return false;
                                        }
                                    }
                                    true
                                };
                            let hook: Option<
                                &mut dyn FnMut(
                                    &mut crate::decoder_buffer::DecoderBuffer<'_>,
                                ) -> bool,
                            > = if quant_skip_bytes > 0 {
                                Some(&mut skip_fn)
                            } else {
                                None
                            };
                            if !att_decoder.decode_values(
                                pc,
                                point_ids.as_ref().ok_or_else(|| {
                                    DracoError::DracoError(
                                        "Point ids missing for quantized attribute decoder"
                                            .to_string(),
                                    )
                                })?,
                                buffer,
                                None,
                                None,
                                None,
                                Some(&mut portable),
                                None,
                                hook,
                            ) {
                                return Err(DracoError::DracoError(
                                    "Failed to decode quantized portable values".to_string(),
                                ));
                            }
                            pending_quant.push(PendingQuant {
                                att_id,
                                portable,
                                transform,
                            });
                        }
                        3 => {
                            let mut portable = PointAttribute::default();
                            portable.try_init(
                                GeometryAttributeType::Generic,
                                2,
                                DataType::Uint32,
                                false,
                                num_points,
                            )?;
                            // Legacy compatibility shim: C++ bitstreams with version <= 1.1
                            // store octahedron quantization bits after the prediction header
                            // but before integer values. v1.2+ stores them after.
                            let mut quant_bits: u8 = 0;
                            let normal_skip_bytes = if bitstream_version < 0x0102 {
                                let saved_pos = buffer.position();
                                let method_byte = buffer.decode_u8().map_err(|_| {
                                    DracoError::DracoError("read pred method".to_string())
                                })?;
                                if method_byte != 0xFF {
                                    let _transform_byte = buffer.decode_u8().map_err(|_| {
                                        DracoError::DracoError("read transform".to_string())
                                    })?;
                                }
                                quant_bits = buffer.decode_u8().map_err(|_| {
                                    DracoError::DracoError("read normal quant_bits".to_string())
                                })?;
                                if !AttributeOctahedronTransform::is_valid_quantization_bits(
                                    quant_bits as i32,
                                ) {
                                    return Err(DracoError::DracoError(
                                        "Invalid normal quantization bits".to_string(),
                                    ));
                                }
                                let bytes_consumed = buffer.position() - saved_pos;
                                let pred_header_bytes = if method_byte != 0xFF { 2 } else { 1 };
                                let skip = bytes_consumed - pred_header_bytes;
                                buffer
                                    .set_position(saved_pos)
                                    .map_err(|_| DracoError::DracoError("buf reset".to_string()))?;
                                skip
                            } else {
                                0
                            };
                            let mut att_decoder = SequentialIntegerAttributeDecoder::new();
                            att_decoder.init(self, att_id);
                            let mut skip_fn =
                                move |buf: &mut crate::decoder_buffer::DecoderBuffer<'_>| -> bool {
                                    if normal_skip_bytes > 0 {
                                        if buf.try_advance(normal_skip_bytes).is_err() {
                                            return false;
                                        }
                                    }
                                    true
                                };
                            let hook: Option<
                                &mut dyn FnMut(
                                    &mut crate::decoder_buffer::DecoderBuffer<'_>,
                                ) -> bool,
                            > = if normal_skip_bytes > 0 {
                                Some(&mut skip_fn)
                            } else {
                                None
                            };
                            if !att_decoder.decode_values(
                                pc,
                                point_ids.as_ref().ok_or_else(|| {
                                    DracoError::DracoError(
                                        "Point ids missing for normal attribute decoder"
                                            .to_string(),
                                    )
                                })?,
                                buffer,
                                None,
                                None,
                                None,
                                Some(&mut portable),
                                None,
                                hook,
                            ) {
                                return Err(DracoError::DracoError(
                                    "Failed to decode normal portable values".to_string(),
                                ));
                            }
                            pending_normals.push(PendingNormal {
                                att_id,
                                portable,
                                quantization_bits: quant_bits,
                            });
                        }
                        0 => {
                            // Generic sequential values (raw), matching C++
                            // SequentialAttributeDecoder::DecodeValues().
                            decode_raw_attribute_values(
                                buffer,
                                pc.try_attribute_mut(att_id)?,
                                num_points,
                            )?;
                        }
                        _ => {
                            return Err(DracoError::DracoError(format!(
                                "Unsupported sequential decoder type: {}",
                                decoder_type
                            )));
                        }
                    }
                }

                for (local_i, &att_id) in att_ids.iter().enumerate() {
                    match decoder_types[local_i] {
                        2 => {
                            if bitstream_version >= 0x0102 {
                                let idx = pending_quant
                                    .iter()
                                    .position(|p| p.att_id == att_id)
                                    .ok_or_else(|| {
                                        DracoError::DracoError(
                                            "Missing pending quantized attribute transform"
                                                .to_string(),
                                        )
                                    })?;
                                let original = pc.try_attribute(att_id)?;
                                if !pending_quant[idx]
                                    .transform
                                    .decode_parameters(original, buffer)
                                {
                                    return Err(DracoError::DracoError(
                                        "Failed to decode quantization parameters".to_string(),
                                    ));
                                }
                            }
                        }
                        3 => {
                            if bitstream_version >= 0x0102 {
                                let idx = pending_normals
                                    .iter()
                                    .position(|p| p.att_id == att_id)
                                    .ok_or_else(|| {
                                    DracoError::DracoError(
                                        "Missing pending normal attribute transform".to_string(),
                                    )
                                })?;
                                let quantization_bits = buffer.decode_u8()?;
                                if !AttributeOctahedronTransform::is_valid_quantization_bits(
                                    quantization_bits as i32,
                                ) {
                                    return Err(DracoError::DracoError(
                                        "Invalid normal quantization bits".to_string(),
                                    ));
                                }
                                pending_normals[idx].quantization_bits = quantization_bits;
                            }
                        }
                        _ => {}
                    }
                }

                for q in pending_quant {
                    let dst = pc.try_attribute_mut(q.att_id)?;
                    if !q.transform.inverse_transform_attribute(&q.portable, dst) {
                        return Err(DracoError::DracoError(
                            "Failed to dequantize attribute".to_string(),
                        ));
                    }
                }
                for n in pending_normals {
                    let mut oct = AttributeOctahedronTransform::new(-1);
                    if !oct.set_parameters(n.quantization_bits as i32) {
                        return Err(DracoError::DracoError(
                            "Invalid normal quantization bits".to_string(),
                        ));
                    }
                    let dst = pc.try_attribute_mut(n.att_id)?;
                    if !oct.inverse_transform_attribute(&n.portable, dst) {
                        return Err(DracoError::DracoError(
                            "Failed to decode normals".to_string(),
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    pub fn get_geometry_type(&self) -> EncodedGeometryType {
        self.geometry_type
    }
}

#[cfg(all(test, feature = "point_cloud_decode"))]
mod tests {
    use super::*;

    #[test]
    fn decode_raw_attribute_values_rejects_required_size_overflow() {
        let bytes = [];
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Generic,
            1,
            DataType::Uint32,
            false,
            1,
        );

        let status = decode_raw_attribute_values(&mut buffer, &mut attribute, usize::MAX);

        assert!(status.is_err());
    }

    #[test]
    fn decode_raw_attribute_values_rejects_truncated_input() {
        let bytes = [1u8, 2, 3];
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Generic,
            1,
            DataType::Uint32,
            false,
            1,
        );

        let status = decode_raw_attribute_values(&mut buffer, &mut attribute, 1);

        assert!(status.is_err());
    }
}
