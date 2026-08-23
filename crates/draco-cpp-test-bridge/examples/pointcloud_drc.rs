//! Encodes or decodes a synthetic point cloud N times and nothing else.
//!
//! The point-cloud counterpart of `encode_drc`/`decode_drc`, and it exists for
//! the same reason: under callgrind the difference between `iters=0` and
//! `iters=1` is exactly one operation, because the tool counts instructions
//! and everything outside the loop cancels.
//!
//! There is no `.drc` corpus for point clouds here, so the payload is
//! generated rather than read -- deterministically, from the point count
//! alone, so two runs of the same arguments are the same work. Generation
//! happens before the loop and is therefore cancelled by the subtraction.
//!
//! The KD-tree path is the reason this driver exists: it is a whole encoder
//! and decoder that no mesh benchmark reaches, so nothing measured it. Pass
//! `kdtree` or `sequential` to pick which.
//!
//! ```text
//! valgrind --tool=callgrind --callgrind-out-file=pc.out \
//!     ./pointcloud_drc encode kdtree 50000 1
//! ```
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;

/// The same generator `tests/bench_point_cloud.rs` uses, so a figure taken
/// here and a figure taken there are about the same cloud.
fn build(num_points: usize) -> PointCloud {
    let mut pc = PointCloud::new();

    let mut pos = PointAttribute::new();
    pos.init(
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
        pos.buffer_mut().write(offset, &x.to_le_bytes());
        pos.buffer_mut().write(offset + 4, &y.to_le_bytes());
        pos.buffer_mut().write(offset + 8, &z.to_le_bytes());
    }
    pc.add_attribute(pos);

    let mut color = PointAttribute::new();
    color.init(
        GeometryAttributeType::Color,
        3,
        DataType::Uint8,
        true,
        num_points,
    );
    for i in 0..num_points {
        let bytes = [
            (i & 255) as u8,
            ((i * 3) & 255) as u8,
            ((i * 7) & 255) as u8,
        ];
        color.buffer_mut().write(i * 3, &bytes);
    }
    pc.add_attribute(color);

    pc
}

fn options(method: i32) -> EncoderOptions {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(method);
    options.set_global_int("encoding_speed", 5);
    options.set_attribute_int(0, "quantization_bits", 10);
    options
}

fn encode(pc: &PointCloud, options: &EncoderOptions) -> Vec<u8> {
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc.clone());
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(options, &mut buffer)
        .expect("point-cloud encode failed");
    buffer.data().to_vec()
}

fn decode(encoded: &[u8]) -> PointCloud {
    let mut buffer = DecoderBuffer::new(encoded);
    let mut pc = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    decoder
        .decode(&mut buffer, &mut pc)
        .expect("point-cloud decode failed");
    pc
}

fn main() {
    let mut args = std::env::args().skip(1);
    let operation = args.next().unwrap_or_else(|| "encode".to_string());
    let method_name = args.next().unwrap_or_else(|| "kdtree".to_string());
    let num_points: usize = args
        .next()
        .and_then(|s| s.parse().ok())
        .expect("point count");
    let iters: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);

    let method = match method_name.as_str() {
        "sequential" => 0,
        "kdtree" => 1,
        other => panic!("unknown method {other}: expected sequential or kdtree"),
    };

    let pc = build(num_points);
    let options = options(method);
    // Encoded once outside the loop either way: the decode side needs a
    // bitstream to read, and charging the encode to a decode measurement is
    // exactly what the subtraction is there to prevent.
    let encoded = encode(&pc, &options);

    let mut bytes = encoded.len();
    let mut points = 0;
    match operation.as_str() {
        "encode" => {
            for _ in 0..iters {
                bytes = encode(&pc, &options).len();
            }
        }
        "decode" => {
            for _ in 0..iters {
                points = decode(&encoded).num_points();
            }
        }
        other => panic!("unknown operation {other}: expected encode or decode"),
    }

    eprintln!(
        "rust {operation} {method_name}: {num_points} points -> {bytes} bytes \
         (decoded {points}) x{iters}"
    );
}
