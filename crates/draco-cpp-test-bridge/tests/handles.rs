use draco_cpp_test_bridge::{
    cpp_attribute, decode_cpp_mesh_attribute, decode_cpp_mesh_fingerprint, encode_cpp_mesh,
    is_available, profile_cpp_reencode_mesh, CppMesh,
};

/// A result is sized by the C++ side that made it, so its size has no ceiling
/// on the Rust side. A mesh whose positions alone are 1.2M floats is past any
/// fixed capacity a caller would think to allocate, and must come back whole
/// rather than as a failure.
#[test]
fn test_large_attribute_comes_back_whole() {
    if !is_available() {
        println!("C++ test bridge disabled; skipping test");
        return;
    }

    let side = 640u32;
    let mut positions = Vec::with_capacity((side * side * 3) as usize);
    for y in 0..side {
        for x in 0..side {
            positions.extend_from_slice(&[x as f32, y as f32, 0.0]);
        }
    }
    let mut faces = Vec::new();
    for y in 0..side - 1 {
        for x in 0..side - 1 {
            let i = y * side + x;
            faces.extend_from_slice(&[i, i + 1, i + side, i + 1, i + side + 1, i + side]);
        }
    }

    let encoded = encode_cpp_mesh(&positions, &faces, 10, 10, 14).expect("C++ encode failed");
    let points = decode_cpp_mesh_fingerprint(&encoded)
        .expect("C++ fingerprint failed")
        .num_points as usize;
    let values = decode_cpp_mesh_attribute(&encoded, cpp_attribute::POSITION)
        .expect("a large attribute must not read as a failure");

    assert!(values.len() > 1 << 20);
    assert_eq!(values.len(), points * 3);
}

#[test]
fn test_handle_encode_matches_single() {
    if !is_available() {
        println!("C++ test bridge disabled; skipping test");
        return;
    }

    // Simple triangle
    let positions: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let faces: Vec<u32> = vec![0, 1, 2];

    // Single-shot encoded bytes
    let single =
        encode_cpp_mesh(&positions, &faces, 10, 10, 10).expect("single-shot encoding failed");

    // Build mesh via handles
    let mut mesh = CppMesh::new().expect("failed to create CppMesh");
    let num_points = (positions.len() / 3) as u32;
    mesh.add_position_attribute(num_points, &positions)
        .expect("add_position_attribute failed");
    mesh.set_num_faces(1);
    mesh.set_face(0, 0, 1, 2);

    let handled = mesh
        .encode(10, 10, 10)
        .expect("handle-based encoding failed");

    assert_eq!(
        single, handled,
        "Encoded bytes should match between single-shot and handle encode"
    );
}

#[test]
fn test_reencode_profile_decodes_real_drc_before_encoding() {
    if !is_available() {
        println!("C++ test bridge disabled; skipping test");
        return;
    }

    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
        .join("bunny_cpp_standard.drc");
    let data = std::fs::read(path).expect("read bunny_cpp_standard.drc");

    let profile =
        profile_cpp_reencode_mesh(&data, 5, 5, 10, 2).expect("C++ re-encode profile failed");

    assert_eq!(profile.num_points, 34_834);
    assert_eq!(profile.num_faces, 69_451);
    assert!(profile.num_attributes > 0);
    assert!(profile.output_size > 0);
    assert!(profile.encode_time_us > 0);
}
