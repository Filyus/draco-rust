//! Whether a Gaussian splat can live as a Draco point cloud, and what it costs.
//!
//! A splat is a position and some 56 further floats per point — scale,
//! rotation, opacity, and up to 45 spherical-harmonics coefficients. Nothing in
//! this crate reads one today: the PLY reader drops those properties and glTF
//! forbids a Draco bitstream on a `POINTS` primitive. What these tests pin is
//! the half that does exist, so that a decision about the rest is made against
//! the encoder's real behaviour rather than against an assumption about it.
//!
//! `cost_per_point_of_a_splat_shaped_cloud` is a measurement rather than a
//! test and is `#[ignore]`d; run it with `--ignored --nocapture`.

#![cfg(all(feature = "encoder", feature = "decoder"))]

use draco_core::{
    DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, GeometryAttributeType, Metadata,
    PointAttribute, PointCloud, PointCloudDecoder, PointCloudEncoder,
};

const NUM_POINTS: usize = 64;

/// How the probe's values are shaped.
///
/// A ramp is perfectly predictable and a compressor's best case; noise is its
/// worst. Real splat coefficients sit between, so running both brackets the
/// answer instead of quoting one synthetic number as if it were a measurement.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Ramp,
    Noise,
}

/// One attribute filled with distinct, recoverable values.
fn f32_attribute(
    kind: GeometryAttributeType,
    components: u8,
    seed: f32,
    shape: Shape,
) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(kind, components, DataType::Float32, false, NUM_POINTS);
    let buffer = attribute.buffer_mut();
    // A fixed LCG rather than a dependency: the numbers must be the same on
    // every run for the costs below to be comparable.
    let mut state = (seed as u32).wrapping_mul(2654435761).wrapping_add(1);
    for point in 0..NUM_POINTS {
        for component in 0..components as usize {
            let value = match shape {
                Shape::Ramp => seed + point as f32 + component as f32 * 0.25,
                Shape::Noise => {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    (state >> 8) as f32 / (1 << 24) as f32 * 4.0 - 2.0
                }
            };
            let offset = (point * components as usize + component) * 4;
            buffer.write(offset, &value.to_le_bytes());
        }
    }
    attribute
}

/// The attribute layout a splat needs, grouped the way the data is shaped.
fn splat_layout() -> Vec<(String, u8)> {
    let mut layout = vec![
        ("scale".to_string(), 3u8),
        ("rotation".to_string(), 4),
        ("opacity".to_string(), 1),
        ("f_dc".to_string(), 3),
    ];
    for i in 0..15 {
        layout.push((format!("sh_{i}"), 3));
    }
    layout
}

fn build_splat_cloud(layout: &[(String, u8)], with_names: bool, shape: Shape) -> PointCloud {
    let mut cloud = PointCloud::new();
    cloud.set_num_points(NUM_POINTS);
    cloud.add_attribute(f32_attribute(
        GeometryAttributeType::Position,
        3,
        0.0,
        shape,
    ));

    for (index, (name, components)) in layout.iter().enumerate() {
        let id = cloud.add_attribute(f32_attribute(
            GeometryAttributeType::Generic,
            *components,
            100.0 + index as f32,
            shape,
        ));
        if with_names {
            let unique_id = cloud.attribute(id).unique_id();
            let mut metadata = Metadata::new();
            metadata
                .set_string("name", name.clone())
                .expect("a name is a string entry");
            cloud
                .metadata_or_insert()
                .set_attribute_metadata(unique_id, metadata);
        }
    }
    cloud
}

fn round_trip(
    cloud: PointCloud,
    quantization: Option<i32>,
) -> Result<(Vec<u8>, PointCloud), String> {
    let attribute_count = cloud.num_attributes();
    let mut options = EncoderOptions::new();
    if let Some(bits) = quantization {
        for id in 0..attribute_count {
            options.set_attribute_int(id, "quantization_bits", bits);
        }
    }
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud);
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .map_err(|error| format!("encode failed: {error:?}"))?;

    let bytes = buffer.data().to_vec();
    let mut decoded = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut DecoderBuffer::new(&bytes), &mut decoded)
        .map_err(|error| format!("decode failed: {error:?}"))?;
    Ok((bytes, decoded))
}

#[test]
fn a_grouped_splat_layout_round_trips() {
    let layout = splat_layout();
    let cloud = build_splat_cloud(&layout, false, Shape::Ramp);
    let attributes = cloud.num_attributes();
    println!("grouped layout: {attributes} attributes, {NUM_POINTS} points");

    match round_trip(cloud, Some(14)) {
        Ok((bytes, decoded)) => {
            println!(
                "  encoded {} bytes; decoded {} points, {} attributes",
                bytes.len(),
                decoded.num_points(),
                decoded.num_attributes()
            );
            assert_eq!(decoded.num_points(), NUM_POINTS);
            assert_eq!(decoded.num_attributes(), attributes);
        }
        Err(error) => panic!("grouped splat does not round trip -- {error}"),
    }
}

#[test]
fn a_flat_per_property_layout_round_trips() {
    // What a PLY reader carrying one attribute per property would produce:
    // 3 scale + 4 rot + 1 opacity + 3 dc + 45 rest = 56 scalars, plus position.
    let layout: Vec<(String, u8)> = (0..56).map(|i| (format!("prop_{i}"), 1u8)).collect();
    let cloud = build_splat_cloud(&layout, false, Shape::Ramp);
    let attributes = cloud.num_attributes();
    println!("flat layout: {attributes} attributes");

    match round_trip(cloud, Some(14)) {
        Ok((bytes, decoded)) => {
            println!(
                "  encoded {} bytes; decoded {} attributes",
                bytes.len(),
                decoded.num_attributes()
            );
            assert_eq!(decoded.num_attributes(), attributes);
        }
        Err(error) => panic!("flat layout does not round trip -- {error}"),
    }
}

#[test]
fn generic_attribute_names_survive_the_round_trip() {
    let layout = splat_layout();
    let cloud = build_splat_cloud(&layout, true, Shape::Ramp);
    let expected: Vec<String> = layout.iter().map(|(name, _)| name.clone()).collect();

    let (_, decoded) = round_trip(cloud, Some(14)).expect("round trip");

    let mut recovered = Vec::new();
    for id in 0..decoded.num_attributes() {
        let unique_id = decoded.attribute(id).unique_id();
        if let Some(metadata) = decoded.attribute_metadata_by_unique_id(unique_id) {
            if let Some(name) = metadata.metadata().get_string("name") {
                recovered.push(name.to_string());
            }
        }
    }
    println!("names recovered: {} of {}", recovered.len(), expected.len());
    println!("  {recovered:?}");
    assert_eq!(recovered, expected, "attribute names do not survive");
}

#[test]
#[ignore = "a measurement, not a test: run with --ignored --nocapture"]
fn cost_per_point_of_a_splat_shaped_cloud() {
    // The number the feasibility question turns on: a splat is 59 floats, so
    // 236 bytes per point stored raw. Anything near that is not worth doing.
    const RAW_BYTES_PER_POINT: f32 = 59.0 * 4.0;

    for (label, layout) in [
        ("grouped", splat_layout()),
        (
            "flat",
            (0..56).map(|i| (format!("prop_{i}"), 1u8)).collect(),
        ),
    ] {
        for (shape, shape_label) in [(Shape::Ramp, "ramp"), (Shape::Noise, "noise")] {
            for bits in [None, Some(16), Some(14), Some(11), Some(8)] {
                let cloud = build_splat_cloud(&layout, false, shape);
                let (bytes, _) = round_trip(cloud, bits).expect("round trip");
                let per_point = bytes.len() as f32 / NUM_POINTS as f32;
                let quantization = match bits {
                    Some(bits) => format!("{bits} bits"),
                    None => "lossless".to_string(),
                };
                println!(
                    "COST {label:<8} {shape_label:<6} {quantization:<9} {per_point:>7.1} B/point  \
                     {:>5.1}% of raw",
                    per_point / RAW_BYTES_PER_POINT * 100.0
                );
            }
        }
    }
}

#[test]
fn an_unquantized_generic_attribute_is_byte_identical() {
    let layout = vec![("opacity".to_string(), 1u8), ("scale".to_string(), 3)];
    let cloud = build_splat_cloud(&layout, false, Shape::Ramp);
    let before: Vec<u8> = cloud.attribute(1).buffer().data().to_vec();

    let (_, decoded) = round_trip(cloud, None).expect("round trip without quantization");
    let after: Vec<u8> = decoded.attribute(1).buffer().data().to_vec();

    println!(
        "unquantized generic attribute: {} bytes in, {} bytes out, identical: {}",
        before.len(),
        after.len(),
        before == after
    );
    assert_eq!(before, after, "lossless path is not lossless");
}

/// The whole path, from a splat PLY to a Draco point cloud and back.
///
/// Each link is covered on its own elsewhere; this is the one test that fails
/// if any of them stops meeting the next.
#[test]
fn a_splat_ply_survives_the_round_trip_to_a_draco_point_cloud() {
    use draco_io::ply_reader::PlyReader;

    // Three splats, degree-0 spherical harmonics, in the property order the
    // original 3DGS exporter writes.
    let mut ply = String::from(
        "ply\nformat ascii 1.0\nelement vertex 3\n\
         property float x\nproperty float y\nproperty float z\n\
         property float f_dc_0\nproperty float f_dc_1\nproperty float f_dc_2\n\
         property float opacity\n\
         property float scale_0\nproperty float scale_1\nproperty float scale_2\n\
         property float rot_0\nproperty float rot_1\nproperty float rot_2\nproperty float rot_3\n\
         end_header\n",
    );
    for i in 0..3 {
        let f = i as f32;
        ply.push_str(&format!(
            "{f} 0 0  {} {} {}  {}  -2.5 -0.5 -0.2  1 0 0 0\n",
            1.0 + f,
            2.0 + f,
            3.0 + f,
            -1.0 - f
        ));
    }

    let (mesh, report) = PlyReader::from_bytes(ply.into_bytes())
        .with_generic_attributes(true)
        .read_mesh_reporting_loss()
        .expect("a splat PLY parses");
    assert!(
        report.is_lossless(),
        "everything the file declared is carried: {:?}",
        report.dropped()
    );
    // Position plus the eleven splat properties, one attribute each.
    assert_eq!(mesh.num_attributes(), 12);

    let cloud = mesh.into_point_cloud();
    let names_before = attribute_names(&cloud);
    assert!(names_before.contains(&"f_dc_0".to_string()));
    assert!(names_before.contains(&"rot_3".to_string()));

    let (bytes, decoded) = round_trip(cloud, Some(16)).expect("encodes and decodes");
    assert_eq!(decoded.num_points(), 3);
    assert_eq!(decoded.num_attributes(), 12);
    assert_eq!(
        attribute_names(&decoded),
        names_before,
        "every property keeps its name through the bitstream"
    );
    println!(
        "splat PLY -> drc: {} bytes for 3 points across {} attributes",
        bytes.len(),
        decoded.num_attributes()
    );
}

/// Names of the attributes that carry one, in attribute order.
fn attribute_names(cloud: &PointCloud) -> Vec<String> {
    (0..cloud.num_attributes())
        .filter_map(|id| {
            let unique_id = cloud.attribute(id).unique_id();
            cloud
                .attribute_metadata_by_unique_id(unique_id)?
                .metadata()
                .get_string("name")
                .map(str::to_string)
        })
        .collect()
}
