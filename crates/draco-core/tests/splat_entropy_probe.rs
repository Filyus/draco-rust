//! Is a splat's spherical-harmonics payload incompressible, or is nothing
//! compressing it?
//!
//! `splat_real_scene_probe` measured that lowering the harmonics' bit budget
//! moves a real scene's encoded size by exactly the budget and by nothing else:
//! 1.003 bits stored per bit spent. Two very different things produce that
//! number -- data with nothing left to remove, or a coder that is not looking
//! -- and an encoded size cannot tell them apart. An entropy can.
//!
//! So this measures the quantized values directly and never encodes anything.
//! Order-0 entropy is the floor any symbol-wise coder can reach, so it is both
//! a statement about the data and a bound on what rANS could have done with
//! it. Reported against the 8 bits each value was quantized to:
//!
//!   ~8.0 bits  the values really do fill their range; the budget is the cost
//!   much less   the coder is leaving that difference on the table
//!
//! The same measurement then runs on differences rather than values, which is
//! the question underneath: splats near each other in space should look alike,
//! and nothing in the sequential coder's ordering uses that. Three orderings
//! bracket it -- the file's own, Morton order over the quantized positions, and
//! a shuffle.
//!
//! The shuffle is the control, and the probe is close to worthless without it.
//! Differencing narrows a distribution whenever consecutive values correlate at
//! all, so "Morton deltas have lower entropy than raw values" would be true of
//! data with no spatial structure whatsoever. Only the gap between Morton and
//! shuffled is spatial correlation; the gap between shuffled and raw is what
//! differencing gives away for free.
//!
//! Needs a scene, and one is 180MB, so `DRACO_SPLAT_PLY` must name it:
//!
//! ```text
//! DRACO_SPLAT_PLY=/path/to/point_cloud.ply \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --test splat_entropy_probe -- --ignored --nocapture
//! ```

#![cfg(all(feature = "encoder", feature = "decoder"))]

use std::path::PathBuf;

use draco_core::{GeometryAttributeType, PointAttribute, PointCloud};

/// Everything is quantized to this, matching the SPZ budget the other probes
/// use, so the entropies below are directly comparable to their byte counts.
const BITS: i32 = 8;

fn read_f32(attribute: &PointAttribute, point: usize, component: usize) -> f32 {
    let stride = attribute.byte_stride() as usize;
    let mut bytes = [0u8; 4];
    attribute
        .buffer()
        .read(point * stride + component * 4, &mut bytes);
    f32::from_le_bytes(bytes)
}

/// One attribute's floats as 8-bit levels, one plane per component.
///
/// The range is shared across an attribute's components, as Draco's own
/// transform shares it, so these are the values the encoder saw.
fn quantize_planes(attribute: &PointAttribute, num_points: usize, bits: i32) -> Vec<Vec<u8>> {
    let components = attribute.num_components() as usize;
    let mut min = vec![f32::INFINITY; components];
    let mut max = vec![f32::NEG_INFINITY; components];
    for point in 0..num_points {
        for (c, (low, high)) in min.iter_mut().zip(max.iter_mut()).enumerate() {
            let value = read_f32(attribute, point, c);
            *low = low.min(value);
            *high = high.max(value);
        }
    }
    let range = (0..components)
        .map(|c| max[c] - min[c])
        .fold(0.0f32, f32::max);
    let levels = ((1u64 << bits) - 1) as f32;
    let scale = if range > 0.0 { levels / range } else { 0.0 };

    (0..components)
        .map(|c| {
            (0..num_points)
                .map(|point| (((read_f32(attribute, point, c) - min[c]) * scale) + 0.5) as u8)
                .collect()
        })
        .collect()
}

/// Order-0 entropy in bits per symbol: the floor for any coder that treats
/// symbols independently, which is what the sequential coder's arithmetic
/// stage does.
fn entropy(values: &[u8]) -> f64 {
    let mut counts = [0u64; 256];
    for &value in values {
        counts[value as usize] += 1;
    }
    let total = values.len() as f64;
    let bits: f64 = counts
        .iter()
        .filter(|&&count| count > 0)
        .map(|&count| {
            let p = count as f64 / total;
            -p * p.log2()
        })
        .sum();
    // A constant plane sums a single -1*log2(1) and lands on negative zero,
    // which prints as "-0.000" and reads as a defect rather than as silence.
    bits + 0.0
}

/// How many of the 256 levels the plane ever uses, and how many hold 90% of it.
///
/// Entropy alone cannot distinguish a plane that uses eight levels evenly from
/// one that uses all 256 in a sharp peak, and those want different things from
/// a coder.
fn occupancy(values: &[u8]) -> (usize, usize) {
    let mut counts = [0u64; 256];
    for &value in values {
        counts[value as usize] += 1;
    }
    let used = counts.iter().filter(|&&c| c > 0).count();
    let mut sorted: Vec<u64> = counts.into_iter().filter(|&c| c > 0).collect();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let target = (values.len() as f64 * 0.9) as u64;
    let mut running = 0u64;
    let mut ninety = 0usize;
    for count in sorted {
        if running >= target {
            break;
        }
        running += count;
        ninety += 1;
    }
    (used, ninety)
}

/// Differences between consecutive entries of `order`, wrapped into a byte.
///
/// Wrapping rather than clamping because the difference is what a predictor
/// would encode and a decoder adds back; it is reversible either way only if
/// nothing is lost.
fn deltas(values: &[u8], order: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(order.len());
    let mut previous = 0u8;
    for &index in order {
        let value = values[index as usize];
        out.push(value.wrapping_sub(previous));
        previous = value;
    }
    out
}

/// Point indices in Morton order over the quantized positions.
fn morton_order(positions: &[Vec<u8>]) -> Vec<u32> {
    let num_points = positions[0].len();
    let mut keys: Vec<(u32, u32)> = (0..num_points)
        .map(|point| {
            // Eight bits per axis interleave into 24, which at this point count
            // separates neighbours finely enough for the question being asked.
            let spread = |v: u8| -> u32 {
                let mut x = v as u32;
                x = (x | (x << 8)) & 0x00F0_0F0F;
                x = (x | (x << 4)) & 0x0C30_C30C;
                x = (x | (x << 2)) & 0x2492_4924;
                x
            };
            let key = spread(positions[0][point])
                | (spread(positions[1][point]) << 1)
                | (spread(positions[2][point]) << 2);
            (key, point as u32)
        })
        .collect();
    keys.sort_unstable();
    keys.into_iter().map(|(_, point)| point).collect()
}

/// A fixed shuffle: the control that separates spatial correlation from what
/// differencing gives away on any data at all.
fn shuffled_order(num_points: usize) -> Vec<u32> {
    let mut order: Vec<u32> = (0..num_points as u32).collect();
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    for i in (1..order.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        order.swap(i, (state % (i as u64 + 1)) as usize);
    }
    order
}

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

#[test]
#[ignore = "needs a real scene in DRACO_SPLAT_PLY: run with --ignored --nocapture"]
fn where_the_harmonics_bits_actually_go() {
    let Some(path) = std::env::var_os("DRACO_SPLAT_PLY").map(PathBuf::from) else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let source = std::fs::read(&path).expect("the scene reads");
    let (mesh, report) = draco_io::ply_reader::PlyReader::from_bytes(source)
        .with_generic_attributes(true)
        .read_mesh_reporting_loss()
        .expect("a splat PLY parses");
    assert!(report.is_lossless(), "the scene is carried whole");
    let cloud = mesh.into_point_cloud();
    let num_points = cloud.num_points();
    let names = attribute_names(&cloud);
    println!("scene: {} ({num_points} points)", path.display());

    // Every component of every attribute, as its own plane of 8-bit levels.
    let mut planes: Vec<(String, Vec<u8>)> = Vec::new();
    let mut positions: Vec<Vec<u8>> = Vec::new();
    for id in 0..cloud.num_attributes() {
        let attribute = cloud.attribute(id);
        let label = match (attribute.attribute_type(), names[id as usize].as_deref()) {
            (GeometryAttributeType::Position, _) => "position".to_string(),
            (GeometryAttributeType::Normal, _) => "normal".to_string(),
            (_, Some(name)) => name.to_string(),
            (kind, None) => format!("{kind:?}"),
        };
        let components = quantize_planes(attribute, num_points, BITS);
        if attribute.attribute_type() == GeometryAttributeType::Position {
            positions = components.clone();
        }
        for (c, plane) in components.into_iter().enumerate() {
            planes.push((format!("{label}[{c}]"), plane));
        }
    }

    let morton = morton_order(&positions);
    let shuffled = shuffled_order(num_points);
    let file_order: Vec<u32> = (0..num_points as u32).collect();

    // ---------------------------------------------------------------------
    // Harmonics, which are 45 of the 62 values a splat carries.
    // ---------------------------------------------------------------------
    let is_harmonic = |label: &str| label.starts_with("f_rest_");
    let harmonic: Vec<&(String, Vec<u8>)> = planes
        .iter()
        .filter(|(label, _)| is_harmonic(label))
        .collect();
    println!();
    println!(
        "=== harmonics: {} planes at {BITS} bits ===",
        harmonic.len()
    );
    println!(
        "{:<14} {:>8} {:>8} {:>10} {:>10} {:>9}",
        "plane", "levels", "90% in", "raw bits", "morton", "shuffled"
    );

    let mut totals = [0.0f64; 4]; // raw, file-order delta, morton delta, shuffled delta
    for (label, plane) in &harmonic {
        let (used, ninety) = occupancy(plane);
        let raw = entropy(plane);
        let in_file = entropy(&deltas(plane, &file_order));
        let in_morton = entropy(&deltas(plane, &morton));
        let in_shuffle = entropy(&deltas(plane, &shuffled));
        totals[0] += raw;
        totals[1] += in_file;
        totals[2] += in_morton;
        totals[3] += in_shuffle;
        // Only the first few by name; the rest are summarised below.
        if harmonic.len() <= 6 || label.ends_with("_0]") || label.starts_with("f_rest_44") {
            println!(
                "{label:<14} {used:>8} {ninety:>8} {raw:>10.3} {in_morton:>10.3} {in_shuffle:>9.3}"
            );
        }
    }
    let n = harmonic.len() as f64;
    println!(
        "{:<14} {:>8} {:>8} {:>10.3} {:>10.3} {:>9.3}   <- mean over all {}",
        "MEAN",
        "",
        "",
        totals[0] / n,
        totals[2] / n,
        totals[3] / n,
        harmonic.len()
    );
    println!(
        "{:<14} {:>8} {:>8} {:>10.2} {:>10.2} {:>9.2}   <- B/point for the harmonics",
        "TOTAL",
        "",
        "",
        totals[0] / 8.0,
        totals[2] / 8.0,
        totals[3] / 8.0
    );
    println!(
        "  file order, for comparison: {:.2} B/point",
        totals[1] / 8.0
    );

    println!();
    println!("  what the numbers separate:");
    println!(
        "    budget                {:>7.2} B/point   what the encoder charged",
        harmonic.len() as f64 * BITS as f64 / 8.0
    );
    println!(
        "    order-0 floor         {:>7.2} B/point   what any symbol-wise coder could reach",
        totals[0] / 8.0
    );
    println!(
        "    differencing, no order{:>7.2} B/point   what a delta gives on shuffled points",
        totals[3] / 8.0
    );
    println!(
        "    differencing, Morton  {:>7.2} B/point   the same delta, spatially ordered",
        totals[2] / 8.0
    );
    println!(
        "    spatial correlation is worth {:.2} B/point (Morton against shuffled)",
        (totals[3] - totals[2]) / 8.0
    );

    // ---------------------------------------------------------------------
    // The cross-check. An entropy is a claim about a floor, and a floor no
    // real coder comes near is a number worth doubting -- most often because
    // the values measured are not the values a coder was given. So the same
    // planes go to an actual deflate, on their own rather than mixed with the
    // rest of the scene, and the two numbers are printed together.
    // ---------------------------------------------------------------------
    let harmonic_path = std::env::temp_dir().join("draco_splat_harmonics.bin");
    let mut stream = Vec::with_capacity(harmonic.len() * num_points);
    for (_, plane) in &harmonic {
        stream.extend_from_slice(plane);
    }
    std::fs::write(&harmonic_path, &stream).expect("the harmonic planes write");
    println!();
    println!("  cross-check, the same planes through a real coder:");
    println!(
        "    written raw           {:>7.2} B/point",
        stream.len() as f64 / num_points as f64
    );
    let gzipped = std::process::Command::new("gzip")
        .args(["-9", "-c"])
        .arg(&harmonic_path)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout.len() as f64 / num_points as f64);
    match gzipped {
        Some(gzipped) => {
            println!("    gzip -9               {gzipped:>7.2} B/point");
            println!(
                "    gzip is {:+.2} B/point against the order-0 floor",
                gzipped - totals[0] / 8.0
            );
        }
        None => println!("    gzip -9               (no gzip on PATH; skipped)"),
    }

    // And the encoder itself, on the same planes and nothing else. The slope
    // across budgets already said it charges the full budget, but a slope is
    // an inference about a total; this is the quantity being claimed.
    let mut harmonics_only = PointCloud::new();
    harmonics_only.set_num_points(num_points);
    for (_, plane) in &harmonic {
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Generic,
            1,
            draco_core::DataType::Uint8,
            false,
            num_points,
        );
        let buffer = attribute.buffer_mut();
        for (index, value) in plane.iter().enumerate() {
            buffer.write(index, &[*value]);
        }
        harmonics_only.add_attribute(attribute);
    }
    let mut options = draco_core::EncoderOptions::new();
    options.set_encoding_method(0); // sequential
    let mut encoder = draco_core::PointCloudEncoder::new();
    encoder.set_point_cloud(harmonics_only);
    let mut buffer = draco_core::EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("the harmonics encode");
    let draco = buffer.data().len() as f64 / num_points as f64;
    println!("    draco sequential      {draco:>7.2} B/point");
    print!(
        "    draco is {:+.2} B/point against the order-0 floor",
        draco - totals[0] / 8.0
    );
    match gzipped {
        Some(gzipped) => println!(", {:+.2} against gzip", draco - gzipped),
        None => println!(),
    }

    // ---------------------------------------------------------------------
    // Everything else, for scale: the harmonics are most of a splat but the
    // rest is where any position coder earns its keep.
    // ---------------------------------------------------------------------
    println!();
    println!("=== everything else ===");
    println!(
        "{:<14} {:>8} {:>8} {:>10} {:>10} {:>9}",
        "plane", "levels", "90% in", "raw bits", "morton", "shuffled"
    );
    for (label, plane) in planes.iter().filter(|(label, _)| !is_harmonic(label)) {
        let (used, ninety) = occupancy(plane);
        println!(
            "{label:<14} {used:>8} {ninety:>8} {:>10.3} {:>10.3} {:>9.3}",
            entropy(plane),
            entropy(&deltas(plane, &morton)),
            entropy(&deltas(plane, &shuffled)),
        );
    }
}
