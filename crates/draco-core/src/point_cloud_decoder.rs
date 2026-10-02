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
use crate::parallel;
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
use crate::symbol_encoding::{decode_raw_symbol_pair, decode_raw_symbol_pair_in};

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
    #[cfg(feature = "point_cloud_decode")]
    threads: i32,
}

/// The stream length below which a point cloud stays on the calling thread. A
/// stream this small claiming this many values is what the allocation budget
/// exists for, and it is read by one thread and one budget.
///
/// A fuzzing build takes it down with the other thread gates; see
/// `parallel::ATTRIBUTES_MIN_VALUES`.
#[cfg(all(feature = "point_cloud_decode", not(fuzzing)))]
const PARALLEL_MIN_STREAM_BYTES: usize = 1 << 20;
#[cfg(all(feature = "point_cloud_decode", fuzzing))]
const PARALLEL_MIN_STREAM_BYTES: usize = 64;

/// The components an attribute's integer stream codes, for the sequential
/// decoder types that have one (1 to 3): a normal's are the two of its
/// octahedral coordinates, not the attribute's three.
#[cfg(feature = "point_cloud_decode")]
fn coded_components(decoder_type: u8, attribute: &PointAttribute) -> usize {
    match decoder_type {
        3 => 2,
        _ => attribute.num_components() as usize,
    }
}

// How many attribute streams this thread has decoded side by side, for the
// tests that must see the parallel path run rather than fall back.
#[cfg(all(test, feature = "point_cloud_decode"))]
thread_local! {
    static ENGAGED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    // How many jobs this thread ran on symbols decoded beside another's.
    static PAIRED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    // Set by the tests that need the decode in order, attribute by attribute,
    // as the reference the side-by-side decode is held to.
    static IN_ORDER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    // Set by the tests that hold the decode in order with its pairs to the one
    // without them.
    static PAIRS_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A quantized attribute whose values are decoded and whose inverse transform
/// has not yet run.
#[cfg(feature = "point_cloud_decode")]
struct PendingQuant {
    att_id: i32,
    portable: PointAttribute,
    transform: AttributeQuantizationTransform,
}

/// A normal attribute whose octahedral values are decoded and whose inverse
/// transform has not yet run.
#[cfg(feature = "point_cloud_decode")]
struct PendingNormal {
    att_id: i32,
    portable: PointAttribute,
    quantization_bits: u8,
}

/// Runs the inverse quantization of every pending attribute, one thread each at
/// most. Each writes only its own attribute, so the attributes are taken out of
/// the cloud for the duration and put back. They are taken in attribute order,
/// whatever order the decode left them pending in, so the first error in that
/// order is the one reported, on any number of threads.
#[cfg(feature = "point_cloud_decode")]
fn dequantize_in_parallel(
    pc: &mut PointCloud,
    mut pending: Vec<PendingQuant>,
    threads: usize,
) -> Status {
    pending.sort_by_key(|q| q.att_id);
    let mut work: Vec<(PendingQuant, PointAttribute, Option<DracoError>)> =
        Vec::with_capacity(pending.len());
    for q in pending {
        let dst = std::mem::take(pc.try_attribute_mut(q.att_id)?);
        work.push((q, dst, None));
    }
    parallel::for_each_chunk_mut(&mut work, 1, threads, |_, chunk| {
        let (q, dst, error) = &mut chunk[0];
        if let Err(e) = q.transform.inverse_transform_attribute(&q.portable, dst) {
            *error = Some(DracoError::general(format!(
                "Failed to dequantize attribute: {e}"
            )));
        }
    });
    let mut first_error = None;
    for (q, dst, error) in work {
        *pc.try_attribute_mut(q.att_id)? = dst;
        first_error = first_error.or(error);
    }
    first_error.map_or(Ok(()), Err)
}

/// [`dequantize_in_parallel`] for the octahedral normals.
#[cfg(feature = "point_cloud_decode")]
fn undo_octahedra_in_parallel(
    pc: &mut PointCloud,
    mut pending: Vec<PendingNormal>,
    bitstream_version: u16,
    threads: usize,
) -> Status {
    pending.sort_by_key(|n| n.att_id);
    let mut work: Vec<(PendingNormal, PointAttribute, Option<DracoError>)> =
        Vec::with_capacity(pending.len());
    for n in pending {
        let dst = std::mem::take(pc.try_attribute_mut(n.att_id)?);
        work.push((n, dst, None));
    }
    parallel::for_each_chunk_mut(&mut work, 1, threads, |_, chunk| {
        let (n, dst, error) = &mut chunk[0];
        let mut oct = AttributeOctahedronTransform::new(-1);
        let done = oct
            .set_parameters(n.quantization_bits as i32)
            .and_then(|()| {
                oct.inverse_transform_attribute_with_legacy_octahedron(
                    &n.portable,
                    dst,
                    bitstream_version < 0x0200,
                )
            });
        if let Err(e) = done {
            *error = Some(DracoError::general(format!(
                "Failed to decode normals: {e}"
            )));
        }
    });
    let mut first_error = None;
    for (n, dst, error) in work {
        *pc.try_attribute_mut(n.att_id)? = dst;
        first_error = first_error.or(error);
    }
    first_error.map_or(Ok(()), Err)
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
            #[cfg(feature = "point_cloud_decode")]
            threads: 1,
        }
    }

    /// Caps the threads a decode may use: `1`, the default, keeps everything on
    /// the calling thread, `0` is as many as the machine has, and no count is
    /// taken past sixteen. One by default because a library cannot see what
    /// its caller already runs: a server decoding on a pool of its own would
    /// have every decode multiply its threads by up to sixteen, and a hostile
    /// stream would have the machine's cores by default. A thread the system refuses to start is one
    /// fewer rather than an error. [`MeshDecoder::set_threads`] passes the
    /// same cap to the point-cloud streams a mesh decoder reads.
    ///
    /// [`MeshDecoder::set_threads`]: crate::MeshDecoder::set_threads
    ///
    /// A sequential point cloud with many attributes decodes them side by side
    /// once the stream is large enough to repay it, and the decoded cloud is the
    /// one a single thread produces, value for value. On sixteen threads, against
    /// one: a Gaussian splat of 742 thousand points and 58 attributes decodes in
    /// 0.058 s instead of 0.204 s, one of 3.2 million in 0.217 s instead of
    /// 0.860 s, an airborne lidar capture of 10.7 million and seven attributes in
    /// 0.41 s instead of 0.67 s. A cloud of two attributes, a scan's position and
    /// colour, gains next to nothing: its decode is the position's one chain.
    /// On one thread the attributes are decoded in order. On WebAssembly there are no threads and the value is ignored.
    #[cfg(feature = "point_cloud_decode")]
    pub fn set_threads(&mut self, threads: i32) {
        self.threads = threads;
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

                // On threads, attributes whose streams can be stepped over are
                // decoded side by side, two at a time on each thread; on one,
                // and where that cannot be done or goes wrong in any way, every
                // attribute is decoded in order, two consecutive ones' symbols
                // decoded together where they pair. On one thread the order is
                // the faster of the two: the side-by-side decode gives each
                // attribute a cloud and a buffer of its own, which the decode in
                // order does not pay.
                let threads = parallel::resolve(self.threads);
                #[cfg(test)]
                let in_order = IN_ORDER.with(|in_order| in_order.get());
                #[cfg(not(test))]
                let in_order = false;
                let in_parallel = bitstream_version >= 0x0200
                    && threads > 1
                    && !in_order
                    && num_points.saturating_mul(num_attributes_in_decoder)
                        >= crate::parallel::ATTRIBUTES_MIN_VALUES
                    && buffer.remaining_size() >= PARALLEL_MIN_STREAM_BYTES
                    && decoder_types
                        .iter()
                        .filter(|&&decoder_type| (1..=3).contains(&decoder_type))
                        .count()
                        >= 2;
                let decoded_in_parallel = in_parallel
                    && self.decode_attributes_in_parallel(
                        pc,
                        buffer,
                        &att_ids,
                        &decoder_types,
                        point_ids,
                        num_points,
                        threads,
                        &mut pending_quant,
                        &mut pending_normals,
                    )?;
                if !decoded_in_parallel {
                    // Symbols decoded beside the attribute before, for the one
                    // at hand.
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
                        self.decode_attribute_in_place(
                            pc,
                            buffer,
                            att_id,
                            decoder_type,
                            point_ids,
                            num_points,
                            &mut pending_quant,
                            &mut pending_normals,
                            predecoded,
                        )?;
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

                if in_parallel {
                    dequantize_in_parallel(pc, pending_quant, threads)?;
                    undo_octahedra_in_parallel(pc, pending_normals, bitstream_version, threads)?;
                } else {
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
                        .map_err(|e| {
                            DracoError::general(format!("Failed to decode normals: {e}"))
                        })?;
                    }
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

#[cfg(feature = "point_cloud_decode")]
impl PointCloudDecoder {
    /// The symbols of two consecutive attributes' integer streams, decoded
    /// side by side ([`decode_raw_symbol_pair_in`]), for each attribute's
    /// decode to take instead of decoding its own. The buffer is left where it
    /// stood, at the first attribute's stream, which is then decoded as ever.
    ///
    /// Where the second stream starts is found by stepping over the first
    /// without decoding it. `None` where either attribute's stream is not one
    /// that can be stepped over or paired -- tagged symbols, symbols too wide
    /// for the paired loop, a prediction other than none or a plain
    /// difference, a decoder that is not an integer one -- or where anything
    /// goes wrong; the budget is then put back as it was and the two are
    /// decoded one after the other, which is what reports the error if there
    /// is one.
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
            if !(1..=3).contains(&decoder_type) {
                return None;
            }
            let components = coded_components(decoder_type, pc.try_attribute(att_id).ok()?);
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
            decode_raw_symbol_pair_in(buffer, first, (second_symbols, second.0, second.1))
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

    /// Decodes one attribute's values from `buffer`, in place, the way the
    /// sequential decoder always has.
    #[allow(clippy::too_many_arguments)]
    fn decode_attribute_in_place(
        &self,
        pc: &mut PointCloud,
        buffer: &mut DecoderBuffer,
        att_id: i32,
        decoder_type: u8,
        point_ids: Option<EntryToPointIdMap<'_>>,
        num_points: usize,
        pending_quant: &mut Vec<PendingQuant>,
        pending_normals: &mut Vec<PendingNormal>,
        symbols: Option<Vec<u32>>,
    ) -> Status {
        let bitstream_version = self.bitstream_version();
        match decoder_type {
            1 => {
                let point_ids = point_ids.ok_or_else(|| {
                    DracoError::general(
                        "Point ids missing for integer attribute decoder".to_string(),
                    )
                })?;
                let mut att_decoder = SequentialIntegerAttributeDecoder::new();
                att_decoder.init(self, att_id);
                if let Some(symbols) = symbols {
                    att_decoder.set_predecoded_symbols(symbols);
                }
                att_decoder
                    .decode_values(pc, point_ids, buffer, None, None, None, None, None, None)?;
            }
            2 => {
                let mut att_decoder = SequentialQuantizationAttributeDecoder::new();
                att_decoder.init(self, pc, att_id)?;
                if let Some(symbols) = symbols {
                    att_decoder.set_predecoded_symbols(symbols);
                }
                let portable = att_decoder.decode_values(
                    pc,
                    point_ids.ok_or_else(|| {
                        DracoError::general(
                            "Point ids missing for quantized attribute decoder".to_string(),
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
                if let Some(symbols) = symbols {
                    att_decoder.set_predecoded_symbols(symbols);
                }
                let portable = att_decoder.decode_values(
                    pc,
                    point_ids.ok_or_else(|| {
                        DracoError::general(
                            "Point ids missing for normal attribute decoder".to_string(),
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
                // The identity map costs nothing to build and is all this
                // decoder reads off it -- the values are copied verbatim, in
                // order -- so the arm does not need the shared `point_ids`,
                // which is `None` when every attribute is generic.
                let mut att_decoder = SequentialGenericAttributeDecoder::new();
                att_decoder.init(self, att_id);
                att_decoder.decode_values(pc, EntryToPointIdMap::identity(num_points), buffer)?;
            }
            _ => {
                return Err(DracoError::general(format!(
                    "Unsupported sequential decoder type: {}",
                    decoder_type
                )));
            }
        }
        Ok(())
    }

    /// Decodes the attributes of a 2.0 or later sequential stream side by side.
    ///
    /// The streams are laid end to end with no table of where each starts, so
    /// the main thread walks them once, stepping over the ones whose symbols it
    /// can step over (the raw scheme, which is nearly all of them) and decoding
    /// the rest in place. Each stretch it steps over becomes a job that decodes
    /// it as a buffer of its own. A stream that does not come out exactly the
    /// length it was stepped over as is a stream this did not understand.
    ///
    /// Jobs are run two at a time, their symbols decoded in one loop
    /// (`Job::run_pair`): the rANS run is a chain each symbol waits on, and two
    /// of them overlap. Each job goes to a worker as soon as the walk reaches
    /// it, so the walk, and the in-place decoding that is most of it for a
    /// cloud of a few attributes, goes on while the workers run. Only for more
    /// than one thread: on one the decode in order, which pairs too, is the
    /// faster.
    ///
    /// `Ok(true)` when every attribute is decoded and `buffer` stands after the
    /// last of them. `Ok(false)` when anything at all went wrong -- the position
    /// and the pending lists are put back and the caller decodes in order, which
    /// is what reports the error if there is one. So the result is the serial
    /// decode's, value for value, and only the time differs.
    #[allow(clippy::too_many_arguments)]
    fn decode_attributes_in_parallel(
        &self,
        pc: &mut PointCloud,
        buffer: &mut DecoderBuffer,
        att_ids: &[i32],
        decoder_types: &[u8],
        point_ids: Option<EntryToPointIdMap<'_>>,
        num_points: usize,
        threads: usize,
        pending_quant: &mut Vec<PendingQuant>,
        pending_normals: &mut Vec<PendingNormal>,
    ) -> Result<bool, DracoError> {
        let start = buffer.position();
        let stream = buffer.remaining_data();
        let template = buffer.child_template();

        let (walked, results) = {
            let queue = std::sync::Mutex::new(None::<std::sync::mpsc::Receiver<Job>>);
            let (sender, receiver) = std::sync::mpsc::channel::<Job>();
            *queue.lock().unwrap() = Some(receiver);

            let work = || {
                let mut done = Vec::new();
                loop {
                    // Two at a time where two are waiting, taken together so
                    // that no other worker splits them. A worker never waits
                    // for the second: the walk may be decoding a long stream in
                    // place, and the first job is better started alone than
                    // held until the walk moves on.
                    let (first, second) = {
                        let guard = queue.lock().unwrap();
                        let receiver = guard.as_ref().expect("the queue is open");
                        let Ok(first) = receiver.recv() else { break };
                        (first, receiver.try_recv().ok())
                    };
                    let index = first.index;
                    match second {
                        Some(second) => {
                            let second_index = second.index;
                            let [a, b] =
                                Job::run_pair(first, second, self, &template, stream, num_points);
                            done.push((index, a));
                            done.push((second_index, b));
                        }
                        None => {
                            done.push((index, first.run(self, &template, stream, num_points, None)))
                        }
                    }
                }
                done
            };

            // The walk, on this thread, and the workers beside it, no more of
            // them than there are attributes to hand out. A failure in the walk
            // is not returned from inside the scope: the workers have to be let
            // go of first, and their results, if any, thrown away.
            std::thread::scope(|scope| {
                let workers: Vec<_> = (0..threads.min(att_ids.len()))
                    .map_while(|_| std::thread::Builder::new().spawn_scoped(scope, work).ok())
                    .collect();

                let walked = self.walk_streams(
                    pc,
                    buffer,
                    att_ids,
                    decoder_types,
                    point_ids,
                    num_points,
                    pending_quant,
                    pending_normals,
                    start,
                    |job| sender.send(job).is_ok(),
                );
                // Nothing more is coming: the workers run out of jobs and
                // finish, with this thread working beside them now that its
                // walk is done -- alone, if the system started none of them.
                drop(sender);
                let mut finished = vec![work()];
                finished.extend(workers.into_iter().map(|worker| {
                    worker
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                }));
                let mut results: Vec<Option<Result<Decoded, DracoError>>> = Vec::new();
                for done in finished {
                    for (index, result) in done {
                        if results.len() <= index {
                            results.resize_with(index + 1, || None);
                        }
                        results[index] = Some(result);
                    }
                }
                (walked, results)
            })
        };

        let give_up = |buffer: &mut DecoderBuffer,
                       pending_quant: &mut Vec<PendingQuant>,
                       pending_normals: &mut Vec<PendingNormal>|
         -> Result<bool, DracoError> {
            // The budget as it stood before the walk, which the decode in
            // order that follows charges the walk's own attributes to again.
            buffer.rejoin(&template, false);
            buffer.set_position(start)?;
            pending_quant.clear();
            pending_normals.clear();
            Ok(false)
        };
        let jobs = match walked {
            Ok(Some(count)) => count,
            _ => return give_up(buffer, pending_quant, pending_normals),
        };
        if jobs == 0 {
            buffer.rejoin(&template, true);
            return Ok(true);
        }
        let mut decoded_jobs = Vec::with_capacity(jobs);
        for result in results {
            match result {
                Some(Ok(decoded)) => decoded_jobs.push(decoded),
                _ => return give_up(buffer, pending_quant, pending_normals),
            }
        }
        // Every charge, the walk's and the pieces', was checked against what
        // all of them had spent, so what is left to check is the count.
        if decoded_jobs.len() != jobs {
            return give_up(buffer, pending_quant, pending_normals);
        }
        buffer.rejoin(&template, true);
        #[cfg(test)]
        ENGAGED.with(|engaged| engaged.set(engaged.get() + jobs));
        for decoded in decoded_jobs {
            match decoded {
                Decoded::Values { att_id, attribute } => *pc.try_attribute_mut(att_id)? = attribute,
                Decoded::Quantized {
                    att_id,
                    portable,
                    transform,
                } => pending_quant.push(PendingQuant {
                    att_id,
                    portable,
                    transform,
                }),
                Decoded::Normal {
                    att_id,
                    portable,
                    quantization_bits,
                } => pending_normals.push(PendingNormal {
                    att_id,
                    portable,
                    quantization_bits,
                }),
            }
        }
        Ok(true)
    }

    /// The walk `decode_attributes_in_parallel` makes: each attribute stream
    /// stepped over becomes a job handed to `send`, in order, and each one that
    /// cannot be is decoded in place. `Ok(Some(jobs))` when the walk reached
    /// the end; `Ok(None)` when a stream decoded in place failed or `send`
    /// refused a job, which the caller answers by decoding everything in order.
    #[allow(clippy::too_many_arguments)]
    fn walk_streams(
        &self,
        pc: &mut PointCloud,
        buffer: &mut DecoderBuffer,
        att_ids: &[i32],
        decoder_types: &[u8],
        point_ids: Option<EntryToPointIdMap<'_>>,
        num_points: usize,
        pending_quant: &mut Vec<PendingQuant>,
        pending_normals: &mut Vec<PendingNormal>,
        start: usize,
        mut send: impl FnMut(Job) -> bool,
    ) -> Result<Option<usize>, DracoError> {
        let mut count = 0usize;
        for (local_i, &att_id) in att_ids.iter().enumerate() {
            let decoder_type = decoder_types[local_i];
            if (1..=3).contains(&decoder_type) {
                let before = buffer.position();
                let components = coded_components(decoder_type, pc.try_attribute(att_id)?);
                let skipped = match num_points.checked_mul(components) {
                    Some(values) => {
                        SequentialIntegerAttributeDecoder::skip_values(values, components, buffer)
                    }
                    None => Ok(false),
                };
                if matches!(skipped, Ok(true)) {
                    let job = Job {
                        index: count,
                        att_id,
                        decoder_type,
                        start: before - start,
                        end: buffer.position() - start,
                        seed: pc.try_attribute(att_id)?.clone(),
                    };
                    if !send(job) {
                        return Ok(None);
                    }
                    count += 1;
                    continue;
                }
                buffer.set_position(before)?;
            }
            if self
                .decode_attribute_in_place(
                    pc,
                    buffer,
                    att_id,
                    decoder_type,
                    point_ids,
                    num_points,
                    pending_quant,
                    pending_normals,
                    None,
                )
                .is_err()
            {
                return Ok(None);
            }
        }
        Ok(Some(count))
    }
}

/// One attribute's stream, stepped over, waiting for a thread: where it lies in
/// the stream, and the attribute to decode it into.
#[cfg(feature = "point_cloud_decode")]
struct Job {
    index: usize,
    att_id: i32,
    decoder_type: u8,
    start: usize,
    end: usize,
    seed: PointAttribute,
}

/// What a thread made of a [`Job`].
#[cfg(feature = "point_cloud_decode")]
enum Decoded {
    Values {
        att_id: i32,
        attribute: PointAttribute,
    },
    Quantized {
        att_id: i32,
        portable: PointAttribute,
        transform: AttributeQuantizationTransform,
    },
    Normal {
        att_id: i32,
        portable: PointAttribute,
        quantization_bits: u8,
    },
}

#[cfg(feature = "point_cloud_decode")]
impl Job {
    /// Decodes the stretch of `stream` this job names, as a buffer of its own
    /// over an attribute of its own, and returns what it made and what it spent
    /// of the allocation budget.
    fn run(
        self,
        decoder: &PointCloudDecoder,
        template: &crate::decoder_buffer::ChildTemplate,
        stream: &[u8],
        num_points: usize,
        symbols: Option<Vec<u32>>,
    ) -> Result<Decoded, DracoError> {
        let bitstream_version = decoder.bitstream_version();
        let mut child = template.open(&stream[self.start..self.end]);
        let mut mini = PointCloud::new();
        mini.set_num_points(num_points);
        mini.add_attribute_preserve_unique_id(self.seed);
        let ids = EntryToPointIdMap::identity(num_points);
        let att_id = self.att_id;
        let decoded = match self.decoder_type {
            1 => {
                let mut att_decoder = SequentialIntegerAttributeDecoder::new();
                att_decoder.init(decoder, 0);
                if let Some(symbols) = symbols {
                    att_decoder.set_predecoded_symbols(symbols);
                }
                att_decoder.decode_values(
                    &mut mini, ids, &mut child, None, None, None, None, None, None,
                )?;
                Decoded::Values {
                    att_id,
                    attribute: std::mem::take(mini.try_attribute_mut(0)?),
                }
            }
            2 => {
                let mut att_decoder = SequentialQuantizationAttributeDecoder::new();
                att_decoder.init(decoder, &mini, 0)?;
                if let Some(symbols) = symbols {
                    att_decoder.set_predecoded_symbols(symbols);
                }
                let portable = att_decoder.decode_values(
                    &mut mini,
                    ids,
                    &mut child,
                    bitstream_version,
                    PortableExtent::Declared(num_points),
                    None,
                    None,
                    None,
                    None,
                )?;
                Decoded::Quantized {
                    att_id,
                    portable,
                    transform: att_decoder.into_transform(),
                }
            }
            _ => {
                let mut att_decoder = SequentialNormalAttributeDecoder::new();
                att_decoder.init(decoder, &mini, 0)?;
                if let Some(symbols) = symbols {
                    att_decoder.set_predecoded_symbols(symbols);
                }
                let portable = att_decoder.decode_values(
                    &mut mini,
                    ids,
                    &mut child,
                    bitstream_version,
                    PortableExtent::Declared(num_points),
                    None,
                    None,
                    None,
                    None,
                )?;
                Decoded::Normal {
                    att_id,
                    portable,
                    quantization_bits: att_decoder.quantization_bits(),
                }
            }
        };
        if child.remaining_size() != 0 {
            return Err(DracoError::general(
                "An attribute's stream is not the length it was stepped over as".to_string(),
            ));
        }
        Ok(decoded)
    }

    /// Runs two jobs, their symbols decoded side by side first
    /// (`decode_raw_symbol_pair`) and each job then decoded with its own. Where
    /// the pair cannot be decoded together the jobs run as they would alone,
    /// so what comes of each is what `run` makes of it.
    fn run_pair(
        a: Job,
        b: Job,
        decoder: &PointCloudDecoder,
        template: &crate::decoder_buffer::ChildTemplate,
        stream: &[u8],
        num_points: usize,
    ) -> [Result<Decoded, DracoError>; 2] {
        let mut child_a = template.open(&stream[a.start..a.end]);
        let mut child_b = template.open(&stream[b.start..b.end]);
        let values = |job: &Job| {
            let components = coded_components(job.decoder_type, &job.seed);
            (num_points.checked_mul(components), components)
        };
        let ((values_a, components_a), (values_b, components_b)) = (values(&a), values(&b));
        let symbols = match (values_a, values_b) {
            (Some(values_a), Some(values_b))
                if SequentialIntegerAttributeDecoder::seek_symbols(&mut child_a)
                    && SequentialIntegerAttributeDecoder::seek_symbols(&mut child_b) =>
            {
                decode_raw_symbol_pair(
                    (&mut child_a, values_a, components_a),
                    (&mut child_b, values_b, components_b),
                )
            }
            _ => None,
        };
        #[cfg(test)]
        if symbols.is_some() {
            PAIRED.with(|paired| paired.set(paired.get() + 2));
        }
        let (symbols_a, symbols_b) = symbols.map_or((None, None), |(a, b)| (Some(a), Some(b)));
        [
            a.run(decoder, template, stream, num_points, symbols_a),
            b.run(decoder, template, stream, num_points, symbols_b),
        ]
    }
}

#[cfg(all(test, feature = "point_cloud_decode", feature = "encoder"))]
mod parallel_tests {
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
    /// the coder writes tagged, normals, quantized floats, bytes, raw floats --
    /// with noise in all of them, so the stream is as large as the values.
    fn stream(points: usize, seed: u64) -> Vec<u8> {
        stream_with_threads(points, seed, 1)
    }

    fn stream_with_threads(points: usize, seed: u64, threads: i32) -> Vec<u8> {
        let mut rng = Xorshift(seed);
        let mut cloud = PointCloud::new();
        cloud.set_num_points(points);
        let mut options = EncoderOptions::new();
        options.set_encoding_method(0);
        options.set_threads(threads);
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
        let tags: Vec<u8> = (0..points).map(|_| (rng.next() >> 56) as u8).collect();
        add(
            &mut cloud,
            byte_attribute(GeometryAttributeType::Generic, 1, &tags),
            0,
        );
        // No quantization on a float: the coder copies it as raw bytes.
        let raw: Vec<f32> = (0..points).map(|_| rng.unit()).collect();
        add(
            &mut cloud,
            float_attribute(GeometryAttributeType::Generic, 1, &raw),
            0,
        );
        // Attributes that never vary, as a 3DGS file's normals do not: their
        // alphabet is one symbol and no payload backs the count, so each run
        // is drawn from the allowance the limits admitted, which the threads
        // draw on together.
        for _ in 0..3 {
            let zeros = vec![0.0f32; points];
            add(
                &mut cloud,
                float_attribute(GeometryAttributeType::Generic, 1, &zeros),
                8,
            );
        }
        let same = vec![7u8; points];
        add(
            &mut cloud,
            byte_attribute(GeometryAttributeType::Generic, 1, &same),
            0,
        );
        let mut encoder = PointCloudEncoder::new();
        encoder.set_point_cloud(cloud);
        let mut buffer = EncoderBuffer::new();
        encoder.encode(&options, &mut buffer).expect("encodes");
        buffer.data().to_vec()
    }

    /// What a decode makes of `bytes`: every attribute's bytes, or the error.
    fn decode(bytes: &[u8], threads: i32) -> Result<Vec<Vec<u8>>, String> {
        let mut decoder = PointCloudDecoder::new();
        decoder.set_threads(threads);
        let mut cloud = PointCloud::new();
        decoder
            .decode(&mut DecoderBuffer::new(bytes), &mut cloud)
            .map_err(|e| e.to_string())?;
        Ok((0..cloud.num_attributes())
            .map(|id| cloud.attribute(id).buffer().data().to_vec())
            .collect())
    }

    /// The decode in order, attribute by attribute, that the side-by-side one
    /// is held to.
    fn decode_in_order(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        IN_ORDER.with(|in_order| in_order.set(true));
        let decoded = decode(bytes, 1);
        IN_ORDER.with(|in_order| in_order.set(false));
        decoded
    }

    fn engaged_by<T>(f: impl FnOnce() -> T) -> (T, usize) {
        let before = ENGAGED.with(|e| e.get());
        let out = f();
        (out, ENGAGED.with(|e| e.get()) - before)
    }

    #[test]
    fn side_by_side_decodes_the_same_cloud_value_for_value() {
        let bytes = stream(40_000, 1);
        assert!(
            bytes.len() >= PARALLEL_MIN_STREAM_BYTES,
            "{} bytes",
            bytes.len()
        );
        let (serial, engaged) = engaged_by(|| decode_in_order(&bytes));
        assert_eq!(engaged, 0, "the reference took the side-by-side path");
        let serial = serial.expect("decodes");
        for threads in [1, 2, 5, 16] {
            let paired_before = PAIRED.with(|paired| paired.get());
            let (parallel, engaged) = engaged_by(|| decode(&bytes, threads));
            // One thread decodes in order, pairing as it goes; on the pairs'
            // own thread the count is kept, and every attribute is paired but
            // the ones whose symbols are too wide or tagged.
            if threads == 1 {
                let paired = PAIRED.with(|paired| paired.get()) - paired_before;
                assert_eq!(engaged, 0, "one thread took the side-by-side path");
                assert!(paired >= 40, "{paired} paired in order");
            } else {
                assert!(
                    engaged >= 44,
                    "the parallel path ran for {engaged} streams at {threads} threads"
                );
            }
            assert_eq!(parallel.expect("decodes"), serial, "{threads} threads");
        }
    }

    /// The encoder runs each attribute's encoder on its own thread and appends
    /// their buffers in attribute order. That is the stream one buffer written in
    /// that order holds, so the bytes may not depend on the number of threads.
    #[test]
    fn the_encoded_stream_is_the_same_bytes_at_any_number_of_threads() {
        let single = stream_with_threads(40_000, 5, 1);
        for threads in [2, 7, 16, 0] {
            assert_eq!(
                stream_with_threads(40_000, 5, threads),
                single,
                "{threads} threads"
            );
        }
    }

    /// A cloud of few attributes, each past a million values, gives each
    /// attribute threads of its own for its quantization, gather, prediction
    /// and symbol plan. The stream may not depend on how many, with or without
    /// the prediction search, which plans twice.
    #[test]
    fn an_attribute_split_across_threads_writes_the_same_bytes() {
        let points = 400_000;
        let mut rng = Xorshift(11);
        let mut cloud = PointCloud::new();
        cloud.set_num_points(points);
        // Coarse positions, so equal values and both zeros turn up in the
        // bounds a quantization folds.
        let positions: Vec<f32> = (0..points * 3)
            .map(|i| match i % 97 {
                0 => -0.0,
                1 => 0.0,
                _ => (rng.unit() * 64.0).floor() - 32.0,
            })
            .collect();
        cloud.add_attribute(float_attribute(
            GeometryAttributeType::Position,
            3,
            &positions,
        ));
        let colours: Vec<u8> = (0..points * 3).map(|_| (rng.next() >> 58) as u8).collect();
        cloud.add_attribute(byte_attribute(GeometryAttributeType::Color, 3, &colours));
        let encode = |threads: i32, search: bool| {
            let mut options = EncoderOptions::new();
            options.set_encoding_method(0);
            options.set_attribute_int(0, "quantization_bits", 14);
            options.set_threads(threads);
            options.set_prediction_search(search);
            let mut encoder = PointCloudEncoder::new();
            encoder.set_point_cloud(cloud.clone());
            let mut buffer = EncoderBuffer::new();
            encoder.encode(&options, &mut buffer).expect("encodes");
            buffer.data().to_vec()
        };
        for search in [false, true] {
            let single = encode(1, search);
            for threads in [4, 16] {
                assert!(
                    encode(threads, search) == single,
                    "{threads} threads, search {search}"
                );
            }
        }
    }

    #[test]
    fn a_stream_too_small_to_repay_the_threads_is_decoded_in_place() {
        let bytes = stream(500, 2);
        let (parallel, engaged) = engaged_by(|| decode(&bytes, 16));
        assert_eq!(engaged, 0);
        assert_eq!(parallel, decode_in_order(&bytes));
    }

    #[test]
    fn a_damaged_stream_is_read_by_the_threads_as_by_one() {
        let bytes = stream(40_000, 3);
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
            let serial = decode_in_order(&damaged);
            for threads in [1, 16] {
                let parallel = decode(&damaged, threads);
                assert_eq!(
                    parallel, serial,
                    "trial {trial}: byte {at} damaged, {threads} threads"
                );
            }
        }
        // And cut short.
        for cut in [bytes.len() / 2, bytes.len() - 1, bytes.len() - 40, 5000] {
            for threads in [1, 16] {
                assert_eq!(
                    decode(&bytes[..cut], threads),
                    decode_in_order(&bytes[..cut]),
                    "cut at {cut}, {threads} threads"
                );
            }
        }
    }

    /// Attributes that never vary have runs no payload backs, one symbol's
    /// worth of stream for millions of values, and a big cloud has several.
    /// The allowance and the budget are one for the whole decode, so what the
    /// threads draw together is what one thread would, and such a cloud
    /// decodes on threads as it does in order.
    #[test]
    fn constant_attributes_in_a_big_cloud_do_not_exhaust_what_threads_share() {
        let points = 7_000_000;
        let mut rng = Xorshift(9);
        let mut cloud = PointCloud::new();
        cloud.set_num_points(points);
        let mut options = EncoderOptions::new();
        options.set_encoding_method(0);
        options.set_threads(1);
        let mut next_id = 0;
        let mut add = |cloud: &mut PointCloud, attribute: PointAttribute| {
            cloud.add_attribute(attribute);
            options.set_attribute_int(next_id, "quantization_bits", 8);
            next_id += 1;
        };
        for _ in 0..3 {
            let noise: Vec<f32> = (0..points).map(|_| rng.unit()).collect();
            add(
                &mut cloud,
                float_attribute(GeometryAttributeType::Generic, 1, &noise),
            );
        }
        for _ in 0..4 {
            let zeros = vec![0.0f32; points];
            add(
                &mut cloud,
                float_attribute(GeometryAttributeType::Generic, 1, &zeros),
            );
        }
        let mut encoder = PointCloudEncoder::new();
        encoder.set_point_cloud(cloud);
        let mut buffer = EncoderBuffer::new();
        encoder.encode(&options, &mut buffer).expect("encodes");
        let bytes = buffer.data();
        assert!(bytes.len() >= PARALLEL_MIN_STREAM_BYTES);
        let serial = decode_in_order(bytes).expect("decodes");
        for threads in [1, 4, 12, 16] {
            let (parallel, engaged) = engaged_by(|| decode(bytes, threads));
            assert_eq!(
                engaged,
                if threads > 1 { 7 } else { 0 },
                "{threads} threads: the parallel path ran for {engaged} of 7 streams"
            );
            assert_eq!(parallel.expect("decodes"), serial, "{threads} threads");
        }
    }

    /// Constant attributes past what the allocation backstop holds, beside one
    /// that makes the stream long enough for the threads. A constant run is
    /// drawn from the values the caller's limits admitted, and the pieces
    /// decoded side by side draw on that one allowance as the decode in order
    /// does: the threads and the pairing run on every stream, rather than
    /// failing on the budget and leaving it all to one thread.
    #[test]
    fn constant_attributes_past_the_backstop_decode_side_by_side() {
        const CONSTANT: usize = 4;
        const COMPONENTS: usize = 3;
        const CEILING_IN_VALUES: usize =
            crate::decode_budget::MAX_UNBACKED_BYTES / std::mem::size_of::<u32>();
        // A quarter past the ceiling, so no payload the runs carry brings them
        // back under it.
        const POINTS: usize = CEILING_IN_VALUES / (CONSTANT * COMPONENTS) * 5 / 4;
        let mut cloud = PointCloud::new();
        cloud.set_num_points(POINTS);
        for _ in 0..CONSTANT {
            let colour = vec![0u8; POINTS * COMPONENTS];
            cloud.add_attribute(byte_attribute(
                GeometryAttributeType::Color,
                COMPONENTS as u8,
                &colour,
            ));
        }
        let mut rng = Xorshift(21);
        let noise: Vec<u8> = (0..POINTS).map(|_| (rng.next() >> 56) as u8).collect();
        cloud.add_attribute(byte_attribute(GeometryAttributeType::Generic, 1, &noise));
        let mut options = EncoderOptions::new();
        options.set_encoding_method(0);
        options.set_threads(1);
        let mut encoder = PointCloudEncoder::new();
        encoder.set_point_cloud(cloud);
        let mut buffer = EncoderBuffer::new();
        encoder.encode(&options, &mut buffer).expect("encodes");
        let bytes = buffer.data();
        assert!(bytes.len() >= PARALLEL_MIN_STREAM_BYTES);
        let serial = decode_in_order(bytes).expect("decodes");
        for threads in [1, 16] {
            let (parallel, engaged) = engaged_by(|| decode(bytes, threads));
            assert_eq!(
                engaged,
                if threads > 1 { CONSTANT + 1 } else { 0 },
                "{threads} threads: {engaged} streams decoded aside"
            );
            assert_eq!(parallel.expect("decodes"), serial, "{threads} threads");
        }
    }

    /// Every synthetic shape, in its own order and shuffled, from no points to
    /// past the thresholds that hand a cloud to threads: the same bytes encoded
    /// on one thread and on several, the same values decoded in order, paired
    /// and side by side, and those values the source's -- to the bit where
    /// nothing was quantized, within half a step where it was.
    #[test]
    fn synthetic_clouds_come_back_the_same_by_every_path() {
        use crate::synthetic_cloud::{self, Cloud};
        let mut clouds: Vec<Cloud> = [0, 1, 2, 3, 97]
            .into_iter()
            .flat_map(|points| synthetic_cloud::all(points, 1))
            .collect();
        // Big enough that every stream but the constant cloud's passes
        // `PARALLEL_MIN_STREAM_BYTES`, so the threads and the pairing run.
        let big = synthetic_cloud::all(400_000, 2);
        clouds.extend(big.iter().map(|cloud| cloud.shuffled(3)));
        clouds.extend(big);
        std::thread::scope(|scope| {
            for cloud in &clouds {
                scope.spawn(|| every_path_agrees(cloud));
            }
        });
    }

    fn every_path_agrees(cloud: &crate::synthetic_cloud::Cloud) {
        let name = format!("{} of {} points", cloud.name, cloud.points);
        let encode = |threads: i32| {
            let mut options = EncoderOptions::new();
            options.set_encoding_method(0);
            options.set_threads(threads);
            for (id, column) in cloud.columns.iter().enumerate() {
                if column.quantization_bits > 0 {
                    options.set_attribute_int(
                        id as i32,
                        "quantization_bits",
                        column.quantization_bits,
                    );
                }
            }
            let mut encoder = PointCloudEncoder::new();
            encoder.set_point_cloud(cloud.to_point_cloud());
            let mut buffer = EncoderBuffer::new();
            encoder
                .encode(&options, &mut buffer)
                .map(|()| buffer.data().to_vec())
                .map_err(|e| e.to_string())
        };
        let single = encode(1);
        for threads in [4, 16] {
            assert!(
                encode(threads) == single,
                "{name}: the stream on {threads} threads differs"
            );
        }
        // An empty attribute has no range to quantize over, and the encoder
        // says so rather than read a value that is not there.
        let bytes = match single {
            Err(e) if cloud.points == 0 && e.contains("empty attribute") => return,
            result => result.unwrap_or_else(|e| panic!("{name}: {e}")),
        };
        let decoded = decode_in_order(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let big = cloud.points >= 400_000 && !cloud.name.starts_with("constant");
        assert!(
            !big || bytes.len() >= PARALLEL_MIN_STREAM_BYTES,
            "{name}: {} bytes are too few to send the decode to threads",
            bytes.len()
        );
        for threads in [1, 4, 16] {
            let paired = PAIRED.with(|paired| paired.get());
            let (parallel, engaged) = engaged_by(|| decode(&bytes, threads));
            let paired = PAIRED.with(|paired| paired.get()) - paired;
            assert!(
                parallel.as_ref() == Ok(&decoded),
                "{name}: the decode on {threads} threads differs"
            );
            assert!(
                !big || if threads > 1 {
                    engaged > 0
                } else {
                    engaged == 0
                },
                "{name}: {threads} threads decoded {engaged} streams aside, {paired} paired"
            );
        }
        for (column, got) in cloud.columns.iter().zip(&decoded) {
            let source = column.values.bytes();
            let got = &got[..source.len()];
            if column.quantization_bits == 0 {
                assert!(got == source, "{name}: {} changed", column.name);
                continue;
            }
            let values = |bytes: &[u8]| -> Vec<f32> {
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|&b| f32::from_le_bytes(b))
                    .collect()
            };
            let (source, got) = (values(&source), values(got));
            if column.kind == GeometryAttributeType::Normal {
                // Octahedral coordinates on a grid of `bits` a side: a unit
                // vector comes back within a few cells. A zero one, which the
                // octahedron has no point for, comes back as some direction.
                let cell = 4.0 / (1u32 << column.quantization_bits) as f32;
                let pairs = source.as_chunks::<3>().0.iter().zip(got.as_chunks::<3>().0);
                for (i, (want, have)) in pairs.enumerate() {
                    if want.iter().all(|&v| v == 0.0) {
                        continue;
                    }
                    let error = want
                        .iter()
                        .zip(have)
                        .map(|(w, h)| (w - h).abs())
                        .fold(0.0, f32::max);
                    assert!(
                        error <= 2.0 * cell,
                        "{name}: normal {i} came back {have:?} for {want:?}"
                    );
                }
                continue;
            }
            // The quantizer's grid spans the widest component's range.
            let range = (0..column.components)
                .map(|c| {
                    let component = source.iter().skip(c).step_by(column.components);
                    let (lo, hi) =
                        component.fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
                    hi - lo
                })
                .fold(0.0f32, f32::max);
            let step = range / ((1u32 << column.quantization_bits) - 1) as f32;
            for (i, (&want, &have)) in source.iter().zip(&got).enumerate() {
                let slack = (want.abs() + range) * f32::EPSILON * 4.0;
                assert!(
                    (want - have).abs() <= step * 0.5 + slack,
                    "{name}: {} value {i} came back {have} for {want}, step {step}",
                    column.name
                );
            }
        }
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
        // On one thread, which decodes in order and is where the pairs run.
        let mut decoder = PointCloudDecoder::new();
        decoder.set_threads(1);
        let decoded = decoder
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

    /// Every synthetic shape, in its own order and shuffled, from no points to
    /// past where the decode's run forms pay: the same values and budget
    /// decoded with the pairs and without, and those values the source's -- to
    /// the bit where nothing was quantized, within half a step where it was.
    /// The large clouds must pair somewhere, and `skewed`'s bimodal attribute
    /// is decoded through the rANS buckets.
    #[test]
    fn synthetic_clouds_come_back_the_same_by_every_path() {
        use crate::synthetic_cloud::{self, Cloud};
        let mut clouds: Vec<Cloud> = [0, 1, 2, 3, 97]
            .into_iter()
            .flat_map(|points| synthetic_cloud::all(points, 1))
            .collect();
        let big = synthetic_cloud::all(400_000, 2);
        clouds.extend(big.iter().map(|cloud| cloud.shuffled(3)));
        clouds.extend(big);
        let paired: usize = std::thread::scope(|scope| {
            let runs: Vec<_> = clouds
                .iter()
                .map(|cloud| scope.spawn(|| paired_by(|| every_path_agrees(cloud)).1))
                .collect();
            runs.into_iter()
                .map(|run| run.join().expect("a cloud's check panicked"))
                .sum()
        });
        assert!(paired > 0, "no synthetic cloud paired a single stream");
    }

    fn every_path_agrees(cloud: &crate::synthetic_cloud::Cloud) {
        let name = format!("{} of {} points", cloud.name, cloud.points);
        let mut options = EncoderOptions::new();
        options.set_encoding_method(0);
        for (id, column) in cloud.columns.iter().enumerate() {
            if column.quantization_bits > 0 {
                options.set_attribute_int(id as i32, "quantization_bits", column.quantization_bits);
            }
        }
        let mut encoder = PointCloudEncoder::new();
        encoder.set_point_cloud(cloud.to_point_cloud());
        let mut buffer = EncoderBuffer::new();
        // An empty attribute has no range to quantize over, and the encoder
        // says so rather than read a value that is not there.
        match encoder.encode(&options, &mut buffer) {
            Err(e) if cloud.points == 0 && e.to_string().contains("empty attribute") => return,
            result => result.unwrap_or_else(|e| panic!("{name}: {e}")),
        }
        let bytes = buffer.data().to_vec();
        let (alone, budget_alone) = decode(&bytes, false);
        let decoded = alone.unwrap_or_else(|e| panic!("{name}: {e}"));
        let (together, budget_together) = decode(&bytes, true);
        assert!(
            together.as_ref() == Ok(&decoded),
            "{name}: the paired decode differs"
        );
        assert_eq!(budget_together, budget_alone, "{name}: the budget");
        for (column, got) in cloud.columns.iter().zip(&decoded) {
            let source = column.values.bytes();
            let got = &got[..source.len()];
            if column.quantization_bits == 0 {
                assert!(got == source, "{name}: {} changed", column.name);
                continue;
            }
            let values = |bytes: &[u8]| -> Vec<f32> {
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|&b| f32::from_le_bytes(b))
                    .collect()
            };
            let (source, got) = (values(&source), values(got));
            if column.kind == GeometryAttributeType::Normal {
                // Octahedral coordinates on a grid of `bits` a side: a unit
                // vector comes back within a few cells. A zero one, which the
                // octahedron has no point for, comes back as some direction.
                let cell = 4.0 / (1u32 << column.quantization_bits) as f32;
                let pairs = source.as_chunks::<3>().0.iter().zip(got.as_chunks::<3>().0);
                for (i, (want, have)) in pairs.enumerate() {
                    if want.iter().all(|&v| v == 0.0) {
                        continue;
                    }
                    let error = want
                        .iter()
                        .zip(have)
                        .map(|(w, h)| (w - h).abs())
                        .fold(0.0, f32::max);
                    assert!(
                        error <= 2.0 * cell,
                        "{name}: normal {i} came back {have:?} for {want:?}"
                    );
                }
                continue;
            }
            // The quantizer's grid spans the widest component's range.
            let range = (0..column.components)
                .map(|c| {
                    let component = source.iter().skip(c).step_by(column.components);
                    let (lo, hi) =
                        component.fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
                    hi - lo
                })
                .fold(0.0f32, f32::max);
            let step = range / ((1u32 << column.quantization_bits) - 1) as f32;
            for (i, (&want, &have)) in source.iter().zip(&got).enumerate() {
                let slack = (want.abs() + range) * f32::EPSILON * 4.0;
                assert!(
                    (want - have).abs() <= step * 0.5 + slack,
                    "{name}: {} value {i} came back {have} for {want}, step {step}",
                    column.name
                );
            }
        }
    }
}
