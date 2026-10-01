//! Times one encode or decode of a PLY point cloud and prints the best of N
//! with the stream's size and a hash of what was written or read back, so a
//! variant that changes the output shows up next to its time.
//!
//! usage: pcprof <enc|dec> <cloud.ply> [iterations] [threads] [search|curve|plain] [speed]
use draco_core::{
    DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, GeometryAttributeType, PointCloud,
    PointCloudDecoder, PointCloudEncoder,
};
use std::time::Instant;

fn names(cloud: &PointCloud) -> Vec<Option<String>> {
    (0..cloud.num_attributes())
        .map(|id| {
            let unique_id = cloud.attribute(id).unique_id();
            cloud
                .attribute_metadata_by_unique_id(unique_id)?
                .metadata()
                .get_string("name")
                .map(str::to_string)
        })
        .collect()
}

fn options(cloud: &PointCloud, order: &str, speed: i32, threads: i32) -> EncoderOptions {
    let names = names(cloud);
    let mut options = EncoderOptions::new();
    options.set_encoding_method(0);
    options.set_prediction_search(true);
    options.set_global_int("encoding_speed", speed);
    options.set_threads(threads);
    match order {
        "search" => options.set_point_order_search(true),
        "curve" => options.set_spatial_point_order(true),
        _ => {}
    }
    for id in 0..cloud.num_attributes() {
        if cloud.attribute(id).data_type() != DataType::Float32 {
            continue;
        }
        let bits = match cloud.attribute(id).attribute_type() {
            GeometryAttributeType::Position => 16,
            _ => match names[id as usize].as_deref() {
                Some(name) if name.starts_with("f_rest_") => 6,
                _ => 8,
            },
        };
        options.set_attribute_int(id, "quantization_bits", bits);
    }
    options
}

fn encode(cloud: &PointCloud, options: &EncoderOptions) -> Vec<u8> {
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(options, &mut buffer).expect("encodes");
    buffer.data().to_vec()
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (mode, path) = (args[1].as_str(), &args[2]);
    let iterations: usize = args.get(3).map_or(5, |v| v.parse().unwrap());
    let threads: i32 = args.get(4).map_or(1, |v| v.parse().unwrap());
    let order = args.get(5).map_or("plain", String::as_str);
    let speed: i32 = args.get(6).map_or(5, |v| v.parse().unwrap());
    let cloud = draco_io::ply_reader::PlyReader::from_bytes(std::fs::read(path).expect("reads"))
        .with_generic_attributes(true)
        .read_mesh()
        .expect("parses")
        .into_point_cloud();
    let options = options(&cloud, order, speed, threads);
    let mut best = f64::MAX;
    match mode {
        "enc" => {
            let mut bytes = Vec::new();
            for _ in 0..iterations {
                let started = Instant::now();
                bytes = encode(&cloud, &options);
                best = best.min(started.elapsed().as_secs_f64());
            }
            eprintln!(
                "encode best {best:.5} s, {} bytes, hash {:016x}",
                bytes.len(),
                fnv(&bytes)
            );
        }
        "dec" => {
            let bytes = encode(&cloud, &options);
            let mut hash = 0u64;
            for _ in 0..iterations {
                let mut decoder = PointCloudDecoder::new();
                decoder.set_threads(threads);
                let mut decoded = PointCloud::new();
                let started = Instant::now();
                decoder
                    .decode(&mut DecoderBuffer::new(&bytes), &mut decoded)
                    .expect("decodes");
                best = best.min(started.elapsed().as_secs_f64());
                hash = (0..decoded.num_attributes()).fold(0, |h, id| {
                    h ^ fnv(decoded.attribute(id).buffer().data()).rotate_left(id as u32)
                });
            }
            eprintln!("decode best {best:.5} s, {} bytes, hash {hash:016x}", bytes.len());
        }
        _ => panic!("mode {mode}"),
    }
}
