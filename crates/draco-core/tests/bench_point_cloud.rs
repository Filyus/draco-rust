use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;
use std::time::Instant;

fn create_point_cloud(num_points: usize) -> PointCloud {
    let mut pc = PointCloud::new();

    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    for i in 0..num_points {
        let x = ((i * 17) % 997) as f32 * 0.125;
        let y = ((i * 31) % 991) as f32 * 0.25;
        let z = ((i * 47) % 983) as f32 * 0.5;
        let offset = i * 12;
        pos_att.buffer_mut().write(offset, &x.to_le_bytes());
        pos_att.buffer_mut().write(offset + 4, &y.to_le_bytes());
        pos_att.buffer_mut().write(offset + 8, &z.to_le_bytes());
    }
    pc.add_attribute(pos_att);

    let mut color_att = PointAttribute::new();
    color_att.init(
        GeometryAttributeType::Color,
        3,
        DataType::Uint8,
        true,
        num_points,
    );
    for i in 0..num_points {
        let color = [
            (i & 255) as u8,
            ((i * 3) & 255) as u8,
            ((i * 7) & 255) as u8,
        ];
        color_att.buffer_mut().write(i * 3, &color);
    }
    pc.add_attribute(color_att);

    pc
}

fn encode_point_cloud(pc: &PointCloud, method: i32) -> Vec<u8> {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(method);
    options.set_global_int("encoding_speed", 5);
    options.set_attribute_int(0, "quantization_bits", 10);

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc.clone());
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("point-cloud encode failed");
    buffer.data().to_vec()
}

fn decode_point_cloud(encoded: &[u8]) -> PointCloud {
    let mut buffer = DecoderBuffer::new(encoded);
    let mut pc = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    decoder
        .decode(&mut buffer, &mut pc)
        .expect("point-cloud decode failed");
    pc
}

#[test]
fn point_cloud_encode_decode_performance_smoke() {
    println!("\nPoint-cloud encode/decode performance smoke");
    println!(
        "{:>10} {:>8} {:>7} {:>9} {:>10} {:>10}",
        "Method", "Points", "Iters", "Bytes", "Encode us", "Decode us"
    );
    println!("{}", "-".repeat(68));

    for (num_points, iterations) in [(1_000usize, 30u32), (10_000, 10), (50_000, 3)] {
        let pc = create_point_cloud(num_points);
        for (method_name, method) in [("sequential", 0), ("kd-tree", 1)] {
            let encoded = encode_point_cloud(&pc, method);
            let decoded = decode_point_cloud(&encoded);
            assert_eq!(decoded.num_points(), num_points);
            assert_eq!(decoded.num_attributes(), 2);

            let start = Instant::now();
            for _ in 0..iterations {
                std::hint::black_box(encode_point_cloud(&pc, method));
            }
            let encode_us = start.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(iterations);

            let start = Instant::now();
            for _ in 0..iterations {
                std::hint::black_box(decode_point_cloud(&encoded));
            }
            let decode_us = start.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(iterations);

            println!(
                "{:>10} {:>8} {:>7} {:>9} {:>10.1} {:>10.1}",
                method_name,
                num_points,
                iterations,
                encoded.len(),
                encode_us,
                decode_us
            );
        }
        println!("{}", "-".repeat(68));
    }
}
