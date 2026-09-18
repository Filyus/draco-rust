//! Do the two point-cloud encoder options help anything other than a splat?
//!
//! `set_prediction_search` and `set_spatial_point_order` were both motivated
//! entirely by Gaussian splats, and neither knows the word: they act on any
//! point cloud. Whether they are a general feature of this crate or a feature
//! for data of one particular shape is the difference between documenting them
//! in the README and documenting them with a caveat, and it is not answerable
//! from the splat measurements.
//!
//! So this runs the same arms over a photogrammetry capture, which differs from
//! a splat in every way that might matter: two attributes instead of 58, seven
//! components instead of 62, colour instead of spherical harmonics, and points
//! that came from a scanner rather than from training.
//!
//! Needs a glTF or GLB holding `POINTS` primitives:
//!
//! ```text
//! DRACO_POINT_CLOUD_GLB=/path/to/cloud.glb \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-gltf --release \
//!   --test point_cloud_options_probe -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use draco_core::{
    DataType, EncoderBuffer, EncoderOptions, GeometryAttributeType, PointAttribute, PointCloud,
    PointCloudEncoder,
};
use draco_gltf::{ComponentType, MeshIndex, PrimitiveIndex, ValidationProfile};

/// Sequential, the coder a per-attribute budget reaches.
const SEQUENTIAL: i32 = 0;

/// What a caller encoding a coloured scan would actually ask for: positions
/// fine enough to keep the surface, colour at the byte it arrived as.
const POSITION_BITS: i32 = 14;
const COLOR_BITS: i32 = 8;

fn f32_values(attribute: &draco_gltf::PackedAttribute) -> Option<Vec<f32>> {
    // Only the float form is read here. A normalized byte colour would want
    // its own arm rather than a silent conversion, and this capture has none.
    if attribute.component_type() != ComponentType::F32 {
        return None;
    }
    Some(
        attribute
            .bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect(),
    )
}

/// Every `POINTS` primitive in the document, concatenated into one cloud.
///
/// The capture is tiled into hundreds of primitives that share one coordinate
/// space, so joining them is what makes a spatial order mean anything: sorting
/// inside a tile would only be re-sorting something already local.
fn load(path: &PathBuf) -> Option<(PointCloud, usize)> {
    let import = draco_gltf::open(path, ValidationProfile::Gltf20).expect("the file parses");
    let mesh_count = import.document.meshes().len();

    let mut positions: Vec<f32> = Vec::new();
    let mut colors: Vec<f32> = Vec::new();
    let mut color_components = 0u8;
    let mut primitives = 0usize;

    for mesh in 0..mesh_count {
        let index = MeshIndex(mesh);
        if import.document.mesh(index).is_none() {
            continue;
        }
        // A mesh does not publish its primitive count, so walk until the
        // document stops answering.
        let mut primitive = 0usize;
        while import.document.primitive(index, primitive).is_some() {
            let geometry = import
                .read_primitive(PrimitiveIndex::new(index, primitive))
                .expect("the primitive materializes");
            primitive += 1;
            if geometry.mode() != draco_gltf::PrimitiveMode::Points {
                continue;
            }
            primitives += 1;
            for attribute in geometry.attributes() {
                match attribute.semantic() {
                    "POSITION" => {
                        positions.extend(f32_values(attribute)?);
                    }
                    "COLOR_0" => {
                        color_components = attribute.components();
                        colors.extend(f32_values(attribute)?);
                    }
                    _ => {}
                }
            }
        }
    }

    let num_points = positions.len() / 3;
    if num_points == 0 {
        return None;
    }
    println!("{primitives} POINTS primitives, {num_points} points");

    let mut cloud = PointCloud::new();
    cloud.set_num_points(num_points);
    cloud.add_attribute(float_attribute(
        GeometryAttributeType::Position,
        3,
        &positions,
        num_points,
    ));
    if colors.len() == num_points * color_components as usize {
        cloud.add_attribute(float_attribute(
            GeometryAttributeType::Color,
            color_components,
            &colors,
            num_points,
        ));
    }
    Some((cloud, num_points))
}

fn float_attribute(
    kind: GeometryAttributeType,
    components: u8,
    values: &[f32],
    num_points: usize,
) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(kind, components, DataType::Float32, false, num_points);
    let buffer = attribute.buffer_mut();
    for (index, value) in values.iter().enumerate() {
        buffer.write(index * 4, &value.to_le_bytes());
    }
    attribute
}

fn encode(cloud: &PointCloud, search: bool, spatial: bool) -> usize {
    encode_full(cloud, search, spatial, None)
}

fn encode_full(
    cloud: &PointCloud,
    search: bool,
    spatial: bool,
    forced_prediction: Option<i32>,
) -> usize {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_prediction_search(search);
    options.set_spatial_point_order(spatial);
    if let Some(scheme) = forced_prediction {
        options.set_prediction_scheme(scheme);
    }
    options.set_attribute_int(0, "quantization_bits", POSITION_BITS);
    if cloud.num_attributes() > 1 {
        options.set_attribute_int(1, "quantization_bits", COLOR_BITS);
    }
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encodes");
    buffer.data().len()
}

#[test]
#[ignore = "needs a cloud in DRACO_POINT_CLOUD_GLB: run with --ignored --nocapture"]
fn the_options_on_a_scanned_cloud() {
    let Some(path) = std::env::var_os("DRACO_POINT_CLOUD_GLB").map(PathBuf::from) else {
        println!("DRACO_POINT_CLOUD_GLB is not set; nothing to measure");
        return;
    };
    let file_bytes = std::fs::metadata(&path).expect("the file exists").len();
    println!(
        "cloud: {} ({:.1} MB)",
        path.display(),
        file_bytes as f64 / 1e6
    );

    let Some((cloud, num_points)) = load(&path) else {
        println!("no POINTS primitives with float attributes; nothing to measure");
        return;
    };
    let components: usize = (0..cloud.num_attributes())
        .map(|id| cloud.attribute(id).num_components() as usize)
        .sum();
    let raw = components as f32 * 4.0;
    println!(
        "{} attributes, {components} components, raw {raw} B/point",
        cloud.num_attributes()
    );
    println!("budget: position {POSITION_BITS} bits, colour {COLOR_BITS} bits");
    println!();

    let per_point = |bytes: usize| bytes as f32 / num_points as f32;
    let base = per_point(encode(&cloud, false, false));
    let searched = per_point(encode(&cloud, true, false));
    let spatial = per_point(encode(&cloud, false, true));
    let both = per_point(encode(&cloud, true, true));

    println!("{:<30} {:>10} {:>9}", "arm", "B/point", "off");
    for (label, value) in [
        ("as encoded today", base),
        ("set_prediction_search(true)", searched),
        ("set_spatial_point_order(true)", spatial),
        ("both", both),
    ] {
        println!(
            "{label:<30} {value:>10.3} {:>8.1}%",
            (1.0 - value / base) * 100.0
        );
    }
    println!();
    println!(
        "  {:.2}x the raw floats, against {:.2}x today",
        raw / both,
        raw / base
    );

    // The control. "The search changed nothing" and "the search never ran"
    // print the same number, and only one of them is a result. Forcing
    // PREDICTION_NONE shows what the search was choosing between: if that is
    // worse than the default, the search kept the default because the default
    // was right, which is a finding. If it were better, the search would be
    // broken.
    println!();
    println!("  control: what the search was choosing between");
    let unpredicted = per_point(encode_full(&cloud, false, false, Some(-2)));
    println!("    difference-predicted (the default)  {base:>8.3} B/point");
    println!("    PREDICTION_NONE, forced             {unpredicted:>8.3} B/point");
    if unpredicted > base {
        println!(
            "    the default is better by {:.1}%, so finding nothing is the right answer",
            (unpredicted / base - 1.0) * 100.0
        );
    } else {
        println!("    WARNING: the candidate was smaller and the search did not take it");
    }
}
