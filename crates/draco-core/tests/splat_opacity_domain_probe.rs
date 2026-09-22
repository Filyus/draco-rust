//! Opacity costs the same byte either way. Which byte is it?
//!
//! 3DGS writes opacity as a logit and a renderer applies a sigmoid to it, so a
//! PLY hands this crate the logit and the quantizer spreads eight bits over
//! whatever range the logit happens to span. SPZ stores `sigmoid(opacity)`
//! instead — the same one byte, but of the number someone looks at.
//!
//! Two things separate the two, and only one of them is a size:
//!
//! - **Entropy.** A logit has long tails, and the values out in them all mean
//!   the same thing once the sigmoid has run. Eight bits spent separating them
//!   are eight bits spent on nothing.
//! - **Where the resolution lands.** `d(alpha)/d(logit)` is `a(1-a)`, largest
//!   at `a = 0.5`. So a uniform step in the logit is a coarse step in alpha
//!   exactly in the middle, where alpha is visible, and a needlessly fine one
//!   at the ends, where it is not. A uniform step in alpha is the other way
//!   round. Both are reported below; neither is a rendering.
//!
//! ```text
//! DRACO_SPLAT_PLY=../../dev/splat-corpus/train_30000.ply \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --features encoder,decoder --test splat_opacity_domain_probe -- --ignored --nocapture
//! ```

#![cfg(all(feature = "encoder", feature = "decoder"))]

use std::path::PathBuf;

use draco_core::{
    EncoderBuffer, EncoderOptions, GeometryAttributeType, Metadata, PointAttribute, PointCloud,
    PointCloudEncoder, PointIndex,
};

const SEQUENTIAL: i32 = 0;
const OPACITY_BITS: i32 = 8;

fn attribute_names(cloud: &PointCloud) -> Vec<Option<String>> {
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

fn read_f32(attribute: &PointAttribute, point: usize, component: usize) -> f32 {
    let stride = attribute.byte_stride() as usize;
    let mut bytes = [0u8; 4];
    attribute
        .buffer()
        .read(point * stride + component * 4, &mut bytes);
    f32::from_le_bytes(bytes)
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// The cloud with one attribute's values replaced.
fn with_values(cloud: &PointCloud, target: i32, values: &[f32]) -> PointCloud {
    let names = attribute_names(cloud);
    let mut out = PointCloud::new();
    out.set_num_points(cloud.num_points());
    for id in 0..cloud.num_attributes() {
        let source = cloud.attribute(id);
        let components = source.num_components() as usize;
        let mut attribute = PointAttribute::new();
        attribute.init(
            source.attribute_type(),
            source.num_components(),
            source.data_type(),
            source.normalized(),
            cloud.num_points(),
        );
        let buffer = attribute.buffer_mut();
        // The index addresses three different things -- the replacement
        // values, the source's point mapping, and the write offset -- so it
        // stays an index.
        #[allow(clippy::needless_range_loop)]
        for point in 0..cloud.num_points() {
            let value_index = source.mapped_index(PointIndex(point as u32));
            for component in 0..components {
                let value = if id == target {
                    values[point]
                } else {
                    read_f32(source, value_index.0 as usize, component)
                };
                buffer.write((point * components + component) * 4, &value.to_le_bytes());
            }
        }
        let new_id = out.add_attribute(attribute);
        if let Some(name) = names[id as usize].clone() {
            let unique_id = out.attribute(new_id).unique_id();
            let mut metadata = Metadata::new();
            metadata.set_string("name", name).expect("string entry");
            out.metadata_or_insert()
                .set_attribute_metadata(unique_id, metadata);
        }
    }
    out
}

/// One attribute on its own, so its cost is not read off a difference.
fn only(cloud: &PointCloud, id: i32) -> PointCloud {
    let mut out = PointCloud::new();
    out.set_num_points(cloud.num_points());
    out.add_attribute(cloud.attribute(id).clone());
    out
}

/// `opacity_bits` applies to `opacity_id`; everything else takes the usual
/// budget, positions at 16 bits and the rest at a byte.
fn encode(cloud: &PointCloud, opacity_id: i32, opacity_bits: i32) -> usize {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_prediction_search(true);
    options.set_spatial_point_order(true);
    for id in 0..cloud.num_attributes() {
        let bits = if id == opacity_id {
            opacity_bits
        } else {
            match cloud.attribute(id).attribute_type() {
                GeometryAttributeType::Position => 16,
                _ => 8,
            }
        };
        options.set_attribute_int(id, "quantization_bits", bits);
    }
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encodes");
    buffer.data().len()
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn which_byte_opacity_should_be() {
    let Some(path) = std::env::var_os("DRACO_SPLAT_PLY").map(PathBuf::from) else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };

    let source = std::fs::read(&path).expect("the scene reads");
    let mesh = draco_io::ply_reader::PlyReader::from_bytes(source)
        .with_generic_attributes(true)
        .read_mesh()
        .expect("the scene parses");
    let cloud = mesh.into_point_cloud();
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} splats)", path.display());

    let names = attribute_names(&cloud);
    let Some(opacity_id) =
        (0..cloud.num_attributes()).find(|id| names[*id as usize].as_deref() == Some("opacity"))
    else {
        println!("no `opacity` attribute; nothing to measure");
        return;
    };

    let opacity = cloud.attribute(opacity_id);
    let logits: Vec<f32> = (0..num_points)
        .map(|point| {
            let value_index = opacity.mapped_index(PointIndex(point as u32));
            read_f32(opacity, value_index.0 as usize, 0)
        })
        .collect();
    let alphas: Vec<f32> = logits.iter().copied().map(sigmoid).collect();

    let span = |values: &[f32]| -> (f32, f32) {
        values
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| {
                (lo.min(v), hi.max(v))
            })
    };
    let (logit_low, logit_high) = span(&logits);
    let (alpha_low, alpha_high) = span(&alphas);
    let levels = ((1i64 << OPACITY_BITS) - 1) as f32;

    println!();
    println!("what the byte is spread over, at {OPACITY_BITS} bits:");
    println!(
        "  logit  [{logit_low:>8.3}, {logit_high:>7.3}]  step {:.5} in the logit",
        (logit_high - logit_low) / levels
    );
    println!(
        "  alpha  [{alpha_low:>8.5}, {alpha_high:>7.5}]  step {:.5} in alpha",
        (alpha_high - alpha_low) / levels
    );
    // The logit step is worst in alpha where alpha matters: d(alpha)/d(logit)
    // peaks at 0.25.
    println!(
        "  the logit's step is {:.5} in alpha at alpha = 0.5, against {:.5} for the other",
        (logit_high - logit_low) / levels * 0.25,
        (alpha_high - alpha_low) / levels
    );

    let sigmoid_cloud = with_values(&cloud, opacity_id, &alphas);

    // The exchange rate, not just the two corners. The question the sizes alone
    // cannot answer is how many bits of alpha buy the logit's accuracy where
    // alpha is visible, and what those bits cost.
    println!();
    println!(
        "{:<28} {:>9} {:>14} {:>14} {:>13}",
        "arm", "bits", "opacity B/pt", "whole B/pt", "step at a=0.5"
    );
    let logit_only = encode(&only(&cloud, opacity_id), 0, OPACITY_BITS);
    let logit_whole = encode(&cloud, opacity_id, OPACITY_BITS);
    let logit_step = (logit_high - logit_low) / levels * 0.25;
    println!(
        "{:<28} {OPACITY_BITS:>9} {:>14.4} {:>14.4} {logit_step:>13.5}",
        "the logit, as the PLY has it",
        logit_only as f64 / num_points as f64,
        logit_whole as f64 / num_points as f64,
    );
    for bits in [8, 7, 6, 5, 4] {
        let alpha_levels = ((1i64 << bits) - 1) as f32;
        let step = (alpha_high - alpha_low) / alpha_levels;
        let only_bytes = encode(&only(&sigmoid_cloud, opacity_id), 0, bits);
        let whole = encode(&sigmoid_cloud, opacity_id, bits);
        println!(
            "{:<28} {bits:>9} {:>14.4} {:>14.4} {step:>13.5}{}",
            "alpha, as SPZ has it",
            only_bytes as f64 / num_points as f64,
            whole as f64 / num_points as f64,
            if step <= logit_step {
                "  <- finer at a = 0.5"
            } else {
                ""
            }
        );
    }

    println!();
    println!("  the marked rows are finer than the logit at a = 0.5, and coarser");
    println!("  than it everywhere near a = 1: a uniform step in alpha is uniform,");
    println!("  where the logit's step in alpha is a(1-a) and vanishes at the ends.");
    println!("  A rendered surface is made of the opaque end, which is why the");
    println!("  render arm scores alpha at 6 bits below the logit at 8 despite");
    println!("  this column -- see splat_render_arms_probe.");
}
