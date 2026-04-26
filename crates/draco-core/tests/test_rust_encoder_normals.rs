#[test]
fn test_rust_encoder_normal_bytes() {
    use draco_core::draco_types::DataType;
    use draco_core::encoder_buffer::EncoderBuffer;
    use draco_core::encoder_options::EncoderOptions;
    use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
    use draco_core::geometry_indices::PointIndex;
    use draco_core::mesh::Mesh;
    use draco_core::mesh_encoder::MeshEncoder;

    let mut mesh = Mesh::new();

    // Positions
    let positions: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0];

    let mut pos_attr = PointAttribute::new();
    pos_attr.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        3,
    );
    for (i, chunk) in positions.chunks(3).enumerate() {
        let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
        pos_attr.buffer_mut().write(i * 12, &bytes);
    }
    mesh.add_attribute(pos_attr);

    // Normals - these should be distinct!
    let normals: Vec<f32> = vec![
        1.0, 0.0, 0.0, // +X
        -1.0, 0.0, 0.0, // -X
        0.0, 1.0, 0.0, // +Y
    ];

    let mut norm_attr = PointAttribute::new();
    norm_attr.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        3,
    );
    for (i, chunk) in normals.chunks(3).enumerate() {
        let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
        norm_attr.buffer_mut().write(i * 12, &bytes);
    }
    mesh.add_attribute(norm_attr);

    // Add face
    mesh.add_face([PointIndex(0), PointIndex(1), PointIndex(2)]);

    // Encode
    let mut encoder = MeshEncoder::new();
    let mut enc_buffer = EncoderBuffer::new();
    let mut options = EncoderOptions::default();
    options.set_attribute_int(0, "quantization_bits", 14);
    options.set_attribute_int(1, "quantization_bits", 10);

    encoder.set_mesh(mesh);
    encoder
        .encode(&options, &mut enc_buffer)
        .expect("Encode failed");

    let data = enc_buffer.data();
    println!("Encoded {} bytes", data.len());
    println!("Hex:");
    for (i, b) in data.iter().enumerate() {
        print!("{:02X} ", b);
        if (i + 1) % 16 == 0 {
            println!();
        }
    }
    println!();

    // Write to temp file (optional, doesn't fail if temp dir doesn't exist)
    use std::env;
    use std::fs;
    let temp_dir = env::temp_dir();
    let path = temp_dir.join("rust_test_normals.drc");
    if let Err(e) = fs::write(&path, data) {
        eprintln!("Note: Could not write temp file: {:?}", e);
    } else {
        println!("Saved to {:?}", path);
    }
}
