use std::path::PathBuf;

use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;

mod fingerprint;
use fingerprint::*;

fn testdata_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
}

fn read_varint(data: &[u8], offset: &mut usize) -> u64 {
    let mut value = 0u64;
    let mut shift = 0;
    loop {
        let byte = data[*offset];
        *offset += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if (byte & 0x80) == 0 {
            return value;
        }
        shift += 7;
    }
}

fn sequential_connectivity_method(data: &[u8]) -> Option<u8> {
    if data.len() < 12 || &data[0..5] != b"DRACO" {
        return None;
    }
    let geometry_type = data[7];
    let method = data[8];
    if geometry_type != 1 || method != 0 {
        return None;
    }

    let mut offset = 11;
    let major = data[5];
    let minor = data[6];
    if (major, minor) >= (2, 2) {
        let _num_faces = read_varint(data, &mut offset);
        let _num_points = read_varint(data, &mut offset);
    } else {
        offset += 8;
    }
    data.get(offset).copied()
}

#[test]
fn cpp_and_rust_decode_fingerprints_match_for_mesh_fixtures() {
    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    let base = testdata_dir();
    let cases = [
        "reference_cpp/cpp_encoded_cube_speed_10.drc",
        "reference_cpp/cpp_encoded_cube_speed_0.drc",
        "legacy_draco/cube_att.mesh_seq.1.0.0.drc",
        "legacy_draco/cube_att.mesh_eb.1.1.0.drc",
        "production_draco/cube_att.mesh_eb.v2.2.pos_norm_uv.drc",
        "production_draco/test_pos_color.mesh_eb.v2.2.pos_color.drc",
    ];

    for case in cases {
        let data = std::fs::read(base.join(case)).expect("failed to read fixture");
        let rust = rust_decode_fingerprint(&data);
        let cpp =
            draco_cpp_test_bridge::decode_cpp_mesh_fingerprint(&data).expect("C++ decode failed");

        assert_eq!(rust.num_points, cpp.num_points, "{case}: num_points");
        assert_eq!(rust.num_faces, cpp.num_faces, "{case}: num_faces");
        assert_eq!(
            rust.num_attributes, cpp.num_attributes,
            "{case}: num_attributes"
        );
        assert_eq!(rust.face_hash, cpp.face_hash, "{case}: face_hash");
        assert_eq!(
            rust.canonical_corner_hash, cpp.canonical_corner_hash,
            "{case}: canonical_corner_hash"
        );
        assert_eq!(
            rust.attribute_hash, cpp.attribute_hash,
            "{case}: attribute_hash"
        );
    }
}

#[test]
fn cpp_and_rust_decode_fingerprints_match_for_multi_color_fixture() {
    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    let case = "production_draco/blender_multi_color.mesh_eb.v2.2.pos_norm_uv_color012.drc";
    let data = std::fs::read(testdata_dir().join(case)).expect("failed to read fixture");
    let rust = rust_decode_fingerprint(&data);
    let cpp = draco_cpp_test_bridge::decode_cpp_mesh_fingerprint(&data).expect("C++ decode failed");

    assert_eq!(rust.num_points, cpp.num_points, "{case}: num_points");
    assert_eq!(rust.num_faces, cpp.num_faces, "{case}: num_faces");
    assert_eq!(
        rust.num_attributes, cpp.num_attributes,
        "{case}: num_attributes"
    );
    assert_eq!(rust.face_hash, cpp.face_hash, "{case}: face_hash");
    assert_eq!(
        rust.canonical_corner_hash, cpp.canonical_corner_hash,
        "{case}: canonical_corner_hash"
    );
    assert_eq!(
        rust.attribute_hash, cpp.attribute_hash,
        "{case}: attribute_hash"
    );
}

#[test]
fn cpp_and_rust_decode_fingerprints_match_for_point_cloud_fixtures() {
    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    let base = testdata_dir();
    let cases = [
        "legacy_draco/point_cloud_pos_norm.seq.1.0.0.drc",
        "legacy_draco/point_cloud_pos_norm.seq.1.1.0.drc",
        "legacy_draco/point_cloud_pos_norm.kd.1.3.0.drc",
        "legacy_draco/point_cloud_pos.kd.1.0.0.drc",
        "legacy_draco/point_cloud_pos.kd.1.1.0.drc",
        "legacy_draco/point_cloud_pos.kd.1.2.5.drc",
        "production_draco/bpy_point_cloud.seq.v2.3.pos_norm_color.drc",
        "production_draco/bpy_point_cloud.kd.v2.3.pos_norm_color.drc",
    ];

    for case in cases {
        let data = std::fs::read(base.join(case)).expect("failed to read fixture");
        let rust = rust_decode_point_cloud_fingerprint(&data);
        let cpp = draco_cpp_test_bridge::decode_cpp_point_cloud_fingerprint(&data)
            .expect("C++ point-cloud decode failed");

        assert_eq!(rust.num_points, cpp.num_points, "{case}: num_points");
        assert_eq!(
            rust.num_attributes, cpp.num_attributes,
            "{case}: num_attributes"
        );
        assert_eq!(rust.num_faces, cpp.num_faces, "{case}: num_faces");
        assert_eq!(rust.face_hash, cpp.face_hash, "{case}: face_hash");
        assert_eq!(
            rust.canonical_corner_hash, cpp.canonical_corner_hash,
            "{case}: canonical_corner_hash"
        );
        assert_eq!(
            rust.attribute_hash, cpp.attribute_hash,
            "{case}: attribute_hash"
        );
    }
}

#[test]
fn cpp_compressed_sequential_connectivity_matches_rust_decode() {
    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    let positions = [
        0.0f32, 0.0, 0.0, //
        1.0, 0.0, 0.0, //
        1.0, 1.0, 0.0, //
        0.0, 1.0, 0.0, //
        2.0, 0.0, 0.0, //
        2.0, 1.0, 0.0, //
    ];
    let faces = [
        0u32, 2, 1, //
        0, 3, 2, //
        1, 5, 4, //
        1, 2, 5, //
        0, 5, 3, //
    ];

    let data =
        draco_cpp_test_bridge::encode_cpp_mesh_sequential(&positions, &faces, 5, 5, 14, true)
            .expect("C++ sequential compressed encode failed");
    assert_eq!(sequential_connectivity_method(&data), Some(0));

    let rust = rust_decode_fingerprint(&data);
    let cpp = draco_cpp_test_bridge::decode_cpp_mesh_fingerprint(&data).expect("C++ decode failed");

    assert_eq!(rust.num_points, cpp.num_points);
    assert_eq!(rust.num_faces, cpp.num_faces);
    assert_eq!(rust.num_attributes, cpp.num_attributes);
    assert_eq!(rust.face_hash, cpp.face_hash);
    assert_eq!(rust.attribute_hash, cpp.attribute_hash);
    assert_eq!(rust.canonical_corner_hash, cpp.canonical_corner_hash);
}

/// Points and no faces: C++ writes the connectivity method byte anyway, raw or
/// compressed, and reads it back. This encoder must write the raw stream byte
/// for byte, and this decoder must read both to what C++ reads.
#[test]
fn a_sequential_mesh_without_faces_matches_cpp_both_ways() {
    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    let positions = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let raw = draco_cpp_test_bridge::encode_cpp_mesh_sequential(&positions, &[], 10, 10, 11, false)
        .expect("C++ sequential encode failed");
    let compressed =
        draco_cpp_test_bridge::encode_cpp_mesh_sequential(&positions, &[], 10, 10, 11, true)
            .expect("C++ sequential compressed encode failed");
    assert_eq!(sequential_connectivity_method(&raw), Some(1));
    assert_eq!(sequential_connectivity_method(&compressed), Some(0));

    let mut mesh = Mesh::new();
    mesh.set_num_points(3);
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        3,
    );
    position.buffer_mut().update_f32s_le(0, &positions);
    mesh.add_attribute(position);
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", 10);
    options.set_global_int("decoding_speed", 10);
    options.set_attribute_int(0, "quantization_bits", 11);
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut written = EncoderBuffer::new();
    encoder
        .encode(&options, &mut written)
        .expect("Rust encode failed");
    assert_eq!(written.data(), &raw[..], "Rust and C++ raw streams differ");

    for (name, data) in [("raw", &raw), ("compressed", &compressed)] {
        let rust = rust_decode_fingerprint(data);
        let cpp =
            draco_cpp_test_bridge::decode_cpp_mesh_fingerprint(data).expect("C++ decode failed");
        assert_eq!(
            (
                rust.num_points,
                rust.num_faces,
                rust.num_attributes,
                rust.face_hash,
                rust.attribute_hash,
                rust.canonical_corner_hash
            ),
            (
                cpp.num_points,
                cpp.num_faces,
                cpp.num_attributes,
                cpp.face_hash,
                cpp.attribute_hash,
                cpp.canonical_corner_hash
            ),
            "{name}"
        );
        assert_eq!(rust.num_attributes, 1, "{name}");
    }
}

/// The pre-2.3 integer KD-tree layout, which no command-line encoder of that
/// era writes, built from this crate's 2.3 stream: version 2.2, a method byte
/// of 1 ahead of the compression level, the point count again behind it, and
/// the same tree. C++ Draco 1.5.7 reading it to the same fingerprint as this
/// decoder is what says the layout was read the way upstream reads it.
#[test]
fn cpp_reads_a_pre_2_3_integer_kd_tree_as_rust_does() {
    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }
    const POINTS: usize = 40;
    let mut cloud = draco_core::point_cloud::PointCloud::new();
    cloud.set_num_points(POINTS);
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Uint32,
        false,
        POINTS,
    );
    let mut weight = PointAttribute::new();
    weight.init(
        GeometryAttributeType::Generic,
        1,
        DataType::Uint8,
        false,
        POINTS,
    );
    for p in 0..POINTS {
        let v = p as u32;
        let xyz = [v * 97 % 5000, v * 31 % 300, 4000 - v * 13];
        let bytes: Vec<u8> = xyz.iter().flat_map(|c| c.to_le_bytes()).collect();
        position.buffer_mut().update(&bytes, Some(p * 12));
        weight.buffer_mut().update(&[(v * 7) as u8], Some(p));
    }
    cloud.add_attribute(position);
    cloud.add_attribute(weight);
    let mut encoder = draco_core::point_cloud_encoder::PointCloudEncoder::new();
    encoder.set_point_cloud(cloud);
    let mut written = EncoderBuffer::new();
    encoder
        .encode(&EncoderOptions::new(), &mut written)
        .unwrap();
    let modern = written.data().to_vec();
    assert_eq!((modern[5], modern[6], modern[8]), (2, 3, 1));

    const LEVEL: usize = 11 + 4 + 1 + 1 + 5 + 5;
    let mut legacy = modern[..LEVEL].to_vec();
    legacy[6] = 2;
    legacy.push(1);
    legacy.push(modern[LEVEL]);
    legacy.extend_from_slice(&(POINTS as u32).to_le_bytes());
    legacy.extend_from_slice(&modern[LEVEL + 1..]);

    let rust = rust_decode_point_cloud_fingerprint(&legacy);
    let cpp = draco_cpp_test_bridge::decode_cpp_point_cloud_fingerprint(&legacy)
        .expect("C++ point-cloud decode failed");
    assert_eq!(rust.num_points, cpp.num_points);
    assert_eq!(rust.num_attributes, cpp.num_attributes);
    assert_eq!(rust.attribute_hash, cpp.attribute_hash);
    assert_eq!(rust, rust_decode_point_cloud_fingerprint(&modern));
}
