//! Test that compares actual triangle geometry (vertices) after encode/decode roundtrip.
//! This is more robust than comparing indices, which may be reordered.

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use std::collections::HashSet;

/// A triangle represented by its three vertex positions, sorted for comparison.
#[derive(Debug, Clone, PartialEq)]
struct Triangle {
    // Vertices sorted by (x, y, z) for order-independent comparison
    vertices: Vec<[i32; 3]>,
}

impl Triangle {
    fn new(v0: [f32; 3], v1: [f32; 3], v2: [f32; 3], precision: i32) -> Self {
        let mut vertices: Vec<[i32; 3]> = vec![
            Self::quantize(v0, precision),
            Self::quantize(v1, precision),
            Self::quantize(v2, precision),
        ];
        // Sort vertices for order-independent comparison
        vertices.sort();
        Self { vertices }
    }

    fn quantize(v: [f32; 3], precision: i32) -> [i32; 3] {
        let scale = precision as f32;
        [
            (v[0] * scale).round() as i32,
            (v[1] * scale).round() as i32,
            (v[2] * scale).round() as i32,
        ]
    }
}

impl Eq for Triangle {}

impl std::hash::Hash for Triangle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.vertices.hash(state);
    }
}

/// Extract triangles from a mesh as a set for comparison.
fn extract_triangles(mesh: &Mesh, att_id: i32, precision: i32) -> HashSet<Triangle> {
    let mut triangles = HashSet::new();

    let att = mesh.attribute(att_id);
    let buffer = att.buffer();
    let byte_stride = att.byte_stride() as usize;

    // Debug: print the point->attribute mapping
    println!("DEBUG: Extracting triangles with point->attribute mapping:");
    for p in 0..mesh.num_points() {
        let val_index = att.mapped_index(PointIndex(p as u32));
        let byte_offset = val_index.0 as usize * byte_stride;
        let mut bytes = [0u8; 12];
        buffer.read(byte_offset, &mut bytes);
        let pos = [
            f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        ];
        println!(
            "  point {} -> attr_value {} -> pos {:?}",
            p, val_index.0, pos
        );
    }

    for face_idx in 0..mesh.num_faces() {
        let face = mesh.face(FaceIndex(face_idx as u32));

        let mut vertices = [[0.0f32; 3]; 3];
        for (i, &point_idx) in face.iter().enumerate() {
            let val_index = att.mapped_index(point_idx);
            let byte_offset = val_index.0 as usize * byte_stride;

            let mut bytes = [0u8; 12];
            buffer.read(byte_offset, &mut bytes);

            vertices[i] = [
                f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
                f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            ];
        }

        triangles.insert(Triangle::new(
            vertices[0],
            vertices[1],
            vertices[2],
            precision,
        ));
    }

    triangles
}

/// Create a test mesh with given positions and faces.
fn create_test_mesh(positions: &[[f32; 3]], faces: &[[u32; 3]]) -> Mesh {
    let mut mesh = Mesh::new();
    let mut pos_att = PointAttribute::new();

    let num_points = positions.len();
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );

    let buffer = pos_att.buffer_mut();
    for (i, pos) in positions.iter().enumerate() {
        let bytes = [
            pos[0].to_le_bytes(),
            pos[1].to_le_bytes(),
            pos[2].to_le_bytes(),
        ]
        .concat();
        buffer.write(i * 12, &bytes);
    }

    mesh.add_attribute(pos_att);
    mesh.set_num_faces(faces.len());

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

/// Roundtrip encode/decode and verify triangles match.
fn roundtrip_and_compare(mesh: Mesh, method: i32, precision: i32) -> Result<(), String> {
    // Get original triangles
    let original_triangles = extract_triangles(&mesh, 0, precision);

    println!(
        "Original mesh: {} faces, {} points",
        mesh.num_faces(),
        mesh.num_points()
    );
    println!("Original triangles:");
    for (i, face_idx) in (0..mesh.num_faces()).enumerate() {
        let face = mesh.face(FaceIndex(face_idx as u32));
        println!(
            "  Face {}: [{}, {}, {}]",
            i, face[0].0, face[1].0, face[2].0
        );
    }

    // Encode
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", method);
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut enc_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut enc_buffer)
        .map_err(|e| format!("Encoding failed: {:?}", e))?;

    println!("Encoded {} bytes", enc_buffer.data().len());

    // Decode
    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder
        .decode(&mut dec_buffer, &mut decoded_mesh)
        .map_err(|e| format!("Decoding failed: {:?}", e))?;

    println!(
        "Decoded mesh: {} faces, {} points",
        decoded_mesh.num_faces(),
        decoded_mesh.num_points()
    );

    println!("Decoded triangles:");
    for face_idx in 0..decoded_mesh.num_faces() {
        let face = decoded_mesh.face(FaceIndex(face_idx as u32));
        println!(
            "  Face {}: [{}, {}, {}]",
            face_idx, face[0].0, face[1].0, face[2].0
        );
    }

    // Get decoded triangles
    let decoded_triangles = extract_triangles(&decoded_mesh, 0, precision);

    // Compare
    if original_triangles.len() != decoded_triangles.len() {
        return Err(format!(
            "Triangle count mismatch: original={}, decoded={}",
            original_triangles.len(),
            decoded_triangles.len()
        ));
    }

    // Find missing triangles
    let missing: Vec<_> = original_triangles.difference(&decoded_triangles).collect();
    let extra: Vec<_> = decoded_triangles.difference(&original_triangles).collect();

    if !missing.is_empty() || !extra.is_empty() {
        let mut msg = String::new();
        if !missing.is_empty() {
            msg.push_str(&format!("Missing triangles: {:?}\n", missing));
        }
        if !extra.is_empty() {
            msg.push_str(&format!("Extra triangles: {:?}\n", extra));
        }
        return Err(msg);
    }

    println!("All {} triangles matched!", original_triangles.len());
    Ok(())
}

#[test]
fn test_single_triangle_sequential() {
    let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 1.0, 0.0]];
    let faces = [[0, 1, 2]];

    let mesh = create_test_mesh(&positions, &faces);
    roundtrip_and_compare(mesh, 0, 1000).expect("Single triangle sequential failed");
}

#[test]
fn test_single_triangle_edgebreaker() {
    let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 1.0, 0.0]];
    let faces = [[0, 1, 2]];

    let mesh = create_test_mesh(&positions, &faces);
    roundtrip_and_compare(mesh, 1, 1000).expect("Single triangle edgebreaker failed");
}

#[test]
fn test_quad_two_triangles_sequential() {
    // Quad: two triangles sharing an edge
    //  3 --- 2
    //  | \   |
    //  |  \  |
    //  0 --- 1
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let faces = [[0, 1, 2], [0, 2, 3]];

    let mesh = create_test_mesh(&positions, &faces);
    roundtrip_and_compare(mesh, 0, 1000).expect("Quad sequential failed");
}

#[test]
fn test_quad_two_triangles_edgebreaker() {
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let faces = [[0, 1, 2], [0, 2, 3]];

    let mesh = create_test_mesh(&positions, &faces);
    roundtrip_and_compare(mesh, 1, 1000).expect("Quad edgebreaker failed");
}

#[test]
fn test_tetrahedron_sequential() {
    // Regular tetrahedron with correct face winding (outward facing normals)
    // All triangles should have CCW winding when viewed from outside
    let positions = [
        [0.0, 0.0, 0.0],     // 0: bottom-left-front
        [1.0, 0.0, 0.0],     // 1: bottom-right-front
        [0.5, 0.0, 0.866],   // 2: bottom-back
        [0.5, 0.816, 0.289], // 3: top
    ];
    // Proper manifold winding: each edge appears once in each direction
    let faces = [
        [0, 2, 1], // bottom (viewed from below)
        [0, 1, 3], // front-left
        [1, 2, 3], // right
        [2, 0, 3], // back-left
    ];

    let mesh = create_test_mesh(&positions, &faces);
    roundtrip_and_compare(mesh, 0, 1000).expect("Tetrahedron sequential failed");
}

#[test]
fn test_tetrahedron_edgebreaker() {
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.5, 0.0, 0.866],
        [0.5, 0.816, 0.289],
    ];
    let faces = [[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]];

    let mesh = create_test_mesh(&positions, &faces);
    roundtrip_and_compare(mesh, 1, 1000).expect("Tetrahedron edgebreaker failed");
}

#[test]
fn test_cube_sequential() {
    // Cube with 8 vertices, 12 triangles
    let positions = [
        [0.0, 0.0, 0.0], // 0
        [1.0, 0.0, 0.0], // 1
        [1.0, 1.0, 0.0], // 2
        [0.0, 1.0, 0.0], // 3
        [0.0, 0.0, 1.0], // 4
        [1.0, 0.0, 1.0], // 5
        [1.0, 1.0, 1.0], // 6
        [0.0, 1.0, 1.0], // 7
    ];
    let faces = [
        // Bottom (z=0)
        [0, 2, 1],
        [0, 3, 2],
        // Top (z=1)
        [4, 5, 6],
        [4, 6, 7],
        // Front (y=0)
        [0, 1, 5],
        [0, 5, 4],
        // Back (y=1)
        [3, 6, 2],
        [3, 7, 6],
        // Left (x=0)
        [0, 4, 7],
        [0, 7, 3],
        // Right (x=1)
        [1, 2, 6],
        [1, 6, 5],
    ];

    let mesh = create_test_mesh(&positions, &faces);
    roundtrip_and_compare(mesh, 0, 1000).expect("Cube sequential failed");
}

#[test]
fn test_cube_edgebreaker() {
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ];
    let faces = [
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [3, 6, 2],
        [3, 7, 6],
        [0, 4, 7],
        [0, 7, 3],
        [1, 2, 6],
        [1, 6, 5],
    ];

    let mesh = create_test_mesh(&positions, &faces);
    roundtrip_and_compare(mesh, 1, 1000).expect("Cube edgebreaker failed");
}
