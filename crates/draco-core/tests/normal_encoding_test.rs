use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::normal_compression_utils::OctahedronToolBox;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;

/// Test OctahedronToolBox roundtrip with various normals
#[test]
fn test_octahedron_toolbox_roundtrip() {
    let mut toolbox = OctahedronToolBox::new();
    toolbox.set_quantization_bits(10);

    let test_normals: [[f32; 3]; 14] = [
        [1.0, 0.0, 0.0],          // +X
        [-1.0, 0.0, 0.0],         // -X (LEFT)
        [0.0, 1.0, 0.0],          // +Y
        [0.0, -1.0, 0.0],         // -Y
        [0.0, 0.0, 1.0],          // +Z
        [0.0, 0.0, -1.0],         // -Z
        [0.577, 0.577, 0.577],    // +X+Y+Z
        [-0.577, 0.577, 0.577],   // -X+Y+Z (LEFT)
        [0.577, -0.577, 0.577],   // +X-Y+Z
        [0.577, 0.577, -0.577],   // +X+Y-Z
        [-0.577, -0.577, 0.577],  // -X-Y+Z (LEFT)
        [-0.577, 0.577, -0.577],  // -X+Y-Z (LEFT)
        [0.577, -0.577, -0.577],  // +X-Y-Z
        [-0.577, -0.577, -0.577], // -X-Y-Z (LEFT)
    ];

    println!("\n=== OCTAHEDRON TOOLBOX ROUNDTRIP TEST ===");
    for normal in test_normals.iter() {
        // Normalize input
        let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        let normalized = [normal[0] / len, normal[1] / len, normal[2] / len];

        let (s, t) = toolbox.float_vector_to_quantized_octahedral_coords(&normalized);
        let decoded = toolbox.quantized_octahedral_coords_to_unit_vector(s, t);

        let dot =
            normalized[0] * decoded[0] + normalized[1] * decoded[1] + normalized[2] * decoded[2];
        let is_left = normal[0] < 0.0;

        println!("{} Normal ({:7.4}, {:7.4}, {:7.4}) -> s={:4}, t={:4} -> ({:7.4}, {:7.4}, {:7.4}), dot={:.4}",
                 if is_left { "LEFT " } else { "RIGHT" },
                 normalized[0], normalized[1], normalized[2], s, t,
                 decoded[0], decoded[1], decoded[2], dot);

        assert!(
            dot > 0.98,
            "Normal {:?} roundtrip failed, got {:?}, dot={}",
            normalized,
            decoded,
            dot
        );
    }
}

#[test]
fn test_normal_encoding_decoding() {
    let mut pc = PointCloud::new();
    pc.set_num_points(4);

    // Add Normal attribute
    let mut att = PointAttribute::new();
    att.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        4,
    );

    // Set some normal values (unit vectors)
    // (1, 0, 0), (0, 1, 0), (0, 0, 1), (0.577, 0.577, 0.577)
    let normals: Vec<f32> = vec![
        1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.57735, 0.57735, 0.57735,
    ];

    // Write data to buffer
    // PointAttribute buffer expects bytes.
    let mut byte_data = Vec::with_capacity(normals.len() * 4);
    for val in &normals {
        byte_data.extend_from_slice(&val.to_le_bytes());
    }
    att.buffer_mut().write(0, &byte_data);

    let att_id = pc.add_attribute(att);

    let mut options = EncoderOptions::default();
    options.set_attribute_int(att_id, "quantization_bits", 10); // 10 bits for better precision

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);

    let mut out_buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut out_buffer);
    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());

    let mut decoder = PointCloudDecoder::new();
    let mut in_buffer = DecoderBuffer::new(out_buffer.data());
    let mut out_pc = PointCloud::new();
    let status = decoder.decode(&mut in_buffer, &mut out_pc);
    assert!(status.is_ok(), "Decoding failed: {:?}", status.err());

    assert_eq!(out_pc.num_points(), 4);
    assert_eq!(out_pc.num_attributes(), 1);

    let out_att = out_pc.attribute(0);
    assert_eq!(out_att.attribute_type(), GeometryAttributeType::Normal);

    // Check values
    let buffer = out_att.buffer();
    let data = buffer.data();

    for i in 0..4 {
        let offset = i * 3 * 4;
        let x = f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        let y = f32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
        let z = f32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap());

        let expected_x = normals[i * 3];
        let expected_y = normals[i * 3 + 1];
        let expected_z = normals[i * 3 + 2];

        // Error tolerance for 10 bits quantization
        let tolerance = 0.01;

        assert!(
            (x - expected_x).abs() < tolerance,
            "Point {}: x mismatch: got {}, expected {}",
            i,
            x,
            expected_x
        );
        assert!(
            (y - expected_y).abs() < tolerance,
            "Point {}: y mismatch: got {}, expected {}",
            i,
            y,
            expected_y
        );
        assert!(
            (z - expected_z).abs() < tolerance,
            "Point {}: z mismatch: got {}, expected {}",
            i,
            z,
            expected_z
        );
    }
}
