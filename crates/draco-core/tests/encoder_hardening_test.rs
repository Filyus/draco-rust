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
