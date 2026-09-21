//! EXPERIMENT: how much is left in the point order at all?
//!
//! Hilbert beats Morton by 0.8% on a splat and 2.6% on a capture, which raises
//! the question the curve comparison cannot answer: is that the last of it, or
//! is a space-filling curve simply a poor way to order points and a better one
//! waits?
//!
//! A curve is a compromise — it is computed per point without looking at the
//! others, which is why it is cheap. A greedy nearest-neighbour tour is the
//! opposite: it looks at where the points actually are, has no block structure
//! to jump across, and produces steps no curve can beat except by luck. It is
//! not optimal — it strands itself and has to jump — but it is far better at
//! locality than any curve, so what it buys over Hilbert bounds what is left in
//! ordering.
//!
//! All three orders are applied the same way, by permuting the cloud and
//! encoding with `set_spatial_point_order` off, so the encoder does the same
//! work in every arm and only the permutation differs.
//!
//! ```text
//! DRACO_SPLAT_PLY=/path/to/point_cloud.ply \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --features encoder,decoder --test order_ceiling_probe -- --ignored --nocapture
//! ```

#![cfg(all(feature = "encoder", feature = "decoder"))]

use std::collections::HashMap;
use std::path::PathBuf;

use draco_core::{
    EncoderBuffer, EncoderOptions, GeometryAttributeType, Metadata, PointAttribute, PointCloud,
    PointCloudEncoder, PointIndex,
};

const SEQUENTIAL: i32 = 0;

/// What the probes quantize positions to, and so the grid every order here is
/// laid over: ordering finer than this sorts by differences the encode drops.
const AXIS_BITS: u32 = 16;

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

/// The same cloud with its points in the given order.
fn permute(cloud: &PointCloud, order: &[u32]) -> PointCloud {
    let names = attribute_names(cloud);
    let mut out = PointCloud::new();
    out.set_num_points(order.len());
    for id in 0..cloud.num_attributes() {
        let source = cloud.attribute(id);
        let components = source.num_components() as usize;
        let mut attribute = PointAttribute::new();
        attribute.init(
            source.attribute_type(),
            source.num_components(),
            source.data_type(),
            source.normalized(),
            order.len(),
        );
        let buffer = attribute.buffer_mut();
        for (slot, &point) in order.iter().enumerate() {
            let value_index = source.mapped_index(PointIndex(point));
            for component in 0..components {
                let value = read_f32(source, value_index.0 as usize, component);
                buffer.write((slot * components + component) * 4, &value.to_le_bytes());
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

fn encode(cloud: &PointCloud, search: bool) -> usize {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_prediction_search(search);
    // Off: the order is the cloud's own, put there by the caller.
    options.set_spatial_point_order(false);
    for id in 0..cloud.num_attributes() {
        let bits = match cloud.attribute(id).attribute_type() {
            GeometryAttributeType::Position => AXIS_BITS as i32,
            _ => 8,
        };
        options.set_attribute_int(id, "quantization_bits", bits);
    }
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encodes");
    buffer.data().len()
}

/// Positions on the same integer grid the encoder quantizes them to.
fn quantized_positions(cloud: &PointCloud) -> Option<Vec<[u32; 3]>> {
    let att_id = (0..cloud.num_attributes())
        .find(|id| cloud.attribute(*id).attribute_type() == GeometryAttributeType::Position)?;
    let attribute = cloud.attribute(att_id);
    let num_points = cloud.num_points();
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut raw = Vec::with_capacity(num_points);
    for point in 0..num_points {
        let value_index = attribute.mapped_index(PointIndex(point as u32));
        let mut coordinates = [0.0f64; 3];
        for (axis, slot) in coordinates.iter_mut().enumerate() {
            *slot = f64::from(read_f32(attribute, value_index.0 as usize, axis));
            min[axis] = min[axis].min(*slot);
            max[axis] = max[axis].max(*slot);
        }
        raw.push(coordinates);
    }
    let levels = ((1u64 << AXIS_BITS) - 1) as f64;
    Some(
        raw.into_iter()
            .map(|coordinates| {
                let mut cell = [0u32; 3];
                for (axis, slot) in cell.iter_mut().enumerate() {
                    let span = max[axis] - min[axis];
                    let normalized = if span > 0.0 {
                        (coordinates[axis] - min[axis]) / span
                    } else {
                        0.0
                    };
                    *slot = (normalized * levels) as u32;
                }
                cell
            })
            .collect(),
    )
}

fn morton_order(cells: &[[u32; 3]]) -> Vec<u32> {
    let spread = |v: u32| -> u64 {
        let mut x = u64::from(v) & 0x1f_ffff;
        x = (x | (x << 32)) & 0x001f_0000_0000_ffff;
        x = (x | (x << 16)) & 0x001f_0000_ff00_00ff;
        x = (x | (x << 8)) & 0x100f_00f0_0f00_f00f;
        x = (x | (x << 4)) & 0x10c3_0c30_c30c_30c3;
        x = (x | (x << 2)) & 0x1249_2492_4924_9249;
        x
    };
    let mut keyed: Vec<(u64, u32)> = cells
        .iter()
        .enumerate()
        .map(|(point, cell)| {
            let key = spread(cell[0]) | (spread(cell[1]) << 1) | (spread(cell[2]) << 2);
            (key, point as u32)
        })
        .collect();
    keyed.sort_unstable();
    keyed.into_iter().map(|(_, point)| point).collect()
}

fn hilbert_order(cells: &[[u32; 3]]) -> Vec<u32> {
    let mut keyed: Vec<(u64, u32)> = cells
        .iter()
        .enumerate()
        .map(|(point, cell)| {
            (
                hilbert_key(cell[0], cell[1], cell[2], AXIS_BITS),
                point as u32,
            )
        })
        .collect();
    keyed.sort_unstable();
    keyed.into_iter().map(|(_, point)| point).collect()
}

/// A greedy nearest-neighbour tour over the quantized cells.
///
/// Points go into a uniform grid sized for a handful per occupied cell, and
/// each step searches outward from the current cell in shells until it finds an
/// unvisited point, then one shell further so that a diagonal neighbour cannot
/// beat an axis one that was found first. When a shell search runs past
/// `MAX_SHELLS` the tour is stranded and jumps to the next unvisited point in
/// Morton order, which is the part that keeps this a heuristic.
fn nearest_neighbour_order(cells: &[[u32; 3]], fallback: &[u32]) -> (Vec<u32>, usize) {
    // A cell side of 2^SHIFT grid units. Chosen so the buckets hold a few
    // points each on a scene of this density; too small and the shell search
    // walks empty space, too large and each bucket is a linear scan.
    const SHIFT: u32 = 6;
    const MAX_SHELLS: i64 = 12;

    let bucket_of = |cell: &[u32; 3]| -> (i64, i64, i64) {
        (
            (cell[0] >> SHIFT) as i64,
            (cell[1] >> SHIFT) as i64,
            (cell[2] >> SHIFT) as i64,
        )
    };

    let mut buckets: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    for (point, cell) in cells.iter().enumerate() {
        buckets
            .entry(bucket_of(cell))
            .or_default()
            .push(point as u32);
    }

    let distance = |a: &[u32; 3], b: &[u32; 3]| -> i64 {
        let dx = i64::from(a[0]) - i64::from(b[0]);
        let dy = i64::from(a[1]) - i64::from(b[1]);
        let dz = i64::from(a[2]) - i64::from(b[2]);
        dx * dx + dy * dy + dz * dz
    };

    let mut visited = vec![false; cells.len()];
    let mut order = Vec::with_capacity(cells.len());
    let mut strandings = 0usize;
    let mut fallback_cursor = 0usize;

    let take = |point: u32, buckets: &mut HashMap<(i64, i64, i64), Vec<u32>>| {
        let bucket = bucket_of(&cells[point as usize]);
        if let Some(entry) = buckets.get_mut(&bucket) {
            if let Some(position) = entry.iter().position(|&p| p == point) {
                entry.swap_remove(position);
            }
            if entry.is_empty() {
                buckets.remove(&bucket);
            }
        }
    };

    let mut current = fallback[0];
    visited[current as usize] = true;
    take(current, &mut buckets);
    order.push(current);

    while order.len() < cells.len() {
        let origin = bucket_of(&cells[current as usize]);
        let mut best: Option<(i64, u32)> = None;
        let mut shell = 0i64;
        let mut shells_since_hit = 0i64;
        while shell <= MAX_SHELLS {
            // The shell's surface only: cells whose Chebyshev distance from the
            // origin bucket is exactly `shell`.
            for dx in -shell..=shell {
                for dy in -shell..=shell {
                    for dz in -shell..=shell {
                        if dx.abs() != shell && dy.abs() != shell && dz.abs() != shell {
                            continue;
                        }
                        let key = (origin.0 + dx, origin.1 + dy, origin.2 + dz);
                        let Some(entry) = buckets.get(&key) else {
                            continue;
                        };
                        for &candidate in entry {
                            let d = distance(&cells[current as usize], &cells[candidate as usize]);
                            if best.is_none_or(|(best_d, _)| d < best_d) {
                                best = Some((d, candidate));
                            }
                        }
                    }
                }
            }
            if best.is_some() {
                shells_since_hit += 1;
                // One shell past the first hit, so a nearer point in a
                // diagonal bucket is not missed.
                if shells_since_hit > 1 {
                    break;
                }
            }
            shell += 1;
        }

        let next = match best {
            Some((_, candidate)) => candidate,
            None => {
                strandings += 1;
                while fallback_cursor < fallback.len()
                    && visited[fallback[fallback_cursor] as usize]
                {
                    fallback_cursor += 1;
                }
                if fallback_cursor >= fallback.len() {
                    break;
                }
                fallback[fallback_cursor]
            }
        };
        visited[next as usize] = true;
        take(next, &mut buckets);
        order.push(next);
        current = next;
    }

    (order, strandings)
}

/// The 3D Hilbert key, as in `point_cloud_encoder`.
fn hilbert_key(x: u32, y: u32, z: u32, bits: u32) -> u64 {
    let mut key = 0u64;
    let mut state = 0usize;
    let mut shift = bits;
    while shift > 0 {
        shift -= 1;
        let octant = (((x >> shift) & 1) << 2) | (((y >> shift) & 1) << 1) | ((z >> shift) & 1);
        let entry = HILBERT3_STEP_LUT[state * 8 + octant as usize];
        key = (key << 3) | u64::from(entry & 7);
        state = (entry >> 3) as usize;
    }
    key
}

const HILBERT3_STEP_LUT: [u8; 192] = build_hilbert3_step_lut();

const fn build_hilbert3_step_lut() -> [u8; 192] {
    let mut table = [0u8; 192];
    let mut state = 0usize;
    while state < 24 {
        let c = (state & 7) as u32;
        let n = (state / 8) as u32;
        let mut m = 0u32;
        while m < 8 {
            let gray = rotate_right_3(c ^ m, n);
            let i = gray_to_integer_3(gray);
            let without_high_bit = gray & 0b011;
            let next_rotation = if without_high_bit == 0 {
                1
            } else if (without_high_bit & 1) != 0 {
                2
            } else {
                3
            };
            let transform = if i == 0 {
                0
            } else {
                let low_bit = i & 0u32.wrapping_sub(i);
                gray ^ (low_bit | 1)
            };
            let next_c = c ^ rotate_left_3(transform, n);
            let next_n = (n + next_rotation) % 3;
            table[state * 8 + m as usize] = (((next_n * 8 + next_c) as u8) << 3) | (i as u8);
            m += 1;
        }
        state += 1;
    }
    table
}

const fn rotate_left_3(value: u32, shift: u32) -> u32 {
    match shift {
        0 => value & 7,
        1 => ((value << 1) | (value >> 2)) & 7,
        _ => ((value << 2) | (value >> 1)) & 7,
    }
}

const fn rotate_right_3(value: u32, shift: u32) -> u32 {
    match shift {
        0 => value & 7,
        1 => ((value >> 1) | (value << 2)) & 7,
        _ => ((value >> 2) | (value << 1)) & 7,
    }
}

const fn gray_to_integer_3(mut gray: u32) -> u32 {
    gray ^= gray >> 1;
    gray ^= gray >> 2;
    gray & 7
}

fn mean_step(cells: &[[u32; 3]], order: &[u32]) -> f64 {
    let mut total = 0.0;
    for pair in order.windows(2) {
        let a = &cells[pair[0] as usize];
        let b = &cells[pair[1] as usize];
        let dx = f64::from(a[0]) - f64::from(b[0]);
        let dy = f64::from(a[1]) - f64::from(b[1]);
        let dz = f64::from(a[2]) - f64::from(b[2]);
        total += (dx * dx + dy * dy + dz * dz).sqrt();
    }
    total / (order.len() - 1) as f64
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn how_much_is_left_in_the_order() {
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
    println!(
        "scene: {} ({num_points} points, {} attributes), {AXIS_BITS} bits an axis",
        path.display(),
        cloud.num_attributes()
    );

    let Some(cells) = quantized_positions(&cloud) else {
        println!("no readable position attribute; nothing to measure");
        return;
    };

    let file_order: Vec<u32> = (0..num_points as u32).collect();
    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let started = std::time::Instant::now();
    let (tour, strandings) = nearest_neighbour_order(&cells, &morton);
    println!(
        "nearest-neighbour tour: {} points, {strandings} strandings, built in {:.1}s",
        tour.len(),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(tour.len(), num_points, "the tour missed points");
    println!();

    println!(
        "{:<22} {:>12} {:>14} {:>10} {:>14} {:>10}",
        "order", "mean step", "search off", "vs Morton", "search on", "vs Morton"
    );
    let mut reference = (0usize, 0usize);
    for (label, order) in [
        ("file order", &file_order),
        ("Morton", &morton),
        ("Hilbert", &hilbert),
        ("nearest neighbour", &tour),
    ] {
        let permuted = permute(&cloud, order);
        let plain = encode(&permuted, false);
        let searched = encode(&permuted, true);
        if label == "Morton" {
            reference = (plain, searched);
        }
        let against = |bytes: usize, base: usize| -> String {
            if base == 0 {
                "-".to_string()
            } else {
                format!("{:+.2}%", (bytes as f64 / base as f64 - 1.0) * 100.0)
            }
        };
        println!(
            "{label:<22} {:>12.1} {:>14.3} {:>10} {:>14.3} {:>10}",
            mean_step(&cells, order),
            plain as f64 / num_points as f64,
            against(plain, reference.0),
            searched as f64 / num_points as f64,
            against(searched, reference.1),
        );
    }
}
