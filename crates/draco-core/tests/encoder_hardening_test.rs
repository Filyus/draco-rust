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
use draco_core::keyframe_animation::KeyframeAnimation;
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

/// Two triangles with float32 positions.
fn quad() -> Mesh {
    let mut mesh = Mesh::new();
    let mut pos = PointAttribute::new();
    pos.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        4,
    );
    let coords: [f32; 12] = [0., 0., 0., 1., 0., 0., 0., 1., 0., 1., 1., 0.];
    for (i, value) in coords.iter().enumerate() {
        pos.buffer_mut().write(i * 4, &value.to_le_bytes());
    }
    mesh.set_num_points(4);
    mesh.add_attribute(pos);
    mesh.set_num_faces(2);
    mesh.set_face_from_indices(0, [0, 1, 2]);
    mesh.set_face_from_indices(1, [1, 3, 2]);
    mesh
}

/// The same quad with a texture coordinate, so the EdgeBreaker path builds
/// attribute connectivity the next encode must not inherit.
fn attributed_quad() -> Mesh {
    let mut mesh = quad();
    let mut uv = PointAttribute::new();
    uv.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Float32,
        false,
        4,
    );
    for i in 0..4 {
        uv.buffer_mut()
            .write(i * 8, &((i % 3) as f32).to_le_bytes());
        uv.buffer_mut()
            .write(i * 8 + 4, &((i % 2) as f32).to_le_bytes());
    }
    mesh.add_attribute(uv);
    mesh
}

/// The quad plus `num_generic` seamed generic attributes, each of which becomes
/// its own attribute-connectivity group.
fn seamed_quad(num_generic: usize) -> Mesh {
    let mut mesh = quad();
    for _ in 0..num_generic {
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Generic,
            1,
            DataType::Float32,
            false,
            2,
        );
        attribute.buffer_mut().write(0, &0.0f32.to_le_bytes());
        attribute.buffer_mut().write(4, &1.0f32.to_le_bytes());
        attribute.set_explicit_mapping(4);
        for point in 0..4u32 {
            let _ = attribute.try_set_point_map_entry(
                PointIndex(point),
                AttributeValueIndex(u32::from(point >= 2)),
            );
        }
        mesh.add_attribute(attribute);
    }
    mesh
}

/// A point cloud of `n` distinct float32 positions.
fn cloud(n: usize) -> PointCloud {
    let mut pc = PointCloud::new();
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        n,
    );
    for i in 0..n {
        let offset = i * 12;
        position
            .buffer_mut()
            .write(offset, &(i as f32).to_le_bytes());
        position
            .buffer_mut()
            .write(offset + 4, &(i as f32 * 0.5).to_le_bytes());
        position
            .buffer_mut()
            .write(offset + 8, &(n as f32).to_le_bytes());
    }
    pc.set_num_points(n);
    pc.add_attribute(position);
    pc
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
fn a_value_buffer_too_short_for_the_value_count_is_refused() {
    // The mapping check answers "is this value one of ours"; this one answers
    // "is that value in the buffer". `buffer_mut()` is public and its `resize`
    // with it, so a loader can truncate storage without touching `size()`.
    // Integer attributes never enter the quantization transform, whose own
    // bounds check would otherwise catch it, and go straight to the sequential
    // and KD-tree readers, which slice unchecked.
    for data_type in [DataType::Int32, DataType::Uint16, DataType::Int8] {
        let mut pc = PointCloud::new();
        pc.set_num_points(8);
        let mut attribute = PointAttribute::new();
        attribute.init(GeometryAttributeType::Position, 3, data_type, false, 8);
        attribute.buffer_mut().resize(12);
        pc.add_attribute(attribute);

        let error = encode_point_cloud(pc, &EncoderOptions::new())
            .expect_err(&format!("{data_type:?} truncated buffer must be refused"));
        assert!(
            error.contains("but its buffer holds 12"),
            "unexpected error for {data_type:?}: {error}"
        );
    }
}

#[test]
fn a_component_count_wider_than_the_stride_is_refused() {
    // `set_num_components` does not recompute the separately stored
    // `byte_stride`, so widening it after `init` makes every value read run
    // past the element the buffer was sized for.
    let mut pc = PointCloud::new();
    pc.set_num_points(4);
    let mut attribute = PointAttribute::new();
    attribute.init(GeometryAttributeType::Generic, 3, DataType::Int32, false, 4);
    attribute.set_num_components(6);
    pc.add_attribute(attribute);

    let error = encode_point_cloud(pc, &EncoderOptions::new())
        .expect_err("a stride narrower than the element must be refused");
    assert!(
        error.contains("12-byte stride for 24-byte values"),
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

#[test]
fn a_reused_encoder_encodes_what_a_fresh_one_does() {
    // Each encode caches connectivity for the attribute stage to read back, and
    // the sequential branch does not build a corner table - so after an
    // EdgeBreaker encode it inherited the previous mesh's one and wrote
    // attributes against topology the new stream does not describe. The result
    // decoded as a different mesh, or not at all.
    let mut edgebreaker = EncoderOptions::new();
    edgebreaker.set_global_int("encoding_method", 1);
    edgebreaker.set_attribute_int(0, "quantization_bits", 11);
    edgebreaker.set_attribute_int(1, "quantization_bits", 10);
    let mut sequential = EncoderOptions::new();
    sequential.set_global_int("encoding_method", 0);
    sequential.set_attribute_int(0, "quantization_bits", 11);

    let from_fresh = encode_mesh(quad(), &sequential).expect("fresh encode");

    let mut reused = MeshEncoder::new();
    reused.set_mesh(attributed_quad());
    let mut first = EncoderBuffer::new();
    reused
        .encode(&edgebreaker, &mut first)
        .expect("first encode");
    reused.set_mesh(quad());
    let mut second = EncoderBuffer::new();
    reused
        .encode(&sequential, &mut second)
        .expect("second encode");

    assert_eq!(
        from_fresh,
        second.data(),
        "a reused encoder produced a different stream than a fresh one"
    );
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(second.data()), &mut decoded)
        .expect("the reused encoder's stream must decode");
}

#[test]
fn a_reused_point_cloud_encoder_encodes_what_a_fresh_one_does() {
    // The mesh encoder carried a cache across encodes and got this wrong; the
    // point-cloud encoder holds no derived state, so it is right by
    // construction. Pinned anyway, because "by construction" is what changes
    // when someone adds a cache - and switching the method between the two
    // encodes is what made the mesh version fail.
    let mut sequential = EncoderOptions::new();
    sequential.set_global_int("encoding_method", 0);
    sequential.set_attribute_int(0, "quantization_bits", 11);
    let mut kd_tree = EncoderOptions::new();
    kd_tree.set_global_int("encoding_method", 1);
    kd_tree.set_attribute_int(0, "quantization_bits", 11);

    for (first, second) in [
        (&kd_tree, &sequential),
        (&sequential, &kd_tree),
        (&sequential, &sequential),
    ] {
        let mut fresh = PointCloudEncoder::new();
        fresh.set_point_cloud(cloud(6));
        let mut expected = EncoderBuffer::new();
        fresh.encode(second, &mut expected).expect("fresh encode");

        let mut reused = PointCloudEncoder::new();
        reused.set_point_cloud(cloud(20));
        let mut first_buffer = EncoderBuffer::new();
        reused
            .encode(first, &mut first_buffer)
            .expect("first encode");
        reused.set_point_cloud(cloud(6));
        let mut second_buffer = EncoderBuffer::new();
        reused
            .encode(second, &mut second_buffer)
            .expect("second encode");

        assert_eq!(
            expected.data(),
            second_buffer.data(),
            "a reused point-cloud encoder produced a different stream than a fresh one"
        );
    }
}

#[test]
fn a_tex_coord_prediction_scheme_is_refused_for_other_attributes() {
    // Both tex-coord predictors work on two components, and a normal presents
    // two once the octahedron transform has folded it from three - so a scheme
    // meant for UVs was accepted for normals and wrote values the normal
    // decoder cannot read back. Three-component attributes were already
    // refused, which is why only normals slipped through.
    for scheme in [3i32, 5] {
        let mut mesh = quad();
        let mut normal = PointAttribute::new();
        normal.init(
            GeometryAttributeType::Normal,
            3,
            DataType::Float32,
            false,
            4,
        );
        for i in 0..4 {
            normal.buffer_mut().write(i * 12 + 8, &1.0f32.to_le_bytes());
        }
        mesh.add_attribute(normal);

        let mut options = EncoderOptions::new();
        options.set_attribute_int(0, "quantization_bits", 10);
        options.set_attribute_int(1, "quantization_bits", 10);
        options.set_attribute_int(1, "prediction_scheme", scheme);

        let error = encode_mesh(mesh, &options)
            .expect_err(&format!("scheme {scheme} on a normal must be refused"));
        assert!(
            error.contains("predicts texture coordinates"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn more_attribute_groups_than_the_count_field_holds_is_refused() {
    // The group count is one byte, so 256 groups truncated to 0 and the decoder
    // read the following bytes as attribute data. The boundary is measured:
    // 255 groups still encode and decode, 256 are refused.
    for (num_generic, expect_ok) in [(254usize, true), (255, false)] {
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_method", 1);
        options.set_global_int("encoding_speed", 0);
        options.set_global_int("decoding_speed", 0);
        for id in 0..=num_generic {
            options.set_attribute_int(id as i32, "quantization_bits", 8);
        }

        let result = encode_mesh(seamed_quad(num_generic), &options);
        match (result, expect_ok) {
            (Ok(bytes), true) => {
                let mut decoded = Mesh::new();
                MeshDecoder::new()
                    .decode(&mut DecoderBuffer::new(&bytes), &mut decoded)
                    .expect("a stream with 255 groups must decode");
                assert_eq!(decoded.num_attributes(), num_generic as i32 + 1);
            }
            (Err(error), false) => assert!(
                error.contains("attribute groups but the bitstream field holds 255"),
                "unexpected error: {error}"
            ),
            (Ok(_), false) => panic!("{num_generic} generic attributes should have been refused"),
            (Err(error), true) => panic!("{num_generic} generic attributes must encode: {error}"),
        }
    }
}

#[test]
fn keyframe_tracks_that_do_not_fit_their_descriptor_are_refused() {
    // `add_keyframes` takes the component count and the scalar type as separate
    // parameters from the slice itself, and only those two size the buffer it
    // writes the slice into. Both disagreements used to write past it.
    let mut animation = KeyframeAnimation::new();
    assert!(animation.set_timestamps(&[0.0f32, 1.0]));

    // A count wider than the u8 the attribute stores it in.
    assert_eq!(
        animation.add_keyframes(DataType::Float32, 256, &[0.0f32; 512]),
        -1
    );
    // A scalar type that is not the element type of the slice.
    assert_eq!(animation.add_keyframes(DataType::Int8, 3, &[0.0f64; 6]), -1);
    // The agreeing case still works.
    assert!(animation.add_keyframes(DataType::Float32, 3, &[0.0f32; 6]) >= 0);
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
