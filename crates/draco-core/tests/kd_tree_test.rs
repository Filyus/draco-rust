use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;

fn create_point_cloud_with_color(num_points: usize) -> PointCloud {
    let mut pc = PointCloud::new();

    // Position (Float32)
    let mut pos_att = PointAttribute::new();
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

    // Color (UInt8)
    let mut color_att = PointAttribute::new();
    color_att.init(
        GeometryAttributeType::Color,
        3,
        DataType::Uint8,
        true,
        num_points,
    );
    let buffer = color_att.buffer_mut();
    for i in 0..num_points {
        let r = (i % 256) as u8;
        let g = ((i * 2) % 256) as u8;
        let b = ((i * 3) % 256) as u8;
        buffer.write(i * 3, &[r]);
        buffer.write(i * 3 + 1, &[g]);
        buffer.write(i * 3 + 2, &[b]);
    }
    pc.add_attribute(color_att);

    pc
}

#[test]
fn test_kd_tree_multi_attribute() {
    let num_points = 100;
    let pc = create_point_cloud_with_color(num_points);

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);

    let mut options = EncoderOptions::new();
    options.set_encoding_method(1); // KD-Tree
    options.set_attribute_int(0, "quantization_bits", 10);
    options.set_global_int("encoding_speed", 5);

    let mut enc_buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut enc_buffer);
    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_pc = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    let status = decoder.decode(&mut dec_buffer, &mut decoded_pc);
    assert!(status.is_ok(), "Decoding failed: {:?}", status.err());

    assert_eq!(decoded_pc.num_points(), num_points);
    assert_eq!(decoded_pc.num_attributes(), 2);

    let mut decoded_points = Vec::new();
    let pos_att = decoded_pc.attribute(0);
    let color_att = decoded_pc.attribute(1);

    for i in 0..num_points {
        // Read Pos
        let mut bytes = [0u8; 4];
        pos_att.buffer().read(i * 12, &mut bytes);
        let x = f32::from_le_bytes(bytes);
        pos_att.buffer().read(i * 12 + 4, &mut bytes);
        let y = f32::from_le_bytes(bytes);
        pos_att.buffer().read(i * 12 + 8, &mut bytes);
        let z = f32::from_le_bytes(bytes);

        // Read Color
        let mut c_bytes = [0u8; 1];
        color_att.buffer().read(i * 3, &mut c_bytes);
        let r = c_bytes[0];
        color_att.buffer().read(i * 3 + 1, &mut c_bytes);
        let g = c_bytes[0];
        color_att.buffer().read(i * 3 + 2, &mut c_bytes);
        let b = c_bytes[0];

        decoded_points.push(((x, y, z), (r, g, b)));
    }

    // Sort by X, Y, Z
    decoded_points.sort_by(|a, b| {
        a.0 .0
            .partial_cmp(&b.0 .0)
            .unwrap()
            .then(a.0 .1.partial_cmp(&b.0 .1).unwrap())
            .then(a.0 .2.partial_cmp(&b.0 .2).unwrap())
    });

    for (i, &((x, y, z), (r, g, b))) in decoded_points.iter().enumerate().take(num_points) {
        let expected_x = i as f32;
        let expected_y = (i * 2) as f32;
        let expected_z = (i * 3) as f32;

        let expected_r = (i % 256) as u8;
        let expected_g = ((i * 2) % 256) as u8;
        let expected_b = ((i * 3) % 256) as u8;

        // Position check
        assert!((x - expected_x).abs() < 0.5);
        assert!((y - expected_y).abs() < 0.5);
        assert!((z - expected_z).abs() < 0.5);

        // Color check - Should be EXACT
        assert_eq!(r, expected_r, "Color R mismatch at index {}", i);
        assert_eq!(g, expected_g, "Color G mismatch at index {}", i);
        assert_eq!(b, expected_b, "Color B mismatch at index {}", i);
    }
}

#[test]
fn test_kd_tree_signed_integers() {
    let num_points = 50;
    let mut pc = PointCloud::new();

    // Int16 Attribute
    let mut att = PointAttribute::new();
    att.init(
        GeometryAttributeType::Generic,
        2,
        DataType::Int16,
        false,
        num_points,
    );
    let buffer = att.buffer_mut();
    for i in 0..num_points {
        let val1 = (i as i16) - 25; // Range -25 to 24
        let val2 = -(i as i16); // Range 0 to -49
        buffer.write(i * 4, &val1.to_le_bytes());
        buffer.write(i * 4 + 2, &val2.to_le_bytes());
    }
    pc.add_attribute(att);

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);

    let mut options = EncoderOptions::new();
    options.set_encoding_method(1); // KD-Tree
    options.set_global_int("encoding_speed", 5);

    let mut enc_buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut enc_buffer);
    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_pc = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    let status = decoder.decode(&mut dec_buffer, &mut decoded_pc);
    assert!(status.is_ok(), "Decoding failed: {:?}", status.err());

    let att = decoded_pc.attribute(0);
    let mut decoded_values = Vec::new();
    for i in 0..num_points {
        let mut bytes = [0u8; 2];
        att.buffer().read(i * 4, &mut bytes);
        let v1 = i16::from_le_bytes(bytes);
        att.buffer().read(i * 4 + 2, &mut bytes);
        let v2 = i16::from_le_bytes(bytes);
        decoded_values.push((v1, v2));
    }

    // Sort by v1 then v2
    decoded_values.sort();

    for (i, &(v1, v2)) in decoded_values.iter().enumerate().take(num_points) {
        // Reconstruct expected values based on sorted order.
        // Since v1 = i - 25, and i goes 0..49, v1 goes -25..24.
        // v1 is unique and strictly increasing with i.
        // So after sorting, the i-th element should correspond to original i.

        let expected_v1 = (i as i16) - 25;
        let expected_v2 = -(i as i16);

        assert_eq!(v1, expected_v1);
        assert_eq!(v2, expected_v2);
    }
}
