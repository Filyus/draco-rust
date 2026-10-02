//! PROBE: decode times of streams written once, for an A/B between builds.
//! `emit` writes the streams; `decode` times them, best of seven.

#![cfg(all(feature = "encoder", feature = "point_cloud_decode"))]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use draco_core::{
    DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, GeometryAttributeType, PointCloud,
    PointCloudDecoder, PointCloudEncoder,
};

fn options(cloud: &PointCloud) -> EncoderOptions {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(0);
    options.set_prediction_search(true);
    for id in 0..cloud.num_attributes() {
        if cloud.attribute(id).data_type() != DataType::Float32 {
            continue;
        }
        let unique_id = cloud.attribute(id).unique_id();
        let name = cloud
            .attribute_metadata_by_unique_id(unique_id)
            .and_then(|m| m.metadata().get_string("name").map(str::to_string));
        let bits = match cloud.attribute(id).attribute_type() {
            GeometryAttributeType::Position => 16,
            _ => match name.as_deref() {
                Some(name) if name.starts_with("f_rest_") => 6,
                _ => 8,
            },
        };
        options.set_attribute_int(id, "quantization_bits", bits);
    }
    options
}

#[test]
#[ignore = "PROBE"]
fn emit() {
    let (Some(out), Some(clouds)) = (
        std::env::var_os("DRACO_STREAM_DIR").map(PathBuf::from),
        std::env::var_os("DRACO_ORDER_CLOUDS"),
    ) else {
        eprintln!("set DRACO_STREAM_DIR and DRACO_ORDER_CLOUDS to emit streams");
        return;
    };
    for path in std::env::split_paths(&clouds) {
        let source = std::fs::read(&path).expect("reads");
        let cloud = draco_io::ply_reader::PlyReader::from_bytes(source)
            .with_generic_attributes(true)
            .read_mesh()
            .expect("parses")
            .into_point_cloud();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        for (arm, search) in [("spatial", false), ("search", true)] {
            let mut o = options(&cloud);
            o.set_spatial_point_order(!search);
            o.set_point_order_search(search);
            let mut encoder = PointCloudEncoder::new();
            encoder.set_point_cloud(cloud.clone());
            let mut buffer = EncoderBuffer::new();
            encoder.encode(&o, &mut buffer).expect("encodes");
            std::fs::write(out.join(format!("{name}_{arm}.drc")), buffer.data()).unwrap();
        }
    }
}

#[test]
#[ignore = "PROBE"]
fn decode() {
    let Some(dir) = std::env::var_os("DRACO_STREAM_DIR").map(PathBuf::from) else {
        eprintln!("set DRACO_STREAM_DIR to the streams to decode");
        return;
    };
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "drc"))
        .collect();
    paths.sort();
    for path in paths {
        let bytes = std::fs::read(&path).unwrap();
        let mut best = f64::MAX;
        let mut hash = 0u64;
        for _ in 0..7 {
            std::thread::sleep(Duration::from_millis(300));
            let started = Instant::now();
            let mut decoded = PointCloud::new();
            // On one thread: the arms differ in the work one decode does, which
            // threads would spread over however many cores the machine has.
            let mut decoder = PointCloudDecoder::new();
            decoder.set_threads(1);
            decoder
                .decode(&mut DecoderBuffer::new(&bytes), &mut decoded)
                .expect("decodes");
            best = best.min(started.elapsed().as_secs_f64());
            hash = (0..decoded.num_attributes()).fold(0xcbf2_9ce4_8422_2325u64, |h, id| {
                decoded
                    .attribute(id)
                    .buffer()
                    .data()
                    .iter()
                    .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
            });
        }
        println!(
            "{:<28} {:.4} s  {hash:016x}",
            path.file_stem().unwrap().to_string_lossy(),
            best
        );
    }
}
