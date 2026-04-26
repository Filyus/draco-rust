//! Test that verifies mesh vertex positions are correctly preserved through Draco encoding/decoding roundtrip.

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;

/// Creates a simple cube mesh for testing.
fn create_cube_mesh() -> Mesh {
    // Cube with 8 unique vertex positions
    let positions: [f32; 24] = [
        // 8 corners of a unit cube centered at origin
        -0.5, -0.5, -0.5, // 0
        0.5, -0.5, -0.5, // 1
        0.5, 0.5, -0.5, // 2
        -0.5, 0.5, -0.5, // 3
        -0.5, -0.5, 0.5, // 4
        0.5, -0.5, 0.5, // 5
        0.5, 0.5, 0.5, // 6
        -0.5, 0.5, 0.5, // 7
    ];

    // 12 triangles (2 per face)
    let indices: [u32; 36] = [
        // Front face
        0, 1, 2, 0, 2, 3, // Back face
        5, 4, 7, 5, 7, 6, // Top face
        3, 2, 6, 3, 6, 7, // Bottom face
        4, 5, 1, 4, 1, 0, // Right face
        1, 5, 6, 1, 6, 2, // Left face
        4, 0, 3, 4, 3, 7,
    ];

    let mut mesh = Mesh::new();

    // Position attribute
    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        8,
    );
    let buffer = pos_att.buffer_mut();
    for (i, &position) in positions.iter().enumerate() {
        buffer.write(i * 4, &position.to_le_bytes());
    }
    mesh.add_attribute(pos_att);

    // Add faces
    for i in 0..12 {
        mesh.add_face([
            PointIndex(indices[i * 3]),
            PointIndex(indices[i * 3 + 1]),
            PointIndex(indices[i * 3 + 2]),
        ]);
    }

    mesh
}

/// Reads all vertex positions from a mesh
fn read_positions(mesh: &Mesh) -> Vec<[f32; 3]> {
    let att = mesh.attribute(mesh.named_attribute_id(GeometryAttributeType::Position));
    let buffer = att.buffer();
    let byte_stride = att.byte_stride() as usize;
    let num_vertices = att.size();

    let mut positions = Vec::with_capacity(num_vertices);
    for i in 0..num_vertices {
        let offset = i * byte_stride;
        let mut x_bytes = [0u8; 4];
        let mut y_bytes = [0u8; 4];
        let mut z_bytes = [0u8; 4];
        buffer.read(offset, &mut x_bytes);
        buffer.read(offset + 4, &mut y_bytes);
        buffer.read(offset + 8, &mut z_bytes);
        let x = f32::from_le_bytes(x_bytes);
        let y = f32::from_le_bytes(y_bytes);
        let z = f32::from_le_bytes(z_bytes);
        positions.push([x, y, z]);
    }
    positions
}

/// Reads all face indices from a mesh
fn read_faces(mesh: &Mesh) -> Vec<[u32; 3]> {
    let num_faces = mesh.num_faces();
    let mut faces = Vec::with_capacity(num_faces);
    for i in 0..num_faces {
        let face = mesh.face(FaceIndex(i as u32));
        faces.push([face[0].0, face[1].0, face[2].0]);
    }
    faces
}

#[test]
fn test_mesh_sequential_position_roundtrip() {
    let mesh = create_cube_mesh();
    let original_positions = read_positions(&mesh);
    let original_faces = read_faces(&mesh);

    eprintln!(
        "Original mesh: {} vertices, {} faces",
        original_positions.len(),
        original_faces.len()
    );
    for (i, pos) in original_positions.iter().enumerate() {
        eprintln!(
            "  Vertex {}: ({:.4}, {:.4}, {:.4})",
            i, pos[0], pos[1], pos[2]
        );
    }

    // Encode with Sequential method and quantization
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 0); // Sequential
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut enc_buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut enc_buffer);
    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());

    eprintln!("Encoded {} bytes", enc_buffer.data().len());

    // Decode
    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let status = decoder.decode(&mut dec_buffer, &mut decoded_mesh);
    assert!(status.is_ok(), "Decoding failed: {:?}", status.err());

    let decoded_positions = read_positions(&decoded_mesh);
    let decoded_faces = read_faces(&decoded_mesh);

    eprintln!(
        "Decoded mesh: {} vertices, {} faces",
        decoded_positions.len(),
        decoded_faces.len()
    );
    for (i, pos) in decoded_positions.iter().enumerate() {
        eprintln!(
            "  Vertex {}: ({:.4}, {:.4}, {:.4})",
            i, pos[0], pos[1], pos[2]
        );
    }

    // Check vertex count matches
    assert_eq!(
        decoded_positions.len(),
        original_positions.len(),
        "Vertex count mismatch: {} vs {}",
        decoded_positions.len(),
        original_positions.len()
    );

    // Check face count matches
    assert_eq!(
        decoded_faces.len(),
        original_faces.len(),
        "Face count mismatch: {} vs {}",
        decoded_faces.len(),
        original_faces.len()
    );

    // Check vertex positions within quantization tolerance
    // For 14-bit quantization of range 1.0, expected error is about 1.0 / 16384 ≈ 0.00006
    let max_tolerance = 0.001; // Allow some margin

    for (i, (orig, decoded)) in original_positions
        .iter()
        .zip(decoded_positions.iter())
        .enumerate()
    {
        let diff_x = (orig[0] - decoded[0]).abs();
        let diff_y = (orig[1] - decoded[1]).abs();
        let diff_z = (orig[2] - decoded[2]).abs();
        let max_diff = diff_x.max(diff_y).max(diff_z);

        eprintln!(
            "Vertex {}: orig=({:.4}, {:.4}, {:.4}) decoded=({:.4}, {:.4}, {:.4}) max_diff={:.6}",
            i, orig[0], orig[1], orig[2], decoded[0], decoded[1], decoded[2], max_diff
        );

        assert!(
            max_diff < max_tolerance,
            "Vertex {} position differs too much: orig={:?}, decoded={:?}, max_diff={}",
            i,
            orig,
            decoded,
            max_diff
        );
    }
}

#[test]
fn test_mesh_edgebreaker_position_roundtrip() {
    let mesh = create_cube_mesh();
    let original_positions = read_positions(&mesh);

    eprintln!(
        "Original mesh: {} vertices, {} faces",
        original_positions.len(),
        mesh.num_faces()
    );

    // Encode with Edgebreaker method and quantization
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1); // Edgebreaker
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut enc_buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut enc_buffer);
    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());

    eprintln!("Encoded {} bytes", enc_buffer.data().len());

    // Decode
    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let status = decoder.decode(&mut dec_buffer, &mut decoded_mesh);
    assert!(status.is_ok(), "Decoding failed: {:?}", status.err());

    let decoded_positions = read_positions(&decoded_mesh);

    eprintln!(
        "Decoded mesh: {} vertices, {} faces",
        decoded_positions.len(),
        decoded_mesh.num_faces()
    );
    for (i, pos) in decoded_positions.iter().enumerate() {
        eprintln!(
            "  Decoded Vertex {}: ({:.4}, {:.4}, {:.4})",
            i, pos[0], pos[1], pos[2]
        );
    }

    // Note: Edgebreaker may reorder vertices, so we need to check that each original
    // vertex appears in the decoded mesh (not necessarily at the same index)

    // Check vertex count matches
    assert_eq!(
        decoded_positions.len(),
        original_positions.len(),
        "Vertex count mismatch: {} vs {}",
        decoded_positions.len(),
        original_positions.len()
    );

    // For each original vertex, find a matching decoded vertex
    let tolerance = 0.001;
    for (i, orig) in original_positions.iter().enumerate() {
        let mut found = false;
        for decoded in &decoded_positions {
            let diff_x = (orig[0] - decoded[0]).abs();
            let diff_y = (orig[1] - decoded[1]).abs();
            let diff_z = (orig[2] - decoded[2]).abs();
            if diff_x < tolerance && diff_y < tolerance && diff_z < tolerance {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "Original vertex {} ({:?}) not found in decoded mesh",
            i, orig
        );
    }

    eprintln!("All vertices matched!");
}
