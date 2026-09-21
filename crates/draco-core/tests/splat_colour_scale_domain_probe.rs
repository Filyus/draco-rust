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

/// The two-sided `fraction` percentile of the values, as a clipping window.
fn percentile_window(values: &[f32], fraction: f64) -> (f32, f32) {
    let mut sorted: Vec<f32> = values.to_vec();
    sorted.sort_unstable_by(f32::total_cmp);
    let last = sorted.len() - 1;
    let low = (fraction * sorted.len() as f64) as usize;
    let high = last - low.min(last);
    (sorted[low.min(last)], sorted[high])
}

/// What an arm does to the values, and how the stored value is read back in
/// the unit the arm is judged in.
///
/// The second half is what keeps the arms comparable: an arm that stores RGB
/// and one that stores `f_dc` are both scored by the error a viewer would see,
/// so each has to say how its stored number becomes that.
struct Arm<'a> {
    label: &'a str,
    /// The original value to what this arm actually stores.
    store: Box<dyn Fn(f32) -> f32 + 'a>,
    /// A stored value to the unit the arm is judged in.
    judge: Box<dyn Fn(f32) -> f32 + 'a>,
    /// The original value to that same unit, exactly — what the stored one is
    /// compared against. For an arm that stores `f_dc` this is the RGB the
    /// value would have had; for one that stores clamped RGB it is the RGB
    /// before the clamp, which is the whole point.
    reference: Box<dyn Fn(f32) -> f32 + 'a>,
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
    let to_rgb = |v: f32| SH_C0 * v + 0.5;
    let (p01_low, p01_high) = percentile_window(&raw, 0.001);
    let (p1_low, p1_high) = percentile_window(&raw, 0.01);
    println!("  the 0.1% percentile window is [{p01_low:.3}, {p01_high:.3}]");
    println!("  the 1%   percentile window is [{p1_low:.3}, {p1_high:.3}]");
    let colour_arms = vec![
        Arm {
            label: "f_dc, as the PLY has it",
            store: Box::new(|v| v),
            judge: Box::new(to_rgb),
            reference: Box::new(to_rgb),
        },
        Arm {
            label: "f_dc, SPZ's fixed window",
            store: Box::new(|v: f32| v.clamp(-SPZ_COLOUR_HALF_WIDTH, SPZ_COLOUR_HALF_WIDTH)),
            judge: Box::new(to_rgb),
            reference: Box::new(to_rgb),
        },
        Arm {
            label: "f_dc, clipped at 0.1%",
            store: Box::new(move |v: f32| v.clamp(p01_low, p01_high)),
            judge: Box::new(to_rgb),
            reference: Box::new(to_rgb),
        },
        Arm {
            label: "f_dc, clipped at 1%",
            store: Box::new(move |v: f32| v.clamp(p1_low, p1_high)),
            judge: Box::new(to_rgb),
            reference: Box::new(to_rgb),
        },
        Arm {
            label: "RGB, clamped to visible",
            store: Box::new(move |v: f32| to_rgb(v).clamp(0.0, 1.0)),
            judge: Box::new(|v| v),
            reference: Box::new(to_rgb),
        },
    ];
    report(
        &cloud,
        &colour,
        &raw,
        &colour_arms,
        num_points,
        "colour B/pt",
        |e| format!("{e:.5}"),
    );

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
    let (p01_low, p01_high) = percentile_window(&raw, 0.001);
    let (p1_low, p1_high) = percentile_window(&raw, 0.01);
    println!("  the 0.1% percentile window is [{p01_low:.3}, {p01_high:.3}]");
    println!("  the 1%   percentile window is [{p1_low:.3}, {p1_high:.3}]");
    let scale_arms = vec![
        Arm {
            label: "log scale, as the PLY has it",
            store: Box::new(|v| v),
            judge: Box::new(|v| v),
            reference: Box::new(|v| v),
        },
        Arm {
            label: "SPZ's fixed window",
            store: Box::new(|v: f32| v.clamp(SPZ_SCALE_LOW, SPZ_SCALE_HIGH)),
            judge: Box::new(|v| v),
            reference: Box::new(|v| v),
        },
        Arm {
            label: "clipped at 0.1%",
            store: Box::new(move |v: f32| v.clamp(p01_low, p01_high)),
            judge: Box::new(|v| v),
            reference: Box::new(|v| v),
        },
        Arm {
            label: "clipped at 1%",
            store: Box::new(move |v: f32| v.clamp(p1_low, p1_high)),
            judge: Box::new(|v| v),
            reference: Box::new(|v| v),
        },
    ];
    // A log step of `d` is a factor of `exp(d)`, so both the step and the
    // clipping displacement are reported as the relative error in the axis
    // length they imply.
    report(
        &cloud,
        &scale,
        &raw,
        &scale_arms,
        num_points,
        "scale B/pt",
        |e| format!("{:.2}%", (e.exp() - 1.0) * 100.0),
    );

    println!();
    println!("  `step` is the error every value carries; `worst clip` is the error");
    println!("  the clipped ones carry instead, and `clipped` is how many. A window");
    println!("  that looks free in the first column is paying in the other two.");
}

/// One table: every arm at three bit depths, scored in the arm's own unit.
fn report(
    cloud: &PointCloud,
    targets: &[i32],
    raw: &[f32],
    arms: &[Arm],
    num_points: usize,
    alone_header: &str,
    unit: impl Fn(f32) -> String,
) {
    println!();
    println!(
        "{:<30} {:>5} {:>12} {:>12} {:>10} {:>10} {:>11}",
        "arm", "bits", alone_header, "whole B/pt", "step", "clipped", "worst clip"
    );
    for arm in arms {
        // What clipping does, before the quantizer does anything: the error the
        // displaced values carry, and how many of them there are.
        let mut clipped = 0usize;
        let mut worst = 0.0f32;
        let mut low = f32::INFINITY;
        let mut high = f32::NEG_INFINITY;
        for &value in raw {
            let stored = (arm.store)(value);
            low = low.min(stored);
            high = high.max(stored);
            let displaced = ((arm.judge)(stored) - (arm.reference)(value)).abs();
            if displaced > 0.0 {
                clipped += 1;
                worst = worst.max(displaced);
            }
        }
        let judged_span = ((arm.judge)(high) - (arm.judge)(low)).abs();
        let permuted = mapped(cloud, targets, &arm.store);
        for bits in [8, 7, 6] {
            let step = judged_span / ((1i64 << bits) - 1) as f32;
            let (alone, ids) = only(&permuted, targets);
            println!(
                "{:<30} {bits:>5} {:>12.4} {:>12.4} {:>10} {:>9.2}% {:>11}",
                arm.label,
                encode(&alone, &ids, bits) as f64 / num_points as f64,
                encode(&permuted, targets, bits) as f64 / num_points as f64,
                unit(step),
                clipped as f64 / raw.len() as f64 * 100.0,
                if clipped == 0 {
                    "-".to_string()
                } else {
                    unit(worst)
                },
            );
        }
    }
}
