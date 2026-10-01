//! The point order search as a caller sees it: encode a real scene through the
//! public options at every encoding speed, on one thread and on the machine's,
//! and read off what it costs and what it saves.
//!
//! ```text
//! DRACO_SPLAT_PLY=/path/to/point_cloud.ply \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --test order_search_api_probe -- --ignored --nocapture
//! ```

#![cfg(all(feature = "encoder", feature = "decoder"))]

use std::path::PathBuf;
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

/// The web converter's splat budget: positions 16 bits, harmonics 6, the rest 8;
/// the prediction search and a sequential coder.
fn converter_options(cloud: &PointCloud) -> EncoderOptions {
    let names = names(cloud);
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
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
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn what_the_order_search_costs_and_saves() {
    let Some(path) = std::env::var_os("DRACO_SPLAT_PLY").map(PathBuf::from) else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let source = std::fs::read(&path).expect("the scene reads");
    let cloud = draco_io::ply_reader::PlyReader::from_bytes(source)
        .with_generic_attributes(true)
        .read_mesh()
        .expect("the scene parses")
        .into_point_cloud();
    let points = cloud.num_points();
    println!(
        "scene: {} ({points} points, {} attributes)",
        path.display(),
        cloud.num_attributes()
    );

    let at_speed = |speed: i32, configure: &dyn Fn(&mut EncoderOptions)| {
        let mut options = converter_options(&cloud);
        options.set_global_int("encoding_speed", speed);
        configure(&mut options);
        encode(&cloud, &options)
    };
    let reference = decoded_set(&at_speed(5, &|_| {}).0);
    println!(
        "{:>6}  {:<26} {:>12} {:>10} {:>10}",
        "speed", "", "bytes", "vs curve", "encode s"
    );
    let speeds: Vec<i32> = std::env::var("DRACO_SPEEDS")
        .ok()
        .map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_else(|| vec![10, 8, 5, 3, 0]);
    for speed in speeds {
        let (file_bytes, file_seconds) = at_speed(speed, &|_| {});
        let (curve_bytes, curve_seconds) = at_speed(speed, &|o| o.set_spatial_point_order(true));
        assert_eq!(
            decoded_set(&curve_bytes),
            reference,
            "speed {speed}: the curve changed the points"
        );
        let against = |bytes: usize| {
            format!(
                "{:+.2}%",
                (bytes as f64 / curve_bytes.len() as f64 - 1.0) * 100.0
            )
        };
        println!(
            "{speed:>6}  {:<26} {:>12} {:>10} {file_seconds:>10.2}",
            "file order",
            file_bytes.len(),
            against(file_bytes.len())
        );
        println!(
            "{speed:>6}  {:<26} {:>12} {:>10} {curve_seconds:>10.2}",
            "Hilbert curve",
            curve_bytes.len(),
            "-"
        );
        for threads in [1, 0] {
            let (bytes, seconds) = at_speed(speed, &|o| {
                o.set_point_order_search(true);
                o.set_threads(threads);
            });
            assert_eq!(
                decoded_set(&bytes),
                reference,
                "speed {speed}: the search changed the points"
            );
            let label = if threads == 1 {
                "search, 1 thread"
            } else {
                "search, auto threads"
            };
            println!(
                "{speed:>6}  {label:<26} {:>12} {:>10} {seconds:>10.2}",
                bytes.len(),
                against(bytes.len())
            );
        }
    }
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn where_a_splat_encode_and_decode_spend_their_time() {
    let Some(path) = std::env::var_os("DRACO_SPLAT_PLY").map(PathBuf::from) else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let source = std::fs::read(&path).expect("the scene reads");
    let cloud = draco_io::ply_reader::PlyReader::from_bytes(source)
        .with_generic_attributes(true)
        .read_mesh()
        .expect("the scene parses")
        .into_point_cloud();
    println!(
        "scene: {} ({} points, {} attributes)",
        path.display(),
        cloud.num_points(),
        cloud.num_attributes()
    );
    for (label, search) in [
        ("prediction search off", false),
        ("prediction search on", true),
    ] {
        let mut options = converter_options(&cloud);
        options.set_prediction_search(search);
        let mut best_encode = f64::MAX;
        let mut bytes = Vec::new();
        for _ in 0..3 {
            let (b, seconds) = encode(&cloud, &options);
            best_encode = best_encode.min(seconds);
            bytes = b;
        }
        let mut best_decode = f64::MAX;
        for _ in 0..3 {
            let started = Instant::now();
            let mut decoded = PointCloud::new();
            PointCloudDecoder::new()
                .decode(&mut DecoderBuffer::new(&bytes), &mut decoded)
                .expect("decodes");
            best_decode = best_decode.min(started.elapsed().as_secs_f64());
        }
        println!(
            "{label:<24} encode {best_encode:.3} s, decode {best_decode:.3} s, {} bytes, {:.1} ns an attribute value to encode, {:.1} to decode",
            bytes.len(),
            best_encode * 1e9 / (cloud.num_points() * cloud.num_attributes() as usize) as f64,
            best_decode * 1e9 / (cloud.num_points() * cloud.num_attributes() as usize) as f64,
        );
    }
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn how_a_splat_encode_scales_with_threads() {
    let Some(path) = std::env::var_os("DRACO_SPLAT_PLY").map(PathBuf::from) else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let source = std::fs::read(&path).expect("the scene reads");
    let cloud = draco_io::ply_reader::PlyReader::from_bytes(source)
        .with_generic_attributes(true)
        .read_mesh()
        .expect("the scene parses")
        .into_point_cloud();
    println!(
        "scene: {} ({} points, {} attributes)",
        path.display(),
        cloud.num_points(),
        cloud.num_attributes()
    );
    let mut reference: Option<Vec<u8>> = None;
    println!("{:>8} {:>10} {:>9}", "threads", "encode s", "speedup");
    let mut base = 0.0;
    for threads in [1, 2, 4, 8, 16] {
        let mut options = converter_options(&cloud);
        options.set_threads(threads);
        let mut best = f64::MAX;
        let mut bytes = Vec::new();
        for _ in 0..5 {
            let (b, seconds) = encode(&cloud, &options);
            best = best.min(seconds);
            bytes = b;
        }
        match &reference {
            None => reference = Some(bytes),
            Some(r) => assert_eq!(r, &bytes, "{threads} threads changed the stream"),
        }
        if threads == 1 {
            base = best;
        }
        println!("{threads:>8} {best:>10.3} {:>8.2}x", base / best);
    }
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn how_long_the_search_alone_takes() {
    let Some(path) = std::env::var_os("DRACO_SPLAT_PLY").map(PathBuf::from) else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let source = std::fs::read(&path).expect("the scene reads");
    let cloud = draco_io::ply_reader::PlyReader::from_bytes(source)
        .with_generic_attributes(true)
        .read_mesh()
        .expect("the scene parses")
        .into_point_cloud();
    println!("scene: {} ({} points)", path.display(), cloud.num_points());
    // The order alone, through an encode of a cloud so small that nothing else
    // shows: none of the attribute work, only what `point_order` does. Done by
    // encoding positions only.
    let mut positions_only = PointCloud::new();
    positions_only.set_num_points(cloud.num_points());
    let position_id = (0..cloud.num_attributes())
        .find(|&id| cloud.attribute(id).attribute_type() == GeometryAttributeType::Position)
        .unwrap();
    positions_only.add_attribute(cloud.attribute(position_id).clone());
    for (label, speed, search, spatial) in [
        ("nothing", 5, false, false),
        ("curve", 5, false, true),
        ("search speed 10", 10, true, false),
        ("search speed 5", 5, true, false),
        ("search speed 3", 3, true, false),
    ] {
        for threads in [1, 0] {
            let mut options = EncoderOptions::new();
            options.set_encoding_method(SEQUENTIAL);
            options.set_attribute_int(0, "quantization_bits", 16);
            options.set_global_int("encoding_speed", speed);
            options.set_point_order_search(search);
            options.set_spatial_point_order(spatial);
            options.set_threads(threads);
            let (_, seconds) = encode(&positions_only, &options);
            println!("{label:<18} threads {threads}: {seconds:.3} s (positions only)");
        }
    }
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn how_a_decode_scales_with_threads() {
    let Some(path) = std::env::var_os("DRACO_SPLAT_PLY").map(PathBuf::from) else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let source = std::fs::read(&path).expect("the scene reads");
    let cloud = draco_io::ply_reader::PlyReader::from_bytes(source)
        .with_generic_attributes(true)
        .read_mesh()
        .expect("the scene parses")
        .into_point_cloud();
    let mut options = converter_options(&cloud);
    options.set_threads(0);
    let (bytes, _) = encode(&cloud, &options);
    println!(
        "scene: {} ({} points, {} attributes), stream {} bytes",
        path.display(),
        cloud.num_points(),
        cloud.num_attributes(),
        bytes.len()
    );
    let mut reference: Option<Vec<Vec<u8>>> = None;
    println!("{:>8} {:>10} {:>9}", "threads", "decode s", "speedup");
    let mut base = 0.0;
    let counts: Vec<i32> = std::env::var("DRACO_THREADS")
        .ok()
        .map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_else(|| vec![1, 2, 4, 8, 16]);
    for threads in counts {
        let mut best = f64::MAX;
        let mut out = Vec::new();
        for _ in 0..5 {
            let mut decoder = PointCloudDecoder::new();
            decoder.set_threads(threads);
            let mut decoded = PointCloud::new();
            let started = Instant::now();
            decoder
                .decode(&mut DecoderBuffer::new(&bytes), &mut decoded)
                .expect("decodes");
            best = best.min(started.elapsed().as_secs_f64());
            out = (0..decoded.num_attributes())
                .map(|id| decoded.attribute(id).buffer().data().to_vec())
                .collect();
        }
        match &reference {
            None => reference = Some(out),
            Some(r) => assert_eq!(r, &out, "{threads} threads changed the decoded cloud"),
        }
        if threads == 1 {
            base = best;
        }
        println!("{threads:>8} {best:>10.3} {:>8.2}x", base / best);
    }
}
