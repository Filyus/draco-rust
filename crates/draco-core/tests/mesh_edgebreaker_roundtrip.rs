use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;

#[test]
fn test_edgebreaker_single_triangle_roundtrip() {
    let mut mesh = Mesh::new();
    mesh.set_num_points(3);
    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);

    let mut options = EncoderOptions::default();
    options.set_global_int("encoding_method", 1); // Edgebreaker

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("Encode failed");

    let mut decoder = MeshDecoder::new();
    let mut decoded_mesh = Mesh::new();
    let mut decoder_buffer = DecoderBuffer::new(buffer.data());
    decoder
        .decode(&mut decoder_buffer, &mut decoded_mesh)
        .expect("Decode failed");

    assert_eq!(decoded_mesh.num_faces(), 1);
    assert_eq!(decoded_mesh.num_points(), 3);
    let face = decoded_mesh.face(FaceIndex(0));
    // Note: Edgebreaker might permute vertices, but for a single triangle
    // it should be consistent if we handle it right.
    // Actually, the order might be different.
    let mut face_vec = vec![face[0].0, face[1].0, face[2].0];
    face_vec.sort();
    assert_eq!(face_vec, vec![0, 1, 2]);
}

#[test]
fn test_edgebreaker_quad_roundtrip() {
    let mut mesh = Mesh::new();
    mesh.set_num_points(4);
    mesh.set_num_faces(2);
    // Two triangles sharing edge (0, 2)
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);

    let mut options = EncoderOptions::default();
    options.set_global_int("encoding_method", 1); // Edgebreaker

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("Encode failed");

    let mut decoder = MeshDecoder::new();
    let mut decoded_mesh = Mesh::new();
    let mut decoder_buffer = DecoderBuffer::new(buffer.data());
    decoder
        .decode(&mut decoder_buffer, &mut decoded_mesh)
        .expect("Decode failed");

    assert_eq!(decoded_mesh.num_faces(), 2);
    assert_eq!(decoded_mesh.num_points(), 4);

    // Verify faces exist and share vertices correctly
    // We don't check exact indices because of permutation, but we check topology.
    let mut all_faces = Vec::new();
    for i in 0..2 {
        let f = decoded_mesh.face(FaceIndex(i));
        let mut f_vec = vec![f[0].0, f[1].0, f[2].0];
        f_vec.sort();
        all_faces.push(f_vec);
    }
    all_faces.sort();

    // The two triangles should share 2 vertices.
    let f0 = &all_faces[0];
    let f1 = &all_faces[1];
    println!("Face 0: {:?}", f0);
    println!("Face 1: {:?}", f1);
    let shared: Vec<_> = f0.iter().filter(|v| f1.contains(v)).collect();
    assert_eq!(shared.len(), 2);
}
