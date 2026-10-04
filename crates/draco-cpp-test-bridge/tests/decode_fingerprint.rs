use std::path::PathBuf;

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
