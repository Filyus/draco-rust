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

/// Versions below the claimed floor are refused rather than written.
///
/// This replaces a test that encoded a point cloud at 1.0 and asserted it
/// decoded. It passed for the wrong reason: it checked only the point count,
/// which survives a stream whose quantization parameters were written in the
/// wrong place. 1.0 through 1.2 are no longer claimed for point clouds, so the
/// honest assertion is that they are refused.
#[test]
fn point_cloud_versions_below_the_claimed_floor_are_refused() {
    for (major, minor) in [(1u8, 0u8), (1, 1), (1, 2)] {
        let mut pc = PointCloud::new();
        pc.set_num_points(4);
        pc.add_attribute(positions(4));

        let mut options = EncoderOptions::new();
        options.set_version(major, minor);
        options.set_attribute_int(0, "quantization_bits", 8);

        let error = encode_point_cloud(pc, &options)
            .expect_err(&format!("version {major}.{minor} must be refused"));
        assert!(
            error.contains("Cannot encode bitstream version"),
            "unexpected error for {major}.{minor}: {error}"
        );
    }
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

/// A normal quantization bit count the octahedron transform cannot carry is
/// refused, and the refusal names the value and the range.
///
/// The normal encoder checked `>= 1` and then discarded the transform's own
/// answer, which accepts only 2..=30. A count of 1 or above 30 therefore left
/// the transform uninitialized while `init` reported success; the encode still
/// failed, but later and with nothing pointing at the bit count.
#[test]
fn a_normal_quantization_bit_count_the_octahedron_cannot_carry_is_refused() {
    for bits in [1i32, 31] {
        let mut mesh = quad();
        let mut normals = PointAttribute::new();
        normals.init(
            GeometryAttributeType::Normal,
            3,
            DataType::Float32,
            false,
            4,
        );
        for i in 0..4 {
            normals.buffer_mut().write(i * 12, &0.0f32.to_le_bytes());
            normals
                .buffer_mut()
                .write(i * 12 + 4, &0.0f32.to_le_bytes());
            normals
                .buffer_mut()
                .write(i * 12 + 8, &1.0f32.to_le_bytes());
        }
        mesh.add_attribute(normals);

        let mut options = EncoderOptions::new();
        options.set_attribute_int(0, "quantization_bits", 14);
        options.set_attribute_int(1, "quantization_bits", bits);

        let error =
            encode_mesh(mesh, &options).expect_err("a bit count outside 2..=30 must be refused");
        assert!(
            error.contains("2..=30"),
            "the refusal should name the range the bit count fell outside, got: {error}"
        );
    }
}

/// The wrap prediction transform stores `1 + (max - min)` of the raw
/// attribute values in an i32 (see `PredictionSchemeWrapEncodingTransform::init`
/// in `prediction_scheme_wrap.rs`), and its own decoder refuses any stream
/// whose span does not fit that i32 -- the matching check in
/// `decode_transform_data`. An unquantized (explicit) integer attribute can
/// legitimately carry raw values spanning close to the full i32 range -- an
/// application-defined `Generic` attribute is under no obligation to look like
/// a normal or a quantized position -- and that used to reach the wrap
/// transform anyway: the encoder's `1 + dif` wrapped in i32 arithmetic, wrote
/// out the (still huge) min/max it started from regardless, and its own
/// decoder then refused the stream it had just produced -- the "anything the
/// encoder accepts must decode" oracle broken by the encoder's own output.
#[test]
fn an_integer_attribute_spanning_close_to_the_full_i32_range_is_encoded_without_a_wrap_transform() {
    let mut pc = PointCloud::new();
    pc.set_num_points(4);
    let mut attribute = PointAttribute::new();
    attribute.init(GeometryAttributeType::Generic, 1, DataType::Int32, false, 4);
    // i32::MIN..=i32::MAX has a span of `u32::MAX` values, one past what
    // `1 + dif` can hold in an i32 -- the narrowest input that exercises the
    // overflow.
    let values: [i32; 4] = [i32::MIN, i32::MAX, 0, -1];
    for (i, value) in values.iter().enumerate() {
        attribute.buffer_mut().write(i * 4, &value.to_le_bytes());
    }
    pc.add_attribute(attribute);

    // Sequential, asked for. An integer attribute is otherwise eligible for
    // the KD-tree coder by default, which sorts points by attribute value and
    // so gives no guarantee that decoded point `i` is encoded point `i` --
    // the sequential path this test means to exercise is the one that keeps
    // that identity and is the one `SequentialIntegerAttributeEncoder` (and
    // the wrap transform inside it) actually belongs to.
    let mut options = EncoderOptions::new();
    options.set_encoding_method(0);
    let stream =
        encode_point_cloud(pc, &options).expect("a wide-range integer attribute must still encode");

    let mut decoded = PointCloud::new();
    let status = PointCloudDecoder::new().decode(&mut DecoderBuffer::new(&stream), &mut decoded);
    assert!(
        status.is_ok(),
        "the encoder's own stream must decode: {:?}",
        status.err()
    );

    let out_att = decoded.attribute(0);
    let buffer = out_att.buffer();
    for (i, expected) in values.iter().enumerate() {
        let mut bytes = [0u8; 4];
        buffer.read(i * 4, &mut bytes);
        assert_eq!(
            i32::from_le_bytes(bytes),
            *expected,
            "value {i} round-tripped incorrectly"
        );
    }
}

/// A one-dimensional KD-tree over values that reach the top of the `u32`
/// range, with few enough distinct values that the groups stay large.
///
/// `bit_length` is then 32, the walk splits 32 times along its single axis,
/// and the leaf it arrives at sits on the last row the stacks hold -- which is
/// the row they are sized for. The decoder refused it anyway, because it asked
/// for room for a child row before deciding whether the node had a child, so
/// an encode the encoder was happy with came back as "KD-tree traversal failed
/// after 0 of 64 points". Minimized input in
/// `fuzz/seeds/encode_drc/kd_tree_full_range_single_component.bin`.
#[test]
fn a_full_range_single_component_kd_tree_round_trips() {
    // Thirteen distinct values spanning the whole `u32` range: the spread is
    // what drives `bit_length` to 32, and the repetition is what keeps every
    // node above two points until the walk is out of levels.
    const VALUES: [u32; 13] = [
        0,
        35,
        255,
        458_752,
        16_777_188,
        17_442_815,
        603_979_776,
        989_395_192,
        2_969_565_210,
        4_177_066_232,
        4_279_894_016,
        4_294_965_325,
        u32::MAX,
    ];
    let num_points = 64usize;

    let mut point_cloud = PointCloud::new();
    point_cloud.set_num_points(num_points);
    let mut attribute = PointAttribute::new();
    attribute.init(
        GeometryAttributeType::Position,
        1,
        DataType::Uint32,
        false,
        num_points,
    );
    let mut expected: Vec<u32> = (0..num_points).map(|i| VALUES[i % VALUES.len()]).collect();
    for (index, value) in expected.iter().enumerate() {
        attribute
            .buffer_mut()
            .write(index * 4, &value.to_le_bytes());
    }
    point_cloud.add_attribute(attribute);

    let mut options = EncoderOptions::new();
    options.set_encoding_method(1);
    let encoded = encode_point_cloud(point_cloud, &options).expect("encode");

    let mut decoded = PointCloud::new();
    let mut buffer = DecoderBuffer::new(&encoded);
    PointCloudDecoder::new()
        .decode(&mut buffer, &mut decoded)
        .expect("a stream this encoder produced must decode");
    assert_eq!(decoded.num_points(), num_points);

    // A KD-tree encode reorders points, so the multiset is what survives.
    let attribute = decoded.attribute(0);
    let mut round_tripped: Vec<u32> = (0..num_points)
        .map(|index| {
            let mut bytes = [0u8; 4];
            attribute.buffer().read(index * 4, &mut bytes);
            u32::from_le_bytes(bytes)
        })
        .collect();
    round_tripped.sort_unstable();
    expected.sort_unstable();
    assert_eq!(round_tripped, expected);
}

/// A normal attribute beside a position that is not a `vec3`.
///
/// Every scheme that predicts from position -- geometric normal and both
/// tex-coord schemes -- refuses a parent that is not a three-component
/// `Position`, and refuses it in `set_parent_attribute`, which runs on the
/// decode side. Nothing checked it at selection time, and nothing checked a
/// scheme the caller asked for by number either, so this encoded happily and
/// came back as `Failed to set parent attribute for GeometricNormal` from the
/// crate's own decoder. Found by `encode_drc`; minimized input in
/// `fuzz/seeds/encode_drc/geometric_normal_without_a_vec3_position.bin`.
#[test]
fn a_normal_predicted_from_a_one_component_position_still_round_trips() {
    const NUM_POINTS: usize = 12;

    let mut mesh = Mesh::new();

    // Position with one component, which is what makes it unusable as a
    // prediction parent while remaining a perfectly encodable attribute.
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        1,
        DataType::Int16,
        false,
        NUM_POINTS,
    );
    for index in 0..NUM_POINTS {
        let value = (index as i16) * 37;
        position.buffer_mut().write(index * 2, &value.to_le_bytes());
    }
    mesh.add_attribute(position);

    let mut normal = PointAttribute::new();
    normal.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        true,
        NUM_POINTS,
    );
    for index in 0..NUM_POINTS {
        let angle = index as f32 * 0.5;
        for (component, value) in [angle.cos(), angle.sin(), 0.0].iter().enumerate() {
            normal
                .buffer_mut()
                .write((index * 3 + component) * 4, &value.to_le_bytes());
        }
    }
    mesh.add_attribute(normal);

    mesh.set_num_faces(NUM_POINTS / 3);
    for face in 0..NUM_POINTS / 3 {
        let base = (face * 3) as u32;
        mesh.set_face(
            draco_core::geometry_indices::FaceIndex(face as u32),
            [base.into(), (base + 1).into(), (base + 2).into()],
        );
    }

    // Both ways the geometric normal scheme can be reached: asked for by
    // number, and left to the encoder to choose.
    for explicit in [true, false] {
        let mut options = EncoderOptions::new();
        options.set_attribute_int(1, "quantization_bits", 10);
        if explicit {
            options.set_attribute_int(1, "prediction_scheme", 6);
        }

        let encoded = match encode_mesh(mesh.clone(), &options) {
            Ok(bytes) => bytes,
            // Refusing to encode is a fine answer; writing a stream the
            // decoder cannot read is not.
            Err(_) => continue,
        };
        let mut decoded = Mesh::new();
        MeshDecoder::new()
            .decode(&mut DecoderBuffer::new(&encoded), &mut decoded)
            .unwrap_or_else(|error| {
                panic!("explicit={explicit}: encoder wrote a stream its decoder rejects: {error}")
            });
        assert_eq!(decoded.num_faces(), NUM_POINTS / 3);
    }
}

/// An octahedron-folded normal only round-trips under a prediction scheme that
/// carries the octahedron transform. Asking for a parallelogram one instead
/// used to be honoured, and below bitstream 2.0 that moved the octahedron's
/// bit count out of the stream entirely -- the byte rides between the
/// prediction header and the values, and it was written only when the header
/// said octahedron. The decoder then read the first value byte as a bit count
/// and refused the file its own encoder had just written.
///
/// Every scheme, not just the one the fuzzer happened to name, and both
/// bitstream eras: 2.0 keeps the byte after the values, so it fails
/// differently or not at all, which is exactly why one version is not a test.
#[test]
fn a_normal_round_trips_whichever_prediction_scheme_is_asked_for() {
    const NUM_POINTS: usize = 96;

    let mut mesh = Mesh::new();
    mesh.set_num_points(NUM_POINTS);

    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        NUM_POINTS,
    );
    for index in 0..NUM_POINTS {
        let angle = index as f32 * 0.37;
        for (component, value) in [angle.cos(), angle.sin(), index as f32].iter().enumerate() {
            position
                .buffer_mut()
                .write((index * 3 + component) * 4, &value.to_le_bytes());
        }
    }
    mesh.add_attribute(position);

    let mut normal = PointAttribute::new();
    normal.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        true,
        NUM_POINTS,
    );
    for index in 0..NUM_POINTS {
        let angle = index as f32 * 0.11;
        for (component, value) in [angle.cos(), angle.sin(), 0.0].iter().enumerate() {
            normal
                .buffer_mut()
                .write((index * 3 + component) * 4, &value.to_le_bytes());
        }
    }
    mesh.add_attribute(normal);

    mesh.set_num_faces(NUM_POINTS / 3);
    for face in 0..NUM_POINTS / 3 {
        let base = (face * 3) as u32;
        mesh.set_face(
            draco_core::geometry_indices::FaceIndex(face as u32),
            [base.into(), (base + 1).into(), (base + 2).into()],
        );
    }

    for version in [(1u8, 1u8), (2, 2)] {
        for scheme in 0..=6 {
            let mut options = EncoderOptions::new();
            options.set_version(version.0, version.1);
            options.set_attribute_int(0, "quantization_bits", 11);
            options.set_attribute_int(1, "quantization_bits", 9);
            options.set_attribute_int(1, "prediction_scheme", scheme);

            let encoded = match encode_mesh(mesh.clone(), &options) {
                Ok(bytes) => bytes,
                // Refusing to encode is a fine answer; writing a stream the
                // decoder cannot read is not.
                Err(_) => continue,
            };
            let mut decoded = Mesh::new();
            MeshDecoder::new()
                .decode(&mut DecoderBuffer::new(&encoded), &mut decoded)
                .unwrap_or_else(|error| {
                    panic!(
                        "version {version:?}, prediction scheme {scheme}:                          encoder wrote a stream its decoder rejects: {error}"
                    )
                });
            assert_eq!(decoded.num_faces(), NUM_POINTS / 3);
        }
    }
}

/// The deprecated texture-coordinate scheme predicts nothing on some meshes,
/// and a prediction that produced no orientations writes a count of zero --
/// which is exactly where its decoder stops, here and upstream. The encoder
/// used to write it anyway and refuse its own file.
///
/// The mesh is what the `encode_drc` campaign reduced to: four faces over a
/// point count far larger than they use, so the traversal reaches almost none
/// of the values and no orientation comes out.
#[test]
fn a_texture_coordinate_scheme_that_predicts_nothing_still_round_trips() {
    const NUM_POINTS: usize = 308;
    const NUM_FACES: usize = 4;

    let mut mesh = Mesh::new();
    mesh.set_num_points(NUM_POINTS);

    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        NUM_POINTS,
    );
    for index in 0..NUM_POINTS {
        let angle = index as f32 * 0.21;
        for (component, value) in [angle.cos(), angle.sin(), index as f32 * 0.01]
            .iter()
            .enumerate()
        {
            position
                .buffer_mut()
                .write((index * 3 + component) * 4, &value.to_le_bytes());
        }
    }
    mesh.add_attribute(position);

    let mut tex_coord = PointAttribute::new();
    tex_coord.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Float32,
        false,
        NUM_POINTS,
    );
    // One texture coordinate for every point. The prediction pushes an
    // orientation only where a triangle's two already-coded corners carry
    // *different* coordinates, so a uniformly mapped attribute produces none at
    // all -- which is a perfectly ordinary thing for a mesh to have.
    for index in 0..NUM_POINTS {
        for (component, value) in [0.25f32, 0.75].iter().enumerate() {
            tex_coord
                .buffer_mut()
                .write((index * 2 + component) * 4, &value.to_le_bytes());
        }
    }
    mesh.add_attribute(tex_coord);

    mesh.set_num_faces(NUM_FACES);
    for face in 0..NUM_FACES {
        let base = (face * 3) as u32;
        mesh.set_face(
            draco_core::geometry_indices::FaceIndex(face as u32),
            [base.into(), (base + 1).into(), (base + 2).into()],
        );
    }

    let encode_with_scheme = |scheme: i32, version: (u8, u8)| {
        let mut options = EncoderOptions::new();
        options.set_version(version.0, version.1);
        options.set_attribute_int(0, "quantization_bits", 14);
        options.set_attribute_int(1, "quantization_bits", 12);
        options.set_attribute_int(1, "prediction_scheme", scheme);
        encode_mesh(mesh.clone(), &options)
    };

    for version in [(1u8, 1u8), (2, 2)] {
        // 3 is the deprecated scheme, by number: nothing selects it
        // automatically. 0 is the difference scheme the downgrade lands on.
        let Ok(encoded) = encode_with_scheme(3, version) else {
            continue;
        };
        let mut decoded = Mesh::new();
        MeshDecoder::new()
            .decode(&mut DecoderBuffer::new(&encoded), &mut decoded)
            .unwrap_or_else(|error| {
                panic!("version {version:?}: encoder wrote a stream its decoder rejects: {error}")
            });
        assert_eq!(decoded.num_faces(), NUM_FACES);

        // The downgrade is not a lossy escape hatch: the stream is the one
        // asking for the difference scheme outright would have produced, byte
        // for byte, so it round-trips exactly as that one does. The header
        // agrees too, since it is written from the method the downgrade left
        // behind rather than the one that was asked for.
        let difference = encode_with_scheme(0, version)
            .unwrap_or_else(|error| panic!("version {version:?}: difference encode: {error}"));
        assert_eq!(
            encoded, difference,
            "version {version:?}: the downgraded stream is not the difference one"
        );

        let mut expected = Mesh::new();
        MeshDecoder::new()
            .decode(&mut DecoderBuffer::new(&difference), &mut expected)
            .expect("difference decode");
        let decoded_uv = decoded.attribute(1).buffer().data().to_vec();
        let expected_uv = expected.attribute(1).buffer().data().to_vec();
        assert_eq!(
            decoded_uv, expected_uv,
            "version {version:?}: texture coordinates differ after the downgrade"
        );
    }
}

/// A scheme that predicts from the position does not survive single
/// connectivity, and the encoder must not choose one there.
///
/// With one corner table over the point indices, the table's vertices are
/// points rather than attribute values; the encoder's traversal then names
/// points the decoder's does not, and the two predict from different
/// positions. Five points and two triangles are enough -- the encoder's point
/// order came out `[3, 4, 1, 1, 2, 0]` against the decoder's `[1, 2, 0, 4, 5,
/// 3]`, and the tex-coord predictor ran out of orientations reading it back.
///
/// `SelectPredictionMethod` never pairs the two: every parent-reading scheme
/// it picks needs speed below 4, and single connectivity starts at 6. Only an
/// explicit `prediction_scheme` reaches the pairing, which is what the
/// `encode_drc` campaign did twice.
#[test]
fn a_position_predicting_scheme_is_not_chosen_under_single_connectivity() {
    const NUM_POINTS: usize = 6;

    let mut mesh = Mesh::new();
    mesh.set_num_points(NUM_POINTS);

    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Uint16,
        true,
        NUM_POINTS,
    );
    for index in 0..NUM_POINTS {
        let value = if index == 1 { 0u16 } else { 48316 };
        for component in 0..3 {
            position
                .buffer_mut()
                .write((index * 3 + component) * 2, &value.to_le_bytes());
        }
    }
    mesh.add_attribute(position);

    let mut tex_coord = PointAttribute::new();
    tex_coord.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Uint16,
        false,
        NUM_POINTS,
    );
    for index in 0..NUM_POINTS {
        let value: u16 = if (1..4).contains(&index) { 48316 } else { 0 };
        for component in 0..2 {
            tex_coord
                .buffer_mut()
                .write((index * 2 + component) * 2, &value.to_le_bytes());
        }
    }
    mesh.add_attribute(tex_coord);

    mesh.set_num_faces(2);
    for (index, face) in [[0u32, 1, 2], [1, 3, 4]].iter().enumerate() {
        mesh.set_face(
            draco_core::geometry_indices::FaceIndex(index as u32),
            [face[0].into(), face[1].into(), face[2].into()],
        );
    }

    // Both ways single connectivity is reached: by speed, and by asking for it.
    for (speed, split) in [(6i32, -1i32), (5, 1), (2, 1)] {
        // 5 is the portable tex-coord scheme, 6 the geometric normal; both
        // name the position as their parent.
        for scheme in [5, 6] {
            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_speed", speed);
            options.set_global_int("decoding_speed", speed);
            if split >= 0 {
                options.set_global_int("split_mesh_on_seams", split);
            }
            options.set_attribute_int(1, "quantization_bits", 12);
            options.set_attribute_int(1, "prediction_scheme", scheme);

            let encoded = match encode_mesh(mesh.clone(), &options) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let mut decoded = Mesh::new();
            MeshDecoder::new()
                .decode(&mut DecoderBuffer::new(&encoded), &mut decoded)
                .unwrap_or_else(|error| {
                    panic!(
                        "speed {speed}, split {split}, scheme {scheme}: encoder wrote a stream its decoder rejects: {error}"
                    )
                });
            assert_eq!(decoded.num_faces(), 2);
        }
    }
}

/// An integral position used as a prediction parent needs its portable form's
/// point map rebuilt, exactly as a quantized one does.
///
/// The values are written in encoding order while a predictor reads its parent
/// as `mapped_index(point_id)`. Left as the identity, the encoder reads the
/// entry sitting at the point's own index and the decoder -- whose parent
/// carries the rebuilt map -- reads a different one. It shows only when the two
/// orders differ, which coincident positions cause: the corner table merges
/// those points into one vertex, so the traversal order stops matching the
/// attribute's own.
#[test]
fn an_integral_prediction_parent_carries_the_traversal_order() {
    const NUM_POINTS: usize = 6;
    // Four of the six share a position, which is what makes the encoding order
    // differ from the attribute's own.
    const POSITIONS: [[u16; 3]; NUM_POINTS] = [
        [0, 0, 0],
        [0, 0, 0],
        [0, 0, 0],
        [12032, 0, 0],
        [47545, 47545, 185],
        [0, 0, 0],
    ];
    const TEX_COORDS: [[u16; 2]; NUM_POINTS] = [[0, 0], [0, 0], [0, 0], [0, 47], [0, 0], [0, 0]];

    let mut mesh = Mesh::new();
    mesh.set_num_points(NUM_POINTS);

    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Uint16,
        true,
        NUM_POINTS,
    );
    for (index, value) in POSITIONS.iter().enumerate() {
        for (component, v) in value.iter().enumerate() {
            position
                .buffer_mut()
                .write((index * 3 + component) * 2, &v.to_le_bytes());
        }
    }
    mesh.add_attribute(position);

    let mut tex_coord = PointAttribute::new();
    tex_coord.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Uint16,
        false,
        NUM_POINTS,
    );
    for (index, value) in TEX_COORDS.iter().enumerate() {
        for (component, v) in value.iter().enumerate() {
            tex_coord
                .buffer_mut()
                .write((index * 2 + component) * 2, &v.to_le_bytes());
        }
    }
    mesh.add_attribute(tex_coord);

    mesh.set_num_faces(2);
    for (index, face) in [[0u32, 1, 2], [1, 3, 4]].iter().enumerate() {
        mesh.set_face(
            draco_core::geometry_indices::FaceIndex(index as u32),
            [face[0].into(), face[1].into(), face[2].into()],
        );
    }

    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 20);
    options.set_attribute_int(1, "quantization_bits", 23);
    options.set_attribute_int(1, "prediction_scheme", 5);

    let encoded = encode_mesh(mesh, &options).expect("encode");
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(&encoded), &mut decoded)
        .expect("encoder wrote a stream its decoder rejects");
    assert_eq!(decoded.num_faces(), 2);
}

/// A position no `i64` can hold is not a position to predict from.
///
/// The portable texture-coordinate scheme reads its parent as integers. Its
/// decoding half refuses a component that does not convert -- one past the
/// `i64` range, or non-finite -- and stops the decode. Its encoding half stood
/// in zeros for the same value and carried on, so the encoder predicted from a
/// position the decoder never reconstructs and wrote a stream its own decoder
/// rejects.
///
/// The mesh is what the `encode_drc` campaign reduced to: two triangles
/// sharing an edge, a third one apart from them and a degenerate fourth, over
/// positions whose bytes read back as a mix of subnormals and `1e34`.
#[test]
fn a_position_outside_the_integer_range_is_not_predicted_from() {
    const NUM_POINTS: usize = 8;

    const POSITIONS: [[f64; 3]; NUM_POINTS] = [
        [0.0, 3.925692213286e-312, 1.0384593717069655e34],
        [0.0, 5.918037e-318, 0.0],
        [3.925692213286e-312, 1.0384593717069655e34, 9e-323],
        [3.560118173611522e-305, 0.0, 5.918037e-318],
        [1.0384593717069655e34, 9e-323, 3.560118173611522e-305],
        [5.990131e-317, 2.5417774863171863e-308, 0.0],
        [9.14e-322, 3.87844465075e-313, 0.0],
        [1.0384593717069655e34, 9e-323, 3.560118173611522e-305],
    ];
    const TEX_COORDS: [[u16; 2]; NUM_POINTS] = [
        [0, 0],
        [0, 4679],
        [0, 0],
        [47360, 0],
        [0, 0],
        [0, 0],
        [0, 0],
        [0, 0],
    ];

    let mut mesh = Mesh::new();
    mesh.set_num_points(NUM_POINTS);

    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float64,
        true,
        NUM_POINTS,
    );
    for (index, value) in POSITIONS.iter().enumerate() {
        for (component, component_value) in value.iter().enumerate() {
            position
                .buffer_mut()
                .write((index * 3 + component) * 8, &component_value.to_le_bytes());
        }
    }
    mesh.add_attribute(position);

    let mut tex_coord = PointAttribute::new();
    tex_coord.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Uint16,
        false,
        NUM_POINTS,
    );
    for (index, value) in TEX_COORDS.iter().enumerate() {
        for (component, component_value) in value.iter().enumerate() {
            tex_coord
                .buffer_mut()
                .write((index * 2 + component) * 2, &component_value.to_le_bytes());
        }
    }
    mesh.add_attribute(tex_coord);

    mesh.set_num_faces(4);
    for (index, face) in [[0u32, 1, 2], [1, 3, 4], [5, 6, 7], [7, 7, 7]]
        .iter()
        .enumerate()
    {
        mesh.set_face(
            draco_core::geometry_indices::FaceIndex(index as u32),
            [face[0].into(), face[1].into(), face[2].into()],
        );
    }

    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 20);
    options.set_attribute_int(0, "prediction_scheme", 4);
    options.set_attribute_int(1, "quantization_bits", 23);
    options.set_attribute_int(1, "prediction_scheme", 5);

    // Refusing the mesh is a fine answer; writing a stream and then rejecting
    // it is not.
    let Ok(encoded) = encode_mesh(mesh, &options) else {
        return;
    };
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(&encoded), &mut decoded)
        .unwrap_or_else(|error| {
            panic!("encoder wrote a stream its decoder rejects: {error}");
        });
}

/// An attribute with no values still carries its headers, and the decoder has
/// to read them.
///
/// The prediction header and the entropy stream's own header go into the
/// bitstream whether or not a value follows. The decoder took a shortcut on an
/// empty attribute and read neither, so the next thing it read -- the
/// octahedral transform's bit count, which trails the values -- came out of the
/// middle of the headers it had skipped, and the decoder rejected the encoder's
/// own stream.
#[test]
fn an_attribute_with_no_values_round_trips_through_its_headers() {
    for num_points in [0usize, 1, 4] {
        let mut point_cloud = PointCloud::new();
        point_cloud.set_num_points(num_points);

        let mut normal = PointAttribute::new();
        normal.init(
            GeometryAttributeType::Normal,
            3,
            DataType::Float32,
            true,
            num_points,
        );
        for index in 0..num_points {
            for component in 0..3 {
                normal.buffer_mut().write(
                    (index * 3 + component) * 4,
                    &(0.5f32 + component as f32).to_le_bytes(),
                );
            }
        }
        point_cloud.add_attribute(normal);

        let mut options = EncoderOptions::new();
        options.set_encoding_method(0);
        options.set_prediction_scheme(1);
        options.set_attribute_int(0, "quantization_bits", 6);

        let encoded = encode_point_cloud(point_cloud, &options)
            .unwrap_or_else(|error| panic!("{num_points} points: encode refused: {error}"));
        let mut decoded = PointCloud::new();
        PointCloudDecoder::new()
            .decode(&mut DecoderBuffer::new(&encoded), &mut decoded)
            .unwrap_or_else(|error| {
                panic!("{num_points} points: encoder wrote a stream its decoder rejects: {error}")
            });
        assert_eq!(decoded.num_points(), num_points);
    }
}

/// Replays the exact byte-decoding the `encode_drc` fuzz target uses, against
/// one committed seed, so the geometry here is the real fuzz-discovered mesh
/// rather than a hand-shrunk guess -- a guessed approximation of this one
/// already passed against the unfixed encoder once.
mod encode_drc_replay {
    use draco_core::draco_types::DataType;
    use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
    use draco_core::geometry_indices::{AttributeValueIndex, PointIndex};

    const MAX_POINTS: usize = 2048;
    const MAX_FACES: usize = 2048;
    const MAX_ATTRIBUTES: usize = 4;

    pub(super) struct Reader<'a> {
        data: &'a [u8],
        pos: usize,
    }

    impl<'a> Reader<'a> {
        pub(super) fn new(data: &'a [u8]) -> Self {
            Self { data, pos: 0 }
        }
        fn u8(&mut self) -> u8 {
            let byte = self.data.get(self.pos).copied().unwrap_or(0);
            self.pos = self.pos.saturating_add(1);
            byte
        }
        fn u16(&mut self) -> u16 {
            u16::from(self.u8()) | (u16::from(self.u8()) << 8)
        }
        fn bool(&mut self) -> bool {
            self.u8() & 1 == 1
        }
        fn in_range(&mut self, min: i32, max: i32) -> i32 {
            if max <= min {
                return min;
            }
            let span = (max - min) as u32 + 1;
            min + (u32::from(self.u16()) % span) as i32
        }
        pub(super) fn rest(&self) -> &'a [u8] {
            self.data.get(self.pos..).unwrap_or(&[])
        }
    }

    pub(super) struct AttributeSpec {
        attribute_type: GeometryAttributeType,
        data_type: DataType,
        num_components: u8,
        normalized: bool,
        num_values: usize,
        explicit_mapping: bool,
        pub(super) quantization_bits: i32,
        pub(super) prediction_scheme: i32,
    }

    pub(super) struct GeometrySpec {
        pub(super) as_point_cloud: bool,
        pub(super) num_points: usize,
        pub(super) faces: Vec<[u32; 3]>,
        pub(super) attributes: Vec<AttributeSpec>,
        #[allow(dead_code)]
        deduplicate: bool,
        pub(super) encoding_method: i32,
        pub(super) prediction_scheme: i32,
        pub(super) encoding_speed: i32,
        pub(super) decoding_speed: i32,
        pub(super) split_on_seams: i32,
        pub(super) force_predictive_traversal: bool,
        pub(super) version: Option<(u8, u8)>,
    }

    /// Faithful copy of `fuzz/fuzz_targets/encode_drc.rs`'s `read_spec`.
    /// `deduplicate` and `store_number_of_encoded_faces` are read in the same
    /// order as the target reads them, so every later field lands at the same
    /// byte offset, and then dropped; the rest is kept.
    pub(super) fn read_spec(reader: &mut Reader) -> GeometrySpec {
        let as_point_cloud = reader.bool();
        let num_points = reader.in_range(0, MAX_POINTS as i32) as usize;
        let num_faces = if as_point_cloud {
            0
        } else {
            reader.in_range(0, MAX_FACES as i32) as usize
        };

        let index_bound = reader.in_range(1, MAX_POINTS as i32 + 16) as u32;
        let mut faces = Vec::with_capacity(num_faces);
        for _ in 0..num_faces {
            faces.push([
                u32::from(reader.u16()) % index_bound,
                u32::from(reader.u16()) % index_bound,
                u32::from(reader.u16()) % index_bound,
            ]);
        }

        let num_attributes = reader.in_range(0, MAX_ATTRIBUTES as i32) as usize;
        let mut attributes = Vec::with_capacity(num_attributes);
        for _ in 0..num_attributes {
            attributes.push(read_attribute_spec(reader, num_points));
        }

        // Field order matches upstream's struct-literal evaluation order
        // exactly (left to right, `deduplicate` through `version`): a
        // struct literal evaluates its initializers in source order, not
        // declaration order, and every field after this one shifts to a
        // different byte offset if that order slips.
        let spec = GeometrySpec {
            as_point_cloud,
            num_points,
            faces,
            attributes,
            deduplicate: reader.bool(),
            encoding_method: reader.in_range(-1, 3),
            prediction_scheme: reader.in_range(-2, 6),
            encoding_speed: reader.in_range(-1, 10),
            decoding_speed: reader.in_range(-1, 10),
            split_on_seams: reader.in_range(-1, 1),
            // Read after the literal, in the order the target reads them.
            force_predictive_traversal: false,
            version: None,
        };
        let _store_number_of_encoded_faces = reader.bool();
        let force_predictive_traversal = reader.bool();
        let version = if reader.bool() {
            Some((reader.u8() % 4, reader.u8() % 8))
        } else {
            None
        };
        GeometrySpec {
            force_predictive_traversal,
            version,
            ..spec
        }
    }

    fn read_attribute_spec(reader: &mut Reader, num_points: usize) -> AttributeSpec {
        let attribute_type = match reader.in_range(0, 4) {
            0 => GeometryAttributeType::Position,
            1 => GeometryAttributeType::Normal,
            2 => GeometryAttributeType::Color,
            3 => GeometryAttributeType::TexCoord,
            _ => GeometryAttributeType::Generic,
        };
        let data_type = match reader.in_range(0, 10) {
            0 => DataType::Int8,
            1 => DataType::Uint8,
            2 => DataType::Int16,
            3 => DataType::Uint16,
            4 => DataType::Int32,
            5 => DataType::Uint32,
            6 => DataType::Int64,
            7 => DataType::Uint64,
            8 => DataType::Float32,
            9 => DataType::Float64,
            _ => DataType::Bool,
        };
        let explicit_mapping = reader.bool();
        let num_values = if explicit_mapping {
            reader.in_range(0, MAX_POINTS as i32) as usize
        } else {
            reader.in_range(0, num_points as i32 + 4) as usize
        };

        AttributeSpec {
            attribute_type,
            data_type,
            num_components: reader.in_range(0, 8) as u8,
            normalized: reader.bool(),
            num_values,
            explicit_mapping,
            quantization_bits: reader.in_range(-2, 34),
            prediction_scheme: reader.in_range(-2, 6),
        }
    }

    pub(super) fn build_attribute(
        spec: &AttributeSpec,
        num_points: usize,
        payload: &[u8],
        seed: usize,
    ) -> Option<PointAttribute> {
        let mut attribute = PointAttribute::new();
        attribute
            .try_init(
                spec.attribute_type,
                spec.num_components,
                spec.data_type,
                spec.normalized,
                spec.num_values,
            )
            .ok()?;

        fill(attribute.buffer_mut().data_mut(), payload, seed);

        if spec.explicit_mapping {
            attribute.set_explicit_mapping(num_points);
            for point in 0..num_points {
                if spec.num_values == 0 {
                    break;
                }
                let value =
                    ((point.wrapping_mul(2654435761).wrapping_add(seed)) % spec.num_values) as u32;
                let _ = attribute
                    .try_set_point_map_entry(PointIndex(point as u32), AttributeValueIndex(value));
            }
        } else {
            attribute.set_identity_mapping();
        }

        Some(attribute)
    }

    fn fill(dst: &mut [u8], src: &[u8], seed: usize) {
        if src.is_empty() {
            return;
        }
        for (i, byte) in dst.iter_mut().enumerate() {
            *byte = src[(i + seed) % src.len()];
        }
    }
}

/// The exact geometry `fuzz/seeds/encode_drc/parallelogram_over_attribute_connectivity.bin`
/// decodes to: 301 points, 45 faces, one explicit-mapping `Color` attribute
/// with more distinct values (693) than points, encoding speed 0, and no
/// `Position` attribute registered at all -- connectivity comes only from the
/// face list. That last part is what the fix here is about: the encoder used
/// `mesh.num_attributes() > 1` to decide whether a non-position attribute
/// needs its own depth-first traversal (position takes MaxPredictionDegree at
/// speed 0), and undercounts by one whenever there is no separate Position
/// attribute to count. See `dev/docs/per-attribute-connectivity.md`.
#[test]
fn a_color_attribute_without_a_position_attribute_round_trips_at_speed_zero() {
    use encode_drc_replay::*;

    let data = include_bytes!(
        "../../../fuzz/seeds/encode_drc/parallelogram_over_attribute_connectivity.bin"
    );
    let mut reader = Reader::new(data);
    let spec = read_spec(&mut reader);
    let payload = reader.rest().to_vec();

    assert!(!spec.as_point_cloud);
    assert_eq!(spec.num_points, 301);
    assert_eq!(spec.faces.len(), 45);
    assert_eq!(spec.attributes.len(), 1);
    assert_eq!(spec.encoding_speed, 0);

    let mut mesh = Mesh::new();
    mesh.set_num_points(spec.num_points);
    mesh.try_set_num_faces(spec.faces.len())
        .expect("face count within bounds");
    for (index, face) in spec.faces.iter().enumerate() {
        mesh.set_face_from_indices(index, *face);
    }

    for (index, attribute_spec) in spec.attributes.iter().enumerate() {
        let attribute = build_attribute(attribute_spec, spec.num_points, &payload, index * 7 + 1)
            .expect("attribute within bounds");
        mesh.add_attribute(attribute);
    }
    assert_eq!(mesh.num_attributes(), 1, "no Position attribute registered");

    let mut options = EncoderOptions::new();
    options.set_encoding_method(spec.encoding_method);
    options.set_prediction_scheme(spec.prediction_scheme);
    options.set_global_int("encoding_speed", spec.encoding_speed);
    options.set_global_int("decoding_speed", spec.decoding_speed);
    if spec.split_on_seams >= 0 {
        options.set_global_int("split_mesh_on_seams", spec.split_on_seams);
    }
    for (id, attribute) in spec.attributes.iter().enumerate() {
        let id = id as i32;
        options.set_attribute_int(id, "quantization_bits", attribute.quantization_bits);
        if attribute.prediction_scheme >= -1 {
            options.set_attribute_int(id, "prediction_scheme", attribute.prediction_scheme);
        }
    }

    let encoded = encode_mesh(mesh, &options).expect("encode");
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(&encoded), &mut decoded)
        .expect("encoder wrote a stream its decoder rejects");
}

/// A pre-2.1 stream carries a seam bit for both sides of an interior edge.
///
/// From 2.1 the decoder skips an edge whose opposite face it has already
/// processed, so one bit an edge is enough. Below that it reads a bit for every
/// corner that has an opposite, and the encoder wrote the newer shape at every
/// version: the bits landed on the wrong edges, the attribute vertex partition
/// came out two entries short of the decoder's, and the values ran out
/// mid-stream -- reported, three bytes later, as an unsupported prediction
/// method.
///
/// Upstream has only the decoding half of the older rule. C++ Draco encodes the
/// current version and nothing else, so there was no encoder to compare against
/// and the divergence sat in a version this project alone writes.
#[test]
fn a_pre_2_1_stream_carries_a_seam_bit_for_both_sides_of_an_edge() {
    use encode_drc_replay::{build_attribute, read_spec, Reader};

    let data = include_bytes!("../../../fuzz/seeds/encode_drc/legacy_predictive_symbol_count.bin");
    let mut reader = Reader::new(data);
    let spec = read_spec(&mut reader);
    let payload = reader.rest().to_vec();

    assert_eq!(spec.num_points, 301);
    assert_eq!(spec.faces.len(), 45);
    assert_eq!(spec.version, Some((1, 1)), "a pre-2.1 target is the point");

    let mut mesh = Mesh::new();
    mesh.set_num_points(spec.num_points);
    mesh.try_set_num_faces(spec.faces.len())
        .expect("face count within bounds");
    for (index, face) in spec.faces.iter().enumerate() {
        mesh.set_face_from_indices(index, *face);
    }
    for (index, attribute_spec) in spec.attributes.iter().enumerate() {
        let attribute = build_attribute(attribute_spec, spec.num_points, &payload, index * 7 + 1)
            .expect("attribute within bounds");
        mesh.add_attribute(attribute);
    }

    let mut options = EncoderOptions::new();
    options.set_encoding_method(spec.encoding_method);
    options.set_prediction_scheme(spec.prediction_scheme);
    options.set_global_int("encoding_speed", spec.encoding_speed);
    options.set_global_int("decoding_speed", spec.decoding_speed);
    if spec.split_on_seams >= 0 {
        options.set_global_int("split_mesh_on_seams", spec.split_on_seams);
    }
    if spec.force_predictive_traversal {
        options.set_global_int("force_predictive_traversal", 1);
    }
    if let Some((major, minor)) = spec.version {
        options.set_version(major, minor);
    }
    for (id, attribute) in spec.attributes.iter().enumerate() {
        let id = id as i32;
        options.set_attribute_int(id, "quantization_bits", attribute.quantization_bits);
        if attribute.prediction_scheme >= -1 {
            options.set_attribute_int(id, "prediction_scheme", attribute.prediction_scheme);
        }
    }

    let encoded = encode_mesh(mesh, &options).expect("encode");
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(&encoded), &mut decoded)
        .expect("encoder wrote a stream its decoder rejects");
}

/// A pre-2.0 stream predicts texture coordinates from the same position the
/// encoder predicted from.
///
/// The portable texture-coordinate scheme reads its parent as integers, which
/// is what "portable" names: the encoder predicts from the quantized position,
/// so the decoder has to as well. It was handed the portable parent only from
/// 2.0, and below that fell back to the attribute the mesh carries -- by then
/// dequantized floats. The two halves then predicted from different numbers,
/// and the decode stopped at the first entry whose float would not convert.
#[test]
fn a_pre_2_0_texcoord_predicts_from_the_portable_position() {
    use encode_drc_replay::{build_attribute, read_spec, Reader};

    let data = include_bytes!(
        "../../../fuzz/seeds/encode_drc/legacy_texcoord_predicts_from_portable_position.bin"
    );
    let mut reader = Reader::new(data);
    let spec = read_spec(&mut reader);
    let payload = reader.rest().to_vec();

    assert_eq!(spec.version, Some((1, 1)), "a pre-2.0 target is the point");

    let mut mesh = Mesh::new();
    mesh.set_num_points(spec.num_points);
    mesh.try_set_num_faces(spec.faces.len())
        .expect("face count within bounds");
    for (index, face) in spec.faces.iter().enumerate() {
        mesh.set_face_from_indices(index, *face);
    }
    for (index, attribute_spec) in spec.attributes.iter().enumerate() {
        let attribute = build_attribute(attribute_spec, spec.num_points, &payload, index * 7 + 1)
            .expect("attribute within bounds");
        mesh.add_attribute(attribute);
    }
    mesh.deduplicate_point_ids();

    let mut options = EncoderOptions::new();
    options.set_prediction_scheme(spec.prediction_scheme);
    options.set_global_int("encoding_speed", spec.encoding_speed);
    options.set_global_int("decoding_speed", spec.decoding_speed);
    if spec.split_on_seams >= 0 {
        options.set_global_int("split_mesh_on_seams", spec.split_on_seams);
    }
    if spec.force_predictive_traversal {
        options.set_global_int("force_predictive_traversal", 1);
    }
    if let Some((major, minor)) = spec.version {
        options.set_version(major, minor);
    }
    for (id, attribute) in spec.attributes.iter().enumerate() {
        let id = id as i32;
        options.set_attribute_int(id, "quantization_bits", attribute.quantization_bits);
        if attribute.prediction_scheme >= -1 {
            options.set_attribute_int(id, "prediction_scheme", attribute.prediction_scheme);
        }
    }

    let encoded = encode_mesh(mesh, &options).expect("encode");
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(&encoded), &mut decoded)
        .expect("encoder wrote a stream its decoder rejects");
}

/// A texture coordinate predicts from the position in the representation the
/// encoder used, including when that position is a `uint32` attribute.
///
/// The encoder predicts from the portable position, which is `int32`. The
/// decoder is handed a portable parent only for the attributes it dequantizes,
/// so an integer position leaves it reading the attribute the mesh carries --
/// the same bits under a `uint32` label. Every value above `i32::MAX` then sat
/// a whole `2^32` from the one the correction was computed against, the scaled
/// products left the range the overflow guard allows, and the decode stopped on
/// a stream this encoder had just written.
#[test]
fn a_texcoord_predicts_from_a_uint32_position_as_the_encoder_read_it() {
    use encode_drc_replay::{build_attribute, read_spec, Reader};

    let data = include_bytes!(
        "../../../fuzz/seeds/encode_drc/texcoord_predicts_from_a_uint32_position.bin"
    );
    let mut reader = Reader::new(data);
    let spec = read_spec(&mut reader);
    let payload = reader.rest().to_vec();

    let mut mesh = Mesh::new();
    mesh.set_num_points(spec.num_points);
    mesh.try_set_num_faces(spec.faces.len())
        .expect("face count within bounds");
    for (index, face) in spec.faces.iter().enumerate() {
        mesh.set_face_from_indices(index, *face);
    }
    for (index, attribute_spec) in spec.attributes.iter().enumerate() {
        let attribute = build_attribute(attribute_spec, spec.num_points, &payload, index * 7 + 1)
            .expect("attribute within bounds");
        mesh.add_attribute(attribute);
    }
    mesh.deduplicate_point_ids();

    let mut options = EncoderOptions::new();
    options.set_prediction_scheme(spec.prediction_scheme);
    options.set_global_int("encoding_speed", spec.encoding_speed);
    options.set_global_int("decoding_speed", spec.decoding_speed);
    if spec.split_on_seams >= 0 {
        options.set_global_int("split_mesh_on_seams", spec.split_on_seams);
    }
    if spec.force_predictive_traversal {
        options.set_global_int("force_predictive_traversal", 1);
    }
    if let Some((major, minor)) = spec.version {
        options.set_version(major, minor);
    }
    for (id, attribute) in spec.attributes.iter().enumerate() {
        let id = id as i32;
        options.set_attribute_int(id, "quantization_bits", attribute.quantization_bits);
        if attribute.prediction_scheme >= -1 {
            options.set_attribute_int(id, "prediction_scheme", attribute.prediction_scheme);
        }
    }

    let encoded = encode_mesh(mesh, &options).expect("encode");
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(&encoded), &mut decoded)
        .expect("encoder wrote a stream its decoder rejects");
}

/// The `traversal_method` byte has to name the order the position values were
/// actually written in.
///
/// At speed 0 the position walks the mesh by max prediction degree -- except
/// when one connectivity is shared by several attributes, where the walk falls
/// back to depth first. The byte was written from the speed alone, so such a
/// stream announced an order its values were not in, and the decoder rebuilt
/// the attribute-to-vertex map from a different walk. Every value then landed
/// on the wrong vertex; the constrained multi-parallelogram scheme is merely
/// the first place that becomes visible, because the crease-edge flags it
/// reads are counted per vertex fan and the two walks disagree on the fans.
#[test]
fn speed_zero_multi_attribute_mesh_round_trips() {
    // Seven faces over six points, from a fuzz artifact: no degenerate and no
    // repeated face, and every edge is shared by at most two faces. What makes
    // it bite is that the two walks part ways over the last two vertices.
    const FACES: &[[u32; 3]] = &[
        [0, 2, 5],
        [0, 4, 3],
        [4, 0, 5],
        [1, 3, 4],
        [1, 0, 3],
        [4, 2, 1],
        [1, 2, 0],
    ];

    let mut mesh = Mesh::new();
    mesh.set_num_points(6);
    mesh.try_set_num_faces(FACES.len()).unwrap();
    for (index, face) in FACES.iter().enumerate() {
        mesh.set_face_from_indices(index, *face);
    }
    // More than one attribute over a single connectivity is the case the byte
    // got wrong; one attribute alone keeps the prediction-degree walk and does
    // not reach it.
    for attribute_index in 0..4usize {
        let mut attribute = PointAttribute::new();
        attribute
            .try_init(GeometryAttributeType::Generic, 6, DataType::Uint8, false, 6)
            .unwrap();
        let data = attribute.buffer_mut().data_mut();
        for (offset, byte) in data.iter_mut().enumerate() {
            *byte = ((offset * 31 + attribute_index * 7 + 1) % 251) as u8;
        }
        attribute.set_identity_mapping();
        mesh.add_attribute(attribute);
    }

    let mut options = EncoderOptions::new();
    options.set_encoding_method(1);
    options.set_global_int("decoding_speed", 0);
    options.set_global_int("split_mesh_on_seams", 1);
    for id in 0..4i32 {
        options.set_attribute_int(id, "quantization_bits", 16);
        // Constrained multi-parallelogram: the scheme whose crease-edge flags
        // are counted per fan.
        options.set_attribute_int(id, "prediction_scheme", 4);
    }

    let mut buffer = EncoderBuffer::new();
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    encoder
        .encode(&options, &mut buffer)
        .expect("the encoder accepts this mesh");

    let mut decoded = Mesh::new();
    let mut decoder_buffer = DecoderBuffer::new(buffer.data());
    MeshDecoder::new()
        .decode(&mut decoder_buffer, &mut decoded)
        .expect("a stream the encoder produced has to decode");
}
