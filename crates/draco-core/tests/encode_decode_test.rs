use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;

#[test]
fn test_point_cloud_encode_decode() {
    let mut pc = PointCloud::new();
    let mut pos_att = PointAttribute::new();

    let num_points = 3;
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );

    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

    let buffer = pos_att.buffer_mut();
    for (i, &position) in positions.iter().enumerate() {
        let bytes = position.to_le_bytes();
        buffer.write(i * 4, &bytes);
    }

    pc.add_attribute(pos_att);

    // Encode
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);

    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut enc_buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut enc_buffer);
    assert!(status.is_ok());

    // Decode
    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_pc = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    let status = decoder.decode(&mut dec_buffer, &mut decoded_pc);
    assert!(status.is_ok(), "Decoding failed: {:?}", status.err());

    assert_eq!(decoded_pc.num_points(), 3);
    assert_eq!(decoded_pc.num_attributes(), 1);

    let decoded_att = decoded_pc.attribute(0);
    assert_eq!(
        decoded_att.attribute_type(),
        GeometryAttributeType::Position
    );
    assert_eq!(decoded_att.num_components(), 3);

    // Check values (approximate due to quantization)
    let decoded_buffer = decoded_att.buffer();
    for (i, &expected) in positions.iter().enumerate() {
        let mut bytes = [0u8; 4];
        decoded_buffer.read(i * 4, &mut bytes);
        let val = f32::from_le_bytes(bytes);

        let diff = (val - expected).abs();
        eprintln!(
            "i {}: decoded={} expected={} diff={}",
            i, val, expected, diff
        );
        assert!(
            diff < 0.001,
            "Value mismatch at {}: {} vs {}",
            i,
            val,
            positions[i]
        );
    }
}

#[test]
fn test_point_cloud_encode_decode_kd_tree() {
    let mut pc = PointCloud::new();
    let mut pos_att = PointAttribute::new();

    let num_points = 100;
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );

    let buffer = pos_att.buffer_mut();
    for i in 0..num_points {
        let x = i as f32;
        let y = (i * 2) as f32;
        let z = (i * 3) as f32;
        buffer.write(i * 12, &x.to_le_bytes());
        buffer.write(i * 12 + 4, &y.to_le_bytes());
        buffer.write(i * 12 + 8, &z.to_le_bytes());
    }

    pc.add_attribute(pos_att);

    // Encode
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);

    let mut options = EncoderOptions::new();
    options.set_encoding_method(1); // KD-Tree
    options.set_attribute_int(0, "quantization_bits", 10);
    options.set_global_int("encoding_speed", 5);

    let mut enc_buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut enc_buffer);
    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());

    // Decode
    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_pc = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    let status = decoder.decode(&mut dec_buffer, &mut decoded_pc);
    assert!(status.is_ok(), "Decoding failed: {:?}", status.err());

    assert_eq!(decoded_pc.num_points(), num_points);
    assert_eq!(decoded_pc.num_attributes(), 1);

    // Verify data (approximate due to quantization)
    // Note: KD-tree encoding does not preserve point order, so we must sort before comparing.
    let mut decoded_points = Vec::new();
    let att = decoded_pc.attribute(0);
    let buffer = att.buffer();
    for i in 0..num_points {
        let mut bytes = [0u8; 4];
        buffer.read(i * 12, &mut bytes);
        let x = f32::from_le_bytes(bytes);
        buffer.read(i * 12 + 4, &mut bytes);
        let y = f32::from_le_bytes(bytes);
        buffer.read(i * 12 + 8, &mut bytes);
        let z = f32::from_le_bytes(bytes);
        decoded_points.push((x, y, z));
    }

    // Sort by X, then Y, then Z
    decoded_points.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap()
            .then(a.1.partial_cmp(&b.1).unwrap())
            .then(a.2.partial_cmp(&b.2).unwrap())
    });

    for (i, &(x, y, z)) in decoded_points.iter().enumerate().take(num_points) {
        let expected_x = i as f32;
        let expected_y = (i * 2) as f32;
        let expected_z = (i * 3) as f32;

        if (x - expected_x).abs() >= 0.5
            || (y - expected_y).abs() >= 0.5
            || (z - expected_z).abs() >= 0.5
        {
            eprintln!(
                "Mismatch at {}: got ({}, {}, {}), expected ({}, {}, {})",
                i, x, y, z, expected_x, expected_y, expected_z
            );
        }

        // Quantization error check (10 bits is decent precision)
        assert!((x - expected_x).abs() < 0.5);
        assert!((y - expected_y).abs() < 0.5);
        assert!((z - expected_z).abs() < 0.5);
    }
}

#[test]
fn test_point_cloud_forced_mesh_prediction_scheme_falls_back_like_cpp() {
    let mut pc = PointCloud::new();
    let mut pos_att = PointAttribute::new();

    let num_points = 3;
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );

    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

    let buffer = pos_att.buffer_mut();
    for (i, &position) in positions.iter().enumerate() {
        buffer.write(i * 4, &position.to_le_bytes());
    }

    pc.add_attribute(pos_att);

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);

    let mut options = EncoderOptions::new();
    options.set_prediction_scheme(1); // MeshPredictionParallelogram, invalid for point clouds.
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut enc_buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut enc_buffer);
    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_pc = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    let status = decoder.decode(&mut dec_buffer, &mut decoded_pc);
    assert!(status.is_ok(), "Decoding failed: {:?}", status.err());
    assert_eq!(decoded_pc.num_points(), num_points);
    assert_eq!(decoded_pc.num_attributes(), 1);
}
