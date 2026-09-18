//! Writes the same point cloud twice, predicted and unpredicted, for upstream
//! C++ Draco to decode.
//!
//! `PREDICTION_NONE` is the cheapest of the two splat improvements measured in
//! `splat_real_scene_probe`, and the whole case for it is that it needs nothing
//! new in the bitstream. That claim is about another implementation, so it is
//! not ours to settle by reading our own encoder. Upstream 1.5.7 handles it on
//! both sides — `sequential_integer_attribute_encoder.cc` writes the method
//! byte and writes a transform byte **only when a scheme exists**, and
//! `sequential_integer_attribute_decoder.cc` reads it back the same way — but
//! what a decoder does with our bytes is a fact about our bytes.
//!
//! So this emits both files and they are decoded by the real
//! `draco_decoder.exe`. Two files rather than one, because "it decoded" is a
//! weaker claim than it looks: the two decode to the same quantized values, so
//! the outputs must match each other exactly. A file that decodes to something
//! else would pass the first check and fail this one.
//!
//! ```text
//! cargo test --manifest-path crates/Cargo.toml -p draco-core \
//!     --test emit_prediction_none_drc -- --ignored --nocapture
//!
//! draco_decoder.exe -i prediction_default.drc -o predicted.ply
//! draco_decoder.exe -i prediction_none.drc    -o unpredicted.ply
//! cmp predicted.ply unpredicted.ply
//! ```

#![cfg(all(feature = "encoder", feature = "decoder"))]

use std::io::Write as _;

use draco_core::{
    DataType, EncoderBuffer, EncoderOptions, GeometryAttributeType, Metadata, PointAttribute,
    PointCloud, PointCloudEncoder,
};

const NUM_POINTS: usize = 4096;

/// Draco's `PREDICTION_NONE`. Upstream reaches the same value through its own
/// options: `GetPredictionMethodFromOptions` maps any negative
/// `prediction_scheme` other than -1 onto it.
const PREDICTION_NONE: i32 = -2;

/// Values with enough structure that prediction changes the output, so the two
/// files differ and the comparison is not vacuous.
fn attribute(kind: GeometryAttributeType, components: u8, seed: u32) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(kind, components, DataType::Float32, false, NUM_POINTS);
    let buffer = attribute.buffer_mut();
    let mut state = seed.wrapping_mul(2654435761).wrapping_add(1);
    for point in 0..NUM_POINTS {
        for component in 0..components as usize {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (state >> 8) as f32 / (1 << 24) as f32 - 0.5;
            let value = point as f32 * 0.01 + component as f32 * 0.25 + noise;
            let offset = (point * components as usize + component) * 4;
            buffer.write(offset, &value.to_le_bytes());
        }
    }
    attribute
}

fn splat_cloud() -> PointCloud {
    let mut cloud = PointCloud::new();
    cloud.set_num_points(NUM_POINTS);
    cloud.add_attribute(attribute(GeometryAttributeType::Position, 3, 1));

    let mut layout: Vec<(String, u8)> = vec![
        ("scale".to_string(), 3),
        ("rotation".to_string(), 4),
        ("opacity".to_string(), 1),
    ];
    for i in 0..9 {
        layout.push((format!("f_rest_{i}"), 1));
    }
    for (index, (name, components)) in layout.into_iter().enumerate() {
        let id = cloud.add_attribute(attribute(
            GeometryAttributeType::Generic,
            components,
            10 + index as u32,
        ));
        let unique_id = cloud.attribute(id).unique_id();
        let mut metadata = Metadata::new();
        metadata.set_string("name", name).expect("string entry");
        cloud
            .metadata_or_insert()
            .set_attribute_metadata(unique_id, metadata);
    }
    cloud
}

fn write(name: &str, bytes: &[u8]) -> String {
    let path = std::env::temp_dir()
        .join(name)
        .to_string_lossy()
        .into_owned();
    // Written and synced through the one handle: a reopened read-only handle
    // cannot be synced on Windows, and the C++ decoder reads this next.
    let mut file = std::fs::File::create(&path).expect("create");
    file.write_all(bytes).expect("write");
    file.flush().expect("flush");
    file.sync_all().expect("fsync");
    println!("WROTE {path} ({} bytes)", bytes.len());
    path
}

#[test]
#[ignore = "emits files for the C++ decoder; run with --ignored --nocapture"]
fn emit() {
    for (name, prediction) in [
        ("prediction_default.drc", None),
        ("prediction_none.drc", Some(PREDICTION_NONE)),
    ] {
        let cloud = splat_cloud();
        let mut options = EncoderOptions::new();
        options.set_encoding_method(0); // sequential
        for id in 0..cloud.num_attributes() {
            options.set_attribute_int(id, "quantization_bits", 8);
            if let Some(prediction) = prediction {
                options.set_attribute_int(id, "prediction_scheme", prediction);
            }
        }
        let mut encoder = PointCloudEncoder::new();
        encoder.set_point_cloud(cloud);
        let mut buffer = EncoderBuffer::new();
        encoder.encode(&options, &mut buffer).expect("encode");
        write(name, buffer.data());
    }
    println!("decode both with draco_decoder.exe and compare the outputs");
}
