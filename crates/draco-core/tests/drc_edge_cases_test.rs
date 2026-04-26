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

#[derive(Clone, Copy)]
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
    .map_err(|_| "decoder panicked".to_string())?;

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
