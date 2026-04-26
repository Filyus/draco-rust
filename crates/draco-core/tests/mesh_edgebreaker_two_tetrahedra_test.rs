use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;

#[test]
fn test_edgebreaker_two_closed_components_roundtrip() {
    // Two disjoint closed components. Ensures we encode/decode multiple
    // start-face configuration bits and stitch all interior start faces.
    let mut mesh = Mesh::new();
    mesh.set_num_points(8);
    mesh.set_num_faces(8);

    // First tetrahedron (0..3).
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(3), PointIndex(1)]);
    mesh.set_face(FaceIndex(2), [PointIndex(0), PointIndex(2), PointIndex(3)]);
    mesh.set_face(FaceIndex(3), [PointIndex(1), PointIndex(3), PointIndex(2)]);

    // Second tetrahedron (4..7).
    mesh.set_face(FaceIndex(4), [PointIndex(4), PointIndex(5), PointIndex(6)]);
    mesh.set_face(FaceIndex(5), [PointIndex(4), PointIndex(7), PointIndex(5)]);
    mesh.set_face(FaceIndex(6), [PointIndex(4), PointIndex(6), PointIndex(7)]);
    mesh.set_face(FaceIndex(7), [PointIndex(5), PointIndex(7), PointIndex(6)]);

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1); // Edgebreaker

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut encoder_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut encoder_buffer)
        .expect("Encode failed");

    let mut decoder = MeshDecoder::new();
    let mut decoder_buffer = DecoderBuffer::new(encoder_buffer.data());
    let mut decoded_mesh = Mesh::new();
    decoder
        .decode(&mut decoder_buffer, &mut decoded_mesh)
        .expect("Decode failed");

    assert_eq!(decoded_mesh.num_faces(), 8);
    assert_eq!(decoded_mesh.num_points(), 8);
}
