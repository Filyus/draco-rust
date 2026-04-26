use draco_core::*;

#[test]
fn test_mesh_encoder_basic() {
    // Create a simple triangle mesh
    let mut mesh = Mesh::new();

    // Add position attribute (3 vertices)
    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3, // 3 components
        DataType::Int32,
        false,
        3, // 3 vertices
    );

    // Write vertex data
    {
        let buffer = pos_att.buffer_mut();

        // Vertex 0: (0, 0, 0)
        buffer.write(0, &0i32.to_le_bytes());
        buffer.write(4, &0i32.to_le_bytes());
        buffer.write(8, &0i32.to_le_bytes());

        // Vertex 1: (10, 0, 0)
        buffer.write(12, &10i32.to_le_bytes());
        buffer.write(16, &0i32.to_le_bytes());
        buffer.write(20, &0i32.to_le_bytes());

        // Vertex 2: (0, 10, 0)
        buffer.write(24, &0i32.to_le_bytes());
        buffer.write(28, &10i32.to_le_bytes());
        buffer.write(32, &0i32.to_le_bytes());
    }

    pos_att.set_identity_mapping();

    mesh.add_attribute(pos_att);

    // Add one triangle face
    mesh.add_face([PointIndex(0), PointIndex(1), PointIndex(2)]);

    // Create encoder
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    // Encode with option to store number of faces
    let mut options = EncoderOptions::default();
    options.set_global_int("store_number_of_encoded_faces", 1);
    let mut out_buffer = EncoderBuffer::new();

    let result = encoder.encode(&options, &mut out_buffer);

    assert!(result.is_ok(), "Encoding should succeed");
    assert!(out_buffer.size() > 0, "Output buffer should not be empty");

    // Check that we encoded 1 face
    assert_eq!(encoder.num_encoded_faces(), 1);
}

#[test]
fn test_mesh_encoder_with_corner_table() {
    // Create a simple quad (2 triangles)
    let mut mesh = Mesh::new();

    // Add position attribute (4 vertices)
    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3, // 3 components
        DataType::Int32,
        false,
        4, // 4 vertices
    );

    // Write vertex data
    {
        let buffer = pos_att.buffer_mut();

        // Vertex 0: (0, 0, 0)
        buffer.write(0, &0i32.to_le_bytes());
        buffer.write(4, &0i32.to_le_bytes());
        buffer.write(8, &0i32.to_le_bytes());

        // Vertex 1: (10, 0, 0)
        buffer.write(12, &10i32.to_le_bytes());
        buffer.write(16, &0i32.to_le_bytes());
        buffer.write(20, &0i32.to_le_bytes());

        // Vertex 2: (10, 10, 0)
        buffer.write(24, &10i32.to_le_bytes());
        buffer.write(28, &10i32.to_le_bytes());
        buffer.write(32, &0i32.to_le_bytes());

        // Vertex 3: (0, 10, 0)
        buffer.write(36, &0i32.to_le_bytes());
        buffer.write(40, &10i32.to_le_bytes());
        buffer.write(44, &0i32.to_le_bytes());
    }

    pos_att.set_identity_mapping();

    mesh.add_attribute(pos_att);

    // Add two triangles forming a quad
    mesh.add_face([PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.add_face([PointIndex(0), PointIndex(2), PointIndex(3)]);

    // Create encoder
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    // Encode
    let options = EncoderOptions::default();
    let mut out_buffer = EncoderBuffer::new();

    let result = encoder.encode(&options, &mut out_buffer);

    assert!(result.is_ok(), "Encoding should succeed");

    // Check corner table was created
    assert!(
        encoder.corner_table().is_some(),
        "Corner table should be created"
    );

    let corner_table = encoder.corner_table().unwrap();
    assert_eq!(corner_table.num_faces(), 2);
    assert_eq!(corner_table.num_corners(), 6); // 2 faces * 3 corners
}
