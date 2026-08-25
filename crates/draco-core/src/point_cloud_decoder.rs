use crate::compression_config::EncodedGeometryType;
use crate::corner_table::CornerTable;
#[cfg(feature = "point_cloud_decode")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "point_cloud_decode")]
use crate::draco_types::DataType;
#[cfg(feature = "point_cloud_decode")]
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
#[cfg(feature = "point_cloud_decode")]
use crate::kd_tree_attributes_decoder::KdTreeAttributesDecoder;
use crate::mesh::Mesh;
use crate::point_cloud::PointCloud;
#[cfg(feature = "point_cloud_decode")]
use crate::prediction_scheme::EntryToPointIdMap;
#[cfg(feature = "point_cloud_decode")]
use crate::sequential_integer_attribute_decoder::{
    PortableExtent, SequentialIntegerAttributeDecoder,
};
#[cfg(feature = "point_cloud_decode")]
use crate::status::{DracoError, Status};

#[cfg(feature = "point_cloud_decode")]
use crate::attribute_octahedron_transform::AttributeOctahedronTransform;
#[cfg(feature = "point_cloud_decode")]
use crate::attribute_quantization_transform::AttributeQuantizationTransform;
#[cfg(feature = "point_cloud_decode")]
use crate::attribute_transform::AttributeTransform;
#[cfg(feature = "point_cloud_decode")]
use crate::sequential_generic_attribute_decoder::SequentialGenericAttributeDecoder;
#[cfg(feature = "point_cloud_decode")]
use crate::sequential_normal_attribute_decoder::SequentialNormalAttributeDecoder;
#[cfg(feature = "point_cloud_decode")]
use crate::sequential_quantization_attribute_decoder::SequentialQuantizationAttributeDecoder;
#[cfg(feature = "point_cloud_decode")]
use crate::version::{version_at_least, VERSION_FLAGS_INTRODUCED};

/// Internal geometry context used by attribute decoders.
pub trait GeometryDecoder {
    /// Returns point-cloud geometry when available.
    fn point_cloud(&self) -> Option<&PointCloud>;
    /// Returns mesh geometry when available.
    fn mesh(&self) -> Option<&Mesh>;
    /// Returns mesh corner-table topology when available.
    fn corner_table(&self) -> Option<&CornerTable>;
    /// Returns the encoded geometry type.
    fn get_geometry_type(&self) -> EncodedGeometryType;
    /// Returns the attribute encoding method for an attribute id, if known.
    fn get_attribute_encoding_method(&self, _att_id: i32) -> Option<i32> {
        None
    }
}

/// Whether a prediction transform byte follows this prediction method byte.
///
/// Upstream writes the transform only when the method is not `PREDICTION_NONE`,
/// which is `-2` and reaches the stream as `0xFE`. `0xFF` is `-1`, which this
/// crate once wrote for the same meaning, so both are read as "nothing follows"
/// -- the same pair `SequentialIntegerAttributeDecoder` accepts.
///
/// The pre-1.2 shims below need this because they walk the prediction header by
/// hand to reach the quantization parameters behind it. Testing `0xFF` alone
/// made them step one byte into a `PREDICTION_NONE` stream and read the
/// parameters shifted: the range came out zero and every position dequantized to
/// the origin, with the point and face counts still right and the decode still
/// reporting success. Draco writes `PREDICTION_NONE` at compression level 0.
/// Gated on the feature its callers live behind, and only that one: both of
/// them are now the pre-2.0 shims inside the shared normal and quantization
/// decoders, so a `point_cloud_decode` build without legacy support compiles
/// neither and would carry this as dead code.
#[cfg(feature = "legacy_bitstream_decode")]
pub(crate) fn carries_transform_byte(method_byte: u8) -> bool {
    method_byte != 0xFF && method_byte != 0xFE
}

/// Decoder for Draco point cloud bitstreams.
///
/// `PointCloudDecoder` reads a point-cloud `.drc` bitstream and reconstructs a
/// [`PointCloud`] with its attributes and metadata. Both
/// KD-tree and sequential attribute encodings are supported (the actual decode
/// requires the `point_cloud_decode` feature).
///
/// A round trip is shown on the `PointCloudEncoder` type docs.
pub struct PointCloudDecoder {
    geometry_type: EncodedGeometryType,
    #[cfg(feature = "point_cloud_decode")]
    method: u8,
    #[cfg(feature = "point_cloud_decode")]
    flags: u16,
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
        return Err(DracoError::general(
            "Invalid number of attributes".to_string(),
        ));
    }
    Ok(())
}

#[cfg(feature = "point_cloud_decode")]
fn validate_num_components(num_components: u8) -> Result<(), DracoError> {
    if num_components == 0 {
        return Err(DracoError::general(
            "Invalid attribute component count".to_string(),
        ));
    }
    Ok(())
}

impl PointCloudDecoder {
    /// Creates a point cloud decoder with default state.
    pub fn new() -> Self {
        Self {
            geometry_type: EncodedGeometryType::PointCloud,
            #[cfg(feature = "point_cloud_decode")]
            method: 0,
            #[cfg(feature = "point_cloud_decode")]
            flags: 0,
            #[cfg(feature = "point_cloud_decode")]
            version_major: 0,
            #[cfg(feature = "point_cloud_decode")]
            version_minor: 0,
        }
    }

    #[cfg(feature = "point_cloud_decode")]
    /// Decodes a Draco point cloud from `in_buffer` into `out_pc`.
    ///
    /// # Errors
    ///
    /// Returns an error if the header is invalid, the bitstream version is
    /// unsupported, or the encoded attributes are malformed.
    pub fn decode(&mut self, in_buffer: &mut DecoderBuffer, out_pc: &mut PointCloud) -> Status {
        // 1. Decode Header
        self.decode_header(in_buffer)?;

        if version_at_least(
            self.version_major,
            self.version_minor,
            VERSION_FLAGS_INTRODUCED,
        ) && (self.flags & crate::metadata::METADATA_FLAG_MASK) != 0
        {
            let metadata = crate::metadata::GeometryMetadata::decode(in_buffer)
                .map_err(|_| DracoError::general("Failed to decode metadata".to_string()))?;
            out_pc.set_metadata(Some(metadata));
        }

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
        self.flags = 0;
        self.geometry_type = EncodedGeometryType::PointCloud;
        self.decode_geometry_data(buffer, out_pc)
    }

    #[cfg(feature = "point_cloud_decode")]
    fn decode_header(&mut self, buffer: &mut DecoderBuffer) -> Status {
        let mut magic = [0u8; 5];
        buffer.decode_bytes(&mut magic)?;
        if &magic != b"DRACO" {
            return Err(DracoError::general("Invalid magic".to_string()));
        }

        self.version_major = buffer.decode_u8()?;
        self.version_minor = buffer.decode_u8()?;
        buffer.set_version(self.version_major, self.version_minor);

        let g_type = buffer.decode_u8()?;
        self.geometry_type = match g_type {
            0 => EncodedGeometryType::PointCloud,
            1 => EncodedGeometryType::TriangularMesh,
            _ => return Err(DracoError::general("Invalid geometry type".to_string())),
        };
        if self.geometry_type != EncodedGeometryType::PointCloud {
            return Err(DracoError::general(
                "PointCloudDecoder cannot decode mesh bitstreams".to_string(),
            ));
        }

        self.method = buffer.decode_u8()?;

        // Flags field is always present in the binary header (C++ reads unconditionally).
        self.flags = buffer
            .decode_u16()
            .map_err(|_| DracoError::general("Failed to decode flags".to_string()))?;

        Ok(())
    }

    #[cfg(feature = "point_cloud_decode")]
    fn decode_geometry_data(&mut self, buffer: &mut DecoderBuffer, pc: &mut PointCloud) -> Status {
        let bitstream_version: u16 =
            crate::version::bitstream_version(self.version_major, self.version_minor);
        // Note: Draco point cloud bitstreams encode the number of points as a
        // fixed-width int32 for both sequential (method=0) and KD-tree
        // (method=1) encodings (see C++ PointCloudSequentialDecoder and
        // PointCloudKdTreeDecoder). It is NOT varint encoded, even for v2.x.
        let num_points: usize = buffer.decode_u32()? as usize;
        // No count guard here, matching upstream: `PointCloudSequentialDecoder`
        // and `PointCloudKdTreeDecoder` read the count and use it, checking only
        // that it is not negative. What bounds the work is the allocation budget
        // applied where the buffers are actually sized - see `decode_budget`.
        pc.set_num_points(num_points);

        let num_attributes_decoders = buffer.decode_u8()? as usize;

        if self.method == 1 {
            // KD-tree encoding.
            for _ in 0..num_attributes_decoders {
                let mut att_decoder = KdTreeAttributesDecoder::new(0);
                att_decoder
                    .decode_attributes_decoder_data(pc, buffer)
                    .map_err(|err| {
                        DracoError::general(format!("Failed to decode attribute metadata: {err}"))
                    })?;
                att_decoder.decode_attributes(pc, buffer).map_err(|err| {
                    DracoError::general(format!("Failed to decode attributes: {err}"))
                })?;
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
                    return Err(DracoError::general(
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
                            DracoError::general(
                                "Raw point cloud attribute byte count overflow".to_string(),
                            )
                        })?;
                        if buffer.remaining_size() < bytes_needed {
                            return Err(DracoError::general(
                                "Not enough data for raw point cloud attribute values".to_string(),
                            ));
                        }
                    }

                    let mut att = PointAttribute::new();
                    // Nothing is charged for this attribute, because nothing is
                    // taken for it: the buffer is left unreserved and sized by
                    // whichever decoder writes the values, once they exist. A
                    // charge here would be for an allocation that no longer
                    // happens, and it is not free -- the budget is a backstop
                    // against unbacked reservations, and billing it for backed
                    // ones is what made it refuse files this crate writes.
                    att.init_deferred(
                        spec.att_type,
                        spec.num_components,
                        spec.data_type,
                        spec.normalized,
                        num_points,
                    )?;
                    att.set_unique_id(spec.unique_id);
                    let att_id = pc.add_attribute_preserve_unique_id(att);
                    att_ids.push(att_id);
                }

                // The identity, and not written out. Entry `i` is point `i`
                // here, so materializing it bought nothing and cost four bytes
                // per point of a count the header supplies -- 134 MB from a
                // 9 KB stream on one fuzz artifact, and gigabytes on a bigger
                // claim. See `EntryToPointIdMap::Identity`.
                let point_ids = if decoder_types.iter().any(|&decoder_type| decoder_type != 0) {
                    Some(EntryToPointIdMap::identity(num_points))
                } else {
                    None
                };

                for (local_i, &att_id) in att_ids.iter().enumerate() {
                    let decoder_type = decoder_types[local_i];
                    match decoder_type {
                        1 => {
                            let point_ids = point_ids.ok_or_else(|| {
                                DracoError::general(
                                    "Point ids missing for integer attribute decoder".to_string(),
                                )
                            })?;
                            let mut att_decoder = SequentialIntegerAttributeDecoder::new();
                            att_decoder.init(self, att_id);
                            att_decoder.decode_values(
                                pc, point_ids, buffer, None, None, None, None, None, None,
                            )?;
                        }
                        2 => {
                            let mut att_decoder = SequentialQuantizationAttributeDecoder::new();
                            att_decoder.init(self, pc, att_id)?;
                            let portable = att_decoder.decode_values(
                                pc,
                                point_ids.ok_or_else(|| {
                                    DracoError::general(
                                        "Point ids missing for quantized attribute decoder"
                                            .to_string(),
                                    )
                                })?,
                                buffer,
                                bitstream_version,
                                PortableExtent::Declared(num_points),
                                None,
                                None,
                                None,
                                None,
                            )?;
                            pending_quant.push(PendingQuant {
                                att_id,
                                portable,
                                transform: att_decoder.into_transform(),
                            });
                        }
                        3 => {
                            let mut att_decoder = SequentialNormalAttributeDecoder::new();
                            att_decoder.init(self, pc, att_id)?;
                            let portable = att_decoder.decode_values(
                                pc,
                                point_ids.ok_or_else(|| {
                                    DracoError::general(
                                        "Point ids missing for normal attribute decoder"
                                            .to_string(),
                                    )
                                })?,
                                buffer,
                                bitstream_version,
                                PortableExtent::Declared(num_points),
                                None,
                                None,
                                None,
                                None,
                            )?;
                            pending_normals.push(PendingNormal {
                                att_id,
                                portable,
                                quantization_bits: att_decoder.quantization_bits(),
                            });
                        }
                        0 => {
                            // The identity map costs nothing to build and is
                            // all this decoder reads off it -- the values are
                            // copied verbatim, in order -- so the arm does not
                            // need the shared `point_ids`, which is `None` when
                            // every attribute is generic.
                            let mut att_decoder = SequentialGenericAttributeDecoder::new();
                            att_decoder.init(self, att_id);
                            att_decoder.decode_values(
                                pc,
                                EntryToPointIdMap::identity(num_points),
                                buffer,
                            )?;
                        }
                        _ => {
                            return Err(DracoError::general(format!(
                                "Unsupported sequential decoder type: {}",
                                decoder_type
                            )));
                        }
                    }
                }

                for (local_i, &att_id) in att_ids.iter().enumerate() {
                    match decoder_types[local_i] {
                        2 if bitstream_version >= 0x0200 => {
                            let idx = pending_quant
                                .iter()
                                .position(|p| p.att_id == att_id)
                                .ok_or_else(|| {
                                    DracoError::general(
                                        "Missing pending quantized attribute transform".to_string(),
                                    )
                                })?;
                            let original = pc.try_attribute(att_id)?;
                            pending_quant[idx]
                                .transform
                                .decode_parameters(original, buffer)
                                .map_err(|e| {
                                    DracoError::general(format!(
                                        "Failed to decode quantization parameters: {e}"
                                    ))
                                })?;
                        }
                        3 if bitstream_version >= 0x0200 => {
                            let idx = pending_normals
                                .iter()
                                .position(|p| p.att_id == att_id)
                                .ok_or_else(|| {
                                    DracoError::general(
                                        "Missing pending normal attribute transform".to_string(),
                                    )
                                })?;
                            let quantization_bits = buffer.decode_u8()?;
                            if !AttributeOctahedronTransform::is_valid_quantization_bits(
                                quantization_bits as i32,
                            ) {
                                return Err(DracoError::general(
                                    "Invalid normal quantization bits".to_string(),
                                ));
                            }
                            pending_normals[idx].quantization_bits = quantization_bits;
                        }
                        _ => {}
                    }
                }

                for q in pending_quant {
                    let dst = pc.try_attribute_mut(q.att_id)?;
                    q.transform
                        .inverse_transform_attribute(&q.portable, dst)
                        .map_err(|e| {
                            DracoError::general(format!("Failed to dequantize attribute: {e}"))
                        })?;
                }
                for n in pending_normals {
                    let mut oct = AttributeOctahedronTransform::new(-1);
                    oct.set_parameters(n.quantization_bits as i32)?;
                    let dst = pc.try_attribute_mut(n.att_id)?;
                    oct.inverse_transform_attribute_with_legacy_octahedron(
                        &n.portable,
                        dst,
                        bitstream_version < 0x0200,
                    )
                    .map_err(|e| DracoError::general(format!("Failed to decode normals: {e}")))?;
                }
            }
        }

        Ok(())
    }

    /// Returns the encoded geometry type handled by this decoder.
    pub fn get_geometry_type(&self) -> EncodedGeometryType {
        self.geometry_type
    }
}
