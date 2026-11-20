use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::DecoderBuffer;

#[test]
fn test_mesh_quantization_encoding() {
    let mut mesh = Mesh::new();

    // Add 3 points
    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        3,
    );
    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let buffer = pos_att.buffer_mut();
    for (i, &position) in positions.iter().enumerate() {
        let bytes = position.to_le_bytes();
        buffer.write(i * 4, &bytes);
    }
    mesh.add_attribute(pos_att);

    // Add 1 face
    mesh.set_num_faces(1);
    mesh.set_face(
        FaceIndex(0),
        [
            draco_core::PointIndex(0),
            draco_core::PointIndex(1),
            draco_core::PointIndex(2),
        ],
    );

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut buffer);

    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());
    assert!(buffer.size() > 0);
}

/// An explicitly named grid -- the keys `ExpertEncoder::SetAttributeExplicitQuantization`
/// writes -- is used as itself, and a grid that covers the data writes the
/// same stream its parameters would have computed: the bitstream never changes
/// shape, only where the encoder takes `min_values` and `range` from.
#[test]
fn explicit_grid_writes_itself_and_matches_computed_parameters_when_it_covers_the_data() {
    let mesh = |positions: &[f32]| {
        let mut mesh = Mesh::new();
        let mut pos_att = PointAttribute::new();
        pos_att.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            4,
        );
        let buffer = pos_att.buffer_mut();
        for (i, position) in positions.iter().enumerate() {
            buffer.write(i * 4, &position.to_le_bytes());
        }
        mesh.add_attribute(pos_att);
        mesh.set_num_faces(2);
        mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
        mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);
        mesh
    };
    let positions = [
        0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0,
    ];

    let encode = |options: &EncoderOptions| {
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh(&positions));
        let mut buffer = EncoderBuffer::new();
        encoder.encode(options, &mut buffer).expect("encode");
        buffer.data().to_vec()
    };

    let mut computed = EncoderOptions::new();
    computed.set_attribute_int(0, "quantization_bits", 10);
    let mut explicit = EncoderOptions::new();
    // The grid the computed parameters would have found: the data's own
    // bounds, one range per attribute.
    explicit.set_attribute_explicit_quantization(0, 10, 3, &[0.0, 0.0, 0.0], 1.0);
    assert_eq!(
        encode(&computed),
        encode(&explicit),
        "a grid covering the data must not change a byte"
    );

    // A grid the computed parameters would not find is written as itself: a
    // wider range shifts where the decoded values sit on it.
    let mut wider = EncoderOptions::new();
    wider.set_attribute_explicit_quantization(0, 10, 3, &[-1.0, -1.0, -1.0], 4.0);
    assert_ne!(encode(&computed), encode(&wider));

    // And the stream it wrote decodes onto that grid: every value a step of
    // 4/1023 away from the origin it named.
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(&encode(&wider)), &mut decoded)
        .expect("decode");
    let id = decoded.named_attribute_id(GeometryAttributeType::Position);
    let values = decoded.attribute(id).read_f32s(decoded.num_points(), 3);
    let step = 4.0f32 / 1023.0;
    for value in values {
        let on_grid = (value - (value / step).round() * step).abs();
        assert!(
            on_grid.max((value + 1.0).abs()) < 1e-3 || on_grid < 1e-3,
            "{value} is not on the explicit grid or at its origin"
        );
        assert!((-1.0..=3.0).contains(&value), "{value} left the grid");
    }
}

/// Values outside an explicit grid come back on its edges: below the origin
/// they would otherwise ride the portable attribute's `u32` storage as its
/// wrap-around, and past the top they would push a kd-tree bit length to
/// thirty-two.
#[test]
fn values_outside_an_explicit_grid_come_back_on_its_edges() {
    let positions = [
        0.0f32, 0.0, 0.0, // 0.5 below the origin on every axis
        2.0, 2.0, 2.0, // 0.5 past the top on every axis
        1.0, 1.0, 1.0, // inside
        1.0, 0.5, 1.0, // on the grid's low edge
    ];
    let mut mesh = Mesh::new();
    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        4,
    );
    let buffer = pos_att.buffer_mut();
    for (i, position) in positions.iter().enumerate() {
        buffer.write(i * 4, &position.to_le_bytes());
    }
    mesh.add_attribute(pos_att);
    mesh.set_num_faces(2);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);

    let mut options = EncoderOptions::new();
    options.set_attribute_explicit_quantization(0, 10, 3, &[0.5, 0.5, 0.5], 1.0);
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("an encode with out-of-grid values must succeed");

    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(buffer.data()), &mut decoded)
        .expect("decode");
    let id = decoded.named_attribute_id(GeometryAttributeType::Position);
    let values = decoded.attribute(id).read_f32s(decoded.num_points(), 3);
    for value in values {
        assert!(
            (0.5..=1.5).contains(&value),
            "{value} came back off the grid it was clamped onto"
        );
    }
}

/// A grid whose origin does not carry one component per attribute component is
/// refused rather than silently replaced by computed parameters: the caller
/// named a grid, so a mismatch is theirs to see.
#[test]
fn an_explicit_grid_of_the_wrong_width_is_refused() {
    let mut mesh = Mesh::new();
    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        1,
    );
    pos_att.buffer_mut().write(0, &0.0f32.to_le_bytes());
    pos_att.buffer_mut().write(4, &0.0f32.to_le_bytes());
    pos_att.buffer_mut().write(8, &0.0f32.to_le_bytes());
    mesh.add_attribute(pos_att);

    let mut options = EncoderOptions::new();
    options.set_attribute_explicit_quantization(0, 10, 2, &[0.0, 0.0], 1.0);
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    let error = encoder
        .encode(&options, &mut buffer)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("quantization origin"),
        "the error should name the grid, got: {error}"
    );
}
