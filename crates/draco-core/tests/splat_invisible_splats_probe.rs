//! What a splat scene carries that is never seen.
//!
//! Every other measurement in this area is bytes per point, and a splat's
//! point count is treated as given. It is not: 3DGS trains to a fixed
//! densification schedule and leaves behind gaussians whose opacity rounds to
//! nothing. They cost a full 62 values each and contribute no pixels, so
//! dropping them is the one change here that moves the file without moving
//! what the file says.
//!
//! Opacity in the PLY is pre-sigmoid, so the threshold is applied to
//! `sigmoid(opacity)` — the alpha the renderer multiplies by. `1/255` is the
//! usual training-time prune threshold, the point below which a gaussian
//! cannot tint an 8-bit pixel even at full coverage.
//!
//! This reports totals, not bytes per point. Bytes per point is the wrong
//! ruler for a change that removes points: it can rise while the file shrinks.
//!
//! ```text
//! DRACO_SPLAT_PLY=/path/to/point_cloud.ply \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --features encoder,decoder --test splat_invisible_splats_probe -- --ignored --nocapture
//! ```

#![cfg(all(feature = "encoder", feature = "decoder"))]

use std::path::PathBuf;

use draco_core::{
    EncoderBuffer, EncoderOptions, GeometryAttributeType, Metadata, PointAttribute, PointCloud,
    PointCloudEncoder, PointIndex,
};

const SEQUENTIAL: i32 = 0;

/// The alpha below which a gaussian cannot tint an 8-bit pixel at any
/// coverage, and the threshold 3DGS itself prunes at during training.
const INVISIBLE: f32 = 1.0 / 255.0;

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

/// The same cloud carrying only the points named, with every attribute, its
/// name and its component layout intact.
fn keep(cloud: &PointCloud, keep: &[usize]) -> PointCloud {
    let names = attribute_names(cloud);
    let mut out = PointCloud::new();
    out.set_num_points(keep.len());
    for id in 0..cloud.num_attributes() {
        let source = cloud.attribute(id);
        let components = source.num_components();
        let mut attribute = PointAttribute::new();
        attribute.init(
            source.attribute_type(),
            components,
            source.data_type(),
            source.normalized(),
            keep.len(),
        );
        let buffer = attribute.buffer_mut();
        for (slot, &point) in keep.iter().enumerate() {
            let value_index = source.mapped_index(PointIndex(point as u32));
            for component in 0..components as usize {
                let value = read_f32(source, value_index.0 as usize, component);
                buffer.write(
                    (slot * components as usize + component) * 4,
                    &value.to_le_bytes(),
                );
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

fn encode(cloud: &PointCloud) -> usize {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_prediction_search(true);
    options.set_spatial_point_order(true);
    for id in 0..cloud.num_attributes() {
        let bits = match cloud.attribute(id).attribute_type() {
            GeometryAttributeType::Position => 16,
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

#[test]
#[ignore = "needs a scene in DRACO_SPLAT_PLY: run with --release --ignored --nocapture"]
fn what_the_invisible_splats_cost() {
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

    // Pre-sigmoid, as 3DGS stores it.
    let alphas: Vec<f32> = (0..num_points)
        .map(|point| {
            let value_index = opacity.mapped_index(PointIndex(point as u32));
            let logit = read_f32(opacity, value_index.0 as usize, 0);
            1.0 / (1.0 + (-logit).exp())
        })
        .collect();

    println!();
    println!(
        "{:<12} {:>12} {:>9} {:>14} {:>9}",
        "alpha below", "splats gone", "share", "encoded bytes", "vs all"
    );
    let whole = encode(&cloud);
    println!("{:<12} {:>12} {:>9} {whole:>14} {:>8.1}%", "-", 0, "-", 0.0);
    for threshold in [INVISIBLE, 0.01, 0.05, 0.1] {
        let kept: Vec<usize> = (0..num_points)
            .filter(|point| alphas[*point] >= threshold)
            .collect();
        let dropped = num_points - kept.len();
        let bytes = encode(&keep(&cloud, &kept));
        println!(
            "{threshold:<12.5} {dropped:>12} {:>8.1}% {bytes:>14} {:>+8.1}%",
            dropped as f64 / num_points as f64 * 100.0,
            (bytes as f64 / whole as f64 - 1.0) * 100.0
        );
    }

    println!();
    println!("  this is a size, and only a size: nothing here was rendered, so");
    println!("  whether dropping these changes an image is not measured");
}
