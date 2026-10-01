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

/// The bits the web converter's splat budget gives a property: harmonics at 6
/// (`DRACO_ORDER_HARMONICS` overrides), everything else but the position at 8.
fn budget_bits(name: Option<&str>) -> i32 {
    match name {
        Some(name) if name.starts_with("f_rest_") => std::env::var("DRACO_ORDER_HARMONICS")
            .ok()
            .and_then(|b| b.parse().ok())
            .unwrap_or(6),
        _ => 8,
    }
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

/// One component as a number, whatever the attribute stores it as.
fn read_value(attribute: &PointAttribute, point: usize, component: usize) -> f32 {
    use draco_core::DataType;
    let stride = attribute.byte_stride() as usize;
    let width = match attribute.data_type() {
        DataType::Int8 | DataType::Uint8 | DataType::Bool => 1,
        DataType::Int16 | DataType::Uint16 => 2,
        DataType::Float64 | DataType::Int64 | DataType::Uint64 => 8,
        _ => 4,
    };
    let mut bytes = [0u8; 8];
    attribute
        .buffer()
        .read(point * stride + component * width, &mut bytes[..width]);
    match attribute.data_type() {
        DataType::Int8 => f32::from(bytes[0] as i8),
        DataType::Uint8 | DataType::Bool => f32::from(bytes[0]),
        DataType::Int16 => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])),
        DataType::Uint16 => f32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
        DataType::Int32 => i32::from_le_bytes(bytes[..4].try_into().unwrap()) as f32,
        DataType::Uint32 => u32::from_le_bytes(bytes[..4].try_into().unwrap()) as f32,
        DataType::Float32 => f32::from_le_bytes(bytes[..4].try_into().unwrap()),
        DataType::Float64 => f64::from_le_bytes(bytes) as f32,
        _ => panic!("the probe does not read {:?}", attribute.data_type()),
    }
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
        let stride = source.byte_stride() as usize;
        let buffer = attribute.buffer_mut();
        let mut raw = vec![0u8; stride];
        for (slot, &point) in order.iter().enumerate() {
            let value_index = source.mapped_index(PointIndex(point));
            source
                .buffer()
                .read(value_index.0 as usize * stride, &mut raw);
            buffer.write(slot * stride, &raw);
        }
        let _ = components;
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
    encode_bytes(cloud, search).len()
}

fn encode_bytes(cloud: &PointCloud, search: bool) -> Vec<u8> {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_prediction_search(search);
    // Off: the order is the cloud's own, put there by the caller.
    options.set_spatial_point_order(false);
    let names = attribute_names(cloud);
    for id in 0..cloud.num_attributes() {
        let bits = match cloud.attribute(id).attribute_type() {
            GeometryAttributeType::Position => AXIS_BITS as i32,
            _ => budget_bits(names[id as usize].as_deref()),
        };
        if cloud.attribute(id).data_type() == draco_core::DataType::Float32 {
            options.set_attribute_int(id, "quantization_bits", bits);
        }
    }
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encodes");
    buffer.data().to_vec()
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

/// The psi tour on the SIMD index rather than the scalar one.
///
/// `finish_simd` produces the layout psi's runtime-dispatched kernels read;
/// the query is otherwise the unbounded walk, so this isolates the index
/// layout from how it is asked.
fn nearest_neighbour_order_psi_simd(cells: &[[u32; 3]]) -> (Vec<u32>, usize) {
    use packed_spatial_index::{Index3DBuilder, Point3D};
    use std::ops::ControlFlow;

    let mut builder = Index3DBuilder::new(cells.len());
    for cell in cells {
        let (x, y, z) = (f64::from(cell[0]), f64::from(cell[1]), f64::from(cell[2]));
        builder.add(packed_spatial_index::Box3D::new(x, y, z, x, y, z));
    }
    let index = builder.finish_simd().expect("the index builds");

    let mut visited = vec![false; cells.len()];
    let mut order = Vec::with_capacity(cells.len());
    let mut current = 0u32;
    visited[0] = true;
    order.push(current);

    while order.len() < cells.len() {
        let cell = &cells[current as usize];
        let query = Point3D::new(f64::from(cell[0]), f64::from(cell[1]), f64::from(cell[2]));
        let mut found = None;
        let _: ControlFlow<()> = index.neighbors_each(query, f64::INFINITY, |item, _| {
            if visited[item] {
                return ControlFlow::Continue(());
            }
            found = Some(item as u32);
            ControlFlow::Break(())
        });
        let Some(next) = found else { break };
        visited[next as usize] = true;
        order.push(next);
        current = next;
    }

    (order, 0)
}

/// The psi tour again, asking the index the way it wants to be asked.
///
/// The unbounded `neighbors_each` below allocates a priority queue per call and
/// is told to search the whole scene, so it walks the growing tail of already
/// visited points every step. This one reuses a workspace, asks for a handful
/// of neighbours inside a radius scaled to the step it just took, and only
/// widens when that comes back with nothing new. The last resort is the
/// unbounded walk, so the tour stays exact and strands nowhere.
fn nearest_neighbour_order_psi_bounded(cells: &[[u32; 3]]) -> (Vec<u32>, usize) {
    use packed_spatial_index::{Index3DBuilder, NeighborWorkspace, Point3D};
    use std::ops::ControlFlow;

    let mut builder = Index3DBuilder::new(cells.len());
    for cell in cells {
        let (x, y, z) = (f64::from(cell[0]), f64::from(cell[1]), f64::from(cell[2]));
        builder.add(packed_spatial_index::Box3D::new(x, y, z, x, y, z));
    }
    let index = builder.finish().expect("the index builds");

    let point_of = |point: u32| -> Point3D {
        let cell = &cells[point as usize];
        Point3D::new(f64::from(cell[0]), f64::from(cell[1]), f64::from(cell[2]))
    };
    let distance = |a: u32, b: u32| -> f64 {
        let (a, b) = (&cells[a as usize], &cells[b as usize]);
        let dx = f64::from(a[0]) - f64::from(b[0]);
        let dy = f64::from(a[1]) - f64::from(b[1]);
        let dz = f64::from(a[2]) - f64::from(b[2]);
        (dx * dx + dy * dy + dz * dz).sqrt()
    };

    let mut workspace = NeighborWorkspace::with_capacity(64, 256);
    let mut visited = vec![false; cells.len()];
    let mut order = Vec::with_capacity(cells.len());
    let mut widenings = 0usize;

    let mut current = 0u32;
    visited[0] = true;
    order.push(current);
    // Seeded at a whole grid cell and then tracked to the tour's own step, so
    // the radius follows the local density rather than a guess about it.
    let mut radius = 16.0f64;

    while order.len() < cells.len() {
        let query = point_of(current);
        let mut next = None;
        for attempt in 0..4 {
            let wanted = 8usize << (attempt * 2);
            let found = index.neighbors_with(query, wanted, radius, &mut workspace);
            next = found
                .iter()
                .find(|&&item| !visited[item])
                .map(|&i| i as u32);
            if next.is_some() {
                break;
            }
            widenings += 1;
            radius *= 4.0;
        }
        let next = match next {
            Some(next) => next,
            None => {
                // Everything nearby is visited; fall back to the exact walk so
                // this stays the same tour as the unbounded version.
                let mut found = None;
                let _: ControlFlow<()> = index.neighbors_each(query, f64::INFINITY, |item, _| {
                    if visited[item] {
                        return ControlFlow::Continue(());
                    }
                    found = Some(item as u32);
                    ControlFlow::Break(())
                });
                match found {
                    Some(next) => next,
                    None => break,
                }
            }
        };
        radius = (distance(current, next) * 4.0).max(16.0);
        visited[next as usize] = true;
        order.push(next);
        current = next;
    }

    (order, widenings)
}

/// The same greedy tour, over `packed_spatial_index`'s 3D index.
///
/// `neighbors_each` visits in nondecreasing distance, so the first unvisited
/// point it reaches is the answer and there is no `k` to guess and no shell
/// radius to cap. The index is static and cannot have visited points removed
/// from it, which is the one thing the hand-rolled grid below does better;
/// whether that matters is what the timing says.
fn nearest_neighbour_order_psi(cells: &[[u32; 3]]) -> (Vec<u32>, usize) {
    use packed_spatial_index::{Index3DBuilder, Point3D};
    use std::ops::ControlFlow;

    let mut builder = Index3DBuilder::new(cells.len());
    for cell in cells {
        let (x, y, z) = (f64::from(cell[0]), f64::from(cell[1]), f64::from(cell[2]));
        builder.add(packed_spatial_index::Box3D::new(x, y, z, x, y, z));
    }
    let index = builder.finish().expect("the index builds");

    let mut visited = vec![false; cells.len()];
    let mut order = Vec::with_capacity(cells.len());
    let mut strandings = 0usize;
    let mut cursor = 0usize;

    let mut current = 0u32;
    visited[0] = true;
    order.push(current);

    while order.len() < cells.len() {
        let cell = &cells[current as usize];
        let query = Point3D::new(f64::from(cell[0]), f64::from(cell[1]), f64::from(cell[2]));
        let mut found: Option<u32> = None;
        let _: ControlFlow<()> = index.neighbors_each(query, f64::INFINITY, |item, _| {
            if visited[item] {
                return ControlFlow::Continue(());
            }
            found = Some(item as u32);
            ControlFlow::Break(())
        });

        let next = match found {
            Some(next) => next,
            None => {
                // Cannot happen while any point is unvisited, since the walk is
                // unbounded; kept so the loop terminates rather than spins if
                // it ever does.
                strandings += 1;
                while cursor < cells.len() && visited[cursor] {
                    cursor += 1;
                }
                if cursor >= cells.len() {
                    break;
                }
                cursor as u32
            }
        };
        visited[next as usize] = true;
        order.push(next);
        current = next;
    }

    (order, strandings)
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
        "tour, hand-rolled grid: {} points, {strandings} strandings, built in {:.1}s",
        tour.len(),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(tour.len(), num_points, "the tour missed points");

    let started = std::time::Instant::now();
    let (psi_tour, psi_strandings) = nearest_neighbour_order_psi(&cells);
    println!(
        "tour, psi unbounded: {} points, {psi_strandings} strandings, built in {:.1}s",
        psi_tour.len(),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(psi_tour.len(), num_points, "the tour missed points");

    let started = std::time::Instant::now();
    let (psi_bounded, widenings) = nearest_neighbour_order_psi_bounded(&cells);
    println!(
        "tour, psi bounded:   {} points, {widenings} widenings, built in {:.1}s",
        psi_bounded.len(),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(psi_bounded.len(), num_points, "the tour missed points");
    assert_eq!(
        psi_bounded, psi_tour,
        "the bounded search found a different tour than the exact one"
    );

    let started = std::time::Instant::now();
    let (psi_simd, _) = nearest_neighbour_order_psi_simd(&cells);
    println!(
        "tour, psi SIMD:      {} points, built in {:.1}s",
        psi_simd.len(),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(
        psi_simd, psi_tour,
        "the SIMD index found a different tour than the scalar one"
    );
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
        ("nearest neighbour, psi", &psi_tour),
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

#[derive(Clone, Copy, Debug)]
enum Cost {
    /// The plain length of the step.
    Linear,
    /// `sqrt` of the length: a long step is dear but not twice as dear.
    Root,
    /// `ln(1 + length)`: what a residual costs to code, roughly.
    Log,
}

impl Cost {
    #[inline]
    fn of(self, a: &[i32; 3], b: &[i32; 3]) -> f64 {
        let dx = i64::from(a[0] - b[0]);
        let dy = i64::from(a[1] - b[1]);
        let dz = i64::from(a[2] - b[2]);
        let squared = (dx * dx + dy * dy + dz * dz) as f64;
        match self {
            Cost::Linear => squared.sqrt(),
            Cost::Root => squared.sqrt().sqrt(),
            Cost::Log => (1.0 + squared.sqrt()).ln(),
        }
    }
}

/// Reverses a stretch of the path whenever that shortens it under `cost`,
/// looking no further than `window` points ahead of the stretch's start. The
/// points are copied into path order first, so the walk reads memory in order
/// instead of chasing indices into the file's own order.
fn two_opt_in_window(
    cells: &[[u32; 3]],
    order: &mut [u32],
    window: usize,
    passes: usize,
    cost: Cost,
) {
    let n = order.len();
    let mut points: Vec<[i32; 3]> = order
        .iter()
        .map(|&p| {
            let c = cells[p as usize];
            [c[0] as i32, c[1] as i32, c[2] as i32]
        })
        .collect();
    for _ in 0..passes {
        let mut improved = false;
        for i in 0..n.saturating_sub(1) {
            for j in i + 1..(i + window).min(n) {
                let b = points[i];
                let c = points[j];
                let mut old = 0.0;
                let mut new = 0.0;
                if i > 0 {
                    old += cost.of(&points[i - 1], &b);
                    new += cost.of(&points[i - 1], &c);
                }
                if j + 1 < n {
                    old += cost.of(&c, &points[j + 1]);
                    new += cost.of(&b, &points[j + 1]);
                }
                if new + 1e-9 < old {
                    points[i..=j].reverse();
                    order[i..=j].reverse();
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
}

fn scene_cloud() -> Option<(PathBuf, PointCloud)> {
    let path = std::env::var_os("DRACO_SPLAT_PLY").map(PathBuf::from)?;
    let source = std::fs::read(&path).expect("the scene reads");
    let mesh = draco_io::ply_reader::PlyReader::from_bytes(source)
        .with_generic_attributes(true)
        .read_mesh()
        .expect("the scene parses");
    Some((path, mesh.into_point_cloud()))
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn how_far_a_local_refinement_gets() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");

    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let mut arms: Vec<(String, Vec<u32>, f64)> = vec![
        ("Morton".into(), morton.clone(), 0.0),
        ("Hilbert".into(), hilbert.clone(), 0.0),
    ];
    for cost in [Cost::Linear, Cost::Root, Cost::Log] {
        for (window, passes) in [(8usize, 1usize), (16, 2), (32, 3)] {
            let mut order = hilbert.clone();
            let started = std::time::Instant::now();
            two_opt_in_window(&cells, &mut order, window, passes, cost);
            let seconds = started.elapsed().as_secs_f64();
            let mut check = order.clone();
            check.sort_unstable();
            assert!(
                check.iter().enumerate().all(|(i, &p)| p == i as u32),
                "a point was lost"
            );
            arms.push((
                format!("H + 2-opt {cost:?} w{window}x{passes}"),
                order,
                seconds,
            ));
        }
    }

    println!(
        "{:<30} {:>10} {:>9} {:>12} {:>10} {:>12} {:>10}",
        "order", "mean step", "refine s", "search off", "vs Morton", "search on", "vs Morton"
    );
    let mut reference = (0usize, 0usize);
    for (label, order, seconds) in &arms {
        let permuted = permute(&cloud, order);
        let plain = encode(&permuted, false);
        let searched = encode(&permuted, true);
        if label == "Morton" {
            reference = (plain, searched);
        }
        let against = |bytes: usize, base: usize| {
            format!("{:+.2}%", (bytes as f64 / base as f64 - 1.0) * 100.0)
        };
        println!(
            "{label:<30} {:>10.1} {:>9.2} {:>12.3} {:>10} {:>12.3} {:>10}",
            mean_step(&cells, order),
            seconds,
            plain as f64 / num_points as f64,
            against(plain, reference.0),
            searched as f64 / num_points as f64,
            against(searched, reference.1),
        );
    }
}

/// 2-opt in a window where a step costs `ln(1 + d)`, with `d` measured over the
/// position cells and the standardized attributes together: an attribute's
/// standard deviation counts as `weight` position cells.
fn two_opt_with_attributes(
    cells: &[[u32; 3]],
    attributes: &[Vec<f32>],
    weight: f32,
    order: &mut [u32],
    window: usize,
    passes: usize,
) {
    const K: usize = 16;
    let n = order.len();
    let scales: Vec<(f32, f32)> = attributes
        .iter()
        .map(|values| {
            let mean = values.iter().sum::<f32>() / values.len() as f32;
            let variance =
                values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / values.len() as f32;
            (mean, weight / variance.sqrt().max(1e-9))
        })
        .collect();
    let mut points: Vec<[f32; K]> = order
        .iter()
        .map(|&p| {
            let mut v = [0f32; K];
            for axis in 0..3 {
                v[axis] = cells[p as usize][axis] as f32;
            }
            for (k, values) in attributes.iter().enumerate().take(K - 3) {
                v[3 + k] = (values[p as usize] - scales[k].0) * scales[k].1;
            }
            v
        })
        .collect();
    let cost = |a: &[f32; K], b: &[f32; K]| -> f32 {
        let mut squared = 0f32;
        for k in 0..K {
            let d = a[k] - b[k];
            squared += d * d;
        }
        (1.0 + squared.sqrt()).ln()
    };
    for _ in 0..passes {
        let mut improved = false;
        for i in 0..n.saturating_sub(1) {
            for j in i + 1..(i + window).min(n) {
                let b = points[i];
                let c = points[j];
                let mut old = 0.0;
                let mut new = 0.0;
                if i > 0 {
                    old += cost(&points[i - 1], &b);
                    new += cost(&points[i - 1], &c);
                }
                if j + 1 < n {
                    old += cost(&c, &points[j + 1]);
                    new += cost(&b, &points[j + 1]);
                }
                if new + 1e-6 < old {
                    points[i..=j].reverse();
                    order[i..=j].reverse();
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn would_the_attributes_belong_in_the_order() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let names = attribute_names(&cloud);
    println!("attributes: {}", names.iter().flatten().count());
    let wanted: Vec<&str> = match std::env::var("DRACO_ORDER_ATTRS").as_deref() {
        Ok("colour") => vec!["f_dc_0", "f_dc_1", "f_dc_2"],
        Ok("shape") => vec!["opacity", "scale_0", "scale_1", "scale_2"],
        Ok("all13") => vec![
            "f_dc_0",
            "f_dc_1",
            "f_dc_2",
            "opacity",
            "scale_0",
            "scale_1",
            "scale_2",
            "rot_0",
            "rot_1",
            "rot_2",
            "rot_3",
            "f_rest_0",
            "f_rest_15",
        ],
        _ => vec![
            "f_dc_0", "f_dc_1", "f_dc_2", "opacity", "scale_0", "scale_1", "scale_2",
        ],
    };
    println!("attributes in the cost: {wanted:?}");
    let attributes: Vec<Vec<f32>> = wanted
        .iter()
        .map(|name| {
            let id = names
                .iter()
                .position(|n| n.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("no attribute named {name}"));
            let attribute = cloud.attribute(id as i32);
            (0..num_points)
                .map(|p| {
                    read_f32(
                        attribute,
                        attribute.mapped_index(PointIndex(p as u32)).0 as usize,
                        0,
                    )
                })
                .collect()
        })
        .collect();

    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let mut arms: Vec<(String, Vec<u32>, f64)> = vec![
        ("Morton".into(), morton, 0.0),
        ("Hilbert".into(), hilbert.clone(), 0.0),
    ];
    let weights: Vec<f32> = std::env::var("DRACO_ORDER_WEIGHTS")
        .ok()
        .map(|w| w.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_else(|| vec![0.0, 0.5, 1.0, 2.0, 4.0]);
    for weight in weights {
        let mut order = hilbert.clone();
        let started = std::time::Instant::now();
        two_opt_with_attributes(&cells, &attributes, weight, &mut order, 16, 2);
        arms.push((
            format!("H + 2-opt Log+attrs w16x2 a{weight}"),
            order,
            started.elapsed().as_secs_f64(),
        ));
    }
    println!(
        "{:<38} {:>10} {:>9} {:>12} {:>10}",
        "order", "mean step", "refine s", "search off", "vs Morton"
    );
    let mut reference = 0usize;
    for (label, order, seconds) in &arms {
        let plain = encode(&permute(&cloud, order), false);
        if label == "Morton" {
            reference = plain;
        }
        println!(
            "{label:<38} {:>10.1} {:>9.2} {:>12.3} {:>9.2}%",
            mean_step(&cells, order),
            seconds,
            plain as f64 / num_points as f64,
            (plain as f64 / reference as f64 - 1.0) * 100.0,
        );
    }
}

/// Every attribute but the position, quantized to the byte the encode uses
/// (`quantization_bits` 8 over each component's own range), one row a point.
fn byte_rows(cloud: &PointCloud) -> Vec<[u8; 64]> {
    byte_rows_mapped(cloud).0
}

/// The rows, and for each attribute of the cloud the column its first
/// component landed in. An attribute that never varies costs nothing to step
/// across and gets no column: 3DGS writes `nx ny nz` as zeros.
fn byte_rows_mapped(cloud: &PointCloud) -> (Vec<[u8; 64]>, Vec<Option<usize>>) {
    let num_points = cloud.num_points();
    let names = attribute_names(cloud);
    let mut rows = vec![[0u8; 64]; num_points];
    let mut column = 0usize;
    let mut map = vec![None; cloud.num_attributes() as usize];
    for id in 0..cloud.num_attributes() {
        let attribute = cloud.attribute(id);
        if attribute.attribute_type() == GeometryAttributeType::Position {
            continue;
        }
        let first_column = column;
        let is_float = attribute.data_type() == draco_core::DataType::Float32;
        let levels = if is_float {
            ((1u32 << budget_bits(names[id as usize].as_deref())) - 1) as f32
        } else {
            255.0
        };
        for component in 0..attribute.num_components() as usize {
            let values: Vec<f32> = (0..num_points)
                .map(|p| {
                    read_value(
                        attribute,
                        attribute.mapped_index(PointIndex(p as u32)).0 as usize,
                        component,
                    )
                })
                .collect();
            let low = values.iter().copied().fold(f32::INFINITY, f32::min);
            let high = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            if high - low < 1e-12 {
                continue;
            }
            let span = high - low;
            let (low, span) = if is_float { (low, span) } else { (0.0, 255.0) };
            for (p, v) in values.iter().enumerate() {
                rows[p][column] = (((v - low) / span) * levels + 0.5) as u8;
            }
            column += 1;
        }
        map[id as usize] = (column > first_column).then_some(first_column);
    }
    assert!(column <= 64, "{column} attribute columns do not fit a row");
    (rows, map)
}

/// 2-opt in a window where a step costs the estimated bits of the residuals it
/// leaves: `log2(1 + |delta|)` summed over the position and every attribute
/// column, each in the units the encode quantizes it to. `columns` limits how
/// many attribute columns count.
fn two_opt_by_estimated_bits(
    cells: &[[u32; 3]],
    rows: &[[u8; 64]],
    columns: usize,
    order: &mut [u32],
    window: usize,
    passes: usize,
) {
    #[derive(Clone, Copy)]
    struct Point {
        position: [i32; 3],
        row: [u8; 64],
    }
    let lut8: Vec<f32> = (0..256).map(|d| (1.0 + d as f32).log2()).collect();
    let lut16: Vec<f32> = (0..65536).map(|d| (1.0 + d as f32).log2()).collect();
    let n = order.len();
    let mut points: Vec<Point> = order
        .iter()
        .map(|&p| Point {
            position: [
                cells[p as usize][0] as i32,
                cells[p as usize][1] as i32,
                cells[p as usize][2] as i32,
            ],
            row: rows[p as usize],
        })
        .collect();
    let cost = |a: &Point, b: &Point| -> f32 {
        let mut bits = 0f32;
        for axis in 0..3 {
            bits += lut16[(a.position[axis] - b.position[axis]).unsigned_abs() as usize];
        }
        for k in 0..columns {
            bits += lut8[a.row[k].abs_diff(b.row[k]) as usize];
        }
        bits
    };
    for _ in 0..passes {
        let mut improved = false;
        for i in 0..n.saturating_sub(1) {
            for j in i + 1..(i + window).min(n) {
                let b = points[i];
                let c = points[j];
                let mut old = 0.0;
                let mut new = 0.0;
                if i > 0 {
                    old += cost(&points[i - 1], &b);
                    new += cost(&points[i - 1], &c);
                }
                if j + 1 < n {
                    old += cost(&c, &points[j + 1]);
                    new += cost(&b, &points[j + 1]);
                }
                if new + 1e-4 < old {
                    points[i..=j].reverse();
                    order[i..=j].reverse();
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn what_the_estimated_bits_objective_reaches() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let rows = byte_rows(&cloud);
    let hilbert = hilbert_order(&cells);
    let mut arms: Vec<(String, Vec<u32>, f64)> = vec![
        ("Morton".into(), morton_order(&cells), 0.0),
        ("Hilbert".into(), hilbert.clone(), 0.0),
    ];
    let configs: Vec<(usize, usize, usize)> = std::env::var("DRACO_ORDER_CONFIGS")
        .ok()
        .map(|c| {
            c.split(';')
                .filter_map(|t| {
                    let v: Vec<usize> = t.split(',').filter_map(|x| x.parse().ok()).collect();
                    (v.len() == 3).then(|| (v[0], v[1], v[2]))
                })
                .collect()
        })
        .unwrap_or_else(|| vec![(56, 16, 2)]);
    for (columns, window, passes) in configs {
        let mut order = hilbert.clone();
        let started = std::time::Instant::now();
        two_opt_by_estimated_bits(&cells, &rows, columns, &mut order, window, passes);
        arms.push((
            format!("H + 2-opt bits c{columns} w{window}x{passes}"),
            order,
            started.elapsed().as_secs_f64(),
        ));
    }
    println!(
        "{:<34} {:>10} {:>9} {:>12} {:>10}",
        "order", "mean step", "refine s", "search off", "vs Morton"
    );
    let mut reference = 0usize;
    for (label, order, seconds) in &arms {
        let plain = encode(&permute(&cloud, order), false);
        if label == "Morton" {
            reference = plain;
        }
        println!(
            "{label:<34} {:>10.1} {:>9.2} {:>12.3} {:>9.2}%",
            mean_step(&cells, order),
            seconds,
            plain as f64 / num_points as f64,
            (plain as f64 / reference as f64 - 1.0) * 100.0,
        );
    }
}

// ---------------------------------------------------------------------------
// Making the estimated-bits refinement cheap, in safe Rust.
// ---------------------------------------------------------------------------

const ROW: usize = 56;

/// One point as the refinement reads it: the attribute bytes and the 16-bit
/// position cell, in exactly one cache line.
#[derive(Clone, Copy)]
#[repr(C, align(64))]
struct Packed {
    row: [u8; ROW],
    pos: [u16; 3],
}

struct Luts {
    l8: Box<[f32; 256]>,
    l16: Box<[f32; 65536]>,
}

impl Luts {
    fn new() -> Self {
        let l8: Vec<f32> = (0..256).map(|d| (1.0 + d as f32).log2()).collect();
        let l16: Vec<f32> = (0..65536).map(|d| (1.0 + d as f32).log2()).collect();
        Luts {
            l8: l8.try_into().expect("256 entries"),
            l16: l16.try_into().expect("65536 entries"),
        }
    }
}

/// The estimated bits of the residuals between two points, or something at
/// least `limit` as soon as the running sum reaches it: every term is
/// non-negative, so a sum already over the limit cannot come back under.
/// `columns` is a multiple of eight.
#[inline(always)]
fn edge_cost(a: &Packed, b: &Packed, columns: usize, luts: &Luts, limit: f32) -> f32 {
    let mut sum = luts.l16[a.pos[0].abs_diff(b.pos[0]) as usize]
        + luts.l16[a.pos[1].abs_diff(b.pos[1]) as usize]
        + luts.l16[a.pos[2].abs_diff(b.pos[2]) as usize];
    for (xa, xb) in a.row[..columns]
        .chunks_exact(8)
        .zip(b.row[..columns].chunks_exact(8))
    {
        for k in 0..8 {
            sum += luts.l8[xa[k].abs_diff(xb[k]) as usize];
        }
        if sum >= limit {
            return sum;
        }
    }
    sum
}

#[derive(Clone, Copy)]
struct Tricks {
    /// Give up on a candidate as soon as it cannot beat what it replaces.
    prune: bool,
    /// Revisit only the starts a reversal could have changed.
    dirty: bool,
}

/// 2-opt over one block of the path with its two end points held fixed, so
/// blocks are independent of one another and the edges between them never
/// change. Step costs are cached: reversing a stretch leaves the costs inside
/// it as they were, in reverse order.
fn refine_block(
    points: &mut [Packed],
    ids: &mut [u32],
    window: usize,
    passes: usize,
    columns: usize,
    luts: &Luts,
    tricks: Tricks,
) {
    let n = points.len();
    if n < 4 {
        return;
    }
    let mut edges: Vec<f32> = (0..n - 1)
        .map(|i| edge_cost(&points[i], &points[i + 1], columns, luts, f32::INFINITY))
        .collect();
    let mut dirty = vec![true; n];
    let mut next = vec![false; n];
    for _ in 0..passes {
        let mut moved = false;
        for i in 1..n - 1 {
            if tricks.dirty && !dirty[i] {
                continue;
            }
            for j in i + 1..(i + window).min(n - 1) {
                let old = edges[i - 1] + edges[j];
                let (limit_a, limit_b) = if tricks.prune {
                    (old, f32::INFINITY)
                } else {
                    (f32::INFINITY, f32::INFINITY)
                };
                let ac = edge_cost(&points[i - 1], &points[j], columns, luts, limit_a);
                if tricks.prune && ac >= old {
                    continue;
                }
                let limit_b = if tricks.prune { old - ac } else { limit_b };
                let bd = edge_cost(&points[i], &points[j + 1], columns, luts, limit_b);
                if ac + bd + 1e-4 < old {
                    points[i..=j].reverse();
                    ids[i..=j].reverse();
                    edges[i..j].reverse();
                    edges[i - 1] = ac;
                    edges[j] = bd;
                    moved = true;
                    let low = i.saturating_sub(window + 2);
                    let high = (j + 2).min(n - 1);
                    next[low..=high].fill(true);
                }
            }
        }
        std::mem::swap(&mut dirty, &mut next);
        next.fill(false);
        if !moved {
            break;
        }
    }
}

/// The columns ordered by how many bits their residuals cost along `order`,
/// dearest first: the ones worth counting come first, and the early exit in
/// `edge_cost` meets its limit sooner.
fn columns_by_cost(rows: &[[u8; 64]], order: &[u32], luts: &Luts) -> Vec<usize> {
    let mut totals = [0f64; ROW];
    for pair in order.windows(2) {
        let a = &rows[pair[0] as usize];
        let b = &rows[pair[1] as usize];
        for k in 0..ROW {
            totals[k] += f64::from(luts.l8[a[k].abs_diff(b[k]) as usize]);
        }
    }
    let mut columns: Vec<usize> = (0..ROW).collect();
    columns.sort_by(|&x, &y| totals[y].partial_cmp(&totals[x]).unwrap());
    columns
}

fn pack(
    cells: &[[u32; 3]],
    rows: &[[u8; 64]],
    column_order: &[usize],
    order: &[u32],
) -> Vec<Packed> {
    order
        .iter()
        .map(|&p| {
            let mut row = [0u8; ROW];
            for (slot, &column) in row.iter_mut().zip(column_order) {
                *slot = rows[p as usize][column];
            }
            let c = cells[p as usize];
            Packed {
                row,
                pos: [c[0] as u16, c[1] as u16, c[2] as u16],
            }
        })
        .collect()
}

/// The objective the refinement minimizes, over the whole path, counting only
/// the first `columns` of `row`.
fn path_bits(points: &[Packed], columns: usize, luts: &Luts) -> f64 {
    points
        .windows(2)
        .map(|w| f64::from(edge_cost(&w[0], &w[1], columns, luts, f32::INFINITY)))
        .sum()
}

fn refine_blocks(
    points: &mut [Packed],
    ids: &mut [u32],
    block: usize,
    threads: usize,
    window: usize,
    passes: usize,
    columns: usize,
    luts: &Luts,
    tricks: Tricks,
) {
    if threads <= 1 {
        for (p, i) in points.chunks_mut(block).zip(ids.chunks_mut(block)) {
            refine_block(p, i, window, passes, columns, luts, tricks);
        }
        return;
    }
    let work = std::sync::Mutex::new(points.chunks_mut(block).zip(ids.chunks_mut(block)));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let Some((p, i)) = work.lock().unwrap().next() else {
                    break;
                };
                refine_block(p, i, window, passes, columns, luts, tricks);
            });
        }
    });
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn how_cheap_the_refinement_gets() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let rows = byte_rows(&cloud);
    let hilbert = hilbert_order(&cells);
    let luts = Luts::new();
    let by_cost = columns_by_cost(&rows, &hilbert, &luts);
    let first: Vec<usize> = (0..ROW).collect();
    let window = 16;
    let passes = 2;
    let both = Tricks {
        prune: true,
        dirty: true,
    };

    println!(
        "{:<44} {:>8} {:>12} {:>9}",
        "variant", "time s", "bits/point", "vs start"
    );
    let report = |label: &str, seconds: f64, points: &[Packed], columns: usize, start: f64| {
        let bits = path_bits(points, columns, &luts);
        println!(
            "{label:<44} {seconds:>8.3} {:>12.3} {:>8.2}%",
            bits / num_points as f64,
            (bits / start - 1.0) * 100.0
        );
    };

    for (columns, column_order, label) in [
        (56usize, &by_cost, "56 cols"),
        (24, &by_cost, "top 24 by cost"),
        (16, &by_cost, "top 16 by cost"),
        (12, &by_cost, "top 12 by cost"),
        (12, &first, "first 12 (the earlier probe)"),
    ] {
        let start_points = pack(&cells, &rows, column_order, &hilbert);
        let start = path_bits(&start_points, columns, &luts);
        println!(
            "-- {label}: Hilbert path is {:.3} bits/point",
            start / num_points as f64
        );

        // The earlier implementation, for the baseline in the same units.
        let mut order = hilbert.clone();
        let mut sub_rows = rows.clone();
        for (r, source) in sub_rows.iter_mut().zip(&rows) {
            for (slot, &column) in r.iter_mut().zip(column_order.iter()) {
                *slot = source[column];
            }
        }
        let t = std::time::Instant::now();
        two_opt_by_estimated_bits(&cells, &sub_rows, columns, &mut order, window, passes);
        let seconds = t.elapsed().as_secs_f64();
        report(
            "  naive (previous implementation)",
            seconds,
            &pack(&cells, &rows, column_order, &order),
            columns,
            start,
        );

        let mut reference_ids: Option<Vec<u32>> = None;
        for (name, tricks, block, threads) in [
            (
                "  cached edges, whole path",
                Tricks {
                    prune: false,
                    dirty: false,
                },
                n_all(num_points),
                1usize,
            ),
            (
                "  + early exit",
                Tricks {
                    prune: true,
                    dirty: false,
                },
                n_all(num_points),
                1,
            ),
            ("  + dirty starts", both, n_all(num_points), 1),
            ("  + blocks of 8192", both, 8192, 1),
            ("  + 4 threads", both, 8192, 4),
            ("  + 16 threads", both, 8192, 16),
        ] {
            let mut points = start_points.clone();
            let mut ids = hilbert.clone();
            let t = std::time::Instant::now();
            refine_blocks(
                &mut points,
                &mut ids,
                block,
                threads,
                window,
                passes,
                columns,
                &luts,
                tricks,
            );
            let seconds = t.elapsed().as_secs_f64();
            report(name, seconds, &points, columns, start);
            if block == 8192 {
                match &reference_ids {
                    None => reference_ids = Some(ids),
                    Some(r) => assert_eq!(r, &ids, "threads changed the result"),
                }
            }
        }
    }
}

fn n_all(n: usize) -> usize {
    n
}

// -- Cost models for the speed stand ---------------------------------------

/// Which way a step is priced. All five estimate the same thing, the bits of a
/// residual, and the quality column of the stand says what the cheaper ones
/// give up.
#[derive(Clone, Copy, Debug)]
enum Model {
    /// `log2(1 + d)` from a float table, one lookup a column.
    Lut,
    /// The same table, but the byte differences are taken in a separate loop
    /// the compiler can vectorize before the lookups.
    DiffLut,
    /// A table of `round(16 * log2(1 + d))` as `u16`: integer sums.
    Fixed,
    /// The length of `d` in bits, from eight comparisons a column, no table.
    BitLength,
    /// Two steps to the bit, from fifteen comparisons a column, no table.
    HalfBit,
    /// `Lut` in chunks of eight columns, the inner loop fixed so it unrolls.
    Chunk8Lut,
    /// Chunks of eight, the differences taken first, then looked up.
    Chunk8Diff,
    /// Chunks of eight, integer table, four partial sums.
    Chunk8Fixed,
}

impl Model {
    const ALL: [Model; 9] = [
        Model::Lut,
        Model::DiffLut,
        Model::Fixed,
        Model::BitLength,
        Model::HalfBit,
        Model::Chunk8Lut,
        Model::Chunk8Diff,
        Model::Chunk8Fixed,
        Model::Lut,
    ];
    const fn from_u8(m: u8) -> Model {
        Model::ALL[m as usize]
    }
}

const HALF_BIT_STEPS: [u8; 15] = [1, 2, 3, 4, 6, 8, 11, 16, 23, 32, 45, 64, 91, 128, 181];

struct Tables {
    luts: Luts,
    fixed: Box<[u16; 256]>,
}

impl Tables {
    fn new() -> Self {
        let fixed: Vec<u16> = (0..256)
            .map(|d| (16.0 * (1.0 + d as f32).log2()).round() as u16)
            .collect();
        Tables {
            luts: Luts::new(),
            fixed: fixed.try_into().expect("256 entries"),
        }
    }
}

#[inline(always)]
fn priced<const M: u8>(a: &Packed, b: &Packed, columns: usize, t: &Tables) -> f32 {
    let model = Model::from_u8(M);
    let position = t.luts.l16[a.pos[0].abs_diff(b.pos[0]) as usize]
        + t.luts.l16[a.pos[1].abs_diff(b.pos[1]) as usize]
        + t.luts.l16[a.pos[2].abs_diff(b.pos[2]) as usize];
    let ra = &a.row[..columns];
    let rb = &b.row[..columns];
    match model {
        Model::Lut => {
            let mut sum = 0f32;
            for (x, y) in ra.iter().zip(rb) {
                sum += t.luts.l8[x.abs_diff(*y) as usize];
            }
            position + sum
        }
        Model::DiffLut => {
            let mut diff = [0u8; ROW];
            for k in 0..ROW {
                diff[k] = a.row[k].abs_diff(b.row[k]);
            }
            let mut sum = 0f32;
            for &d in &diff[..columns] {
                sum += t.luts.l8[d as usize];
            }
            position + sum
        }
        Model::Fixed => {
            let mut diff = [0u8; ROW];
            for k in 0..ROW {
                diff[k] = a.row[k].abs_diff(b.row[k]);
            }
            let mut sum = 0u32;
            for &d in &diff[..columns] {
                sum += u32::from(t.fixed[d as usize]);
            }
            position + sum as f32 * (1.0 / 16.0)
        }
        Model::BitLength => {
            let mut bits = [0u8; ROW];
            for k in 0..ROW {
                let d = a.row[k].abs_diff(b.row[k]);
                bits[k] = u8::from(d >= 1)
                    + u8::from(d >= 2)
                    + u8::from(d >= 4)
                    + u8::from(d >= 8)
                    + u8::from(d >= 16)
                    + u8::from(d >= 32)
                    + u8::from(d >= 64)
                    + u8::from(d >= 128);
            }
            let sum: u32 = bits[..columns].iter().map(|&x| u32::from(x)).sum();
            position + sum as f32
        }
        Model::Chunk8Lut => {
            let mut sum = 0f32;
            for (xa, xb) in ra.chunks_exact(8).zip(rb.chunks_exact(8)) {
                for k in 0..8 {
                    sum += t.luts.l8[xa[k].abs_diff(xb[k]) as usize];
                }
            }
            position + sum
        }
        Model::Chunk8Diff => {
            let mut sum = 0f32;
            for (xa, xb) in ra.chunks_exact(8).zip(rb.chunks_exact(8)) {
                let mut d = [0u8; 8];
                for k in 0..8 {
                    d[k] = xa[k].abs_diff(xb[k]);
                }
                sum += t.luts.l8[d[0] as usize]
                    + t.luts.l8[d[1] as usize]
                    + t.luts.l8[d[2] as usize]
                    + t.luts.l8[d[3] as usize]
                    + t.luts.l8[d[4] as usize]
                    + t.luts.l8[d[5] as usize]
                    + t.luts.l8[d[6] as usize]
                    + t.luts.l8[d[7] as usize];
            }
            position + sum
        }
        Model::Chunk8Fixed => {
            let mut sum = 0u32;
            for (xa, xb) in ra.chunks_exact(8).zip(rb.chunks_exact(8)) {
                let mut d = [0u8; 8];
                for k in 0..8 {
                    d[k] = xa[k].abs_diff(xb[k]);
                }
                let f = |i: usize| u32::from(t.fixed[d[i] as usize]);
                sum += (f(0) + f(1)) + (f(2) + f(3)) + ((f(4) + f(5)) + (f(6) + f(7)));
            }
            position + sum as f32 * (1.0 / 16.0)
        }
        Model::HalfBit => {
            let mut halves = [0u8; ROW];
            for k in 0..ROW {
                let d = a.row[k].abs_diff(b.row[k]);
                let mut h = 0u8;
                for step in HALF_BIT_STEPS {
                    h += u8::from(d >= step);
                }
                halves[k] = h;
            }
            let sum: u32 = halves[..columns].iter().map(|&x| u32::from(x)).sum();
            position + sum as f32 * 0.5
        }
    }
}

fn refine_modelled<const M: u8>(
    points: &mut [Packed],
    ids: &mut [u32],
    window: usize,
    passes: usize,
    columns: usize,
    t: &Tables,
) {
    let n = points.len();
    let mut edges: Vec<f32> = (0..n - 1)
        .map(|i| priced::<M>(&points[i], &points[i + 1], columns, t))
        .collect();
    for _ in 0..passes {
        let mut moved = false;
        for i in 1..n - 1 {
            for j in i + 1..(i + window).min(n - 1) {
                let old = edges[i - 1] + edges[j];
                let ac = priced::<M>(&points[i - 1], &points[j], columns, t);
                let bd = priced::<M>(&points[i], &points[j + 1], columns, t);
                if ac + bd + 1e-4 < old {
                    points[i..=j].reverse();
                    ids[i..=j].reverse();
                    edges[i..j].reverse();
                    edges[i - 1] = ac;
                    edges[j] = bd;
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn which_cost_model_is_cheap_and_good() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let rows = byte_rows(&cloud);
    let hilbert = hilbert_order(&cells);
    let tables = Tables::new();
    let by_cost = columns_by_cost(&rows, &hilbert, &tables.luts);
    let start_points = pack(&cells, &rows, &by_cost, &hilbert);

    let columns_list: Vec<usize> = std::env::var("DRACO_ORDER_COLUMNS")
        .ok()
        .map(|c| c.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_else(|| vec![56, 24, 16]);
    println!(
        "{:<26} {:>8} {:>16} {:>12}",
        "model", "time s", "exact bits/point", "vs Hilbert"
    );
    for columns in columns_list {
        let start = path_bits(&start_points, columns, &tables.luts);
        println!(
            "-- {columns} columns: Hilbert is {:.3} bits/point",
            start / num_points as f64
        );
        let run = |m: u8| -> (f64, f64) {
            let mut best = f64::MAX;
            let mut bits = 0.0;
            for _ in 0..3 {
                let mut points = start_points.clone();
                let mut ids = hilbert.clone();
                let t = std::time::Instant::now();
                match m {
                    0 => refine_modelled::<0>(&mut points, &mut ids, 16, 2, columns, &tables),
                    1 => refine_modelled::<1>(&mut points, &mut ids, 16, 2, columns, &tables),
                    2 => refine_modelled::<2>(&mut points, &mut ids, 16, 2, columns, &tables),
                    5 => refine_modelled::<5>(&mut points, &mut ids, 16, 2, columns, &tables),
                    6 => refine_modelled::<6>(&mut points, &mut ids, 16, 2, columns, &tables),
                    7 => refine_modelled::<7>(&mut points, &mut ids, 16, 2, columns, &tables),
                    _ => unreachable!(),
                }
                best = best.min(t.elapsed().as_secs_f64());
                bits = path_bits(&points, columns, &tables.luts);
            }
            (best, bits)
        };
        for m in [0u8, 1, 2, 5, 6, 7] {
            let (seconds, bits) = run(m);
            println!(
                "{:<26} {seconds:>8.3} {:>16.3} {:>11.2}%",
                format!("{:?}", Model::from_u8(m)),
                bits / num_points as f64,
                (bits / start - 1.0) * 100.0
            );
        }
    }
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn what_the_columns_and_the_window_buy() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let rows = byte_rows(&cloud);
    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let tables = Tables::new();
    let by_cost = columns_by_cost(&rows, &hilbert, &tables.luts);
    let start_points = pack(&cells, &rows, &by_cost, &hilbert);
    let morton_bytes = encode(&permute(&cloud, &morton), false);
    let hilbert_bytes = encode(&permute(&cloud, &hilbert), false);
    let vs = |bytes: usize| {
        format!(
            "{:+.2}%",
            (bytes as f64 / morton_bytes as f64 - 1.0) * 100.0
        )
    };
    println!(
        "Morton {:.3} B/point, Hilbert {:.3} ({})",
        morton_bytes as f64 / num_points as f64,
        hilbert_bytes as f64 / num_points as f64,
        vs(hilbert_bytes)
    );
    println!(
        "{:>8} {:>8} {:>9} {:>11} {:>10}",
        "columns", "window", "passes", "time s (1)", "vs Morton"
    );
    for columns in [8usize, 16, 24, 56] {
        for (window, passes) in [(8usize, 1usize), (16, 1), (16, 2), (32, 2)] {
            let mut points = start_points.clone();
            let mut ids = hilbert.clone();
            let t = std::time::Instant::now();
            refine_modelled::<2>(&mut points, &mut ids, window, passes, columns, &tables);
            let seconds = t.elapsed().as_secs_f64();
            let bytes = encode(&permute(&cloud, &ids), false);
            println!(
                "{columns:>8} {window:>8} {passes:>9} {seconds:>11.3} {:>10}",
                vs(bytes)
            );
        }
    }
}

/// Moves a stretch of one to `max_len` points to wherever inside `window`
/// points of it that shortens the path most, order inside the stretch kept.
/// 2-opt reverses stretches and cannot do this.
fn or_opt_pass<const M: u8>(
    points: &mut [Packed],
    ids: &mut [u32],
    edges: &mut [f32],
    window: usize,
    max_len: usize,
    columns: usize,
    t: &Tables,
) -> bool {
    let n = points.len();
    let mut moved = false;
    let mut i = 1;
    while i + 1 < n {
        let mut best_gain = 1e-4f32;
        let mut best: Option<(usize, usize, usize)> = None; // (len, gap k, ...)
        for len in 1..=max_len {
            if i + len >= n {
                break;
            }
            let last = i + len - 1;
            let bridge = priced::<M>(&points[i - 1], &points[i + len], columns, t);
            let removed = edges[i - 1] + edges[last] - bridge;
            // Gaps after the stretch: between k and k + 1, k from i + len.
            for k in i + len..(i + len + window).min(n - 1) {
                let add = priced::<M>(&points[k], &points[i], columns, t)
                    + priced::<M>(&points[last], &points[k + 1], columns, t)
                    - edges[k];
                let gain = removed - add;
                if gain > best_gain {
                    best_gain = gain;
                    best = Some((len, k, 0));
                }
            }
            // Gaps before it: between k and k + 1, k + 1 <= i - 1.
            for k in i.saturating_sub(window).max(1)..i - 1 {
                let add = priced::<M>(&points[k], &points[i], columns, t)
                    + priced::<M>(&points[last], &points[k + 1], columns, t)
                    - edges[k];
                let gain = removed - add;
                if gain > best_gain {
                    best_gain = gain;
                    best = Some((len, k, 1));
                }
            }
        }
        if let Some((len, k, backward)) = best {
            let (lo, hi) = if backward == 0 {
                points[i..=k].rotate_left(len);
                ids[i..=k].rotate_left(len);
                (i - 1, k)
            } else {
                points[k + 1..i + len].rotate_right(len);
                ids[k + 1..i + len].rotate_right(len);
                (k, i + len)
            };
            for e in lo..=hi.min(n - 2) {
                edges[e] = priced::<M>(&points[e], &points[e + 1], columns, t);
            }
            moved = true;
        }
        i += 1;
    }
    moved
}

fn edges_of<const M: u8>(points: &[Packed], columns: usize, t: &Tables) -> Vec<f32> {
    (0..points.len() - 1)
        .map(|i| priced::<M>(&points[i], &points[i + 1], columns, t))
        .collect()
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn does_moving_stretches_beat_a_wider_window() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let rows = byte_rows(&cloud);
    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let tables = Tables::new();
    let by_cost = columns_by_cost(&rows, &hilbert, &tables.luts);
    let start_points = pack(&cells, &rows, &by_cost, &hilbert);
    let morton_bytes = encode(&permute(&cloud, &morton), false);
    let vs = |bytes: usize| {
        format!(
            "{:+.2}%",
            (bytes as f64 / morton_bytes as f64 - 1.0) * 100.0
        )
    };
    let columns = 16usize;
    println!("{columns} columns");
    println!("{:<34} {:>9} {:>10}", "arm", "time s", "vs Morton");

    type Arm = (&'static str, usize, usize, usize, usize, usize); // two-opt window, passes, or window, or len, or passes
    let arms: [Arm; 8] = [
        ("2-opt w8x1", 8, 1, 0, 0, 0),
        ("2-opt w16x1", 16, 1, 0, 0, 0),
        ("or-opt w8 len1", 0, 0, 8, 1, 1),
        ("or-opt w8 len3", 0, 0, 8, 3, 1),
        ("or-opt w16 len3", 0, 0, 16, 3, 1),
        ("2-opt w8x1 + or-opt w8 len3", 8, 1, 8, 3, 1),
        ("2-opt w16x1 + or-opt w8 len3", 16, 1, 8, 3, 1),
        ("2-opt w16x2 + or-opt w16 len3 x2", 16, 2, 16, 3, 2),
    ];
    for (label, tw, tp, ow, ol, op) in arms {
        let mut points = start_points.clone();
        let mut ids = hilbert.clone();
        let t = std::time::Instant::now();
        if tw > 0 {
            refine_modelled::<2>(&mut points, &mut ids, tw, tp, columns, &tables);
        }
        if ow > 0 {
            let mut edges = edges_of::<2>(&points, columns, &tables);
            for _ in 0..op {
                if !or_opt_pass::<2>(&mut points, &mut ids, &mut edges, ow, ol, columns, &tables) {
                    break;
                }
            }
        }
        let seconds = t.elapsed().as_secs_f64();
        let mut check = ids.clone();
        check.sort_unstable();
        assert!(
            check.iter().enumerate().all(|(i, &p)| p == i as u32),
            "a point was lost"
        );
        let bytes = encode(&permute(&cloud, &ids), false);
        println!("{label:<34} {seconds:>9.3} {:>10}", vs(bytes));
    }
}

/// Candidate costs for one start, all sixteen at once: one lane a candidate,
/// one pass a column. `columns[k]` is that column across the whole path, so the
/// sixteen candidates of a column are sixteen bytes in a row and the loads are
/// plain loads.
#[inline(never)]
fn lane_bits(columns: &[Vec<u8>], count: usize, anchor: usize, first: usize) -> [u16; 16] {
    let mut total = [0u16; 16];
    for column in &columns[..count] {
        let a = column[anchor];
        let window: &[u8; 16] = column[first..first + 16].try_into().unwrap();
        let mut bits = [0u8; 16];
        for lane in 0..16 {
            let d = a.abs_diff(window[lane]);
            bits[lane] = u8::from(d >= 1)
                + u8::from(d >= 2)
                + u8::from(d >= 4)
                + u8::from(d >= 8)
                + u8::from(d >= 16)
                + u8::from(d >= 32)
                + u8::from(d >= 64)
                + u8::from(d >= 128);
        }
        for lane in 0..16 {
            total[lane] += u16::from(bits[lane]);
        }
    }
    total
}

/// The same sum by a different formulation of the bit length: a float's
/// exponent field is `floor(log2(x))` for free.
#[inline(never)]
fn lane_bits_float(columns: &[Vec<u8>], count: usize, anchor: usize, first: usize) -> [u16; 16] {
    let mut total = [0u16; 16];
    for column in &columns[..count] {
        let a = column[anchor];
        let window: &[u8; 16] = column[first..first + 16].try_into().unwrap();
        for lane in 0..16 {
            let d = u32::from(a.abs_diff(window[lane]));
            // 1 + d in 1..=256: the exponent of its float is floor(log2(1 + d)).
            total[lane] += ((((1 + d) as f32).to_bits() >> 23) - 127) as u16;
        }
    }
    total
}

#[inline(never)]
fn lane_bits_leading(columns: &[Vec<u8>], count: usize, anchor: usize, first: usize) -> [u16; 16] {
    let mut total = [0u16; 16];
    for column in &columns[..count] {
        let a = column[anchor];
        let window: &[u8; 16] = column[first..first + 16].try_into().unwrap();
        for lane in 0..16 {
            let d = a.abs_diff(window[lane]);
            total[lane] += (8 - d.leading_zeros()) as u16;
        }
    }
    total
}

#[test]
#[ignore = "a timing, run with --release --ignored --nocapture"]
fn does_the_lane_form_vectorize() {
    let n = 1_000_000usize;
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let columns: Vec<Vec<u8>> = (0..56)
        .map(|_| (0..n).map(|_| (next() >> 40) as u8).collect())
        .collect();
    for count in [56usize, 16] {
        for (name, f) in [
            (
                "compare chain",
                lane_bits as fn(&[Vec<u8>], usize, usize, usize) -> [u16; 16],
            ),
            ("float exponent", lane_bits_float),
            ("leading_zeros", lane_bits_leading),
        ] {
            let mut best = f64::MAX;
            let mut check = 0u64;
            for _ in 0..5 {
                let t = std::time::Instant::now();
                for i in 1..n - 20 {
                    let total = f(&columns, count, i - 1, i + 1);
                    check += u64::from(total[(i & 15) as usize]);
                }
                best = best.min(t.elapsed().as_secs_f64());
            }
            println!(
                "{count} columns, {name:<15} {:.1} ns per start of 16 candidates = {:.2} ns a candidate (check {check})",
                best / n as f64 * 1e9,
                best / n as f64 * 1e9 / 16.0
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The refinement in lane form: columns kept apart, sixteen candidates a step.
// ---------------------------------------------------------------------------

/// `log2(1 + d)` in quarters of a bit, rounded down: the exponent and the two
/// top mantissa bits of the float `1 + d`, which is the whole computation.
#[inline(always)]
fn quarter_bits(d: u32) -> u16 {
    (((1 + d) as f32).to_bits() >> 21) as u16 - 508
}

/// The path with each column stored on its own, so the candidates of one
/// column are consecutive bytes.
struct Lanes {
    n: usize,
    cols: Vec<Vec<u8>>,
    pos: [Vec<u16>; 3],
    ids: Vec<u32>,
    /// `edges[i]` is the cost of the step from point `i` to point `i + 1`.
    edges: Vec<u16>,
    evals: u64,
    moves: u64,
    reversed: u64,
    skipped: u64,
    weights: Vec<u16>,
    pos_weight: u16,
}

/// How far past the path the columns are padded, so a window of any lane count
/// can be loaded as one slice wherever it starts.
const LANE_PAD: usize = 140;

/// The cost from `anchor` to each of the `L` points starting at `first`.
#[inline(always)]
fn lane_costs<const L: usize>(
    cols: &[Vec<u8>],
    pos: &[Vec<u16>; 3],
    anchor: usize,
    first: usize,
) -> [u16; L] {
    let mut total = [0u16; L];
    for axis in pos {
        let a = axis[anchor];
        let w: &[u16; L] = axis[first..first + L].try_into().unwrap();
        for l in 0..L {
            total[l] += quarter_bits(u32::from(a.abs_diff(w[l])));
        }
    }
    for column in cols {
        let a = column[anchor];
        let w: &[u8; L] = column[first..first + L].try_into().unwrap();
        for l in 0..L {
            total[l] += quarter_bits(u32::from(a.abs_diff(w[l])));
        }
    }
    total
}

/// Reverses `buf[i..i + m]` for `m` in `2..=16` as one 128-bit byte swap, a
/// shift and a blend. The column is padded, so sixteen bytes from `i` exist.
#[inline(always)]
fn reverse_short(buf: &mut [u8], i: usize, m: usize) {
    let chunk: &mut [u8; 16] = (&mut buf[i..i + 16]).try_into().unwrap();
    let x = u128::from_le_bytes(*chunk);
    let reversed = x.swap_bytes() >> (8 * (16 - m));
    let mask = if m == 16 {
        u128::MAX
    } else {
        (1u128 << (8 * m)) - 1
    };
    *chunk = ((x & !mask) | (reversed & mask)).to_le_bytes();
}

/// Adds the cost of `cols` from `anchor` to each of the `L` points at `first`.
#[inline(always)]
fn lane_add_columns<const L: usize>(
    cols: &[Vec<u8>],
    weights: &[u16],
    anchor: usize,
    first: usize,
    total: &mut [u16; L],
) {
    for (column, &weight) in cols.iter().zip(weights) {
        let a = column[anchor];
        let w: &[u8; L] = column[first..first + L].try_into().unwrap();
        for l in 0..L {
            total[l] += weight * quarter_bits(u32::from(a.abs_diff(w[l])));
        }
    }
}

/// The cost from `anchor` to each of the `L` points at `first` over the
/// position alone.
#[inline(always)]
fn lane_positions<const L: usize>(
    pos: &[Vec<u16>; 3],
    weight: u16,
    anchor: usize,
    first: usize,
) -> [u16; L] {
    let mut total = [0u16; L];
    for axis in pos {
        let a = axis[anchor];
        let w: &[u16; L] = axis[first..first + L].try_into().unwrap();
        for l in 0..L {
            total[l] += weight * quarter_bits(u32::from(a.abs_diff(w[l])));
        }
    }
    total
}

impl Lanes {
    fn new(points: &[Packed], ids: &[u32], columns: usize) -> Self {
        Self::weighted(points, ids, columns, &vec![1; columns], 1)
    }

    fn weighted(
        points: &[Packed],
        ids: &[u32],
        columns: usize,
        weights: &[u16],
        pos_weight: u16,
    ) -> Self {
        let n = points.len();
        let cols = (0..columns)
            .map(|k| {
                let mut v: Vec<u8> = points.iter().map(|p| p.row[k]).collect();
                v.resize(n + LANE_PAD, 0);
                v
            })
            .collect();
        let pos = std::array::from_fn(|axis| {
            let mut v: Vec<u16> = points.iter().map(|p| p.pos[axis]).collect();
            v.resize(n + LANE_PAD, 0);
            v
        });
        let mut lanes = Lanes {
            n,
            cols,
            pos,
            ids: ids.to_vec(),
            edges: vec![0; n + LANE_PAD],
            evals: 0,
            moves: 0,
            reversed: 0,
            skipped: 0,
            weights: weights[..columns].to_vec(),
            pos_weight,
        };
        for i in 0..n - 1 {
            lanes.edges[i] = lanes.edge(i, i + 1);
        }
        lanes
    }

    fn edge(&self, a: usize, b: usize) -> u16 {
        let mut sum = 0u16;
        for axis in &self.pos {
            sum += self.pos_weight * quarter_bits(u32::from(axis[a].abs_diff(axis[b])));
        }
        for (column, &weight) in self.cols.iter().zip(&self.weights) {
            sum += weight * quarter_bits(u32::from(column[a].abs_diff(column[b])));
        }
        sum
    }

    /// 2-opt with `L` candidate ends a start, taking the best of them and
    /// looking again until none improves. Both end points stay fixed.
    fn refine<const L: usize>(&mut self, passes: usize, threshold: i32, stage: usize) {
        let n = self.n;
        let stage = stage.min(self.cols.len());
        if n < 4 {
            return;
        }
        for _ in 0..passes {
            let mut moved = false;
            for i in 1..n - 1 {
                loop {
                    let last = (i + L).min(n - 2);
                    if i + 1 > last {
                        break;
                    }
                    self.evals += 1;
                    let mut ac = lane_positions::<L>(&self.pos, self.pos_weight, i - 1, i + 1);
                    let mut bd = lane_positions::<L>(&self.pos, self.pos_weight, i, i + 2);
                    let old: &[u16; L] = self.edges[i + 1..i + 1 + L].try_into().unwrap();
                    let before = self.edges[i - 1];
                    if stage < self.cols.len() {
                        lane_add_columns::<L>(
                            &self.cols[..stage],
                            &self.weights[..stage],
                            i - 1,
                            i + 1,
                            &mut ac,
                        );
                        lane_add_columns::<L>(
                            &self.cols[..stage],
                            &self.weights[..stage],
                            i,
                            i + 2,
                            &mut bd,
                        );
                        // What is counted so far is at most what the full cost
                        // is, so this is an upper bound on each lane's gain.
                        let mut any = false;
                        for l in 0..L {
                            let bound = i32::from(before) + i32::from(old[l])
                                - i32::from(ac[l])
                                - i32::from(bd[l]);
                            any |= bound > threshold && i + 1 + l <= last;
                        }
                        if !any {
                            self.skipped += 1;
                            break;
                        }
                        lane_add_columns::<L>(
                            &self.cols[stage..],
                            &self.weights[stage..],
                            i - 1,
                            i + 1,
                            &mut ac,
                        );
                        lane_add_columns::<L>(
                            &self.cols[stage..],
                            &self.weights[stage..],
                            i,
                            i + 2,
                            &mut bd,
                        );
                    } else {
                        lane_add_columns::<L>(&self.cols, &self.weights, i - 1, i + 1, &mut ac);
                        lane_add_columns::<L>(&self.cols, &self.weights, i, i + 2, &mut bd);
                    }
                    let mut best_gain = threshold;
                    let mut best_lane = usize::MAX;
                    for l in 0..L {
                        let j = i + 1 + l;
                        let gain = i32::from(before) + i32::from(old[l])
                            - i32::from(ac[l])
                            - i32::from(bd[l]);
                        if j <= last && gain > best_gain {
                            best_gain = gain;
                            best_lane = l;
                        }
                    }
                    if best_lane == usize::MAX {
                        break;
                    }
                    let j = i + 1 + best_lane;
                    self.moves += 1;
                    self.reversed += (j - i + 1) as u64;
                    let m = j - i + 1;
                    if m <= 16 {
                        for column in &mut self.cols {
                            reverse_short(column, i, m);
                        }
                    } else {
                        for column in &mut self.cols {
                            column[i..=j].reverse();
                        }
                    }
                    for axis in &mut self.pos {
                        axis[i..=j].reverse();
                    }
                    self.ids[i..=j].reverse();
                    self.edges[i..j].reverse();
                    self.edges[i - 1] = ac[best_lane];
                    self.edges[j] = bd[best_lane];
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
    }
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn how_fast_the_lane_form_is() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let rows = byte_rows(&cloud);
    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let start_order: Vec<u32> = if std::env::var("DRACO_ORDER_START").as_deref() == Ok("file") {
        (0..num_points as u32).collect()
    } else {
        hilbert.clone()
    };
    println!(
        "refinement starts from: {}",
        if std::env::var("DRACO_ORDER_START").as_deref() == Ok("file") {
            "file order"
        } else {
            "Hilbert order"
        }
    );
    let tables = Tables::new();
    let by_cost = columns_by_cost(&rows, &start_order, &tables.luts);
    let start_points = pack(&cells, &rows, &by_cost, &start_order);
    let morton_bytes = encode(&permute(&cloud, &morton), false);
    let vs = |bytes: usize| {
        format!(
            "{:+.2}%",
            (bytes as f64 / morton_bytes as f64 - 1.0) * 100.0
        )
    };
    let file_bytes = encode(&cloud, false);
    let hilbert_bytes = encode(&permute(&cloud, &hilbert), false);
    println!(
        "{} attribute columns besides the position; B/point: file order {:.3} ({}), Morton {:.3}, Hilbert {:.3} ({})",
        by_cost.iter().take(ROW).filter(|&&c| rows.iter().take(1000).any(|r| r[c] != 0)).count(),
        file_bytes as f64 / num_points as f64,
        vs(file_bytes),
        morton_bytes as f64 / num_points as f64,
        hilbert_bytes as f64 / num_points as f64,
        vs(hilbert_bytes)
    );
    println!(
        "{:>7} {:>6} {:>6} {:>9} {:>14} {:>10}",
        "columns", "lanes", "passes", "time s", "exact bits/pt", "vs Morton"
    );
    let lane_configs: Vec<(usize, usize)> = std::env::var("DRACO_ORDER_LANES")
        .ok()
        .map(|c| {
            c.split(';')
                .filter_map(|t| {
                    let v: Vec<usize> = t.split(',').filter_map(|x| x.parse().ok()).collect();
                    (v.len() == 2).then(|| (v[0], v[1]))
                })
                .collect()
        })
        .unwrap_or_else(|| vec![(16, 2), (32, 2)]);
    let threshold: i32 = std::env::var("DRACO_ORDER_THRESHOLD")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(0);
    let stage: usize = std::env::var("DRACO_ORDER_STAGE")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(usize::MAX);
    println!("gain threshold: {threshold} quarter-bits, first stage: {stage} columns");
    let columns_list: Vec<usize> = std::env::var("DRACO_ORDER_COLUMNS")
        .ok()
        .map(|c| c.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_else(|| vec![16, 24, 56]);
    for columns in columns_list {
        for (lanes, passes) in lane_configs.iter().copied() {
            let mut best = f64::MAX;
            let mut ids = Vec::new();
            for _ in 0..3 {
                let mut state = Lanes::new(&start_points, &start_order, columns);
                let t = std::time::Instant::now();
                match lanes {
                    8 => state.refine::<8>(passes, threshold, stage),
                    16 => state.refine::<16>(passes, threshold, stage),
                    32 => state.refine::<32>(passes, threshold, stage),
                    64 => state.refine::<64>(passes, threshold, stage),
                    _ => state.refine::<128>(passes, threshold, stage),
                }
                best = best.min(t.elapsed().as_secs_f64());
                if columns == 56 && lanes == 16 && passes == 2 {
                    println!("   counters: {} starts evaluated, {} stopped after the first stage, {} moves, mean reversal {:.1} points", state.evals, state.skipped, state.moves, state.reversed as f64 / state.moves.max(1) as f64);
                }
                ids = state.ids;
            }
            let mut check = ids.clone();
            check.sort_unstable();
            assert!(
                check.iter().enumerate().all(|(i, &p)| p == i as u32),
                "a point was lost"
            );
            let refined = pack(&cells, &rows, &by_cost, &ids);
            let bits = path_bits(&refined, columns, &tables.luts) / num_points as f64;
            let bytes = encode(&permute(&cloud, &ids), false);
            println!(
                "{columns:>7} {lanes:>6} {passes:>6} {best:>9.3} {bits:>14.3} {:>10}",
                vs(bytes)
            );
        }
    }
}

/// The lane-form refinement over blocks of the path that are independent of
/// one another, across `threads` threads. The result does not depend on the
/// number of threads: a block's outcome is a function of the block alone.
fn refine_lane_blocks<const L: usize>(
    points: &[Packed],
    ids: &mut [u32],
    block: usize,
    threads: usize,
    passes: usize,
    columns: usize,
) {
    let run = |p: &[Packed], i: &mut [u32]| {
        if p.len() < 4 {
            return;
        }
        let mut lanes = Lanes::new(p, i, columns);
        lanes.refine::<L>(passes, 0, usize::MAX);
        i.copy_from_slice(&lanes.ids);
    };
    if threads <= 1 {
        for (p, i) in points.chunks(block).zip(ids.chunks_mut(block)) {
            run(p, i);
        }
        return;
    }
    let work = std::sync::Mutex::new(points.chunks(block).zip(ids.chunks_mut(block)));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let Some((p, i)) = work.lock().unwrap().next() else {
                    break;
                };
                run(p, i);
            });
        }
    });
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn what_the_whole_pipeline_costs() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let t = std::time::Instant::now();
    let rows = byte_rows(&cloud);
    println!(
        "quantize the attributes to bytes (the probe's slow reader): {:.3} s",
        t.elapsed().as_secs_f64()
    );
    let morton = morton_order(&cells);
    let t = std::time::Instant::now();
    let hilbert = hilbert_order(&cells);
    println!(
        "Hilbert order of the cells: {:.3} s",
        t.elapsed().as_secs_f64()
    );
    let tables = Tables::new();
    let t = std::time::Instant::now();
    let by_cost = columns_by_cost(&rows, &hilbert, &tables.luts);
    println!(
        "rank the columns by cost along the path: {:.3} s",
        t.elapsed().as_secs_f64()
    );
    let t = std::time::Instant::now();
    let start_points = pack(&cells, &rows, &by_cost, &hilbert);
    println!(
        "gather the points in path order: {:.3} s",
        t.elapsed().as_secs_f64()
    );
    let morton_bytes = encode(&permute(&cloud, &morton), false);
    let vs = |bytes: usize| {
        format!(
            "{:+.2}%",
            (bytes as f64 / morton_bytes as f64 - 1.0) * 100.0
        )
    };

    println!(
        "{:>7} {:>8} {:>8} {:>9} {:>10}",
        "columns", "block", "threads", "time s", "vs Morton"
    );
    let mut reference: std::collections::HashMap<(usize, usize), Vec<u32>> = Default::default();
    for columns in [16usize, 56] {
        for (block, threads) in [
            (usize::MAX, 1usize),
            (8192, 1),
            (8192, 4),
            (8192, 16),
            (2048, 16),
        ] {
            let block_len = block.min(num_points);
            let mut best = f64::MAX;
            let mut ids = hilbert.clone();
            for _ in 0..3 {
                ids = hilbert.clone();
                let t = std::time::Instant::now();
                refine_lane_blocks::<16>(&start_points, &mut ids, block_len, threads, 2, columns);
                best = best.min(t.elapsed().as_secs_f64());
            }
            let bytes = encode(&permute(&cloud, &ids), false);
            if block != usize::MAX {
                match reference.get(&(columns, block)) {
                    None => {
                        reference.insert((columns, block), ids);
                    }
                    Some(r) => assert_eq!(r, &ids, "the thread count changed the result"),
                }
            }
            let label = if block == usize::MAX {
                "whole".to_string()
            } else {
                block.to_string()
            };
            println!(
                "{columns:>7} {label:>8} {threads:>8} {best:>9.3} {:>10}",
                vs(bytes)
            );
        }
    }
}

/// One hash a point, over every attribute's decoded bits, sorted: the cloud as
/// a set, with the order taken out.
fn decoded_point_set(bytes: &[u8]) -> Vec<u64> {
    let mut decoded = PointCloud::new();
    draco_core::PointCloudDecoder::new()
        .decode(&mut draco_core::DecoderBuffer::new(bytes), &mut decoded)
        .expect("what it wrote, it reads");
    let mut hashes = vec![0xcbf2_9ce4_8422_2325u64; decoded.num_points()];
    for id in 0..decoded.num_attributes() {
        let attribute = decoded.attribute(id);
        let components = attribute.num_components() as usize;
        for (point, hash) in hashes.iter_mut().enumerate() {
            let index = attribute.mapped_index(PointIndex(point as u32)).0 as usize;
            for component in 0..components {
                let bits = read_value(attribute, index, component).to_bits();
                *hash = (*hash ^ u64::from(bits)).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    hashes.sort_unstable();
    hashes
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn does_the_refined_order_decode_to_the_same_points() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let rows = byte_rows(&cloud);
    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let tables = Tables::new();
    let by_cost = columns_by_cost(&rows, &hilbert, &tables.luts);
    let start_points = pack(&cells, &rows, &by_cost, &hilbert);

    let morton_bytes = encode_bytes(&permute(&cloud, &morton), true);
    let morton_set = decoded_point_set(&morton_bytes);
    println!("Morton, search on: {} bytes", morton_bytes.len());
    for (columns, label) in [(16usize, "16 columns"), (56, "56 columns")] {
        let mut ids = hilbert.clone();
        refine_lane_blocks::<16>(&start_points, &mut ids, 8192, 4, 2, columns);
        let bytes = encode_bytes(&permute(&cloud, &ids), true);
        let t = std::time::Instant::now();
        let set = decoded_point_set(&bytes);
        assert_eq!(
            set, morton_set,
            "{label}: the decoded points are not the same set"
        );
        println!(
            "{label}: {} bytes ({:+.2}% vs Morton), decodes to the same {} points as Morton (decode and hash {:.2} s)",
            bytes.len(),
            (bytes.len() as f64 / morton_bytes.len() as f64 - 1.0) * 100.0,
            set.len(),
            t.elapsed().as_secs_f64()
        );
    }
}

/// A start order that is coarse in position and then in the dearest columns:
/// the Hilbert index of the cell `coarse_bits` an axis, then the bit planes of
/// the first `columns` columns of `rows`, most significant plane first and the
/// columns interleaved within a plane, so points in one cell end up beside the
/// ones that look like them.
fn cell_then_attributes_order(
    cells: &[[u32; 3]],
    rows: &[[u8; 64]],
    column_order: &[usize],
    coarse_bits: u32,
    columns: usize,
) -> Vec<u32> {
    let key_bits = 64 - 3 * coarse_bits as usize;
    let planes = (key_bits / columns.max(1)).min(8);
    let shift = AXIS_BITS - coarse_bits;
    let mut keyed: Vec<(u64, u32)> = cells
        .iter()
        .zip(rows)
        .enumerate()
        .map(|(point, (cell, row))| {
            let coarse = hilbert_key(
                cell[0] >> shift,
                cell[1] >> shift,
                cell[2] >> shift,
                coarse_bits,
            );
            let mut attributes = 0u64;
            for plane in 0..planes {
                for &column in &column_order[..columns] {
                    attributes = (attributes << 1) | u64::from((row[column] >> (7 - plane)) & 1);
                }
            }
            let used = planes * columns;
            let key =
                (coarse << (64 - 3 * coarse_bits as usize)) | (attributes << (key_bits - used));
            (key, point as u32)
        })
        .collect();
    keyed.sort_unstable();
    keyed.into_iter().map(|(_, p)| p).collect()
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn which_start_order_refines_best() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let rows = byte_rows(&cloud);
    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let tables = Tables::new();
    let by_cost = columns_by_cost(&rows, &hilbert, &tables.luts);
    let morton_bytes = encode(&permute(&cloud, &morton), false);
    let vs = |bytes: usize| {
        format!(
            "{:+.2}%",
            (bytes as f64 / morton_bytes as f64 - 1.0) * 100.0
        )
    };
    let columns = 16usize;

    let mut starts: Vec<(String, Vec<u32>)> = vec![("Hilbert, 16 bits".into(), hilbert.clone())];
    for coarse in [4u32, 6, 8] {
        for attrs in [2usize, 3, 4] {
            starts.push((
                format!("cell {coarse} bits, then {attrs} columns"),
                cell_then_attributes_order(&cells, &rows, &by_cost, coarse, attrs),
            ));
        }
    }
    println!(
        "{:<34} {:>12} {:>10} {:>12} {:>10}",
        "start order", "start alone", "vs Morton", "refined 16x2", "vs Morton"
    );
    for (label, order) in starts {
        let alone = encode(&permute(&cloud, &order), false);
        let start_points = pack(&cells, &rows, &by_cost, &order);
        let mut lanes = Lanes::new(&start_points, &order, columns);
        lanes.refine::<16>(2, 0, usize::MAX);
        let refined = encode(&permute(&cloud, &lanes.ids), false);
        println!(
            "{label:<34} {:>12.3} {:>10} {:>12.3} {:>10}",
            alone as f64 / num_points as f64,
            vs(alone),
            refined as f64 / num_points as f64,
            vs(refined)
        );
    }
}

/// The cloud in `order`, keeping only the attributes in `keep`.
fn permute_subset(cloud: &PointCloud, order: &[u32], keep: &[i32]) -> PointCloud {
    let names = attribute_names(cloud);
    let mut out = PointCloud::new();
    out.set_num_points(order.len());
    for &id in keep {
        let source = cloud.attribute(id);
        let mut attribute = PointAttribute::new();
        attribute.init(
            source.attribute_type(),
            source.num_components(),
            source.data_type(),
            source.normalized(),
            order.len(),
        );
        let stride = source.byte_stride() as usize;
        let buffer = attribute.buffer_mut();
        let mut raw = vec![0u8; stride];
        for (slot, &point) in order.iter().enumerate() {
            let value_index = source.mapped_index(PointIndex(point));
            source
                .buffer()
                .read(value_index.0 as usize * stride, &mut raw);
            buffer.write(slot * stride, &raw);
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

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn does_weighting_the_columns_by_what_they_really_cost_help() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let rows = byte_rows(&cloud);
    let names = attribute_names(&cloud);
    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let tables = Tables::new();
    let by_cost = columns_by_cost(&rows, &hilbert, &tables.luts);

    let (_, map) = byte_rows_mapped(&cloud);
    let position_id = (0..cloud.num_attributes())
        .find(|id| cloud.attribute(*id).attribute_type() == GeometryAttributeType::Position)
        .unwrap();
    let n = num_points as f64;
    let position_only = encode(&permute_subset(&cloud, &hilbert, &[position_id]), false);
    let columns_total = map.iter().flatten().count();
    let mut real = vec![0f64; columns_total];
    let mut estimated = vec![0f64; columns_total];
    let mut column_name = vec![String::new(); columns_total];
    for id in 0..cloud.num_attributes() {
        let Some(column) = map[id as usize] else {
            continue;
        };
        assert_eq!(
            cloud.attribute(id).num_components(),
            1,
            "one column an attribute here"
        );
        let bytes = encode(&permute_subset(&cloud, &hilbert, &[position_id, id]), false);
        real[column] = (bytes as f64 - position_only as f64) * 8.0 / n;
        column_name[column] = names[id as usize].clone().unwrap_or_default();
        for pair in hilbert.windows(2) {
            let d = rows[pair[0] as usize][column].abs_diff(rows[pair[1] as usize][column]);
            estimated[column] += f64::from(tables.luts.l8[d as usize]);
        }
        estimated[column] /= n;
    }
    let mut position_estimate = 0.0;
    for pair in hilbert.windows(2) {
        for axis in 0..3 {
            let d = cells[pair[0] as usize][axis].abs_diff(cells[pair[1] as usize][axis]);
            position_estimate += f64::from(tables.luts.l16[d.min(65535) as usize]);
        }
    }
    position_estimate /= n;
    let position_real = position_only as f64 * 8.0 / n;
    let ratios: Vec<f64> = (0..columns_total)
        .map(|k| real[k] / estimated[k].max(1e-9))
        .collect();
    let mut sorted = ratios.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2];
    println!(
        "bits a point along the Hilbert path, estimate against the coder's own; position {:.2} vs {:.2} (ratio {:.2})",
        position_estimate, position_real, position_real / position_estimate
    );
    println!(
        "ratio over {columns_total} columns: min {:.2}, median {:.2}, max {:.2}",
        sorted[0],
        median,
        sorted[sorted.len() - 1]
    );
    for k in 0..columns_total {
        if k % 6 == 0 || ratios[k] <= sorted[1] || ratios[k] >= sorted[sorted.len() - 2] {
            println!(
                "  {:<10} est {:>6.2}  real {:>6.2}  ratio {:>5.2}",
                column_name[k], estimated[k], real[k], ratios[k]
            );
        }
    }

    let morton_bytes = encode(&permute(&cloud, &morton), false);
    let vs = |bytes: usize| {
        format!(
            "{:+.2}%",
            (bytes as f64 / morton_bytes as f64 - 1.0) * 100.0
        )
    };
    let by_real: Vec<usize> = {
        let mut order: Vec<usize> = (0..columns_total).collect();
        order.sort_by(|&a, &b| real[b].partial_cmp(&real[a]).unwrap());
        order
    };
    let weight_of =
        |k: usize| -> u16 { ((8.0 * ratios[k] / median).round() as i64).clamp(1, 24) as u16 };
    let pos_weight =
        ((8.0 * (position_real / position_estimate) / median).round() as i64).clamp(1, 24) as u16;
    println!("position weight {pos_weight} / 8");
    println!("{:<52} {:>10}", "arm (window 16 x 2)", "vs Morton");
    for (label, rank, weighted, columns) in [
        (
            "16 columns ranked by estimate, unweighted",
            &by_cost,
            false,
            16usize,
        ),
        (
            "16 columns ranked by estimate, weighted",
            &by_cost,
            true,
            16,
        ),
        (
            "16 columns ranked by real cost, weighted",
            &by_real,
            true,
            16,
        ),
        ("all columns, unweighted", &by_cost, false, columns_total),
        ("all columns, weighted", &by_cost, true, columns_total),
    ] {
        let start_points = pack(&cells, &rows, rank, &hilbert);
        let weights: Vec<u16> = rank
            .iter()
            .map(|&k| if weighted { weight_of(k) } else { 8 })
            .collect();
        let mut lanes = Lanes::weighted(
            &start_points,
            &hilbert,
            columns,
            &weights,
            if weighted { pos_weight } else { 8 },
        );
        lanes.refine::<16>(2, 0, usize::MAX);
        let bytes = encode(&permute(&cloud, &lanes.ids), false);
        println!("{label:<52} {:>10}", vs(bytes));
    }
}

/// Greedy with a bounded view: from the point just placed, take the cheapest of
/// the next `view` points not yet placed in the start order. The pool refills
/// from the start order, so the path stays near where the start order is.
fn windowed_greedy(
    points: &[Packed],
    ids: &[u32],
    view: usize,
    columns: usize,
    t: &Tables,
) -> (Vec<Packed>, Vec<u32>) {
    let n = points.len();
    let mut pool: Vec<usize> = (0..view.min(n)).collect();
    let mut next = pool.len();
    let mut out_points = Vec::with_capacity(n);
    let mut out_ids = Vec::with_capacity(n);
    let mut current = points[0];
    out_points.push(current);
    out_ids.push(ids[0]);
    // The first point of the start order is placed; the pool starts after it.
    pool.remove(0);
    if next < n {
        pool.push(next);
        next += 1;
    }
    while !pool.is_empty() {
        let mut best = 0;
        let mut best_cost = f32::MAX;
        for (slot, &p) in pool.iter().enumerate() {
            let c = priced::<2>(&current, &points[p], columns, t);
            if c < best_cost {
                best_cost = c;
                best = slot;
            }
        }
        let chosen = pool[best];
        current = points[chosen];
        out_points.push(current);
        out_ids.push(ids[chosen]);
        if next < n {
            pool[best] = next;
            next += 1;
        } else {
            pool.swap_remove(best);
        }
    }
    (out_points, out_ids)
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn does_a_greedy_start_beat_the_plain_curve() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let rows = byte_rows(&cloud);
    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let tables = Tables::new();
    let by_cost = columns_by_cost(&rows, &hilbert, &tables.luts);
    let start_points = pack(&cells, &rows, &by_cost, &hilbert);
    let morton_bytes = encode(&permute(&cloud, &morton), false);
    let vs = |bytes: usize| {
        format!(
            "{:+.2}%",
            (bytes as f64 / morton_bytes as f64 - 1.0) * 100.0
        )
    };
    let columns = 16usize;
    println!(
        "{:<44} {:>9} {:>10}",
        "arm (16 columns)", "time s", "vs Morton"
    );

    let mut lanes = Lanes::new(&start_points, &hilbert, columns);
    let t = std::time::Instant::now();
    lanes.refine::<16>(2, 0, usize::MAX);
    let seconds = t.elapsed().as_secs_f64();
    let bytes = encode(&permute(&cloud, &lanes.ids), false);
    println!(
        "{:<44} {seconds:>9.3} {:>10}",
        "Hilbert + 2-opt w16x2 (the reference)",
        vs(bytes)
    );

    for view in [8usize, 16, 32, 64] {
        let t = std::time::Instant::now();
        let (points, ids) = windowed_greedy(&start_points, &hilbert, view, columns, &tables);
        let build = t.elapsed().as_secs_f64();
        let mut check = ids.clone();
        check.sort_unstable();
        assert!(
            check.iter().enumerate().all(|(i, &p)| p == i as u32),
            "a point was lost"
        );
        let alone = encode(&permute(&cloud, &ids), false);
        println!(
            "{:<44} {build:>9.3} {:>10}",
            format!("greedy, view {view}"),
            vs(alone)
        );
        let mut lanes = Lanes::new(&points, &ids, columns);
        let t = std::time::Instant::now();
        lanes.refine::<16>(2, 0, usize::MAX);
        let seconds = t.elapsed().as_secs_f64() + build;
        let bytes = encode(&permute(&cloud, &lanes.ids), false);
        println!(
            "{:<44} {seconds:>9.3} {:>10}",
            format!("greedy, view {view}, then 2-opt w16x2"),
            vs(bytes)
        );
    }
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn greedy_against_the_window_at_equal_time() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let rows = byte_rows(&cloud);
    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let tables = Tables::new();
    let by_cost = columns_by_cost(&rows, &hilbert, &tables.luts);
    let start_points = pack(&cells, &rows, &by_cost, &hilbert);
    let morton_bytes = encode(&permute(&cloud, &morton), false);
    let vs = |bytes: usize| {
        format!(
            "{:+.2}%",
            (bytes as f64 / morton_bytes as f64 - 1.0) * 100.0
        )
    };
    let columns_list: Vec<usize> = std::env::var("DRACO_ORDER_COLUMNS")
        .ok()
        .map(|c| c.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_else(|| vec![16, 56]);
    for columns in columns_list {
        println!("-- {columns} columns, one thread");
        println!("{:<40} {:>9} {:>10}", "arm", "time s", "vs Morton");
        let refine =
            |points: &[Packed], ids: &[u32], lanes: usize, passes: usize| -> (f64, Vec<u32>) {
                let mut state = Lanes::new(points, ids, columns);
                let t = std::time::Instant::now();
                match lanes {
                    16 => state.refine::<16>(passes, 0, usize::MAX),
                    32 => state.refine::<32>(passes, 0, usize::MAX),
                    _ => state.refine::<64>(passes, 0, usize::MAX),
                }
                (t.elapsed().as_secs_f64(), state.ids)
            };
        for lanes in [16usize, 32, 64] {
            let (seconds, ids) = refine(&start_points, &hilbert, lanes, 2);
            let bytes = encode(&permute(&cloud, &ids), false);
            println!(
                "{:<40} {seconds:>9.3} {:>10}",
                format!("Hilbert + 2-opt w{lanes}x2"),
                vs(bytes)
            );
        }
        for view in [32usize, 64, 128] {
            let t = std::time::Instant::now();
            let (points, ids) = windowed_greedy(&start_points, &hilbert, view, columns, &tables);
            let build = t.elapsed().as_secs_f64();
            let alone = encode(&permute(&cloud, &ids), false);
            println!(
                "{:<40} {build:>9.3} {:>10}",
                format!("greedy view {view}"),
                vs(alone)
            );
            for lanes in [16usize, 32] {
                let (seconds, refined) = refine(&points, &ids, lanes, 2);
                let bytes = encode(&permute(&cloud, &refined), false);
                println!(
                    "{:<40} {:>9.3} {:>10}",
                    format!("greedy view {view} + 2-opt w{lanes}x2"),
                    build + seconds,
                    vs(bytes)
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The preparation, cheap: columns from the start, no rows, no gather of rows.
// ---------------------------------------------------------------------------

/// Calls `work` for every index in `0..count` across `threads` threads, each
/// taking the next undone index, and returns the results in index order.
fn parallel_map<T: Send>(count: usize, threads: usize, work: impl Fn(usize) -> T + Sync) -> Vec<T> {
    if threads <= 1 {
        return (0..count).map(work).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: std::sync::Mutex<Vec<Option<T>>> =
        std::sync::Mutex::new((0..count).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= count {
                    break;
                }
                let value = work(i);
                results.lock().unwrap()[i] = Some(value);
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(Option::unwrap)
        .collect()
}

/// One component of an attribute read straight off its buffer, as `f32`.
/// Assumes the attribute maps points to values one to one, as a cloud read
/// from a file does.
fn component_values(attribute: &PointAttribute, component: usize, count: usize) -> Vec<f32> {
    use draco_core::DataType;
    let data = attribute.buffer().data();
    let stride = attribute.byte_stride() as usize;
    // A cloud read from a file maps points to values one to one; a cloud whose
    // equal values were merged does not, and then every read goes by the map.
    let identity = data.len() / stride.max(1) == count;
    let value = |p: usize| {
        if identity {
            p
        } else {
            attribute.mapped_index(PointIndex(p as u32)).0 as usize
        }
    };
    match attribute.data_type() {
        DataType::Float32 => (0..count)
            .map(|p| {
                let at = value(p) * stride + component * 4;
                f32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
            })
            .collect(),
        DataType::Uint8 => (0..count)
            .map(|p| f32::from(data[value(p) * stride + component]))
            .collect(),
        other => panic!("the fast reader does not read {other:?}"),
    }
}

struct Prepared {
    /// The position on its 16-bit grid, a column an axis, in file order.
    cells: [Vec<u16>; 3],
    /// The attribute columns that vary, quantized to the budget, file order.
    columns: Vec<Vec<u8>>,
}

fn prepare(cloud: &PointCloud, threads: usize) -> Prepared {
    let count = cloud.num_points();
    let names = attribute_names(cloud);
    let position_id = (0..cloud.num_attributes())
        .find(|id| cloud.attribute(*id).attribute_type() == GeometryAttributeType::Position)
        .expect("a position");
    let position = cloud.attribute(position_id);
    let axes = parallel_map(3, threads.min(3), |axis| {
        let values = component_values(position, axis, count);
        let low = values.iter().copied().fold(f32::INFINITY, f32::min);
        let high = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let (low, span) = (f64::from(low), f64::from(high) - f64::from(low));
        let levels = ((1u64 << AXIS_BITS) - 1) as f64;
        values
            .iter()
            .map(|&v| {
                let normalized = if span > 0.0 {
                    (f64::from(v) - low) / span
                } else {
                    0.0
                };
                (normalized * levels) as u16
            })
            .collect::<Vec<u16>>()
    });
    let mut axes = axes.into_iter();
    let cells = [
        axes.next().unwrap(),
        axes.next().unwrap(),
        axes.next().unwrap(),
    ];

    let ids: Vec<i32> = (0..cloud.num_attributes())
        .filter(|&id| id != position_id)
        .collect();
    let per_attribute = parallel_map(ids.len(), threads, |i| {
        let id = ids[i];
        let attribute = cloud.attribute(id);
        let is_float = attribute.data_type() == draco_core::DataType::Float32;
        let levels = if is_float {
            ((1u32 << budget_bits(names[id as usize].as_deref())) - 1) as f32
        } else {
            255.0
        };
        let mut columns = Vec::new();
        for component in 0..attribute.num_components() as usize {
            let values = component_values(attribute, component, count);
            let low = values.iter().copied().fold(f32::INFINITY, f32::min);
            let high = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            if high - low < 1e-12 {
                continue;
            }
            let (low, span) = if is_float {
                (low, high - low)
            } else {
                (0.0, 255.0)
            };
            let scale = levels / span;
            columns.push(
                values
                    .iter()
                    .map(|&v| ((v - low) * scale + 0.5) as u8)
                    .collect::<Vec<u8>>(),
            );
        }
        columns
    });
    Prepared {
        cells,
        columns: per_attribute.into_iter().flatten().collect(),
    }
}

fn aos_cells(cells: &[Vec<u16>; 3]) -> Vec<[u32; 3]> {
    (0..cells[0].len())
        .map(|p| {
            [
                u32::from(cells[0][p]),
                u32::from(cells[1][p]),
                u32::from(cells[2][p]),
            ]
        })
        .collect()
}

/// The columns dearest along `order`, judged on every `stride`-th step.
fn rank_columns_sampled(
    columns: &[Vec<u8>],
    order: &[u32],
    stride: usize,
    luts: &Luts,
) -> Vec<usize> {
    let sample: Vec<(u32, u32)> = order
        .windows(2)
        .step_by(stride)
        .map(|w| (w[0], w[1]))
        .collect();
    let mut totals: Vec<(f64, usize)> = columns
        .iter()
        .enumerate()
        .map(|(k, column)| {
            let sum: f64 = sample
                .iter()
                .map(|&(a, b)| {
                    f64::from(luts.l8[column[a as usize].abs_diff(column[b as usize]) as usize])
                })
                .sum();
            (sum, k)
        })
        .collect();
    totals.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap());
    totals.into_iter().map(|(_, k)| k).collect()
}

impl Lanes {
    /// A block of the path built column by column from the file-order
    /// columns: the block's ids pick the entries, one column at a time, so the
    /// reads stay inside one column's worth of memory.
    fn gather(prepared: &Prepared, chosen: &[usize], ids: &[u32]) -> Self {
        let n = ids.len();
        let pad = |mut v: Vec<u8>| {
            v.resize(n + LANE_PAD, 0);
            v
        };
        let cols: Vec<Vec<u8>> = chosen
            .iter()
            .map(|&k| {
                pad(ids
                    .iter()
                    .map(|&p| prepared.columns[k][p as usize])
                    .collect())
            })
            .collect();
        let pos: [Vec<u16>; 3] = std::array::from_fn(|axis| {
            let mut v: Vec<u16> = ids
                .iter()
                .map(|&p| prepared.cells[axis][p as usize])
                .collect();
            v.resize(n + LANE_PAD, 0);
            v
        });
        let mut lanes = Lanes {
            n,
            weights: vec![1; cols.len()],
            cols,
            pos,
            ids: ids.to_vec(),
            edges: vec![0; n + LANE_PAD],
            evals: 0,
            moves: 0,
            reversed: 0,
            skipped: 0,
            pos_weight: 1,
        };
        for i in 0..n.saturating_sub(1) {
            lanes.edges[i] = lanes.edge(i, i + 1);
        }
        lanes
    }
}

struct PipelineTimes {
    hilbert: f64,
    quantize: f64,
    rank: f64,
    refine: f64,
}

fn order_pipeline<const L: usize>(
    cloud: &PointCloud,
    threads: usize,
    top: usize,
    passes: usize,
    block: usize,
    luts: &Luts,
) -> (Vec<u32>, PipelineTimes) {
    let t = std::time::Instant::now();
    let prepared = prepare(cloud, threads);
    let quantize = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    let cells = aos_cells(&prepared.cells);
    let mut order = hilbert_order(&cells);
    let hilbert = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    let ranked = rank_columns_sampled(&prepared.columns, &order, 16, luts);
    let chosen: Vec<usize> = ranked.into_iter().take(top).collect();
    let rank = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    let work = std::sync::Mutex::new(order.chunks_mut(block));
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| loop {
                let Some(ids) = work.lock().unwrap().next() else {
                    break;
                };
                if ids.len() < 4 {
                    continue;
                }
                let mut lanes = Lanes::gather(&prepared, &chosen, ids);
                lanes.refine::<L>(passes, 0, usize::MAX);
                ids.copy_from_slice(&lanes.ids);
            });
        }
    });
    let refine = t.elapsed().as_secs_f64();
    (
        order,
        PipelineTimes {
            hilbert,
            quantize,
            rank,
            refine,
        },
    )
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn what_the_prepared_pipeline_costs() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let luts = Luts::new();
    let cells_slow = quantized_positions(&cloud).expect("positions");
    let morton_bytes = encode(&permute(&cloud, &morton_order(&cells_slow)), false);
    let vs = |bytes: usize| {
        format!(
            "{:+.2}%",
            (bytes as f64 / morton_bytes as f64 - 1.0) * 100.0
        )
    };
    println!(
        "{:>7} {:>8} {:>10} {:>10} {:>9} {:>9} {:>9} {:>10}",
        "columns",
        "threads",
        "quantize s",
        "Hilbert s",
        "rank s",
        "refine s",
        "total s",
        "vs Morton"
    );
    for top in [16usize, 56] {
        for threads in [1usize, 4, 16] {
            let mut best: Option<(f64, PipelineTimes, Vec<u32>)> = None;
            for _ in 0..3 {
                let (ids, times) = order_pipeline::<16>(&cloud, threads, top, 2, 8192, &luts);
                let total = times.quantize + times.hilbert + times.rank + times.refine;
                if best.as_ref().map_or(true, |b| total < b.0) {
                    best = Some((total, times, ids));
                }
            }
            let (total, times, ids) = best.unwrap();
            let mut check = ids.clone();
            check.sort_unstable();
            assert!(
                check.iter().enumerate().all(|(i, &p)| p == i as u32),
                "a point was lost"
            );
            let bytes = encode(&permute(&cloud, &ids), false);
            println!(
                "{top:>7} {threads:>8} {:>10.3} {:>10.3} {:>9.3} {:>9.3} {total:>9.3} {:>10}",
                times.quantize,
                times.hilbert,
                times.rank,
                times.refine,
                vs(bytes)
            );
        }
    }
}

/// The objective over a whole path, exactly: `log2(1 + |delta|)` summed over
/// the position and every column, however many there are.
fn estimated_bits_per_point(
    cells: &[[u32; 3]],
    rows: &[[u8; 64]],
    columns: usize,
    order: &[u32],
    luts: &Luts,
) -> f64 {
    let mut total = 0f64;
    for pair in order.windows(2) {
        let (a, b) = (pair[0] as usize, pair[1] as usize);
        for axis in 0..3 {
            total +=
                f64::from(luts.l16[cells[a][axis].abs_diff(cells[b][axis]).min(65535) as usize]);
        }
        for k in 0..columns {
            total += f64::from(luts.l8[rows[a][k].abs_diff(rows[b][k]) as usize]);
        }
    }
    total / order.len() as f64
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn does_the_estimate_say_when_to_leave_the_order_alone() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let (rows, map) = byte_rows_mapped(&cloud);
    let columns_total = map.iter().flatten().count()
        + (0..cloud.num_attributes())
            .filter(|&id| cloud.attribute(id).num_components() > 1)
            .map(|id| cloud.attribute(id).num_components() as usize - 1)
            .sum::<usize>();
    let columns_total = columns_total.min(56);
    let luts = Luts::new();
    let tables = Tables::new();
    let file: Vec<u32> = (0..num_points as u32).collect();
    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let by_cost = columns_by_cost(&rows, &hilbert, &luts);
    let top = columns_total.min(16);

    let refine_from = |start: &[u32], columns: usize| -> Vec<u32> {
        let points = pack(&cells, &rows, &by_cost, start);
        let mut lanes = Lanes::new(&points, start, columns);
        lanes.refine::<16>(2, 0, usize::MAX);
        lanes.ids
    };
    let arms: Vec<(String, Vec<u32>)> = vec![
        ("file order".into(), file.clone()),
        ("Morton".into(), morton),
        ("Hilbert".into(), hilbert.clone()),
        (
            format!("Hilbert, refined ({top} cols)"),
            refine_from(&hilbert, top),
        ),
        (
            format!("Hilbert, refined ({columns_total} cols)"),
            refine_from(&hilbert, columns_total),
        ),
        (
            format!("file order, refined ({top} cols)"),
            refine_from(&file, top),
        ),
        (
            format!("file order, refined ({columns_total} cols)"),
            refine_from(&file, columns_total),
        ),
    ];
    let _ = &tables;
    println!(
        "{columns_total} columns; estimate over all of them, the coder's own size, bits a point"
    );
    println!(
        "{:<34} {:>10} {:>10} {:>9}",
        "order", "estimate", "coder", "ratio"
    );
    for (label, order) in &arms {
        let est = estimated_bits_per_point(&cells, &rows, columns_total, order, &luts);
        let real = encode(&permute(&cloud, order), false) as f64 * 8.0 / num_points as f64;
        println!("{label:<34} {est:>10.2} {real:>10.2} {:>9.3}", real / est);
    }
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn when_does_the_estimate_pick_wrong() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let cells = quantized_positions(&cloud).expect("positions");
    let (rows, _) = byte_rows_mapped(&cloud);
    let luts = Luts::new();
    let columns_total = std::env::var("DRACO_ORDER_ALL_COLUMNS")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(56usize);
    let file: Vec<u32> = (0..num_points as u32).collect();
    let morton = morton_order(&cells);
    let hilbert = hilbert_order(&cells);
    let by_cost = columns_by_cost(&rows, &hilbert, &luts);
    let top = columns_total.min(16);
    let refine_from = |start: &[u32]| -> Vec<u32> {
        let points = pack(&cells, &rows, &by_cost, start);
        let mut lanes = Lanes::new(&points, start, top);
        lanes.refine::<16>(2, 0, usize::MAX);
        lanes.ids
    };
    let refined_hilbert = refine_from(&hilbert);
    let mut inputs: Vec<(&str, Vec<u32>)> = vec![
        ("file", file),
        ("Morton", morton),
        ("Hilbert", hilbert.clone()),
    ];
    if std::env::var("DRACO_ORDER_ALL_COLUMNS").is_err() {
        inputs.push(("Hilbert refined", refined_hilbert.clone()));
    }
    println!("an input order X, and what could be written instead; estimate and the coder's own size, bits a point");
    println!(
        "{:<18} {:<26} {:>9} {:>9}",
        "input X", "candidate", "estimate", "coder"
    );
    for (name, x) in &inputs {
        let candidates: Vec<(String, Vec<u32>)> = vec![
            ("X as it is".into(), x.clone()),
            ("X refined".into(), refine_from(x)),
            ("Hilbert refined".into(), refined_hilbert.clone()),
        ];
        let measured: Vec<(f64, f64)> = candidates
            .iter()
            .map(|(_, order)| {
                (
                    estimated_bits_per_point(&cells, &rows, columns_total, order, &luts),
                    encode(&permute(&cloud, order), false) as f64 * 8.0 / num_points as f64,
                )
            })
            .collect();
        let by_estimate = (0..3)
            .min_by(|&a, &b| measured[a].0.partial_cmp(&measured[b].0).unwrap())
            .unwrap();
        let by_coder = (0..3)
            .min_by(|&a, &b| measured[a].1.partial_cmp(&measured[b].1).unwrap())
            .unwrap();
        for (k, ((label, _), (est, real))) in candidates.iter().zip(&measured).enumerate() {
            println!(
                "{:<18} {label:<26} {est:>9.2} {real:>9.2}  {}{}",
                if k == 0 { *name } else { "" },
                if k == by_estimate {
                    "<- estimate's pick "
                } else {
                    ""
                },
                if k == by_coder { "<- coder's pick" } else { "" }
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Leave the order alone, or touch it.
// ---------------------------------------------------------------------------

/// How much better than the Hilbert order the input order may be before the
/// refinement, which starts from Hilbert, is not worth running: the estimate of
/// the input over the estimate of Hilbert must stay above this. Refining
/// Hilbert takes the estimate to about 0.8 of it on a splat and 0.85 on a scan,
/// so 0.93 leaves room for a worse refinement than either.
const WORTH_TRYING_ABOVE: f64 = 0.93;

/// The refined order must undercut the input by at least this, in estimate. The
/// estimate and the coder disagree by a few percent of the estimate: on the
/// scan, refining its file order lowered the estimate 1.7% and raised the
/// coder's size 0.9%.
const WORTH_KEEPING_BELOW: f64 = 0.97;

/// Mean estimated bits a step over every `stride`-th step of `order`, all
/// columns and the position.
fn sampled_estimate(prepared: &Prepared, order: &[u32], stride: usize, luts: &Luts) -> f64 {
    let mut total = 0f64;
    let mut steps = 0usize;
    for pair in order.windows(2).step_by(stride) {
        let (a, b) = (pair[0] as usize, pair[1] as usize);
        for axis in &prepared.cells {
            total += f64::from(luts.l16[axis[a].abs_diff(axis[b]) as usize]);
        }
        for column in &prepared.columns {
            total += f64::from(luts.l8[column[a].abs_diff(column[b]) as usize]);
        }
        steps += 1;
    }
    total / steps.max(1) as f64
}

struct Decision {
    order: Vec<u32>,
    touched: bool,
    reason: String,
    estimate_in: f64,
    estimate_hilbert: f64,
    estimate_out: Option<f64>,
    seconds: f64,
}

/// The order to write for `cloud`, whose points are in the order it was read.
fn decide_order<const L: usize>(cloud: &PointCloud, threads: usize, luts: &Luts) -> Decision {
    let started = std::time::Instant::now();
    let count = cloud.num_points();
    let identity: Vec<u32> = (0..count as u32).collect();
    let prepared = prepare(cloud, threads);
    let hilbert = hilbert_order(&aos_cells(&prepared.cells));
    let estimate_in = sampled_estimate(&prepared, &identity, 17, luts);
    let estimate_hilbert = sampled_estimate(&prepared, &hilbert, 17, luts);
    let keep = |reason: String, estimate_out: Option<f64>| Decision {
        order: identity.clone(),
        touched: false,
        reason,
        estimate_in,
        estimate_hilbert,
        estimate_out,
        seconds: started.elapsed().as_secs_f64(),
    };
    if estimate_in < WORTH_TRYING_ABOVE * estimate_hilbert {
        return keep(
            format!(
                "the input is already {:.0}% under Hilbert",
                (1.0 - estimate_in / estimate_hilbert) * 100.0
            ),
            None,
        );
    }
    let ranked = rank_columns_sampled(&prepared.columns, &hilbert, 16, luts);
    let chosen: Vec<usize> = ranked.into_iter().take(16).collect();
    let mut order = hilbert;
    let work = std::sync::Mutex::new(order.chunks_mut(8192));
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| loop {
                let Some(ids) = work.lock().unwrap().next() else {
                    break;
                };
                if ids.len() < 4 {
                    continue;
                }
                let mut lanes = Lanes::gather(&prepared, &chosen, ids);
                lanes.refine::<L>(2, 0, usize::MAX);
                ids.copy_from_slice(&lanes.ids);
            });
        }
    });
    let estimate_out = sampled_estimate(&prepared, &order, 5, luts);
    if estimate_out > WORTH_KEEPING_BELOW * estimate_in {
        return keep(
            format!(
                "refined only {:.1}% under the input",
                (1.0 - estimate_out / estimate_in) * 100.0
            ),
            Some(estimate_out),
        );
    }
    Decision {
        order,
        touched: true,
        reason: format!(
            "refined {:.0}% under the input",
            (1.0 - estimate_out / estimate_in) * 100.0
        ),
        estimate_in,
        estimate_hilbert,
        estimate_out: Some(estimate_out),
        seconds: started.elapsed().as_secs_f64(),
    }
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn does_the_rule_leave_alone_what_it_should() {
    let Some((path, cloud)) = scene_cloud() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let num_points = cloud.num_points();
    println!("scene: {} ({num_points} points)", path.display());
    let luts = Luts::new();
    let cells = quantized_positions(&cloud).expect("positions");
    let (rows, _) = byte_rows_mapped(&cloud);
    let hilbert = hilbert_order(&cells);
    let by_cost = columns_by_cost(&rows, &hilbert, &luts);
    let columns = 16usize;
    let refined_hilbert = {
        let points = pack(&cells, &rows, &by_cost, &hilbert);
        let mut lanes = Lanes::new(&points, &hilbert, columns);
        lanes.refine::<16>(2, 0, usize::MAX);
        lanes.ids
    };
    let inputs: Vec<(&str, PointCloud)> = vec![
        ("file order", cloud.clone()),
        ("Morton order", permute(&cloud, &morton_order(&cells))),
        ("Hilbert order", permute(&cloud, &hilbert)),
        ("Hilbert, refined", permute(&cloud, &refined_hilbert)),
    ];
    println!(
        "{:<18} {:>9} {:>9} {:>9} {:>8} {:>10} {:>10}  {}",
        "input", "est in", "est Hilb", "est out", "time s", "as read", "written", "decision"
    );
    for (label, input) in &inputs {
        let d = decide_order::<16>(input, 16, &luts);
        let as_read = encode(input, false) as f64 * 8.0 / num_points as f64;
        let written = encode(&permute(input, &d.order), false) as f64 * 8.0 / num_points as f64;
        println!(
            "{label:<18} {:>9.2} {:>9.2} {:>9} {:>8.3} {as_read:>10.2} {written:>10.2}  {} ({})",
            d.estimate_in,
            d.estimate_hilbert,
            d.estimate_out
                .map_or("-".to_string(), |e| format!("{e:.2}")),
            d.seconds,
            if d.touched { "TOUCH" } else { "LEAVE" },
            d.reason
        );
    }
}
