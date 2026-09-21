//! EXPERIMENT: why the curve barely moves the file.
//!
//! Hilbert order beats Morton by 0.8% of a splat scene, against the 5% that
//! raising the grid from 10 to 16 bits an axis buys. Two explanations fit that
//! number and they call for opposite conclusions: the curve hardly changes the
//! order the predictor sees, or it changes it plenty and the positions are too
//! small a part of the file to show it.
//!
//! So this splits the file. The position attribute is encoded alone, under
//! both curves, which is the cost the curve can actually act on; and the steps
//! between consecutive points are measured directly in quantized space, where
//! Morton's block-boundary jumps either exist in quantity or do not.
//!
//! ```text
//! DRACO_SPATIAL_BITS=16 DRACO_SPLAT_PLY=/path/to/point_cloud.ply \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --features encoder,decoder --test curve_anatomy_probe -- --ignored --nocapture
//! ```

#![cfg(all(feature = "encoder", feature = "decoder"))]

use std::path::PathBuf;

use draco_core::{
    EncoderBuffer, EncoderOptions, GeometryAttributeType, PointCloud, PointCloudEncoder,
};

const SEQUENTIAL: i32 = 0;
const POSITION_BITS: i32 = 16;

fn encode(cloud: &PointCloud, spatial: bool) -> usize {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_spatial_point_order(spatial);
    for id in 0..cloud.num_attributes() {
        let bits = match cloud.attribute(id).attribute_type() {
            GeometryAttributeType::Position => POSITION_BITS,
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

/// The same cloud carrying nothing but its positions.
fn positions_only(cloud: &PointCloud) -> PointCloud {
    let att_id = (0..cloud.num_attributes())
        .find(|id| cloud.attribute(*id).attribute_type() == GeometryAttributeType::Position)
        .expect("the scene has positions");
    let mut only = PointCloud::new();
    only.set_num_points(cloud.num_points());
    only.add_attribute(cloud.attribute(att_id).clone());
    only
}

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn where_the_curve_acts() {
    let Some(path) = std::env::var_os("DRACO_SPLAT_PLY").map(PathBuf::from) else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let curve = std::env::var("DRACO_SPATIAL_CURVE").unwrap_or_else(|_| "morton".to_string());
    let bits = std::env::var("DRACO_SPATIAL_BITS").unwrap_or_else(|_| "10".to_string());

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
    println!("curve: {curve}, {bits} bits an axis");
    println!();

    let per_point = |bytes: usize| bytes as f64 / num_points as f64;
    let whole_plain = per_point(encode(&cloud, false));
    let whole_spatial = per_point(encode(&cloud, true));

    let only = positions_only(&cloud);
    let positions_plain = per_point(encode(&only, false));
    let positions_spatial = per_point(encode(&only, true));

    println!("{:<28} {:>10} {:>10}", "", "as given", "spatial");
    println!(
        "{:<28} {whole_plain:>10.3} {whole_spatial:>10.3}",
        "whole file, B/point"
    );
    println!(
        "{:<28} {positions_plain:>10.3} {positions_spatial:>10.3}",
        "positions alone, B/point"
    );
    println!();
    println!(
        "  positions are {:.1}% of the spatially ordered file",
        positions_spatial / whole_spatial * 100.0
    );
    println!(
        "  so 1% off the positions is {:.2}% off the file",
        positions_spatial / whole_spatial
    );

    // And the grid, which moved the file far more than the curve did. The
    // claim to check is that ten bits an axis puts distinct points in one cell,
    // where the order between them is whatever the sort left: that is not a
    // property of either curve, and it would explain why both improve together
    // when the grid gets finer.
    println!();
    println!("  points sharing a cell with another point:");
    let attribute = cloud.attribute(
        (0..cloud.num_attributes())
            .find(|id| cloud.attribute(*id).attribute_type() == GeometryAttributeType::Position)
            .expect("the scene has positions"),
    );
    let stride = attribute.byte_stride() as usize;
    let read = |point: usize, axis: usize| -> f64 {
        let mut bytes = [0u8; 4];
        attribute
            .buffer()
            .read(point * stride + axis * 4, &mut bytes);
        f64::from(f32::from_le_bytes(bytes))
    };
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for point in 0..num_points {
        for axis in 0..3 {
            let value = read(point, axis);
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    for axis_bits in [10u32, 14, 16, 21] {
        let levels = ((1u64 << axis_bits) - 1) as f64;
        let mut cells: Vec<u64> = (0..num_points)
            .map(|point| {
                let mut cell = 0u64;
                for axis in 0..3 {
                    let span = max[axis] - min[axis];
                    let normalized = if span > 0.0 {
                        (read(point, axis) - min[axis]) / span
                    } else {
                        0.0
                    };
                    cell = (cell << axis_bits) | (normalized * levels) as u64;
                }
                cell
            })
            .collect();
        cells.sort_unstable();
        let distinct = 1 + cells.windows(2).filter(|pair| pair[0] != pair[1]).count();
        println!(
            "    {axis_bits:>2} bits an axis: {:>6.2}%  ({distinct} distinct cells)",
            (num_points - distinct) as f64 / num_points as f64 * 100.0
        );
    }
}
