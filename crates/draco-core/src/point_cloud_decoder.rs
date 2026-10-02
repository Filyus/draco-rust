use crate::compression_config::EncodedGeometryType;
#[cfg(feature = "point_cloud_decode")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "point_cloud_decode")]
use crate::draco_types::DataType;
#[cfg(feature = "point_cloud_decode")]
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
#[cfg(feature = "point_cloud_decode")]
use crate::kd_tree_attributes_decoder::KdTreeAttributesDecoder;
#[cfg(feature = "point_cloud_decode")]
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
use crate::symbol_encoding::decode_raw_symbol_pair;

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
    /// Ungated, unlike the fields above: `bitstream_version` is read by the
    /// attribute decoders on both the mesh and the point-cloud path, and the
    /// mesh path exists without `point_cloud_decode`.
    version_major: u8,
    version_minor: u8,
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
            version_major: 0,
            version_minor: 0,
        }
    }

    /// The packed bitstream version (`0xMMmm`), `0` before a header was read.
    ///
    /// The attribute decoders read it when they bind prediction parents, which
    /// is upstream's `decoder_->bitstream_version()` inside
    /// `InitPredictionScheme`.
    pub(crate) fn bitstream_version(&self) -> u16 {
        crate::version::bitstream_version(self.version_major, self.version_minor)
    }

    /// Carries the version to a decoder that did not read the header itself.
    ///
    /// The mesh path parses its own header and then hands attributes to these
    /// decoders through a `PointCloudDecoder` it constructs on the spot, so
    /// without this that decoder reports version zero -- and every parent
    /// binding on the fallback path would read as pre-2.0.
    pub(crate) fn set_bitstream_version(&mut self, major: u8, minor: u8) {
        self.version_major = major;
        self.version_minor = minor;
    }

    #[cfg(feature = "point_cloud_decode")]
    /// Decodes a Draco point cloud from `in_buffer` into `out_pc`.
    ///
    /// `out_pc` need not be empty: whatever it held is replaced, the same way
    /// [`MeshDecoder::decode`](crate::mesh_decoder::MeshDecoder::decode)
    /// replaces its mesh. `decode_after_header` does not clear, because its
    /// caller has already done so and has decoded metadata since.
    ///
    /// # Errors
    ///
    /// Returns an error if the header is invalid, the bitstream version is
    /// unsupported, or the encoded attributes are malformed.
    pub fn decode(&mut self, in_buffer: &mut DecoderBuffer, out_pc: &mut PointCloud) -> Status {
        out_pc.clear();

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
        // Read as the `int32_t` upstream reads, and refused when negative for
        // the same reason `PointCloudSequentialDecoder` and
        // `PointCloudKdTreeDecoder` refuse it: no encoder writes a count with
        // the sign bit set, and C++ Draco stops on one before it allocates
        // anything. Taking the same bytes unsigned is how a header claiming
        // 2,147,483,652 points reached the KD-tree walk, which then expanded a
        // single run-length node into 2.4 GB -- an OOM the `decode_drc` soak
        // found, on a file upstream rejects in a fifth of a second having
        // touched no memory at all.
        //
        // Past this the count is used but not guarded, which is also upstream's
        // shape: what bounds the work is the allocation budget applied where
        // the buffers are sized -- see `decode_budget`.
        let declared_points = buffer.decode_u32()? as i32;
        if declared_points < 0 {
            return Err(DracoError::general(format!(
                "Point cloud declares {declared_points} points"
            )));
        }
        let num_points: usize = declared_points as usize;
        buffer.check_points(num_points)?;
        pc.set_num_points(num_points);

        let num_attributes_decoders = buffer.decode_u8()? as usize;

        if self.method == 1 {
            // KD-tree encoding.
            for _ in 0..num_attributes_decoders {
                let mut att_decoder = KdTreeAttributesDecoder::new(0);
                att_decoder
                    .decode_attributes_decoder_data(pc, buffer)
                    .map_err(|err| err.context("Failed to decode attribute metadata"))?;
                att_decoder
                    .decode_attributes(pc, buffer)
                    .map_err(|err| err.context("Failed to decode attributes"))?;
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

                    buffer.admit_attribute(
                        num_points,
                        spec.num_components as usize,
                        spec.data_type.byte_length(),
                    )?;
                    let mut att = PointAttribute::new();
                    // Nothing is charged against the *budget* for this
                    // attribute, because nothing is taken for it: the buffer is
                    // left unreserved and sized by whichever decoder writes the
                    // values, once they exist. A charge there would be for an
                    // allocation that no longer happens, and it is not free --
                    // the budget is a backstop against unbacked reservations,
                    // and billing it for backed ones is what made it refuse
                    // files this crate writes. The caller's ceiling above is
                    // the other question and is charged: it bounds what the
                    // decode may produce at all, backed or not.
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

                // Symbols decoded beside the attribute before, for the one at
                // hand.
                let mut ahead: Option<Vec<u32>> = None;
                for (local_i, &att_id) in att_ids.iter().enumerate() {
                    let decoder_type = decoder_types[local_i];
                    let mut predecoded = ahead.take();
                    if predecoded.is_none() && bitstream_version >= 0x0200 {
                        if let (Some(&next_id), Some(&next_type)) =
                            (att_ids.get(local_i + 1), decoder_types.get(local_i + 1))
                        {
                            if let Some((first, second)) = Self::pair_symbols(
                                pc,
                                buffer,
                                (att_id, decoder_type),
                                (next_id, next_type),
                                num_points,
                            ) {
                                predecoded = Some(first);
                                ahead = Some(second);
                            }
                        }
                    }
                    match decoder_type {
                        1 => {
                            let point_ids = point_ids.ok_or_else(|| {
                                DracoError::general(
                                    "Point ids missing for integer attribute decoder".to_string(),
                                )
                            })?;
                            let mut att_decoder = SequentialIntegerAttributeDecoder::new();
                            att_decoder.init(self, att_id);
                            if let Some(symbols) = predecoded {
                                att_decoder.set_predecoded_symbols(symbols);
                            }
                            att_decoder.decode_values(
                                pc, point_ids, buffer, None, None, None, None, None, None,
                            )?;
                        }
                        2 => {
                            let mut att_decoder = SequentialQuantizationAttributeDecoder::new();
                            att_decoder.init(self, pc, att_id)?;
                            if let Some(symbols) = predecoded {
                                att_decoder.set_predecoded_symbols(symbols);
                            }
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
                            if let Some(symbols) = predecoded {
                                att_decoder.set_predecoded_symbols(symbols);
                            }
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

#[cfg(test)]
thread_local! {
    /// Attributes whose symbols were decoded beside another's.
    static PAIRED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Set by tests that hold the paired decode to the one without it.
    static PAIRS_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(feature = "point_cloud_decode")]
impl PointCloudDecoder {
    /// The symbols of two consecutive attributes' integer streams, decoded
    /// side by side ([`decode_raw_symbol_pair`]), for each attribute's decode
    /// to take instead of decoding its own. The buffer is left where it stood,
    /// at the first attribute's stream, which is then decoded as ever.
    ///
    /// Where the second stream starts is found by stepping over the first
    /// without decoding it. `None` where either attribute's stream is not one
    /// that can be stepped over or paired -- tagged symbols, symbols too wide
    /// for the paired loop, a prediction other than none or a plain
    /// difference, a decoder that is not an integer one -- or where anything
    /// goes wrong; the budget is then put
    /// back as it was and the two are decoded one after the other, which is
    /// what reports the error if there is one.
    ///
    /// A normal's stream codes its two octahedral coordinates, not the
    /// attribute's three components.
    fn pair_symbols(
        pc: &PointCloud,
        buffer: &mut DecoderBuffer,
        first: (i32, u8),
        second: (i32, u8),
        num_points: usize,
    ) -> Option<(Vec<u32>, Vec<u32>)> {
        #[cfg(test)]
        if PAIRS_OFF.with(|off| off.get()) {
            return None;
        }
        let coded = |(att_id, decoder_type): (i32, u8)| -> Option<(usize, usize)> {
            let components = match decoder_type {
                1 | 2 => pc.try_attribute(att_id).ok()?.num_components() as usize,
                3 => 2,
                _ => return None,
            };
            Some((num_points.checked_mul(components)?, components))
        };
        // The paired loop runs on steps, which only a table of at most 2^16
        // slots has: raw symbols of at most 11 bits. Past that each run would
        // be decoded alone after its table had been read twice, so the two
        // bytes that say so are read first, from a buffer of their own.
        let pairs = |at: &DecoderBuffer| -> bool {
            let Ok(mut look) = at.fork_at(at.position()) else {
                return false;
            };
            matches!(
                (look.decode_u8(), look.decode_u8()),
                (Ok(1), Ok(bits)) if (1..=11).contains(&bits)
            )
        };
        let (first, second) = (coded(first)?, coded(second)?);
        let start = buffer.position();
        let budget = buffer.budget();
        let mut attempt = || -> Option<(Vec<u32>, Vec<u32>)> {
            if !SequentialIntegerAttributeDecoder::seek_symbols(buffer) || !pairs(buffer) {
                return None;
            }
            let first_symbols = buffer.position();
            buffer.set_position(start).ok()?;
            if !SequentialIntegerAttributeDecoder::skip_values(first.0, first.1, buffer).ok()? {
                return None;
            }
            let mut probe = buffer.fork_at(buffer.position()).ok()?;
            if !SequentialIntegerAttributeDecoder::seek_symbols(&mut probe) || !pairs(&probe) {
                return None;
            }
            let second_symbols = probe.position();
            buffer.set_position(first_symbols).ok()?;
            decode_raw_symbol_pair(buffer, first, (second_symbols, second.0, second.1))
        };
        let symbols = attempt();
        // `start` is a position this buffer held a moment ago.
        let _ = buffer.set_position(start);
        if symbols.is_none() {
            buffer.restore_budget(budget);
        }
        #[cfg(test)]
        if symbols.is_some() {
            PAIRED.with(|paired| paired.set(paired.get() + 2));
        }
        symbols
    }
}

#[cfg(all(test, feature = "point_cloud_decode", feature = "encoder"))]
mod pair_tests {
    use super::*;
    use crate::encoder_buffer::EncoderBuffer;
    use crate::encoder_options::EncoderOptions;
    use crate::point_cloud_encoder::PointCloudEncoder;

    struct Xorshift(u64);

    impl Xorshift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn unit(&mut self) -> f32 {
            (self.next() >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    fn float_attribute(
        kind: GeometryAttributeType,
        components: u8,
        values: &[f32],
    ) -> PointAttribute {
        let points = values.len() / components as usize;
        let mut attribute = PointAttribute::new();
        attribute.init(kind, components, DataType::Float32, false, points);
        for (i, value) in values.iter().enumerate() {
            attribute.buffer_mut().write(i * 4, &value.to_le_bytes());
        }
        attribute
    }

    fn byte_attribute(
        kind: GeometryAttributeType,
        components: u8,
        values: &[u8],
    ) -> PointAttribute {
        let points = values.len() / components as usize;
        let mut attribute = PointAttribute::new();
        attribute.init(kind, components, DataType::Uint8, false, points);
        for (i, value) in values.iter().enumerate() {
            attribute.buffer_mut().write(i, &[*value]);
        }
        attribute
    }

    /// Every kind of attribute stream a sequential cloud carries -- a position
    /// the coder writes tagged, normals, quantized floats, bytes, raw floats,
    /// attributes that never vary -- with noise in the rest, so the stream is
    /// as large as the values.
    fn stream(points: usize, seed: u64) -> Vec<u8> {
        let mut rng = Xorshift(seed);
        let mut cloud = PointCloud::new();
        cloud.set_num_points(points);
        let mut options = EncoderOptions::new();
        options.set_encoding_method(0);
        let mut next_id = 0;
        let mut add = |cloud: &mut PointCloud, attribute: PointAttribute, bits: i32| {
            cloud.add_attribute(attribute);
            if bits > 0 {
                options.set_attribute_int(next_id, "quantization_bits", bits);
            }
            next_id += 1;
        };
        let positions: Vec<f32> = (0..points * 3).map(|_| rng.unit() * 100.0).collect();
        add(
            &mut cloud,
            float_attribute(GeometryAttributeType::Position, 3, &positions),
            16,
        );
        let normals: Vec<f32> = (0..points * 3).map(|_| rng.unit() * 2.0 - 1.0).collect();
        add(
            &mut cloud,
            float_attribute(GeometryAttributeType::Normal, 3, &normals),
            8,
        );
        for _ in 0..40 {
            let values: Vec<f32> = (0..points).map(|_| rng.unit()).collect();
            add(
                &mut cloud,
                float_attribute(GeometryAttributeType::Generic, 1, &values),
                8,
            );
        }
        let colours: Vec<u8> = (0..points * 3).map(|_| (rng.next() >> 56) as u8).collect();
        add(
            &mut cloud,
            byte_attribute(GeometryAttributeType::Color, 3, &colours),
            0,
        );
        // No quantization on a float: the coder copies it as raw bytes.
        let raw: Vec<f32> = (0..points).map(|_| rng.unit()).collect();
        add(
            &mut cloud,
            float_attribute(GeometryAttributeType::Generic, 1, &raw),
            0,
        );
        for _ in 0..3 {
            let zeros = vec![0.0f32; points];
            add(
                &mut cloud,
                float_attribute(GeometryAttributeType::Generic, 1, &zeros),
                8,
            );
        }
        let tags: Vec<u8> = (0..points).map(|_| (rng.next() >> 56) as u8).collect();
        add(
            &mut cloud,
            byte_attribute(GeometryAttributeType::Generic, 1, &tags),
            0,
        );
        let mut encoder = PointCloudEncoder::new();
        encoder.set_point_cloud(cloud);
        let mut buffer = EncoderBuffer::new();
        encoder.encode(&options, &mut buffer).expect("encodes");
        buffer.data().to_vec()
    }

    /// What a decode makes of `bytes` -- every attribute's bytes or the error
    /// -- and where it left the allocation budget, with the pairs on or off.
    fn decode(
        bytes: &[u8],
        pairs: bool,
    ) -> (Result<Vec<Vec<u8>>, String>, crate::decoder_buffer::Budget) {
        PAIRS_OFF.with(|off| off.set(!pairs));
        let mut buffer = DecoderBuffer::new(bytes);
        let mut cloud = PointCloud::new();
        let decoded = PointCloudDecoder::new()
            .decode(&mut buffer, &mut cloud)
            .map_err(|e| e.to_string())
            .map(|()| {
                (0..cloud.num_attributes())
                    .map(|id| cloud.attribute(id).buffer().data().to_vec())
                    .collect()
            });
        PAIRS_OFF.with(|off| off.set(false));
        (decoded, buffer.budget())
    }

    fn paired_by<T>(f: impl FnOnce() -> T) -> (T, usize) {
        let before = PAIRED.with(|paired| paired.get());
        let out = f();
        (out, PAIRED.with(|paired| paired.get()) - before)
    }

    /// The pairs decode the cloud the one-after-the-other decode does, value
    /// for value, and leave the budget where it leaves it. All but the tagged
    /// position, the raw float and an odd one out are paired, the normals'
    /// two octahedral coordinates included.
    #[test]
    fn paired_streams_decode_the_same_cloud_value_for_value() {
        let bytes = stream(40_000, 1);
        let ((alone, budget_alone), paired) = paired_by(|| decode(&bytes, false));
        assert_eq!(paired, 0, "the reference paired");
        let ((together, budget_together), paired) = paired_by(|| decode(&bytes, true));
        assert!(paired >= 44, "{paired} attributes paired");
        assert_eq!(together.expect("decodes"), alone.expect("decodes"));
        assert_eq!(budget_together, budget_alone, "the budget");
    }

    #[test]
    fn a_damaged_stream_is_read_paired_as_alone() {
        let bytes = stream(20_000, 3);
        let mut rng = Xorshift(77);
        for trial in 0..40 {
            let mut damaged = bytes.clone();
            // Bytes anywhere, and bytes in the header and the attribute
            // descriptions, where the lengths come from.
            let at = if trial % 2 == 0 {
                rng.next() as usize % damaged.len()
            } else {
                rng.next() as usize % 400
            };
            damaged[at] ^= 1 << (rng.next() % 8);
            assert_eq!(
                decode(&damaged, true),
                decode(&damaged, false),
                "trial {trial}: byte {at} damaged"
            );
        }
        // And cut short.
        for cut in [bytes.len() / 2, bytes.len() - 1, bytes.len() - 40, 5000] {
            assert_eq!(
                decode(&bytes[..cut], true),
                decode(&bytes[..cut], false),
                "cut at {cut}"
            );
        }
    }
}
