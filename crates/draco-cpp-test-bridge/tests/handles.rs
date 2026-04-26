use draco_cpp_test_bridge::{encode_cpp_mesh, encode_with_handles, is_available, CppMesh};

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

    let handled = encode_with_handles(&mesh, 10, 10, 10).expect("handle-based encoding failed");

    assert_eq!(
        single, handled,
        "Encoded bytes should match between single-shot and handle encode"
    );
}
