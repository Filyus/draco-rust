use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;

fn repo_testdata_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

#[derive(Clone, Copy, Debug)]
enum DecoderKind {
    Mesh,
    PointCloud,
}

fn draco_header(major: u8, minor: u8, geometry: u8, method: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"DRACO");
    bytes.push(major);
    bytes.push(minor);
    bytes.push(geometry);
    bytes.push(method);
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

fn append_varint(bytes: &mut Vec<u8>, value: u64) {
    let mut value = value;
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Decodes `bytes` and returns what the decoder *reported*: `Ok` for a stream
/// it accepted, `Err` for one it refused. A panic is neither, and is the thing
/// every test in this file exists to catch, so it is turned into a failure of
/// the calling test right here rather than handed back as a value a caller
/// could discard -- which is what a `let _ =` at half the call sites did, for
/// as long as the panic came back as an ordinary `Err`.
fn decode_malformed_without_panic(kind: DecoderKind, bytes: &[u8]) -> Result<(), String> {
    let status = panic::catch_unwind(AssertUnwindSafe(|| match kind {
        DecoderKind::Mesh => {
            let mut buffer = DecoderBuffer::new(bytes);
            let mut mesh = Mesh::new();
            let mut decoder = MeshDecoder::new();
            decoder.decode(&mut buffer, &mut mesh)
        }
        DecoderKind::PointCloud => {
            let mut buffer = DecoderBuffer::new(bytes);
            let mut pc = PointCloud::new();
            let mut decoder = PointCloudDecoder::new();
            decoder.decode(&mut buffer, &mut pc)
        }
    }))
    .unwrap_or_else(|_| {
        // The panic's own message and location have already gone to stderr
        // through the default hook; this names the input that produced it.
        panic!(
            "{kind:?} decoder panicked on a {}-byte stream (its panic is printed above)",
            bytes.len()
        )
    });

    status.map_err(|e| format!("{e:?}"))
}

fn decode_by_header_without_panic(bytes: &[u8]) -> Result<(), String> {
    let kind = if bytes.len() > 7 && bytes[0..5] == *b"DRACO" && bytes[7] == 0 {
        DecoderKind::PointCloud
    } else {
        DecoderKind::Mesh
    };
    decode_malformed_without_panic(kind, bytes)
}

/// Runs both decoders over the same bytes for their panic behaviour alone.
///
/// Either verdict is a pass: a reproducer of this kind is malformed, and a
/// decoder that refuses it is doing its job. The failure this asserts on is
/// raised by `decode_malformed_without_panic` itself.
fn assert_both_decoders_do_not_panic(bytes: &[u8]) {
    let _ = decode_malformed_without_panic(DecoderKind::Mesh, bytes);
    let _ = decode_malformed_without_panic(DecoderKind::PointCloud, bytes);
}

fn fill_pseudo_random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        bytes.push((state >> 32) as u8);
    }
    bytes
}

fn deterministic_fuzz_case(seed: u64, len: usize) -> Vec<u8> {
    let mut bytes = fill_pseudo_random_bytes(seed, len);

    if len >= 10 && (seed & 1) == 0 {
        let geometry = if (seed & 2) == 0 { 1 } else { 0 };
        let method = ((seed >> 8) & 3) as u8;
        bytes[0..5].copy_from_slice(b"DRACO");
        bytes[5] = 2;
        bytes[6] = if (seed & 4) == 0 { 2 } else { 0 };
        bytes[7] = geometry;
        bytes[8] = method;
        bytes[9] = 0;
    }

    for bit in 0..4 {
        if bytes.is_empty() {
            break;
        }
        let idx = ((seed.rotate_left(bit * 13) as usize) ^ (len.wrapping_mul(17 + bit as usize)))
            % bytes.len();
        bytes[idx] ^= 1 << bit;
    }

    bytes
}

#[test]
fn decode_rejects_invalid_magic() {
    let mut bytes = vec![0u8; 32];
    bytes[0..5].copy_from_slice(b"XXXXX");

    let mut buffer = DecoderBuffer::new(&bytes);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let status = decoder.decode(&mut buffer, &mut mesh);

    assert!(status.is_err());
}

#[test]
fn decode_rejects_invalid_geometry_type_in_header() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"DRACO");
    bytes.push(2); // major
    bytes.push(2); // minor
    bytes.push(99); // invalid geometry type
    bytes.push(0); // method
    bytes.extend_from_slice(&0u16.to_le_bytes()); // flags

    let mut buffer = DecoderBuffer::new(&bytes);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let status = decoder.decode(&mut buffer, &mut mesh);

    assert!(status.is_err());
}

#[test]
fn point_cloud_decoder_rejects_mesh_geometry_header() {
    let bytes = draco_header(2, 3, 1, 1);

    let error = decode_malformed_without_panic(DecoderKind::PointCloud, &bytes)
        .expect_err("point-cloud decoder should reject mesh geometry headers");
    assert!(
        error.contains("cannot decode mesh"),
        "unexpected point-cloud error: {error}"
    );
}

#[test]
fn malformed_legacy_edgebreaker_counts_fail_before_allocation() {
    let fuzz_17_oom = [
        68, 82, 65, 67, 79, 0, 155, 1, 1, 0, 0, 2, 99, 170, 1, 1, 170, 1, 17, 2, 35, 34, 18, 39,
        37, 3, 47, 111, 219, 182, 221, 243, 54, 218, 214, 163, 165, 165, 165, 197, 195, 163, 109,
        161, 69, 181, 236, 121, 208, 210, 226, 225, 209, 210, 238, 105, 8, 165, 22, 164, 30, 255,
        211, 120, 193, 87, 216, 136, 85, 41, 187, 216, 20, 146, 56, 109, 22, 211, 235, 3, 217, 96,
        243, 151, 241, 81, 129, 184, 247, 187, 116, 60, 21, 176, 7, 174, 189, 73, 191, 114, 255,
        69, 146, 73, 26, 217, 209, 67, 181, 43, 140, 138, 37, 239, 175, 12, 92, 130, 201, 55, 10,
        59, 3, 215, 35, 10, 120, 170, 200, 48, 242, 22, 56, 162, 11, 89, 40, 57, 169, 80, 33, 140,
        255, 15, 0, 0, 255, 7, 0, 0, 255, 2, 207, 71, 12,
    ];

    let mesh_error = decode_malformed_without_panic(DecoderKind::Mesh, &fuzz_17_oom)
        .expect_err("mesh decoder should reject malformed legacy counts before allocation");
    assert!(
        mesh_error.contains("num_faces is smaller than num_symbols"),
        "unexpected mesh error: {mesh_error}"
    );

    decode_malformed_without_panic(DecoderKind::PointCloud, &fuzz_17_oom)
        .expect_err("point-cloud decoder should reject the fuzz artifact before allocation");
}

/// A 2.0.0 regression, found by the `decode_drc` fuzz target: a mesh header
/// naming 1,095,910,464 faces reserved 13 GB of indices from a 26 KB stream.
///
/// The ratio guard that replaced 1.2.0's count check accepts it — 13 GB over
/// 2^20 is 12 KB, and the stream is larger than that — which is the whole
/// shape of the problem: a ratio scales with the input, and the input is what
/// an attacker supplies. 1.2.0 refused the same bytes with "Declared count
/// 1095910464 exceeds what the remaining 26367 bytes can describe".
///
/// The body is zeros because the guard fires before anything reads it; only
/// the length matters, and it has to clear the ratio for this to test the
/// symbol bound rather than the ratio. Padded to 12,600 bytes for that reason:
/// at 24 bytes the ratio catches it on its own and the case proves nothing.
#[test]
fn a_face_count_beyond_one_bit_each_is_refused_before_allocation() {
    let header = [
        68, 82, 65, 67, 79, 2, 0, 1, 0, 0, 0, 64, 68, 82, 65, 67, 79, 2, 2, 0, 0, 0, 0, 53,
    ];
    let mut stream = header.to_vec();
    stream.resize(12_600, 0);

    let error = decode_malformed_without_panic(DecoderKind::Mesh, &stream)
        .expect_err("a face count no stream that size can carry must be refused");
    assert!(
        error.contains("declared 3287731392 symbols"),
        "expected the symbol-count refusal, got: {error}"
    );

    decode_malformed_without_panic(DecoderKind::PointCloud, &stream)
        .expect_err("the point-cloud decoder refuses a mesh bitstream");
}

#[test]
fn malformed_drc_inputs_fail_without_panic() {
    let mut truncated_mesh_payload = draco_header(2, 0, 1, 0);
    truncated_mesh_payload.extend_from_slice(&8u32.to_le_bytes());

    let mut truncated_point_cloud_payload = draco_header(2, 0, 0, 0);
    truncated_point_cloud_payload.extend_from_slice(&4u32.to_le_bytes());

    let mut corrupt_point_cloud_varint = draco_header(2, 2, 0, 0);
    corrupt_point_cloud_varint.extend_from_slice(&1u32.to_le_bytes());
    corrupt_point_cloud_varint.push(1); // one attributes decoder
    corrupt_point_cloud_varint.extend_from_slice(&[0x80; 10]);

    let mut truncated_point_cloud_attribute_metadata = draco_header(2, 2, 0, 0);
    truncated_point_cloud_attribute_metadata.extend_from_slice(&1u32.to_le_bytes());
    truncated_point_cloud_attribute_metadata.push(1); // one attributes decoder
    truncated_point_cloud_attribute_metadata.push(1); // one attribute in decoder

    let cases = [
        ("empty mesh stream", DecoderKind::Mesh, Vec::new()),
        ("short mesh header", DecoderKind::Mesh, b"DRAC".to_vec()),
        ("invalid mesh magic", DecoderKind::Mesh, vec![0u8; 16]),
        (
            "invalid mesh geometry type",
            DecoderKind::Mesh,
            draco_header(2, 2, 99, 0),
        ),
        (
            "truncated mesh payload",
            DecoderKind::Mesh,
            truncated_mesh_payload,
        ),
        (
            "empty point-cloud stream",
            DecoderKind::PointCloud,
            Vec::new(),
        ),
        (
            "short point-cloud header",
            DecoderKind::PointCloud,
            b"DRAC".to_vec(),
        ),
        (
            "truncated point-cloud payload",
            DecoderKind::PointCloud,
            truncated_point_cloud_payload,
        ),
        (
            "corrupt point-cloud varint",
            DecoderKind::PointCloud,
            corrupt_point_cloud_varint,
        ),
        (
            "truncated point-cloud attribute metadata",
            DecoderKind::PointCloud,
            truncated_point_cloud_attribute_metadata,
        ),
    ];

    for (name, kind, bytes) in cases {
        assert!(
            decode_malformed_without_panic(kind, &bytes).is_err(),
            "{name} unexpectedly decoded successfully"
        );
    }
}

/// The file the count guard used to refuse: geometry whose values are all
/// equal entropy-codes to a size independent of how many there are.
///
/// The decoder asserted at least one bit per point, so it rejected a 171-byte
/// stream that says 100,000 points - a stream this crate writes, and C++ Draco
/// writes too. What bounds the work now is an allocation budget measured
/// against the input size, which this file passes with orders of magnitude to
/// spare while the malformed headers below still fail.
#[test]
fn a_point_cloud_that_compresses_below_a_bit_per_point_decodes() {
    use draco_core::draco_types::DataType;
    use draco_core::encoder_buffer::EncoderBuffer;
    use draco_core::encoder_options::EncoderOptions;
    use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
    use draco_core::point_cloud_encoder::PointCloudEncoder;

    const NUM_POINTS: usize = 100_000;

    let mut point_cloud = PointCloud::new();
    point_cloud.set_num_points(NUM_POINTS);
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        NUM_POINTS,
    );
    point_cloud.add_attribute(position);

    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 8);
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(point_cloud);
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encode");

    assert!(
        buffer.data().len() < 8 * NUM_POINTS / 8,
        "the point of this test is that the stream is far under a bit per point"
    );

    let mut decoded = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut DecoderBuffer::new(buffer.data()), &mut decoded)
        .expect("a stream this crate wrote must decode");
    assert_eq!(decoded.num_points(), NUM_POINTS);
}

#[test]
fn oversized_drc_counts_fail_before_large_allocation() {
    let mut oversized_mesh_faces = draco_header(2, 0, 1, 0);
    oversized_mesh_faces.extend_from_slice(&u32::MAX.to_le_bytes());
    oversized_mesh_faces.extend_from_slice(&8u32.to_le_bytes());
    oversized_mesh_faces.push(1); // raw connectivity, but no index payload

    let mut oversized_point_cloud_points = draco_header(2, 0, 0, 0);
    oversized_point_cloud_points.extend_from_slice(&u32::MAX.to_le_bytes());
    oversized_point_cloud_points.push(1); // one attribute decoder
    oversized_point_cloud_points.push(1); // one attribute in decoder (varint)
    oversized_point_cloud_points.push(0); // POSITION
    oversized_point_cloud_points.push(9); // FLOAT32
    oversized_point_cloud_points.push(3); // 3 components
    oversized_point_cloud_points.push(0); // not normalized
    oversized_point_cloud_points.push(0); // unique id (varint)
    oversized_point_cloud_points.push(0); // raw decoder type

    let mut oversized_kd_point_cloud_points = draco_header(2, 0, 0, 1);
    oversized_kd_point_cloud_points.extend_from_slice(&u32::MAX.to_le_bytes());
    oversized_kd_point_cloud_points.push(1); // one attribute decoder
    append_varint(&mut oversized_kd_point_cloud_points, 1); // one attribute
    oversized_kd_point_cloud_points.push(0); // POSITION
    oversized_kd_point_cloud_points.push(9); // FLOAT32
    oversized_kd_point_cloud_points.push(3); // 3 components
    oversized_kd_point_cloud_points.push(0); // not normalized
    append_varint(&mut oversized_kd_point_cloud_points, 0); // unique id

    let cases = [
        (
            "oversized mesh face count",
            DecoderKind::Mesh,
            oversized_mesh_faces,
        ),
        (
            "oversized point-cloud point count",
            DecoderKind::PointCloud,
            oversized_point_cloud_points,
        ),
        (
            "oversized KD-tree point-cloud point count",
            DecoderKind::PointCloud,
            oversized_kd_point_cloud_points,
        ),
    ];

    for (name, kind, bytes) in cases {
        assert!(
            decode_malformed_without_panic(kind, &bytes).is_err(),
            "{name} unexpectedly decoded successfully"
        );
    }
}

#[test]
fn oversized_edgebreaker_counts_fail_before_large_allocation() {
    // Exact libFuzzer reproducer (fuzz target `decode_drc`) for an EdgeBreaker
    // mesh stream whose declared geometry counts drove a multi-gigabyte
    // allocation before any payload was read. It must now fail as a controlled
    // error without panicking or exhausting memory.
    let oom_reproducer: [u8; 115] = [
        68, 82, 65, 67, 79, 2, 2, 1, 1, 0, 0, 0, 25, 32, 0, 32, 3, 0, 10, 239, 211, 234, 83, 173,
        234, 83, 213, 170, 26, 255, 1, 17, 1, 255, 0, 0, 1, 0, 9, 3, 0, 0, 2, 1, 1, 1, 0, 15, 3,
        21, 46, 61, 10, 39, 33, 5, 145, 2, 6, 168, 166, 116, 234, 255, 161, 255, 255, 255, 15, 0,
        252, 127, 255, 15, 0, 0, 64, 255, 7, 0, 64, 128, 8, 0, 0, 68, 0, 4, 4, 2, 0, 0, 0, 0, 255,
        63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 64, 14,
    ];
    assert_both_decoders_do_not_panic(&oom_reproducer);

    // Synthetic v2.2 EdgeBreaker mesh declaring an impossible face count. The
    // geometric count guards must reject it before allocating connectivity.
    let mut oversized_edgebreaker_faces = draco_header(2, 2, 1, 1);
    oversized_edgebreaker_faces.push(0); // traversal decoder type = standard
    append_varint(&mut oversized_edgebreaker_faces, 4); // num_encoded_vertices
    append_varint(&mut oversized_edgebreaker_faces, u32::MAX as u64); // num_faces
    assert!(
        decode_malformed_without_panic(DecoderKind::Mesh, &oversized_edgebreaker_faces).is_err(),
        "oversized EdgeBreaker face count unexpectedly decoded successfully"
    );
}

#[test]
fn oversized_sequential_mesh_point_count_fails_before_large_allocation() {
    // libFuzzer reproducer (fuzz target `decode_drc`): an 18-byte v2.2 sequential
    // mesh with num_faces = 0 but a ~billion-point num_points varint. Connectivity
    // is skipped, but attribute decode then sized per-attribute buffers by
    // num_points and accumulated multiple gigabytes. The point count must be
    // rejected against the remaining input before those buffers are allocated.
    let seq_mesh_point_count_oom: [u8; 18] = [
        68, 82, 65, 67, 79, 2, 2, 1, 0, 0, 1, 0, 255, 255, 255, 255, 68, 11,
    ];
    assert!(
        decode_malformed_without_panic(DecoderKind::Mesh, &seq_mesh_point_count_oom).is_err(),
        "oversized sequential mesh point count unexpectedly decoded successfully"
    );
}

#[test]
fn sequential_mesh_identity_mapping_does_not_materialize_huge_point_count() {
    // The identity mapping is implicit in sequential attribute data. Before
    // the fix, this header made MeshDecoder allocate four bytes per point even
    // though there are no attributes to decode. Padding reaches the old
    // input-relative allocation threshold; the decoder must still do no large
    // allocation because the identity is now represented symbolically.
    let mut stream = draco_header(2, 2, 1, 0);
    append_varint(&mut stream, 0); // zero faces, so connectivity is skipped
    append_varint(&mut stream, 1 << 30); // a hostile but representable point count
    stream.push(0); // no attribute decoders
    stream.resize(4096, 0); // 4 GiB / 2^20: inclusive budget boundary

    let mut buffer = DecoderBuffer::new(&stream);
    let mut mesh = Mesh::new();
    MeshDecoder::new()
        .decode(&mut buffer, &mut mesh)
        .expect("symbolic identity mapping should avoid the huge allocation");
    assert_eq!(mesh.num_points(), 1 << 30);
}

#[test]
fn sequential_mesh_attribute_buffer_does_not_materialize_huge_point_count() {
    // Attribute metadata is read before its payload. The decoder must not
    // reserve one value per claimed point before checking whether the input
    // can back that allocation.
    let mut stream = draco_header(2, 2, 1, 0);
    append_varint(&mut stream, 0); // zero faces, so connectivity is skipped
    append_varint(&mut stream, 1 << 30); // a hostile but representable point count
    append_varint(&mut stream, 1); // one attribute decoder
    append_varint(&mut stream, 1); // one attribute in the decoder
    stream.extend_from_slice(&[0, 9, 1, 0]); // position, float32, one component, unnormalized
    append_varint(&mut stream, 0); // unique id
    stream.push(0); // generic decoder

    assert!(
        decode_malformed_without_panic(DecoderKind::Mesh, &stream).is_err(),
        "huge sequential attribute buffer unexpectedly decoded successfully"
    );
}

#[test]
fn oversized_texcoord_orientations_decode_quickly_without_hang() {
    // libFuzzer reproducer (fuzz target `decode_drc`): a v2.2 EdgeBreaker mesh
    // whose portable-texcoord prediction declared a ~2 billion orientation count
    // (raw i32). Each orientation is rANS-bit-decoded in a loop, so the decode
    // spun for ~2 seconds and reserved gigabytes before failing. The count must
    // now be rejected against the remaining input so decode returns promptly.
    let texcoord_orientation_dos: [u8; 224] = [
        68, 82, 65, 67, 79, 2, 2, 1, 1, 0, 0, 0, 8, 12, 2, 11, 0, 0, 3, 95, 75, 21, 1, 1, 16, 85,
        4, 138, 172, 164, 70, 85, 4, 138, 172, 164, 70, 3, 255, 0, 1, 0, 1, 0, 1, 1, 0, 1, 0, 9, 3,
        0, 0, 2, 1, 3, 9, 2, 0, 1, 2, 1, 1, 9, 3, 0, 2, 3, 1, 1, 1, 0, 3, 3, 1, 48, 1, 16, 3, 0,
        40, 150, 142, 8, 4, 0, 0, 0, 0, 0, 255, 255, 15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 128, 63, 20, 5, 1, 1, 0, 13, 3, 85, 13, 173, 18, 7, 1, 8, 23, 1, 24, 8, 141, 130, 114,
        195, 183, 60, 131, 141, 6, 188, 252, 191, 191, 229, 251, 191, 191, 252, 203, 191, 186, 129,
        252, 203, 63, 154, 252, 203, 255, 251, 191, 127, 2, 246, 251, 203, 62, 46, 255, 239, 255,
        254, 242, 15, 12, 0, 0, 62, 1, 2, 192, 64, 0, 0, 0, 0, 255, 15, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 128, 63, 12, 6, 3, 1, 1, 3, 1, 1, 64, 1, 0, 255, 15, 0, 0, 255, 7, 0, 0, 255, 2,
        161, 65, 12,
    ];
    let start = std::time::Instant::now();
    assert_both_decoders_do_not_panic(&texcoord_orientation_dos);
    // The fixed decode returns in well under a millisecond; the pre-fix bug took
    // ~2 seconds. A 1-second budget catches a regression without CI flakiness.
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "texcoord orientation decode took too long: {:?}",
        start.elapsed()
    );
}

#[test]
fn oversized_rans_symbol_table_fails_before_large_allocation() {
    // libFuzzer reproducer (fuzz target `decode_drc`): a v2.2 EdgeBreaker mesh
    // whose tagged-symbol attribute stream declared a ~5 billion entry rANS
    // probability table, which drove a ~43 GB allocation in
    // RAnsSymbolDecoder::decode_table. The declared table size must now be
    // rejected against the remaining input before the table is allocated.
    let rans_table_oom: [u8; 209] = [
        68, 82, 65, 67, 79, 2, 2, 1, 1, 0, 0, 0, 12, 4, 0, 4, 0, 0, 2, 255, 15, 255, 2, 68, 64, 1,
        255, 0, 0, 2, 0, 9, 3, 0, 0, 1, 9, 3, 0, 1, 2, 3, 0, 1, 1, 0, 17, 3, 85, 5, 85, 5, 51, 89,
        53, 4, 181, 166, 147, 128, 255, 255, 1, 128, 0, 0, 255, 255, 2, 128, 0, 0, 233, 255, 15, 0,
        4, 0, 34, 240, 255, 23, 0, 4, 0, 232, 255, 7, 0, 12, 0, 252, 255, 255, 255, 92, 92, 92, 92,
        92, 92, 92, 92, 92, 92, 92, 92, 92, 92, 92, 92, 92, 92, 92, 92, 92, 92, 92, 92, 19, 0, 252,
        255, 15, 0, 4, 0, 0, 0, 232, 255, 15, 0, 244, 255, 255, 255, 19, 0, 20, 254, 255, 1, 0, 3,
        0, 1, 0, 0, 0, 0, 255, 255, 0, 0, 0, 3, 1, 0, 9, 3, 173, 42, 19, 85, 5, 1, 16, 4, 4, 189,
        250, 145, 128, 63, 88, 111, 194, 240, 48, 28, 234, 3, 255, 0, 0, 0, 127, 0, 0, 0, 10, 215,
        35, 188, 10, 215, 163, 187, 10, 215, 163, 187, 10, 215, 163, 60, 16, 8,
    ];
    assert_both_decoders_do_not_panic(&rans_table_oom);
}

#[test]
fn semantically_invalid_drc_payloads_fail_without_panic() {
    let mut impossible_point_cloud_attribute_count = draco_header(2, 0, 0, 0);
    impossible_point_cloud_attribute_count.extend_from_slice(&1u32.to_le_bytes());
    impossible_point_cloud_attribute_count.push(1); // one attribute decoder
    append_varint(&mut impossible_point_cloud_attribute_count, u32::MAX as u64);

    let mut invalid_point_cloud_attribute_type = draco_header(2, 0, 0, 0);
    invalid_point_cloud_attribute_type.extend_from_slice(&1u32.to_le_bytes());
    invalid_point_cloud_attribute_type.push(1); // one attribute decoder
    append_varint(&mut invalid_point_cloud_attribute_type, 1); // one attribute
    invalid_point_cloud_attribute_type.push(99); // invalid attribute type
    invalid_point_cloud_attribute_type.push(9); // FLOAT32
    invalid_point_cloud_attribute_type.push(3); // components
    invalid_point_cloud_attribute_type.push(0); // normalized
    append_varint(&mut invalid_point_cloud_attribute_type, 0); // unique id
    invalid_point_cloud_attribute_type.push(0); // raw decoder
    invalid_point_cloud_attribute_type.extend_from_slice(&[0; 12]);

    let mut zero_component_mesh_attribute = draco_header(2, 0, 1, 0);
    zero_component_mesh_attribute.extend_from_slice(&1u32.to_le_bytes()); // faces
    zero_component_mesh_attribute.extend_from_slice(&1u32.to_le_bytes()); // points
    zero_component_mesh_attribute.push(1); // raw connectivity
    zero_component_mesh_attribute.extend_from_slice(&[0, 0, 0]); // u8 indices
    zero_component_mesh_attribute.push(1); // one attribute decoder
    append_varint(&mut zero_component_mesh_attribute, 1); // one attribute
    zero_component_mesh_attribute.push(0); // POSITION
    zero_component_mesh_attribute.push(9); // FLOAT32
    zero_component_mesh_attribute.push(0); // invalid component count
    zero_component_mesh_attribute.push(0); // normalized
    append_varint(&mut zero_component_mesh_attribute, 0); // unique id
    zero_component_mesh_attribute.push(0); // raw decoder

    let cases = [
        (
            "impossible point-cloud attribute count",
            DecoderKind::PointCloud,
            impossible_point_cloud_attribute_count,
        ),
        (
            "invalid point-cloud attribute type",
            DecoderKind::PointCloud,
            invalid_point_cloud_attribute_type,
        ),
        (
            "zero-component mesh attribute",
            DecoderKind::Mesh,
            zero_component_mesh_attribute,
        ),
    ];

    for (name, kind, bytes) in cases {
        assert!(
            decode_malformed_without_panic(kind, &bytes).is_err(),
            "{name} unexpectedly decoded successfully"
        );
    }
}

#[test]
fn mutated_supported_drc_inputs_do_not_panic() {
    let fixture_names = [
        "legacy_draco/cube_att.mesh_seq.1.0.0.drc",
        "legacy_draco/cube_att.mesh_eb.1.1.0.drc",
        "legacy_draco/point_cloud_pos_norm.seq.1.0.0.drc",
        "point_cloud_no_qp.drc",
    ];

    for fixture in fixture_names {
        let original = std::fs::read(repo_testdata_dir().join(fixture))
            .unwrap_or_else(|e| panic!("failed to read {fixture}: {e}"));
        assert!(
            decode_by_header_without_panic(&original).is_ok(),
            "{fixture} should be a valid baseline fixture"
        );

        let truncation_points = [
            0,
            1,
            4,
            5,
            8,
            10,
            original.len() / 4,
            original.len() / 2,
            original.len().saturating_sub(1),
        ];
        for len in truncation_points {
            let len = len.min(original.len());
            let truncated = &original[..len];
            let _ = decode_by_header_without_panic(truncated);
        }

        let mutation_offsets = [
            0,
            5,
            6,
            7,
            8,
            10,
            original.len() / 3,
            original.len() / 2,
            original.len().saturating_sub(1),
        ];
        for offset in mutation_offsets {
            if offset >= original.len() {
                continue;
            }
            let mut mutated = original.clone();
            mutated[offset] ^= 0xA5;
            let _ = decode_by_header_without_panic(&mutated);
        }

        let mut extended = original.clone();
        extended.extend_from_slice(&[0x80, 0x80, 0x80, 0x80, 0x00]);
        let _ = decode_by_header_without_panic(&extended);
    }
}

#[test]
fn corrupted_edgebreaker_drc_sections_do_not_panic() {
    let fixture_names = [
        // Has attribute seam data.
        "production_draco/cube_att.mesh_eb.v2.2.pos_norm_uv.drc",
        // Has split symbols.
        "production_draco/test_pos_color.mesh_eb.v2.2.pos_color.drc",
        // Has multiple attribute payloads and seam-style side streams.
        "production_draco/blender_multi_color.mesh_eb.v2.2.pos_norm_uv_color012.drc",
    ];

    for fixture in fixture_names {
        let original = std::fs::read(repo_testdata_dir().join(fixture))
            .unwrap_or_else(|e| panic!("failed to read {fixture}: {e}"));
        assert!(
            decode_malformed_without_panic(DecoderKind::Mesh, &original).is_ok(),
            "{fixture} should be a valid baseline fixture"
        );

        let truncation_points = [
            original.len() / 3,
            original.len() / 2,
            original.len() * 2 / 3,
            original.len().saturating_sub(32),
            original.len().saturating_sub(16),
            original.len().saturating_sub(8),
            original.len().saturating_sub(1),
        ];
        for len in truncation_points {
            let len = len.min(original.len());
            let _ = decode_malformed_without_panic(DecoderKind::Mesh, &original[..len]);
        }

        let mutation_offsets = [
            10,
            original.len() / 4,
            original.len() / 3,
            original.len() / 2,
            original.len() * 2 / 3,
            original.len() * 3 / 4,
            original.len().saturating_sub(24),
            original.len().saturating_sub(12),
            original.len().saturating_sub(2),
        ];
        for offset in mutation_offsets {
            if offset >= original.len() {
                continue;
            }
            for mask in [0x01, 0x7F, 0x80, 0xFF] {
                let mut mutated = original.clone();
                mutated[offset] ^= mask;
                let _ = decode_malformed_without_panic(DecoderKind::Mesh, &mutated);
            }
        }
    }
}

#[test]
fn synthetic_drc_like_inputs_do_not_panic() {
    let lengths = [
        0usize, 1, 2, 4, 5, 8, 10, 11, 12, 16, 24, 31, 32, 48, 64, 96, 128, 192, 256,
    ];
    let seeds = [
        0u64,
        1,
        0x44_52_41_43_4f,
        0x0202_0100,
        0xa5a5_a5a5_a5a5_a5a5,
        0xffff_ffff_ffff_ffff,
    ];

    for len in lengths {
        for seed in seeds {
            let bytes = fill_pseudo_random_bytes(seed, len);
            assert_both_decoders_do_not_panic(&bytes);
        }
    }

    for geometry in [0u8, 1, 2, 255] {
        for method in [0u8, 1, 2, 3, 255] {
            for version in [(0, 0), (1, 0), (1, 1), (2, 0), (2, 2), (255, 255)] {
                let mut bytes = draco_header(version.0, version.1, geometry, method);
                bytes.extend_from_slice(&1u32.to_le_bytes());
                bytes.extend_from_slice(&3u32.to_le_bytes());
                bytes.push(1);
                bytes.extend_from_slice(&fill_pseudo_random_bytes(
                    ((geometry as u64) << 32) | ((method as u64) << 16) | version.0 as u64,
                    64,
                ));
                let _ = decode_by_header_without_panic(&bytes);
            }
        }
    }
}

#[test]
fn deterministic_fuzz_like_drc_inputs_do_not_panic() {
    const CASES: usize = 96;
    const MAX_LEN: usize = 768;

    let mut seed = 0x4452_4143_4f5f_6675u64;
    for case_id in 0..CASES {
        seed = seed
            .wrapping_mul(0xd134_2543_de82_ef95)
            .wrapping_add(0x9e37_79b9_7f4a_7c15);
        let len = ((seed >> 17) as usize % MAX_LEN).saturating_add(case_id % 11);
        let bytes = deterministic_fuzz_case(seed ^ case_id as u64, len);

        assert_both_decoders_do_not_panic(&bytes);
    }
}

#[test]
fn decode_rejects_truncated_file() {
    let path = repo_testdata_dir().join("cube_att.drc");
    let bytes = std::fs::read(&path).expect("failed to read cube_att.drc");
    assert!(bytes.len() > 16, "unexpectedly small cube_att.drc");

    // Truncate the tail; should fail gracefully (no panic).
    let truncated = &bytes[0..bytes.len() - 7];

    // Use header byte to select decoder (this file is a mesh).
    let mut buffer = DecoderBuffer::new(truncated);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let status = decoder.decode(&mut buffer, &mut mesh);

    assert!(status.is_err());
}

#[test]
// #[ignore = "Empty mesh encoding/decoding is an edge case - decoder expects at least one attribute"]
fn encode_decode_empty_mesh() {
    let mesh = Mesh::new();

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let options = EncoderOptions::new();
    let mut enc = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut enc);
    assert!(
        status.is_ok(),
        "empty mesh encode failed: {:?}",
        status.err()
    );

    let mut buffer = DecoderBuffer::new(enc.data());
    let mut decoded = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let status = decoder.decode(&mut buffer, &mut decoded);
    assert!(
        status.is_ok(),
        "empty mesh decode failed: {:?}",
        status.err()
    );

    assert_eq!(decoded.num_faces(), 0);
    assert_eq!(decoded.num_points(), 0);
    assert_eq!(decoded.num_attributes(), 0);
}

#[test]
fn encode_decode_empty_point_cloud() {
    let pc = PointCloud::new();

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);

    let options = EncoderOptions::new();
    let mut enc = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut enc);
    assert!(
        status.is_ok(),
        "empty point cloud encode failed: {:?}",
        status.err()
    );

    let mut buffer = DecoderBuffer::new(enc.data());
    let mut decoded = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    let status = decoder.decode(&mut buffer, &mut decoded);
    assert!(
        status.is_ok(),
        "empty point cloud decode failed: {:?}",
        status.err()
    );

    assert_eq!(decoded.num_points(), 0);
    assert_eq!(decoded.num_attributes(), 0);
}

/// Legacy valence EdgeBreaker streams (bitstream < 2.2) carry a separate main
/// traversal symbol stream plus a split-count/mode prefix that the modern
/// (>= 2.2) valence layout dropped. These fixtures are the same Stanford bunny
/// encoded by the official C++ tools of Draco 0.10.0 (bitstream 1.2), 1.0.0
/// (2.0), and 1.1.0 (2.1) — the two distinct pre-2.2 split-count encodings — and
/// must all decode to the same reference geometry as the modern encoders.
#[cfg(all(
    feature = "legacy_bitstream_decode",
    feature = "edgebreaker_valence_decode"
))]
#[test]
fn legacy_valence_edgebreaker_streams_decode_to_reference_geometry() {
    // Identical to the modern-encoded bun_zipper.ply geometry.
    const EXPECTED_FACES: usize = 69451;
    const EXPECTED_POINTS: usize = 34834;

    let fixtures = [
        "legacy_draco/bun_zipper.mesh_eb_valence.0.10.0.drc",
        "legacy_draco/bun_zipper.mesh_eb_valence.1.0.0.drc",
        "legacy_draco/bun_zipper.mesh_eb_valence.1.1.0.drc",
    ];

    for fixture in fixtures {
        let bytes = std::fs::read(repo_testdata_dir().join(fixture))
            .unwrap_or_else(|e| panic!("failed to read {fixture}: {e}"));

        let mut buffer = DecoderBuffer::new(&bytes);
        let mut mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();
        decoder
            .decode(&mut buffer, &mut mesh)
            .unwrap_or_else(|e| panic!("{fixture} failed to decode: {e:?}"));

        assert_eq!(
            mesh.num_faces(),
            EXPECTED_FACES,
            "{fixture}: unexpected face count"
        );
        assert_eq!(
            mesh.num_points(),
            EXPECTED_POINTS,
            "{fixture}: unexpected point count"
        );
    }
}

/// Pre-2.2 constrained-multi-parallelogram prediction prefixes a prediction-mode
/// byte that the 2.2 layout dropped. Without reading it, that byte is consumed as
/// the first crease-edge flag count, leaving the streams empty and breaking
/// prediction. The same positions-only sphere encoded at bitstream 1.1.0 (cl 10,
/// which selects this predictor) must decode to the same geometry as the 2.2
/// encoding.
#[cfg(feature = "legacy_bitstream_decode")]
#[test]
fn legacy_constrained_multi_parallelogram_decodes_to_reference_geometry() {
    fn decode(fixture: &str) -> Mesh {
        let bytes = std::fs::read(repo_testdata_dir().join(fixture))
            .unwrap_or_else(|e| panic!("read {fixture}: {e}"));
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut mesh = Mesh::new();
        MeshDecoder::new()
            .decode(&mut buffer, &mut mesh)
            .unwrap_or_else(|e| panic!("{fixture} decode: {e:?}"));
        mesh
    }
    fn sorted_positions(mesh: &Mesh) -> Vec<[u32; 3]> {
        use draco_core::geometry_attribute::GeometryAttributeType;
        let att = mesh.attribute(mesh.named_attribute_id(GeometryAttributeType::Position));
        let buffer = att.buffer();
        let stride = att.byte_stride() as usize;
        let mut out = Vec::with_capacity(att.size());
        for i in 0..att.size() {
            let off = i * stride;
            let mut v = [0u32; 3];
            for (k, component) in v.iter_mut().enumerate() {
                let mut b = [0u8; 4];
                buffer.read(off + k * 4, &mut b);
                *component = u32::from_le_bytes(b);
            }
            out.push(v);
        }
        out.sort_unstable();
        out
    }

    let early = decode("legacy_draco/sphere_pos.mesh_eb_cmp.1.1.0.drc");
    let modern = decode("legacy_draco/sphere_pos.mesh_eb_cmp.2.2.drc");
    assert_eq!(early.num_faces(), modern.num_faces(), "face count");
    assert_eq!(early.num_points(), modern.num_points(), "point count");
    assert_eq!(
        sorted_positions(&early),
        sorted_positions(&modern),
        "pre-2.2 constrained-multi geometry differs from the 2.2 encoding"
    );
}

/// Non-position attributes (normals via the octahedron transform, colors) must
/// decode to the same *values* pre-2.2 as at 2.2, not merely without error. Each
/// pair is the same mesh encoded by the C++ tools at bitstream 1.1.0 and 2.2 with
/// identical quantization; every attribute's value set must match.
#[cfg(feature = "legacy_bitstream_decode")]
#[test]
fn legacy_attribute_streams_decode_to_reference_values() {
    fn decode(fixture: &str) -> Mesh {
        let bytes = std::fs::read(repo_testdata_dir().join(fixture))
            .unwrap_or_else(|e| panic!("read {fixture}: {e}"));
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut mesh = Mesh::new();
        MeshDecoder::new()
            .decode(&mut buffer, &mut mesh)
            .unwrap_or_else(|e| panic!("{fixture} decode: {e:?}"));
        mesh
    }
    // For each attribute, its element byte patterns, sorted (order-independent).
    fn sorted_attrs(mesh: &Mesh) -> Vec<Vec<Vec<u8>>> {
        (0..mesh.num_attributes())
            .map(|a| {
                let att = mesh.attribute(a);
                let stride = att.byte_stride() as usize;
                let mut vals: Vec<Vec<u8>> = (0..att.size())
                    .map(|i| {
                        let mut b = vec![0u8; stride];
                        att.buffer().read(i * stride, &mut b);
                        b
                    })
                    .collect();
                vals.sort();
                vals
            })
            .collect()
    }

    for (early_f, modern_f) in [
        (
            "legacy_draco/sphere.mesh_eb_norm.1.1.0.drc",
            "legacy_draco/sphere.mesh_eb_norm.2.2.drc",
        ),
        (
            "legacy_draco/test.mesh_eb_color.1.1.0.drc",
            "legacy_draco/test.mesh_eb_color.2.2.drc",
        ),
    ] {
        let early = decode(early_f);
        let modern = decode(modern_f);
        assert_eq!(early.num_faces(), modern.num_faces(), "{early_f}: faces");
        assert_eq!(
            early.num_attributes(),
            modern.num_attributes(),
            "{early_f}: attribute count"
        );
        assert_eq!(
            sorted_attrs(&early),
            sorted_attrs(&modern),
            "{early_f}: attribute values differ from the 2.2 encoding"
        );
    }
}

/// The pair above never puts a `GENERIC` attribute on a legacy stream, so it
/// never reaches `SequentialIntegerAttributeDecoder` -- position, normal and
/// texcoord are all quantized floats decoded by a different coder entirely.
/// `cube_att_material.obj` is `cube_att.obj` (the source behind every other
/// `cube_att.*` fixture in this suite) with its twelve faces split `usemtl
/// matA`/`matB`; the real `draco_encoder`'s OBJ reader turns that into a
/// `Uint8` `GENERIC` attribute carrying the material id, one value per
/// point. There is no C++-side ground truth to diff against here: the
/// historical 1.0.0 OBJ writer drops a `GENERIC` attribute on export even
/// though the bitstream carries it (verified separately -- the 1.0.0 file is
/// larger with the attribute than without, and this crate decodes four
/// distinct point values from it), and neither writer round-trips one at all
/// without `mtllib`/`usemtl` in the source plus an attribute named "material"
/// in the stream's own metadata. So this compares real 1.0.0/1.1.0 encodes,
/// sequential and EdgeBreaker, against a real 2.2 encode of the same source
/// at matching quantization (`-qp 14 -qt 12 -qn 10`), exactly as the pair
/// above compares normal/color -- except `GENERIC` here is `Uint8`, which
/// quantization never touches, so unlike position/normal/texcoord this one
/// attribute is expected exact rather than tolerance-close for reasons other
/// than the coder being tested.
#[cfg(feature = "legacy_bitstream_decode")]
#[test]
fn legacy_material_attribute_matches_the_modern_reference() {
    use draco_core::draco_types::DataType;
    use draco_core::geometry_attribute::GeometryAttributeType;

    fn decode(fixture: &str) -> Mesh {
        let bytes = std::fs::read(repo_testdata_dir().join(fixture))
            .unwrap_or_else(|e| panic!("read {fixture}: {e}"));
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut mesh = Mesh::new();
        MeshDecoder::new()
            .decode(&mut buffer, &mut mesh)
            .unwrap_or_else(|e| panic!("{fixture} decode: {e:?}"));
        mesh
    }

    // One row per mesh point: position (for matching, not compared directly)
    // paired with the material id read off that point, since GENERIC has no
    // named accessor and must be joined to a stable key by hand.
    fn position_to_material(mesh: &Mesh) -> Vec<([u8; 12], u8)> {
        let pos_id = mesh.named_attribute_id(GeometryAttributeType::Position);
        let mat_id = mesh.named_attribute_id(GeometryAttributeType::Generic);
        assert!(pos_id >= 0, "missing POSITION attribute");
        assert!(mat_id >= 0, "missing GENERIC attribute");
        let pos = mesh.attribute(pos_id);
        let mat = mesh.attribute(mat_id);
        assert_eq!(mat.data_type(), DataType::Uint8, "material should be Uint8");
        assert_eq!(mat.num_components(), 1);

        let mut rows: Vec<([u8; 12], u8)> = (0..mesh.num_points())
            .map(|point_value| {
                let point = draco_core::geometry_indices::PointIndex(point_value as u32);
                let pos_index = pos.mapped_index(point).0 as usize;
                let pos_offset = pos_index * pos.byte_stride() as usize;
                let mut pos_bytes = [0u8; 12];
                pos.buffer().read(pos_offset, &mut pos_bytes);

                let mat_index = mat.mapped_index(point).0 as usize;
                let mut mat_byte = [0u8; 1];
                mat.buffer()
                    .read(mat_index * mat.byte_stride() as usize, &mut mat_byte);

                (pos_bytes, mat_byte[0])
            })
            .collect();
        rows.sort();
        rows
    }

    let modern = decode("legacy_draco/cube_att_material.mesh_eb.2.2.drc");
    let modern_rows = position_to_material(&modern);

    for early_f in [
        "legacy_draco/cube_att_material.mesh_seq.1.0.0.drc",
        "legacy_draco/cube_att_material.mesh_eb.1.0.0.drc",
        "legacy_draco/cube_att_material.mesh_seq.1.1.0.drc",
        "legacy_draco/cube_att_material.mesh_eb.1.1.0.drc",
    ] {
        let early = decode(early_f);
        assert_eq!(early.num_faces(), modern.num_faces(), "{early_f}: faces");
        assert_eq!(early.num_points(), modern.num_points(), "{early_f}: points");
        assert_eq!(
            position_to_material(&early),
            modern_rows,
            "{early_f}: (position, material) pairs differ from the 2.2 encoding"
        );
    }
}

/// The 0.9.1 normals use the legacy non-canonicalized octahedron prediction
/// transform (id 2) over a predictive EdgeBreaker connectivity. Unlike the
/// 1.1.0/2.2 pair above, the 0.9.1 encoder quantizes the octahedral grid
/// differently, so we compare against a golden reference captured from the
/// historical Draco 0.9.1 C++ decoder rather than a 2.2 fixture.
#[cfg(feature = "legacy_bitstream_decode")]
#[test]
fn legacy_091_octahedron_normals_match_historical_decoder() {
    use draco_core::geometry_attribute::GeometryAttributeType;
    let bytes =
        std::fs::read(repo_testdata_dir().join("legacy_draco/sphere.mesh_eb_norm.0.9.1.drc"))
            .expect("read 0.9.1 normal fixture");
    let mut buffer = DecoderBuffer::new(&bytes);
    let mut mesh = Mesh::new();
    MeshDecoder::new()
        .decode(&mut buffer, &mut mesh)
        .expect("decode 0.9.1 normals");

    let att = mesh.attribute(mesh.named_attribute_id(GeometryAttributeType::Normal));
    let stride = att.byte_stride() as usize;
    let mut got: Vec<[u32; 3]> = (0..att.size())
        .map(|i| {
            let off = i * stride;
            let mut n = [0u32; 3];
            for (k, c) in n.iter_mut().enumerate() {
                let mut b = [0u8; 4];
                att.buffer().read(off + k * 4, &mut b);
                *c = u32::from_le_bytes(b);
            }
            n
        })
        .collect();
    got.sort();

    let golden = std::fs::read(
        repo_testdata_dir().join("legacy_draco/sphere.mesh_eb_norm.0.9.1.normals_golden.bin"),
    )
    .expect("read golden normals");
    let expected: Vec<[u32; 3]> = golden
        .as_chunks::<12>()
        .0
        .iter()
        .map(|c| {
            [
                u32::from_le_bytes(c[0..4].try_into().unwrap()),
                u32::from_le_bytes(c[4..8].try_into().unwrap()),
                u32::from_le_bytes(c[8..12].try_into().unwrap()),
            ]
        })
        .collect();

    assert_eq!(
        got, expected,
        "0.9.1 octahedron normals are not byte-exact vs the historical Draco decoder"
    );
}

#[test]
fn fuzz_timeout_edgebreaker_attribute_swing_reproducer_returns_quickly() {
    let bytes = std::fs::read(
        repo_testdata_dir()
            .join("fuzz_regressions/decode_drc_edgebreaker_attribute_swing_timeout.drc"),
    )
    .expect("read timeout reproducer");

    let start = std::time::Instant::now();
    assert!(
        decode_malformed_without_panic(DecoderKind::Mesh, &bytes).is_err(),
        "malformed EdgeBreaker stream should be rejected"
    );
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "malformed EdgeBreaker stream should not hang"
    );

    assert!(
        decode_malformed_without_panic(DecoderKind::PointCloud, &bytes).is_err(),
        "mesh reproducer should not decode as a point cloud"
    );
}

/// Draco 0.9.1 and earlier used the predictive ("type 1") EdgeBreaker traversal
/// by default (a binary prediction stream guessing R/C from local valence),
/// replaced by the valence traversal in 0.10.0. The 0.9.1 bunny must decode to
/// the same geometry as the valence-encoded bunny.
#[cfg(feature = "legacy_bitstream_decode")]
#[test]
fn legacy_predictive_edgebreaker_decodes_to_reference_geometry() {
    fn decode(fixture: &str) -> Mesh {
        let bytes = std::fs::read(repo_testdata_dir().join(fixture))
            .unwrap_or_else(|e| panic!("read {fixture}: {e}"));
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut mesh = Mesh::new();
        MeshDecoder::new()
            .decode(&mut buffer, &mut mesh)
            .unwrap_or_else(|e| panic!("{fixture} decode: {e:?}"));
        mesh
    }
    fn sorted_positions(mesh: &Mesh) -> Vec<[u32; 3]> {
        use draco_core::geometry_attribute::GeometryAttributeType;
        let att = mesh.attribute(mesh.named_attribute_id(GeometryAttributeType::Position));
        let buffer = att.buffer();
        let stride = att.byte_stride() as usize;
        let mut out = Vec::with_capacity(att.size());
        for i in 0..att.size() {
            let off = i * stride;
            let mut v = [0u32; 3];
            for (k, component) in v.iter_mut().enumerate() {
                let mut b = [0u8; 4];
                buffer.read(off + k * 4, &mut b);
                *component = u32::from_le_bytes(b);
            }
            out.push(v);
        }
        out.sort_unstable();
        out
    }

    let predictive = decode("legacy_draco/bun_zipper.mesh_eb_predictive.0.9.1.drc");
    let valence = decode("legacy_draco/bun_zipper.mesh_eb_valence.1.1.0.drc");
    assert_eq!(predictive.num_faces(), 69451, "predictive face count");
    assert_eq!(predictive.num_points(), 34834, "predictive point count");
    assert_eq!(
        sorted_positions(&predictive),
        sorted_positions(&valence),
        "0.9.1 predictive geometry differs from the valence-encoded bunny"
    );
}

/// A stream truncated inside the prediction data reports what ran out, not a
/// bare "failed to decode".
///
/// The prediction schemes and the attribute decoder returned `bool` through
/// 1.x, so every fault below `MeshDecoder::decode` arrived as one fixed
/// sentence. This walks a real encode, cuts it at successive lengths, and
/// requires that at least one truncation names the structure that was short.
#[test]
fn a_truncated_stream_names_what_ran_out() {
    let mut mesh = Mesh::new();
    let mut pos = draco_core::geometry_attribute::PointAttribute::new();
    pos.init(
        draco_core::geometry_attribute::GeometryAttributeType::Position,
        3,
        draco_core::draco_types::DataType::Float32,
        false,
        4,
    );
    let coords: [f32; 12] = [0., 0., 0., 1., 0., 0., 0., 1., 0., 1., 1., 0.];
    for (i, value) in coords.iter().enumerate() {
        pos.buffer_mut().write(i * 4, &value.to_le_bytes());
    }
    mesh.set_num_points(4);
    mesh.add_attribute(pos);
    mesh.set_num_faces(2);
    mesh.set_face_from_indices(0, [0, 1, 2]);
    mesh.set_face_from_indices(1, [1, 3, 2]);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 12);
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encode");
    let bytes = buffer.data().to_vec();

    let mut messages = Vec::new();
    for cut in (8..bytes.len()).rev() {
        let mut decoded = Mesh::new();
        if let Err(error) =
            MeshDecoder::new().decode(&mut DecoderBuffer::new(&bytes[..cut]), &mut decoded)
        {
            messages.push(error.to_string());
        }
    }
    // Specifically the wrap transform's own words: it sits under the prediction
    // scheme, under the integer attribute decoder, under `MeshDecoder::decode`.
    // Before those layers returned `Status` the message stopped at the attribute
    // decoder and arrived as "Failed to decode integer attribute values".
    assert!(
        messages
            .iter()
            .any(|m| m.contains("Stream ends before the wrap transform")),
        "no truncation carried the prediction transform's own message; saw: {messages:?}"
    );
}

#[test]
fn a_one_component_geometric_normal_fails_without_indexing_past_its_pair() {
    // libFuzzer reproducer (fuzz target `decode_drc`): a v2.2 EdgeBreaker mesh
    // whose geometric-normal prediction covers an attribute with a single
    // component. The octahedral transform reads and writes a coordinate pair
    // and indexed the second one unconditionally, so a one-component attribute
    // panicked on the correction it does not have. The encoding half refuses
    // any count but two, so no stream this project writes carries one.
    let one_component_geometric_normal: [u8; 227] = [
        68, 82, 65, 67, 79, 2, 2, 1, 1, 0, 0, 0, 8, 12, 3, 11, 0, 0, 3, 95, 75, 21, 1, 1, 16, 85,
        4, 138, 172, 164, 70, 85, 4, 138, 172, 164, 70, 128, 4, 0, 151, 98, 113, 4, 255, 0, 0, 0,
        1, 0, 1, 1, 0, 2, 1, 0, 1, 0, 9, 3, 0, 0, 2, 1, 3, 9, 2, 0, 2, 1, 1, 1, 9, 3, 0, 2, 3, 1,
        4, 2, 1, 0, 3, 1, 1, 1, 1, 0, 3, 3, 1, 32, 1, 32, 3, 0, 80, 67, 142, 8, 20, 10, 8, 0, 0, 0,
        0, 255, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 63, 14, 5, 1, 1, 1, 2, 3,
        85, 41, 1, 12, 173, 10, 10, 187, 74, 75, 252, 193, 10, 163, 236, 35, 145, 12, 0, 0, 0, 1,
        2, 192, 64, 0, 0, 0, 0, 255, 15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 63, 12, 6, 3, 1,
        1, 1, 1, 1, 64, 1, 0, 255, 3, 0, 0, 255, 1, 0, 0, 255, 2, 161, 65, 10, 1, 1, 1, 1, 2, 3,
        85, 53, 3, 173, 10, 4, 142, 132, 157, 130, 0, 0, 0, 0, 2, 0, 0, 0,
    ];
    // Asserted on the message rather than left to
    // `assert_both_decoders_do_not_panic`, which would pass on any refusal:
    // the component count is what has to be refused, and it has to be refused
    // before the transform runs.
    assert_eq!(
        decode_malformed_without_panic(DecoderKind::Mesh, &one_component_geometric_normal)
            .unwrap_err(),
        "DracoError { kind: InvalidParameter, message: \"Geometric normal prediction needs 2 octahedral components, got 1\" }"
    );
}

#[test]
fn a_bit_stream_longer_than_the_buffer_does_not_move_the_position_past_it() {
    // libFuzzer reproducer (fuzz target `decode_drc`): the size that precedes a
    // bit sequence is a varint out of the stream, and the end position built
    // from it was bounded only against overflow. A size a few bytes short of
    // `usize::MAX` put that position into the buffer's cursor when bit decoding
    // ended, and the next length check -- a sum that wrapped -- waved the read
    // through: `range start index 18446744073709551613 out of range for slice
    // of length 195`.
    //
    // Upstream hands its bit decoder `remaining_size()` and never positions
    // from the claim, so a stream claiming more than it carries is read to the
    // end rather than refused; the end position is clamped here for the same
    // reason.
    let bit_stream_outruns_the_buffer: [u8; 195] = [
        68, 82, 65, 67, 79, 1, 1, 1, 1, 0, 0, 1, 0, 0, 0, 55, 114, 0, 0, 0, 224, 0, 0, 0, 1, 223,
        0, 0, 0, 126, 0, 0, 0, 102, 0, 0, 0, 208, 255, 255, 255, 255, 255, 255, 255, 223, 243, 120,
        180, 143, 246, 209, 106, 219, 182, 109, 31, 218, 71, 235, 125, 180, 192, 67, 251, 104, 181,
        218, 71, 75, 171, 213, 122, 104, 31, 173, 86, 171, 125, 180, 180, 90, 15, 255, 255, 130,
        122, 45, 160, 1, 0, 0, 0, 0, 0, 0, 0, 1, 255, 3, 0, 0, 0, 102, 212, 128, 17, 0, 0, 0, 161,
        24, 0, 0, 0, 61, 183, 195, 79, 70, 62, 230, 227, 99, 170, 92, 154, 47, 153, 174, 54, 4, 7,
        4, 27, 131, 40, 64, 80, 0, 0, 0, 0, 0, 0, 0, 0, 2, 255, 0, 0, 0, 0, 64, 2, 96, 215, 174,
        110, 110, 207, 249, 47, 0, 0, 242, 143, 16, 211, 237, 224, 16, 0, 124, 245, 187, 189, 59,
        0, 42, 3, 242, 127, 117, 112, 0, 0, 255, 1, 0, 161, 65, 0, 0, 10,
    ];
    assert_both_decoders_do_not_panic(&bit_stream_outruns_the_buffer);
}
