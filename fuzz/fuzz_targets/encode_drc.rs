#![no_main]

//! Encoder-side fuzzing.
//!
//! Every other target in this project feeds untrusted bytes to a *parser*. This
//! one feeds them to the *encoder*: the fuzz input is decoded into a geometry
//! description (point count, triangle indices, attribute layouts and payloads,
//! encoder options) and that geometry is handed to `MeshEncoder` /
//! `PointCloudEncoder`.
//!
//! That surface is reachable from untrusted data in practice — a glTF, FBX or
//! OBJ file that some other tool parsed becomes exactly this: a point count and
//! an index buffer nobody re-validated, with attribute payloads and quantization
//! settings taken from the file. So the encoder must reject a mesh it cannot
//! encode instead of panicking, indexing out of bounds, or allocating without a
//! bound.
//!
//! Two oracles:
//!
//! 1. Encoding must never panic, whatever the geometry says. Face indices are
//!    deliberately allowed past the point count, attribute values past the
//!    attribute size, and quantization bits outside any sane range.
//! 2. Anything the encoder *accepts* must decode. A stream the encoder produced
//!    and the decoder rejects is a bitstream bug, and it is invisible to
//!    decode-side fuzzing, which never sees such a stream.
//!
//! The geometry is built by hand from the input bytes rather than through
//! `arbitrary`'s derive, so every bound is explicit and stated here rather than
//! implied by a type: points and faces cap at 2048 and attributes at 4, which
//! keeps a single execution in the millisecond range so the campaign explores
//! encoder configurations instead of one huge mesh.

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{AttributeValueIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;
use draco_core::status::DracoError;
use libfuzzer_sys::fuzz_target;

const MAX_POINTS: usize = 2048;
const MAX_FACES: usize = 2048;
const MAX_ATTRIBUTES: usize = 4;

/// Cursor over the fuzz input. Reads past the end yield zero rather than
/// stopping the run, so a short input still describes a complete geometry and
/// the mutation engine is not pushed towards length-prefix bookkeeping.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn u8(&mut self) -> u8 {
        let byte = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1);
        byte
    }

    fn u16(&mut self) -> u16 {
        u16::from(self.u8()) | (u16::from(self.u8()) << 8)
    }

    fn bool(&mut self) -> bool {
        self.u8() & 1 == 1
    }

    /// Value in `[min, max]`, inclusive. Used for every count and enum-like
    /// field so the fuzzer spends its budget on combinations rather than on
    /// rediscovering which byte values mean anything.
    fn in_range(&mut self, min: i32, max: i32) -> i32 {
        if max <= min {
            return min;
        }
        let span = (max - min) as u32 + 1;
        min + (u32::from(self.u16()) % span) as i32
    }

    /// The remaining input, used as attribute payload. Attribute buffers are
    /// filled by repeating it, so the payload keeps its structure instead of
    /// degenerating into a zero tail once the input runs out.
    fn rest(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or(&[])
    }
}

struct AttributeSpec {
    attribute_type: GeometryAttributeType,
    data_type: DataType,
    num_components: u8,
    normalized: bool,
    num_values: usize,
    explicit_mapping: bool,
    quantization_bits: i32,
    prediction_scheme: i32,
}

struct GeometrySpec {
    as_point_cloud: bool,
    num_points: usize,
    faces: Vec<[u32; 3]>,
    attributes: Vec<AttributeSpec>,
    deduplicate: bool,
    encoding_method: i32,
    prediction_scheme: i32,
    encoding_speed: i32,
    decoding_speed: i32,
    split_on_seams: i32,
    store_number_of_encoded_faces: bool,
    force_predictive_traversal: bool,
    version: Option<(u8, u8)>,
}

fuzz_target!(|data: &[u8]| {
    let mut reader = Reader::new(data);
    let spec = read_spec(&mut reader);
    let payload = reader.rest().to_vec();

    if spec.as_point_cloud {
        fuzz_point_cloud(&spec, &payload, data);
    } else {
        fuzz_mesh(&spec, &payload, data);
    }
});

fn read_spec(reader: &mut Reader) -> GeometrySpec {
    let as_point_cloud = reader.bool();
    let num_points = reader.in_range(0, MAX_POINTS as i32) as usize;
    let num_faces = if as_point_cloud {
        0
    } else {
        reader.in_range(0, MAX_FACES as i32) as usize
    };

    // Indices are drawn from a range wider than the point count on purpose: a
    // face that points past the end of the geometry is the exact shape of a
    // hostile index buffer, and the encoder has to refuse it rather than index
    // into an attribute with it.
    let index_bound = reader.in_range(1, MAX_POINTS as i32 + 16) as u32;
    let mut faces = Vec::with_capacity(num_faces);
    for _ in 0..num_faces {
        faces.push([
            u32::from(reader.u16()) % index_bound,
            u32::from(reader.u16()) % index_bound,
            u32::from(reader.u16()) % index_bound,
        ]);
    }

    let num_attributes = reader.in_range(0, MAX_ATTRIBUTES as i32) as usize;
    let mut attributes = Vec::with_capacity(num_attributes);
    for _ in 0..num_attributes {
        attributes.push(read_attribute_spec(reader, num_points));
    }

    GeometrySpec {
        as_point_cloud,
        num_points,
        faces,
        attributes,
        deduplicate: reader.bool(),
        // -1 leaves the encoder's own choice in play; 0/1 force sequential and
        // EdgeBreaker; 2 and 3 are out of range and must be refused.
        encoding_method: reader.in_range(-1, 3),
        prediction_scheme: reader.in_range(-2, 6),
        encoding_speed: reader.in_range(-1, 10),
        decoding_speed: reader.in_range(-1, 10),
        split_on_seams: reader.in_range(-1, 1),
        store_number_of_encoded_faces: reader.bool(),
        force_predictive_traversal: reader.bool(),
        version: if reader.bool() {
            Some((reader.u8() % 4, reader.u8() % 8))
        } else {
            None
        },
    }
}

fn read_attribute_spec(reader: &mut Reader, num_points: usize) -> AttributeSpec {
    let attribute_type = match reader.in_range(0, 4) {
        0 => GeometryAttributeType::Position,
        1 => GeometryAttributeType::Normal,
        2 => GeometryAttributeType::Color,
        3 => GeometryAttributeType::TexCoord,
        _ => GeometryAttributeType::Generic,
    };
    let data_type = match reader.in_range(0, 10) {
        0 => DataType::Int8,
        1 => DataType::Uint8,
        2 => DataType::Int16,
        3 => DataType::Uint16,
        4 => DataType::Int32,
        5 => DataType::Uint32,
        6 => DataType::Int64,
        7 => DataType::Uint64,
        8 => DataType::Float32,
        9 => DataType::Float64,
        _ => DataType::Bool,
    };
    let explicit_mapping = reader.bool();
    // With identity mapping the attribute must cover every point, so a shorter
    // value array is itself one of the hazards under test; with an explicit map
    // the count is free.
    let num_values = if explicit_mapping {
        reader.in_range(0, MAX_POINTS as i32) as usize
    } else {
        reader.in_range(0, num_points as i32 + 4) as usize
    };

    AttributeSpec {
        attribute_type,
        data_type,
        num_components: reader.in_range(0, 8) as u8,
        normalized: reader.bool(),
        num_values,
        explicit_mapping,
        // Past both ends of the legal 1..=30 range, since the quantization bit
        // count feeds shift arithmetic.
        quantization_bits: reader.in_range(-2, 34),
        prediction_scheme: reader.in_range(-2, 6),
    }
}

fn build_attribute(
    spec: &AttributeSpec,
    num_points: usize,
    payload: &[u8],
    seed: usize,
) -> Option<PointAttribute> {
    let mut attribute = PointAttribute::new();
    attribute
        .try_init(
            spec.attribute_type,
            spec.num_components,
            spec.data_type,
            spec.normalized,
            spec.num_values,
        )
        .ok()?;

    fill(attribute.buffer_mut().data_mut(), payload, seed);

    if spec.explicit_mapping {
        attribute.set_explicit_mapping(num_points);
        for point in 0..num_points {
            // The value index is taken modulo the value count where there is
            // one; an attribute with no values keeps whatever the map was
            // initialised to, which is the invalid index, and the encoder is
            // expected to refuse that rather than dereference it.
            if spec.num_values == 0 {
                break;
            }
            let value =
                ((point.wrapping_mul(2654435761).wrapping_add(seed)) % spec.num_values) as u32;
            let _ = attribute
                .try_set_point_map_entry(PointIndex(point as u32), AttributeValueIndex(value));
        }
    } else {
        attribute.set_identity_mapping();
    }

    Some(attribute)
}

/// Fills `dst` by repeating `src`, offset by `seed` so two attributes of the
/// same size do not receive identical bytes.
fn fill(dst: &mut [u8], src: &[u8], seed: usize) {
    if src.is_empty() {
        return;
    }
    for (i, byte) in dst.iter_mut().enumerate() {
        *byte = src[(i + seed) % src.len()];
    }
}

fn build_options(spec: &GeometrySpec) -> EncoderOptions {
    let mut options = EncoderOptions::new();
    if spec.encoding_method >= 0 {
        options.set_encoding_method(spec.encoding_method);
    }
    if spec.prediction_scheme >= -1 {
        options.set_prediction_scheme(spec.prediction_scheme);
    }
    if spec.encoding_speed >= 0 {
        options.set_global_int("encoding_speed", spec.encoding_speed);
    }
    if spec.decoding_speed >= 0 {
        options.set_global_int("decoding_speed", spec.decoding_speed);
    }
    if spec.split_on_seams >= 0 {
        options.set_global_int("split_mesh_on_seams", spec.split_on_seams);
    }
    if spec.store_number_of_encoded_faces {
        options.set_global_int("store_number_of_encoded_faces", 1);
    }
    if spec.force_predictive_traversal {
        options.set_global_int("force_predictive_traversal", 1);
    }
    if let Some((major, minor)) = spec.version {
        options.set_version(major, minor);
    }
    for (id, attribute) in spec.attributes.iter().enumerate() {
        let id = id as i32;
        options.set_attribute_int(id, "quantization_bits", attribute.quantization_bits);
        if attribute.prediction_scheme >= -1 {
            options.set_attribute_int(id, "prediction_scheme", attribute.prediction_scheme);
        }
    }
    options
}

fn add_attributes(point_cloud: &mut PointCloud, spec: &GeometrySpec, payload: &[u8]) {
    for (index, attribute_spec) in spec.attributes.iter().enumerate() {
        if let Some(attribute) =
            build_attribute(attribute_spec, spec.num_points, payload, index * 7 + 1)
        {
            point_cloud.add_attribute(attribute);
        }
    }
}

fn fuzz_mesh(spec: &GeometrySpec, payload: &[u8], input: &[u8]) {
    let mut mesh = Mesh::new();
    mesh.set_num_points(spec.num_points);
    if mesh.try_set_num_faces(spec.faces.len()).is_err() {
        return;
    }
    for (index, face) in spec.faces.iter().enumerate() {
        mesh.set_face_from_indices(index, *face);
    }
    add_attributes(&mut mesh, spec, payload);
    if spec.deduplicate {
        mesh.deduplicate_point_ids();
    }

    let options = build_options(spec);
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    if encoder.encode(&options, &mut buffer).is_err() {
        return;
    }

    // Oracle 2: whatever the encoder accepted has to decode. Decode-side
    // fuzzing cannot reach this — it never produces a stream the encoder
    // considers valid.
    let mut decoded = Mesh::new();
    let mut decoder_buffer = DecoderBuffer::new(buffer.data());
    if let Err(error) = MeshDecoder::new().decode(&mut decoder_buffer, &mut decoded) {
        if !round_trip_is_claimed(spec, &error) {
            return;
        }
        panic!(
            "encoder produced a mesh stream the decoder rejects: {error}\ninput: {}\n{}",
            hex(input),
            describe(spec)
        );
    }
}

fn fuzz_point_cloud(spec: &GeometrySpec, payload: &[u8], input: &[u8]) {
    let mut point_cloud = PointCloud::new();
    point_cloud.set_num_points(spec.num_points);
    add_attributes(&mut point_cloud, spec, payload);

    let options = build_options(spec);
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(point_cloud);
    let mut buffer = EncoderBuffer::new();
    if encoder.encode(&options, &mut buffer).is_err() {
        return;
    }

    let mut decoded = PointCloud::new();
    let mut decoder_buffer = DecoderBuffer::new(buffer.data());
    if let Err(error) = PointCloudDecoder::new().decode(&mut decoder_buffer, &mut decoded) {
        if !round_trip_is_claimed(spec, &error) {
            return;
        }
        panic!(
            "encoder produced a point-cloud stream the decoder rejects: {error}\ninput: {}\n{}",
            hex(input),
            describe(spec)
        );
    }
}

/// Whether oracle 2 - anything the encoder accepted decodes - applies to this
/// encode, or whether the failure is one of the still-open findings.
///
/// Decided from the geometry and from the error's *variant*, never from its
/// text: an error message is not an interface, and a suppression keyed on one
/// silently stops suppressing when the wording changes and silently starts
/// covering an unrelated failure that happens to share a phrase.
///
/// Two exclusions, both still-open findings recorded in
/// `hardening_status.yaml`:
///
/// 1. **An explicitly requested non-default bitstream version.** Legacy encode
///    is version-gated field by field and several of those gates disagree with
///    the decoder, so such a stream can be unreadable. Those encodes still run,
///    because oracle 1 - no panics - applies to them; only the round-trip claim
///    is held to the default version, which is what the crate documents.
///
/// 2. **`CountExceedsBitstream`.** The decoder's header preflight assumes a
///    stream carries at least one bit per point or face, which is false for
///    geometry whose values are all equal: it entropy-codes to a size
///    independent of the count. Matching the variant costs nothing and excludes
///    exactly those runs - an earlier version of this predicted the guard from
///    the stream length instead, which was far coarser and quietly excluded
///    most of the corpus from the oracle.
fn round_trip_is_claimed(spec: &GeometrySpec, error: &DracoError) -> bool {
    if std::env::var_os("ENCODE_DRC_NO_DECODE_ORACLE").is_some() {
        // Triage knob: with oracle 2 off, a campaign runs past every
        // encoder/decoder disagreement and reports only panics. Used when one
        // known disagreement keeps ending the run before the panics behind it.
        return false;
    }
    if spec.version.is_some() {
        return false;
    }
    !matches!(error, DracoError::CountExceedsBitstream { .. })
}

/// Hex of the whole fuzz input, printed with an oracle failure.
///
/// libFuzzer on Windows/MSVC does not get to write its artifact file when a
/// Rust panic aborts the process, so the reproducer has to travel in the panic
/// message itself. Inputs that reach an oracle are small, and this is what a
/// deterministic regression test gets built from.
fn hex(input: &[u8]) -> String {
    input.iter().map(|b| format!("{b:02x}")).collect()
}

/// One-line rendering of the geometry the encoder was given, printed with an
/// oracle failure so the triage does not start by re-deriving it from the
/// input bytes.
fn describe(spec: &GeometrySpec) -> String {
    let attributes: Vec<String> = spec
        .attributes
        .iter()
        .map(|a| {
            format!(
                "{:?}/{:?}/nc={}/norm={}/values={}/explicit={}/qb={}/pred={}",
                a.attribute_type,
                a.data_type,
                a.num_components,
                a.normalized,
                a.num_values,
                a.explicit_mapping,
                a.quantization_bits,
                a.prediction_scheme
            )
        })
        .collect();
    format!(
        "points={} faces={} dedup={} method={} pred={} speed={}/{} seams={} store_faces={} predictive={} version={:?}
attributes: [{}]",
        spec.num_points,
        spec.faces.len(),
        spec.deduplicate,
        spec.encoding_method,
        spec.prediction_scheme,
        spec.encoding_speed,
        spec.decoding_speed,
        spec.split_on_seams,
        spec.store_number_of_encoded_faces,
        spec.force_predictive_traversal,
        spec.version,
        attributes.join(", ")
    )
}
