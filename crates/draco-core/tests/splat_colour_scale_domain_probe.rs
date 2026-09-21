//! Colour and scale, the way SPZ stores them against the way the PLY does.
//!
//! `splat_opacity_domain_probe` asked this of opacity, where the two differ by
//! a sigmoid. Colour and scale differ less and differently, so they need their
//! own arms:
//!
//! - **Scale** is already log in both. `scale_0..2` in the PLY are the log of
//!   the gaussian's axes and SPZ's byte is `enc / 16 - 10`, the same log space.
//!   What differs is only the range: theirs is fixed at `[-10, 5.94]`, ours is
//!   whatever the attribute spans. Ours is finer wherever the data is narrower
//!   than theirs — and coarser wherever an outlier stretches it, which is the
//!   thing worth checking.
//! - **Colour** is a degree-0 harmonic, and what a renderer wants is
//!   `0.28209 * f_dc + 0.5` clamped to `[0, 1]`. SPZ stores `f_dc * 0.15 + 0.5`
//!   in a byte, a window of `[-3.33, 3.33]` in `f_dc`, and its own comment says
//!   the wide window is deliberate: a colour out of range can be brought back
//!   by the higher bands, so clamping to visible RGB is **not** free the way
//!   the sigmoid was for alpha. The arm is measured anyway, with the count of
//!   what it clamps, because that count is what decides whether it is usable.
//!
//! Accuracy is reported in the unit the arm is judged in: RGB for colour, and
//! for scale the relative error in the axis length, since a step of `d` in a
//! log is a factor of `exp(d)`.
//!
//! ```text
//! DRACO_SPLAT_PLY=../../dev/splat-corpus/train_30000.ply \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --features encoder,decoder --test splat_colour_scale_domain_probe -- --ignored --nocapture
//! ```

#![cfg(all(feature = "encoder", feature = "decoder"))]

use std::path::PathBuf;

use draco_core::{
    EncoderBuffer, EncoderOptions, GeometryAttributeType, Metadata, PointAttribute, PointCloud,
    PointCloudEncoder, PointIndex,
};

const SEQUENTIAL: i32 = 0;

/// The degree-0 spherical harmonic constant: `rgb = SH_C0 * f_dc + 0.5`.
const SH_C0: f32 = 0.282_094_79;

/// SPZ's colour window, from `((byte / 255) - 0.5) / 0.15` in its reader.
const SPZ_COLOUR_HALF_WIDTH: f32 = 1.0 / (2.0 * 0.15);

/// SPZ's log-scale window, from `enc / 16 - 10` over a byte.
const SPZ_SCALE_LOW: f32 = -10.0;
const SPZ_SCALE_HIGH: f32 = 255.0 / 16.0 - 10.0;

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

/// The cloud with a transform applied to every named attribute.
fn mapped(cloud: &PointCloud, targets: &[i32], transform: impl Fn(f32) -> f32) -> PointCloud {
    let names = attribute_names(cloud);
    let mut out = PointCloud::new();
    out.set_num_points(cloud.num_points());
    for id in 0..cloud.num_attributes() {
        let source = cloud.attribute(id);
        let components = source.num_components() as usize;
        let touched = targets.contains(&id);
        let mut attribute = PointAttribute::new();
        attribute.init(
            source.attribute_type(),
            source.num_components(),
            source.data_type(),
            source.normalized(),
            cloud.num_points(),
        );
        let buffer = attribute.buffer_mut();
        #[allow(clippy::needless_range_loop)]
        for point in 0..cloud.num_points() {
            let value_index = source.mapped_index(PointIndex(point as u32));
            for component in 0..components {
                let raw = read_f32(source, value_index.0 as usize, component);
                let value = if touched { transform(raw) } else { raw };
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

/// Just the named attributes, so their cost is measured and not differenced.
///
/// Returns their ids in the new cloud as well, which are not the ids they had:
/// a cloud built from three attributes numbers them from zero, and passing the
/// old numbers on to the encoder silently applies the bit budget to the wrong
/// attributes or to none.
fn only(cloud: &PointCloud, targets: &[i32]) -> (PointCloud, Vec<i32>) {
    let mut out = PointCloud::new();
    out.set_num_points(cloud.num_points());
    let mut ids = Vec::with_capacity(targets.len());
    for &id in targets {
        ids.push(out.add_attribute(cloud.attribute(id).clone()));
    }
    (out, ids)
}

fn encode(cloud: &PointCloud, targets: &[i32], bits: i32) -> usize {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_prediction_search(true);
    options.set_spatial_point_order(true);
    for id in 0..cloud.num_attributes() {
        let attribute_bits = if targets.contains(&id) {
            bits
        } else {
            match cloud.attribute(id).attribute_type() {
                GeometryAttributeType::Position => 16,
                _ => 8,
            }
        };
        options.set_attribute_int(id, "quantization_bits", attribute_bits);
    }
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encodes");
    buffer.data().len()
}

fn span(values: &[f32]) -> (f32, f32) {
    values
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| {
            (lo.min(v), hi.max(v))
        })
}

fn values_of(cloud: &PointCloud, targets: &[i32]) -> Vec<f32> {
    let mut out = Vec::new();
    for &id in targets {
        let attribute = cloud.attribute(id);
        for point in 0..cloud.num_points() {
            let value_index = attribute.mapped_index(PointIndex(point as u32));
            out.push(read_f32(attribute, value_index.0 as usize, 0));
        }
    }
    out
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn which_domain_colour_and_scale_should_be() {
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
    let ids_named = |prefix: &str| -> Vec<i32> {
        (0..cloud.num_attributes())
            .filter(|id| {
                names[*id as usize]
                    .as_deref()
                    .is_some_and(|name| name.starts_with(prefix))
            })
            .collect()
    };
    let colour = ids_named("f_dc_");
    let scale = ids_named("scale_");
    if colour.is_empty() || scale.is_empty() {
        println!("no `f_dc_*` or `scale_*` attributes; nothing to measure");
        return;
    }

    let per_point = |bytes: usize| bytes as f64 / num_points as f64;

    // ---- colour -----------------------------------------------------------
    let raw = values_of(&cloud, &colour);
    let (low, high) = span(&raw);
    let out_of_spz = raw
        .iter()
        .filter(|v| v.abs() > SPZ_COLOUR_HALF_WIDTH)
        .count();
    let out_of_rgb = raw
        .iter()
        .filter(|v| {
            let rgb = SH_C0 * **v + 0.5;
            !(0.0..=1.0).contains(&rgb)
        })
        .count();
    println!();
    println!("colour, {} attributes of f_dc", colour.len());
    println!("  f_dc spans [{low:.3}, {high:.3}]");
    println!(
        "  outside SPZ's window of +-{SPZ_COLOUR_HALF_WIDTH:.3}: {out_of_spz} values, {:.2}%",
        out_of_spz as f64 / raw.len() as f64 * 100.0
    );
    println!(
        "  outside visible RGB once converted:                {out_of_rgb} values, {:.2}%",
        out_of_rgb as f64 / raw.len() as f64 * 100.0
    );
    println!();
    println!(
        "{:<34} {:>5} {:>13} {:>13} {:>13}",
        "arm", "bits", "colour B/pt", "whole B/pt", "step in RGB"
    );
    let colour_arms: Vec<(&str, PointCloud, f32)> = vec![
        (
            "f_dc, as the PLY has it",
            mapped(&cloud, &[], |v| v),
            SH_C0 * (high - low),
        ),
        (
            "f_dc clamped to SPZ's window",
            mapped(&cloud, &colour, |v| {
                v.clamp(-SPZ_COLOUR_HALF_WIDTH, SPZ_COLOUR_HALF_WIDTH)
            }),
            SH_C0 * 2.0 * SPZ_COLOUR_HALF_WIDTH,
        ),
        (
            "RGB, clamped to what is visible",
            mapped(&cloud, &colour, |v| (SH_C0 * v + 0.5).clamp(0.0, 1.0)),
            1.0,
        ),
    ];
    for (label, arm, rgb_span) in &colour_arms {
        for bits in [8, 7, 6] {
            println!(
                "{label:<34} {bits:>5} {:>13.4} {:>13.4} {:>13.5}",
                {
                    let (alone, ids) = only(arm, &colour);
                    per_point(encode(&alone, &ids, bits))
                },
                per_point(encode(arm, &colour, bits)),
                rgb_span / ((1i64 << bits) - 1) as f32,
            );
        }
    }

    // ---- scale ------------------------------------------------------------
    let raw = values_of(&cloud, &scale);
    let (low, high) = span(&raw);
    let out_of_spz = raw
        .iter()
        .filter(|v| **v < SPZ_SCALE_LOW || **v > SPZ_SCALE_HIGH)
        .count();
    println!();
    println!("scale, {} attributes, log of the axis length", scale.len());
    println!("  spans [{low:.3}, {high:.3}]; SPZ's fixed window is [{SPZ_SCALE_LOW:.2}, {SPZ_SCALE_HIGH:.2}]");
    println!(
        "  outside it: {out_of_spz} values, {:.2}%",
        out_of_spz as f64 / raw.len() as f64 * 100.0
    );
    println!();
    println!(
        "{:<34} {:>5} {:>13} {:>13} {:>13}",
        "arm", "bits", "scale B/pt", "whole B/pt", "size error"
    );
    let scale_arms: Vec<(&str, PointCloud, f32)> = vec![
        (
            "log scale, as the PLY has it",
            mapped(&cloud, &[], |v| v),
            high - low,
        ),
        (
            "clamped to SPZ's window",
            mapped(&cloud, &scale, |v| v.clamp(SPZ_SCALE_LOW, SPZ_SCALE_HIGH)),
            SPZ_SCALE_HIGH - SPZ_SCALE_LOW,
        ),
    ];
    for (label, arm, log_span) in &scale_arms {
        for bits in [8, 7, 6] {
            let step = log_span / ((1i64 << bits) - 1) as f32;
            println!(
                "{label:<34} {bits:>5} {:>13.4} {:>13.4} {:>12.2}%",
                {
                    let (alone, ids) = only(arm, &scale);
                    per_point(encode(&alone, &ids, bits))
                },
                per_point(encode(arm, &scale, bits)),
                (step.exp() - 1.0) * 100.0,
            );
        }
    }

    println!();
    println!("  sizes and resolutions, not renderings. The RGB arm discards");
    println!("  everything a higher harmonic band could have brought back, which");
    println!("  is the reason SPZ's window is as wide as it is.");
}
