use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;

#[test]
fn test_edgebreaker_torus_roundtrip() {
    let mut mesh = Mesh::new();
    let n = 3;
    let m = 3;
    mesh.set_num_points(n * m);
    mesh.set_num_faces(2 * n * m);

    let mut face_idx = 0;
    for i in 0..n {
        for j in 0..m {
            let v00 = i * m + j;
            let v10 = ((i + 1) % n) * m + j;
            let v01 = i * m + ((j + 1) % m);
            let v11 = ((i + 1) % n) * m + ((j + 1) % m);

            mesh.set_face(
                FaceIndex(face_idx),
                [
                    PointIndex(v00 as u32),
                    PointIndex(v10 as u32),
                    PointIndex(v01 as u32),
                ],
            );
            face_idx += 1;
            mesh.set_face(
                FaceIndex(face_idx),
                [
                    PointIndex(v10 as u32),
                    PointIndex(v11 as u32),
                    PointIndex(v01 as u32),
                ],
            );
            face_idx += 1;
        }
    }

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

    assert_eq!(decoded_mesh.num_faces(), 18);
    assert_eq!(decoded_mesh.num_points(), 9);
}
