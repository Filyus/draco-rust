//! Test that verifies mesh with positions, normals, and UVs roundtrip correctly.
//! This simulates the glTF workflow with multiple attributes.

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::PointIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;

/// Creates a mesh with positions, normals, and UVs (like a glTF file)
// Returns (Mesh, positions, normals, uvs, faces) tuple for test verification.
// The tuple is complex but each component serves a distinct validation purpose.
#[allow(clippy::type_complexity)]
fn create_gltf_style_mesh() -> (
    Mesh,
    Vec<[f32; 3]>,
    Vec<[f32; 3]>,
    Vec<[f32; 2]>,
    Vec<[u32; 3]>,
) {
    // 4 vertices forming a square
    let positions: Vec<[f32; 3]> = vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];

    // Normals pointing in +Z direction
    let normals: Vec<[f32; 3]> = vec![
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
    ];

    // UVs
    let uvs: Vec<[f32; 2]> = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

    // Two triangles
    let faces: Vec<[u32; 3]> = vec![[0, 1, 2], [0, 2, 3]];

    let mut mesh = Mesh::new();
    let vertex_count = positions.len();

    // Add position attribute
    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        vertex_count,
    );
    for (i, pos) in positions.iter().enumerate() {
        let buffer = pos_att.buffer_mut();
        buffer.write(i * 12, &pos[0].to_le_bytes());
        buffer.write(i * 12 + 4, &pos[1].to_le_bytes());
        buffer.write(i * 12 + 8, &pos[2].to_le_bytes());
    }
    mesh.add_attribute(pos_att);

    // Add normal attribute
    let mut norm_att = PointAttribute::new();
    norm_att.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        vertex_count,
    );
    for (i, norm) in normals.iter().enumerate() {
        let buffer = norm_att.buffer_mut();
        buffer.write(i * 12, &norm[0].to_le_bytes());
        buffer.write(i * 12 + 4, &norm[1].to_le_bytes());
        buffer.write(i * 12 + 8, &norm[2].to_le_bytes());
    }
    mesh.add_attribute(norm_att);

    // Add UV attribute
    let mut uv_att = PointAttribute::new();
    uv_att.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Float32,
        false,
        vertex_count,
    );
    for (i, uv) in uvs.iter().enumerate() {
        let buffer = uv_att.buffer_mut();
        buffer.write(i * 8, &uv[0].to_le_bytes());
        buffer.write(i * 8 + 4, &uv[1].to_le_bytes());
    }
    mesh.add_attribute(uv_att);

    // Add faces
    for face in &faces {
        mesh.add_face([
            PointIndex(face[0]),
            PointIndex(face[1]),
            PointIndex(face[2]),
        ]);
    }

    (mesh, positions, normals, uvs, faces)
}

fn read_vec3_attribute(mesh: &Mesh, att_type: GeometryAttributeType) -> Vec<[f32; 3]> {
    let att_id = mesh.named_attribute_id(att_type);
    if att_id < 0 {
        return vec![];
    }
    let att = mesh.attribute(att_id);
    let buffer = att.buffer();
    let stride = att.byte_stride() as usize;
    let count = att.size();

    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let offset = i * stride;
        let mut x_bytes = [0u8; 4];
        let mut y_bytes = [0u8; 4];
        let mut z_bytes = [0u8; 4];
        buffer.read(offset, &mut x_bytes);
        buffer.read(offset + 4, &mut y_bytes);
        buffer.read(offset + 8, &mut z_bytes);
        result.push([
            f32::from_le_bytes(x_bytes),
            f32::from_le_bytes(y_bytes),
            f32::from_le_bytes(z_bytes),
        ]);
    }
    result
}

fn read_vec2_attribute(mesh: &Mesh, att_type: GeometryAttributeType) -> Vec<[f32; 2]> {
    let att_id = mesh.named_attribute_id(att_type);
    if att_id < 0 {
        return vec![];
    }
    let att = mesh.attribute(att_id);
    let buffer = att.buffer();
    let stride = att.byte_stride() as usize;
    let count = att.size();

    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let offset = i * stride;
        let mut u_bytes = [0u8; 4];
        let mut v_bytes = [0u8; 4];
        buffer.read(offset, &mut u_bytes);
        buffer.read(offset + 4, &mut v_bytes);
        result.push([f32::from_le_bytes(u_bytes), f32::from_le_bytes(v_bytes)]);
    }
    result
}

#[test]
fn test_gltf_style_mesh_sequential_roundtrip() {
    let (mesh, orig_positions, orig_normals, orig_uvs, _) = create_gltf_style_mesh();

    eprintln!("Original mesh:");
    eprintln!("  Positions: {:?}", orig_positions);
    eprintln!("  Normals: {:?}", orig_normals);
    eprintln!("  UVs: {:?}", orig_uvs);

    // Encode with Sequential method and quantization (similar to web app)
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 0); // Sequential
    options.set_attribute_int(0, "quantization_bits", 14); // Position
    options.set_attribute_int(1, "quantization_bits", 10); // Normal
    options.set_attribute_int(2, "quantization_bits", 12); // UV

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

    let decoded_positions = read_vec3_attribute(&decoded_mesh, GeometryAttributeType::Position);
    let decoded_normals = read_vec3_attribute(&decoded_mesh, GeometryAttributeType::Normal);
    let decoded_uvs = read_vec2_attribute(&decoded_mesh, GeometryAttributeType::TexCoord);

    eprintln!("Decoded mesh:");
    eprintln!("  Positions: {:?}", decoded_positions);
    eprintln!("  Normals: {:?}", decoded_normals);
    eprintln!("  UVs: {:?}", decoded_uvs);

    // Check vertex count
    assert_eq!(
        decoded_positions.len(),
        orig_positions.len(),
        "Position count mismatch"
    );
    assert_eq!(
        decoded_normals.len(),
        orig_normals.len(),
        "Normal count mismatch"
    );
    assert_eq!(decoded_uvs.len(), orig_uvs.len(), "UV count mismatch");

    // Check position values (14-bit quantization)
    let pos_tolerance = 1.0 / 16384.0 * 2.0; // 14-bit, some margin
    for (i, (orig, decoded)) in orig_positions
        .iter()
        .zip(decoded_positions.iter())
        .enumerate()
    {
        let diff = (0..3)
            .map(|c| (orig[c] - decoded[c]).abs())
            .fold(0.0f32, |a, b| a.max(b));
        eprintln!(
            "Position {}: orig={:?}, decoded={:?}, diff={:.6}",
            i, orig, decoded, diff
        );
        assert!(
            diff < pos_tolerance,
            "Position {} differs too much: {}",
            i,
            diff
        );
    }

    // Check normal values (10-bit quantization, after octahedron transform)
    // Normals may be less accurate due to octahedron encoding
    let norm_tolerance = 0.1; // Generous tolerance for octahedron-encoded normals
    for (i, (orig, decoded)) in orig_normals.iter().zip(decoded_normals.iter()).enumerate() {
        let diff = (0..3)
            .map(|c| (orig[c] - decoded[c]).abs())
            .fold(0.0f32, |a, b| a.max(b));
        eprintln!(
            "Normal {}: orig={:?}, decoded={:?}, diff={:.6}",
            i, orig, decoded, diff
        );
        assert!(
            diff < norm_tolerance,
            "Normal {} differs too much: {}",
            i,
            diff
        );
    }

    // Check UV values (12-bit quantization)
    let uv_tolerance = 1.0 / 4096.0 * 2.0; // 12-bit, some margin
    for (i, (orig, decoded)) in orig_uvs.iter().zip(decoded_uvs.iter()).enumerate() {
        let diff = (0..2)
            .map(|c| (orig[c] - decoded[c]).abs())
            .fold(0.0f32, |a, b| a.max(b));
        eprintln!(
            "UV {}: orig={:?}, decoded={:?}, diff={:.6}",
            i, orig, decoded, diff
        );
        assert!(diff < uv_tolerance, "UV {} differs too much: {}", i, diff);
    }

    eprintln!("All attributes match within tolerance!");
}
