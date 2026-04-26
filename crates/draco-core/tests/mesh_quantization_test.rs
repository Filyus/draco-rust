use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;

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
