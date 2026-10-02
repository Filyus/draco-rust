//! The point order search as a caller sees it: encode real clouds through the
//! public options, in the order they came, along the spatial order and with
//! the search, and read off what each costs and what it saves.
//!
//! ```text
//! DRACO_ORDER_CLOUDS="/path/a.ply;/path/b.ply" DRACO_SPEEDS=8,5,3 \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --test order_search_api_probe -- --ignored --nocapture
//! ```

#![cfg(all(feature = "encoder", feature = "point_cloud_decode"))]

use std::time::Instant;

use draco_core::{
    DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, GeometryAttributeType, PointAttribute,
    PointCloud, PointCloudDecoder, PointCloudEncoder, PointIndex,
};

const SEQUENTIAL: i32 = 0;

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

/// The web converter's budget: positions 16 bits, harmonics 6, every other
/// float 8; the prediction search on and a sequential coder.
fn converter_options(cloud: &PointCloud, speed: i32) -> EncoderOptions {
    let names = names(cloud);
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_global_int("encoding_speed", speed);
    options.set_prediction_search(std::env::var("DRACO_PRED_SEARCH").as_deref() != Ok("0"));
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

fn encode(cloud: &PointCloud, options: &EncoderOptions) -> (Vec<u8>, f64) {
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    let started = Instant::now();
    encoder.encode(options, &mut buffer).expect("encodes");
    (buffer.data().to_vec(), started.elapsed().as_secs_f64())
}

fn read_value(attribute: &PointAttribute, index: usize, component: usize) -> u32 {
    let stride = attribute.byte_stride() as usize;
    let size = attribute.data_type().byte_length();
    let mut bytes = [0u8; 4];
    attribute
        .buffer()
        .read(index * stride + component * size, &mut bytes[..size.min(4)]);
    u32::from_le_bytes(bytes)
}

/// One hash a point over every decoded attribute, sorted: the points as a set,
/// the order taken out.
fn decoded_set(bytes: &[u8]) -> Vec<u64> {
    let mut decoded = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut DecoderBuffer::new(bytes), &mut decoded)
        .expect("what it wrote, it reads");
    let mut hashes = vec![0xcbf2_9ce4_8422_2325u64; decoded.num_points()];
    for id in 0..decoded.num_attributes() {
        let attribute = decoded.attribute(id);
        for (point, hash) in hashes.iter_mut().enumerate() {
            let index = attribute.mapped_index(PointIndex(point as u32)).0 as usize;
            for component in 0..attribute.num_components() as usize {
                *hash = (*hash ^ u64::from(read_value(attribute, index, component)))
                    .wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    hashes.sort_unstable();
    hashes
}

#[test]
#[ignore = "needs clouds in DRACO_ORDER_CLOUDS: run with --release --ignored --nocapture"]
fn what_the_order_search_costs_and_saves() {
    let Some(paths) = std::env::var_os("DRACO_ORDER_CLOUDS") else {
        println!("DRACO_ORDER_CLOUDS is not set; nothing to measure");
        return;
    };
    let speeds: Vec<i32> = std::env::var("DRACO_SPEEDS")
        .ok()
        .map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_else(|| vec![5]);
    println!(
        "{:<28} {:>5} {:<10} {:>12} {:>9} {:>9} {:>9}",
        "cloud", "speed", "order", "bytes", "vs file", "vs curve", "encode s"
    );
    for path in std::env::split_paths(&paths) {
        let source = std::fs::read(&path).expect("the cloud reads");
        let cloud = draco_io::ply_reader::PlyReader::from_bytes(source)
            .with_generic_attributes(true)
            .read_mesh()
            .expect("the cloud parses")
            .into_point_cloud();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let label = format!("{name} ({}k)", cloud.num_points() / 1000);
        for &speed in &speeds {
            let arm = |configure: &dyn Fn(&mut EncoderOptions)| {
                let mut options = converter_options(&cloud, speed);
                configure(&mut options);
                encode(&cloud, &options)
            };
            let (file, file_s) = arm(&|_| {});
            let (curve, curve_s) = arm(&|o| o.set_spatial_point_order(true));
            let (searched, searched_s) = arm(&|o| o.set_point_order_search(true));
            let expected = decoded_set(&file);
            assert_eq!(
                decoded_set(&curve),
                expected,
                "{name}: the curve changed the points"
            );
            assert_eq!(
                decoded_set(&searched),
                expected,
                "{name}: the search changed the points"
            );
            let pct = |bytes: usize, base: usize| {
                format!("{:+.2}%", (bytes as f64 / base as f64 - 1.0) * 100.0)
            };
            for (order, bytes, seconds) in [
                ("file", &file, file_s),
                ("curve", &curve, curve_s),
                ("search", &searched, searched_s),
            ] {
                println!(
                    "{label:<28} {speed:>5} {order:<10} {:>12} {:>9} {:>9} {seconds:>9.2}",
                    bytes.len(),
                    pct(bytes.len(), file.len()),
                    pct(bytes.len(), curve.len()),
                );
            }
        }
    }
}
