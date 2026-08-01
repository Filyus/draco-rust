//! Deterministic counterparts to the `encode_drc` fuzz target.
//!
//! Every case here came from that campaign: geometry a caller can assemble from
//! a file some other library parsed, which the encoder used to answer with a
//! panic, an out-of-bounds read, or a bitstream its own decoder cannot read.
//! They run on stable CI with no fuzzing toolchain, so the fixes stay pinned
//! whether or not a campaign is run.

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{AttributeValueIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;

/// A float32 position attribute with `num_values` zeroed values.
fn positions(num_values: usize) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_values,
    );
    attribute
}

fn encode_mesh(mesh: Mesh, options: &EncoderOptions) -> Result<Vec<u8>, String> {
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    match encoder.encode(options, &mut buffer) {
        Ok(()) => Ok(buffer.data().to_vec()),
        Err(error) => Err(error.to_string()),
    }
}

fn encode_point_cloud(pc: PointCloud, options: &EncoderOptions) -> Result<Vec<u8>, String> {
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);
    let mut buffer = EncoderBuffer::new();
    match encoder.encode(options, &mut buffer) {
        Ok(()) => Ok(buffer.data().to_vec()),
        Err(error) => Err(error.to_string()),
    }
}

#[test]
fn zero_component_attribute_is_refused() {
    // The KD-tree coder takes the component count as its dimension and indexes
    // a per-axis array with it, so zero components indexed an empty array.
    let mut pc = PointCloud::new();
    pc.set_num_points(4);
    let mut attribute = PointAttribute::new();
    attribute.init(
        GeometryAttributeType::Position,
        0,
        DataType::Float32,
        false,
        4,
    );
    pc.add_attribute(attribute);

    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 8);
    let error = encode_point_cloud(pc, &options).expect_err("zero components must be refused");
    assert!(
        error.contains("zero components"),
        "unexpected error: {error}"
    );
}

#[test]
fn invalid_attribute_data_type_is_refused() {
    let mut pc = PointCloud::new();
    pc.set_num_points(2);
    let mut attribute = PointAttribute::new();
    attribute.init(
        GeometryAttributeType::Generic,
        1,
        DataType::Invalid,
        false,
        2,
    );
    pc.add_attribute(attribute);

    let error =
        encode_point_cloud(pc, &EncoderOptions::new()).expect_err("invalid type must be refused");
    assert!(
        error.contains("invalid data type"),
        "unexpected error: {error}"
    );
}

#[test]
fn attribute_shorter_than_the_point_count_is_refused() {
    // Identity mapping means point i reads value i, so an attribute with fewer
    // values than the geometry has points reads past its own buffer.
    let mut pc = PointCloud::new();
    pc.set_num_points(8);
    pc.add_attribute(positions(3));

    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 8);
    let error = encode_point_cloud(pc, &options).expect_err("short attribute must be refused");
    assert!(
        error.contains("holds 3 values for 8 points"),
        "unexpected error: {error}"
    );
}

#[test]
fn explicit_mapping_past_the_value_array_is_refused() {
    let mut pc = PointCloud::new();
    pc.set_num_points(4);
    let mut attribute = positions(2);
    attribute.set_explicit_mapping(4);
    for point in 0..4u32 {
        // The last entry is out of range; the two before it are fine.
        let value = if point == 3 { 9 } else { point % 2 };
        let _ = attribute.try_set_point_map_entry(PointIndex(point), AttributeValueIndex(value));
    }
    pc.add_attribute(attribute);

    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 8);
    let error = encode_point_cloud(pc, &options).expect_err("bad point map must be refused");
    assert!(error.contains("maps point 3"), "unexpected error: {error}");
}

#[test]
fn face_index_past_the_point_count_is_refused() {
    let mut mesh = Mesh::new();
    mesh.set_num_points(3);
    mesh.add_attribute(positions(3));
    mesh.set_num_faces(1);
    mesh.set_face_from_indices(0, [0, 1, 7]);

    let error =
        encode_mesh(mesh, &EncoderOptions::new()).expect_err("out-of-range face must be refused");
    assert!(
        error.contains("references point 7"),
        "unexpected error: {error}"
    );
}

#[test]
fn unsupported_versions_fail_instead_of_producing_an_unreadable_stream() {
    // Below the 1.0 compatibility floor, and above the newest mesh version this
    // crate writes. Both used to select a header layout piecemeal and emit a
    // stream nothing could read.
    for (major, minor) in [(0u8, 1u8), (3, 0), (2, 9)] {
        let mut mesh = Mesh::new();
        mesh.set_num_points(3);
        mesh.add_attribute(positions(3));
        mesh.set_num_faces(1);
        mesh.set_face_from_indices(0, [0, 1, 2]);

        let mut options = EncoderOptions::new();
        options.set_version(major, minor);
        let error = encode_mesh(mesh, &options)
            .expect_err(&format!("version {major}.{minor} must be refused"));
        assert!(
            error.contains("Cannot encode bitstream version"),
            "unexpected error for {major}.{minor}: {error}"
        );
    }
}

#[test]
fn a_point_cloud_encoded_at_version_1_0_decodes() {
    // The flags field is part of the header for every version this crate
    // encodes. Writing it only from 1.3 left a 1.0 stream two bytes short and
    // its own decoder read the point count from the wrong offset.
    let mut pc = PointCloud::new();
    pc.set_num_points(4);
    pc.add_attribute(positions(4));

    let mut options = EncoderOptions::new();
    options.set_version(1, 0);
    options.set_attribute_int(0, "quantization_bits", 8);
    let bytes = encode_point_cloud(pc, &options).expect("version 1.0 must encode");

    let mut decoded = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut DecoderBuffer::new(&bytes), &mut decoded)
        .expect("a stream this encoder produced must decode");
    assert_eq!(decoded.num_points(), 4);
}

#[test]
fn forced_predictive_traversal_is_refused_on_a_current_version() {
    let mut mesh = Mesh::new();
    mesh.set_num_points(3);
    mesh.add_attribute(positions(3));
    mesh.set_num_faces(1);
    mesh.set_face_from_indices(0, [0, 1, 2]);

    let mut options = EncoderOptions::new();
    options.set_global_int("force_predictive_traversal", 1);
    let error = encode_mesh(mesh, &options).expect_err("predictive traversal needs a < 2.0 target");
    assert!(
        error.contains("force_predictive_traversal"),
        "unexpected error: {error}"
    );
}

/// Empty geometry, under each prediction scheme. Prediction schemes
/// special-case entry 0, which an attribute with no values does not have; the
/// guards added for that are now also unreachable through the attribute
/// validation above, so this pins the surviving public behaviour rather than
/// the guard itself.
#[test]
fn empty_geometry_encodes_without_panicking() {
    for prediction_scheme in [-1i32, 0, 1, 2, 4, 5] {
        let mut mesh = Mesh::new();
        mesh.set_num_points(0);
        mesh.add_attribute(positions(0));

        let mut options = EncoderOptions::new();
        options.set_prediction_scheme(prediction_scheme);
        options.set_attribute_int(0, "quantization_bits", 8);
        // Either outcome is fine; the point is that it is an outcome.
        let bytes = encode_mesh(mesh, &options);
        if let Ok(bytes) = bytes {
            let mut decoded = Mesh::new();
            MeshDecoder::new()
                .decode(&mut DecoderBuffer::new(&bytes), &mut decoded)
                .unwrap_or_else(|error| {
                    panic!("empty mesh stream (scheme {prediction_scheme}) must decode: {error}")
                });
        }
    }
}
