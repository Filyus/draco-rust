use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use std::path::Path;

fn create_annulus_mesh() -> Mesh {
    // Create annulus mesh programmatically
    // Outer square vertices
    let positions: [[f32; 3]; 8] = [
        [-1.0, -1.0, 0.0], // 0
        [1.0, -1.0, 0.0],  // 1
        [1.0, 1.0, 0.0],   // 2
        [-1.0, 1.0, 0.0],  // 3
        // Inner square vertices
        [-0.5, -0.5, 0.0], // 4
        [0.5, -0.5, 0.0],  // 5
        [0.5, 0.5, 0.0],   // 6
        [-0.5, 0.5, 0.0],  // 7
    ];

    // Faces (0-indexed)
    let faces: [[u32; 3]; 8] = [
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];

    let mut mesh = Mesh::new();

    // Create position attribute
    let mut pos_attr = PointAttribute::new();
    pos_attr.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        positions.len(),
    );

    let buffer = pos_attr.buffer_mut();
    for (i, pos) in positions.iter().enumerate() {
        let bytes = [
            pos[0].to_le_bytes(),
            pos[1].to_le_bytes(),
            pos[2].to_le_bytes(),
        ]
        .concat();
        buffer.write(i * 12, &bytes);
    }

    mesh.add_attribute(pos_attr);
    mesh.set_num_faces(faces.len());

    // Add faces
    for (i, face) in faces.iter().enumerate() {
        mesh.set_face(
            FaceIndex(i as u32),
            [
                PointIndex(face[0]),
                PointIndex(face[1]),
                PointIndex(face[2]),
            ],
        );
    }

    mesh
}

#[test]
fn test_encode_annulus_topology_splits() {
    let mesh = create_annulus_mesh();

    eprintln!(
        "Created annulus mesh: {} faces, {} points",
        mesh.num_faces(),
        mesh.num_points()
    );

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1); // Edgebreaker

    let mut buffer = EncoderBuffer::new();

    match encoder.encode(&options, &mut buffer) {
        Ok(_) => {
            eprintln!("Encoding succeeded, {} bytes", buffer.data().len());
        }
        Err(e) => {
            eprintln!("Encoding failed: {:?}", e);
            panic!("Encoding should succeed");
        }
    }

    // Now try to decode it
    let mut decoder_buffer = DecoderBuffer::new(buffer.data());
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();

    match decoder.decode(&mut decoder_buffer, &mut decoded_mesh) {
        Ok(_) => {
            eprintln!("Decoding succeeded!");
            eprintln!(
                "Decoded mesh: {} faces, {} points",
                decoded_mesh.num_faces(),
                decoded_mesh.num_points()
            );
        }
        Err(e) => {
            eprintln!("Decoding failed: {:?}", e);
            panic!("Decoding should succeed for annulus mesh!");
        }
    }
}

#[test]
fn test_decode_cpp_encoded_annulus() {
    // Try to decode the C++ encoded annulus file
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("test_annulus.drc");

    if !test_file.exists() {
        eprintln!("C++ encoded file not found: {:?}, skipping", test_file);
        return;
    }

    let data = std::fs::read(&test_file).expect("Failed to read file");

    let mut decoder_buffer = DecoderBuffer::new(&data);
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();

    match decoder.decode(&mut decoder_buffer, &mut decoded_mesh) {
        Ok(_) => {
            println!("Successfully decoded C++ encoded annulus!");
            println!(
                "Decoded mesh: {} faces, {} points",
                decoded_mesh.num_faces(),
                decoded_mesh.num_points()
            );
        }
        Err(e) => {
            println!("Failed to decode C++ encoded annulus: {:?}", e);
            panic!("Decoding failed");
        }
    }
}
