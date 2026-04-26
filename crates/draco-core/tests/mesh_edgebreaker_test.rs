use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;

#[test]
fn test_mesh_edgebreaker_encoding() {
    let mut mesh = Mesh::new();
    mesh.set_num_points(4);
    mesh.set_num_faces(2);

    // Two triangles forming a quad (0, 1, 2) and (0, 2, 3)
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);

    let mut options = EncoderOptions::default();
    options.set_global_int("encoding_method", 1); // Edgebreaker

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut buffer);

    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());
    assert!(buffer.size() > 0);
}
