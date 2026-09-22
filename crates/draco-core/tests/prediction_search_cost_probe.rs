//! What `set_prediction_search` costs in encode time, and what it produces.
//!
//! The option's documentation says the search costs encode time. This is the
//! measurement behind that sentence: the same scene encoded with the option
//! off and on, clocked, with the sizes beside the times so that a run where
//! the search found nothing is told apart from one where it did.
//!
//! When `DRACO_PROBE_OUT` names a directory the encoded bytes are written
//! there too, one file per arm. That is how a change to how the search decides
//! is checked against how it decided before: build both, run both, `cmp`.
//!
//! Needs a scene, a splat PLY or any point cloud the PLY reader carries whole:
//!
//! ```text
//! DRACO_SPLAT_PLY=<point_cloud.ply> DRACO_PROBE_OUT=<output directory> \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --features encoder --test prediction_search_cost_probe -- --ignored --nocapture
//! ```

#![cfg(feature = "encoder")]

use std::path::PathBuf;
use std::time::Instant;

use draco_core::{
    EncoderBuffer, EncoderOptions, GeometryAttributeType, PointCloud, PointCloudEncoder,
};

const SEQUENTIAL: i32 = 0;

/// Every attribute at a byte, positions at 16 bits: a budget that keeps every
/// attribute in the sequential coder and needs no knowledge of the scene.
fn budget(cloud: &PointCloud, options: &mut EncoderOptions) {
    for id in 0..cloud.num_attributes() {
        let bits = match cloud.attribute(id).attribute_type() {
            GeometryAttributeType::Position => 16,
            _ => 8,
        };
        options.set_attribute_int(id, "quantization_bits", bits);
    }
}

fn encode(cloud: &PointCloud, search: bool, spatial: bool) -> Vec<u8> {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_prediction_search(search);
    options.set_spatial_point_order(spatial);
    budget(cloud, &mut options);
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encodes");
    buffer.data().to_vec()
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn what_the_search_costs() {
    let Some(path) = std::env::var_os("DRACO_SPLAT_PLY").map(PathBuf::from) else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let out_dir = std::env::var_os("DRACO_PROBE_OUT").map(PathBuf::from);

    let source = std::fs::read(&path).expect("the scene reads");
    let mesh = draco_io::ply_reader::PlyReader::from_bytes(source)
        .with_generic_attributes(true)
        .read_mesh()
        .expect("the scene parses");
    let cloud = mesh.into_point_cloud();
    let num_points = cloud.num_points();
    println!(
        "scene: {} ({num_points} points, {} attributes)",
        path.display(),
        cloud.num_attributes()
    );
    println!();

    // Warm once so the first arm does not also pay for page faults.
    let _ = encode(&cloud, false, false);

    const REPEATS: u32 = 3;
    println!(
        "{:<24} {:>10} {:>10} {:>9}",
        "arm", "B/point", "ms (min)", "vs off"
    );
    let mut off_ms = 0.0f64;
    for (label, file, search, spatial) in [
        ("off", "search_off", false, false),
        ("search", "search_on", true, false),
        ("spatial", "spatial", false, true),
        ("search + spatial", "search_spatial", true, true),
    ] {
        let mut best_ms = f64::MAX;
        let mut bytes = Vec::new();
        for _ in 0..REPEATS {
            let started = Instant::now();
            bytes = encode(&cloud, search, spatial);
            best_ms = best_ms.min(started.elapsed().as_secs_f64() * 1e3);
        }
        if !search && !spatial {
            off_ms = best_ms;
        }
        println!(
            "{label:<24} {:>10.3} {best_ms:>10.0} {:>+8.1}%",
            bytes.len() as f64 / num_points as f64,
            (best_ms / off_ms - 1.0) * 100.0
        );
        if let Some(dir) = &out_dir {
            std::fs::create_dir_all(dir).expect("the output directory");
            std::fs::write(dir.join(format!("{file}.drc")), &bytes).expect("the arm is written");
        }
    }
}
