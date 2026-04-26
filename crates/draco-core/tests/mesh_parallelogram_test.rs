use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;

#[test]
fn test_mesh_encode_decode_parallelogram() {
    let mut mesh = Mesh::new();
    let mut pos_att = PointAttribute::new();

    let num_points = 4;
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );

    // A simple quad (2 triangles)
    // 0 --- 1
    // | \   |
    // |  \  |
    // 3 --- 2
    let positions: [f32; 12] = [
        0.0, 0.0, 0.0, // 0
        1.0, 0.0, 0.0, // 1
        1.0, 1.0, 0.0, // 2
        0.0, 1.0, 0.0, // 3
    ];

    let buffer = pos_att.buffer_mut();
    for i in 0..num_points {
        let bytes = [
            positions[i * 3].to_le_bytes(),
            positions[i * 3 + 1].to_le_bytes(),
            positions[i * 3 + 2].to_le_bytes(),
        ]
        .concat();
        buffer.write(i * 12, &bytes);
    }

    mesh.add_attribute(pos_att);

    // Faces
    mesh.set_num_faces(2);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);

    // Encode
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    // Default to Edgebreaker (which is the default in C++ and now in Rust)
    options.set_attribute_int(0, "quantization_bits", 10);
    // Force Parallelogram prediction (1)
    options.set_prediction_scheme(1);

    let mut enc_buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut enc_buffer);
    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());

    // Decode
    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let status = decoder.decode(&mut dec_buffer, &mut decoded_mesh);
    assert!(status.is_ok(), "Decoding failed: {:?}", status.err());

    assert_eq!(decoded_mesh.num_faces(), 2);
    assert_eq!(decoded_mesh.num_points(), 4);

    // Note: Edgebreaker reorders vertices based on traversal, so we validate
    // the actual geometry content rather than exact indices.

    // Build a set of original triangles (as sorted position tuples)
    let original_positions: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0], // 0
        [1.0, 0.0, 0.0], // 1
        [1.0, 1.0, 0.0], // 2
        [0.0, 1.0, 0.0], // 3
    ];
    let original_faces = [
        [0usize, 1, 2], // 0, 1, 2
        [0, 2, 3],      // 0, 2, 3
    ];

    // Helper to get position from decoded mesh (using proper attribute mapping)
    let get_position = |mesh: &Mesh, point_idx: PointIndex| -> [f32; 3] {
        let att = mesh.attribute(0);
        let buffer = att.buffer();
        // Use mapped_index to get the attribute value index for this point
        let val_index = att.mapped_index(point_idx);
        let byte_offset = val_index.0 as usize * att.byte_stride() as usize;
        let mut bytes = [0u8; 12];
        buffer.read(byte_offset, &mut bytes);
        [
            f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        ]
    };

    // Check that all original positions exist in decoded mesh
    println!("=== Decoded positions ===");
    for i in 0..4 {
        let pos = get_position(&decoded_mesh, PointIndex(i as u32));
        println!("Point {}: {:?}", i, pos);
        let mut found = false;
        for orig_pos in &original_positions {
            if (pos[0] - orig_pos[0]).abs() < 0.01
                && (pos[1] - orig_pos[1]).abs() < 0.01
                && (pos[2] - orig_pos[2]).abs() < 0.01
            {
                found = true;
                break;
            }
        }
        assert!(found, "Position {:?} not found in original positions", pos);
    }

    // Check that decoded faces form valid triangles from original geometry
    // (positions should match, though indices may be different)
    println!("=== Decoded faces ===");
    let mut decoded_triangles: Vec<[[f32; 3]; 3]> = Vec::new();
    for f_idx in 0..2 {
        let face = decoded_mesh.face(FaceIndex(f_idx as u32));
        let tri = [
            get_position(&decoded_mesh, face[0]),
            get_position(&decoded_mesh, face[1]),
            get_position(&decoded_mesh, face[2]),
        ];
        println!("Face {}: {:?} -> {:?}", f_idx, face, tri);
        decoded_triangles.push(tri);
    }

    // Verify both original triangles exist in decoded mesh
    for orig_face in &original_faces {
        let orig_tri = [
            original_positions[orig_face[0]],
            original_positions[orig_face[1]],
            original_positions[orig_face[2]],
        ];
        // Check if this triangle exists (may be rotated)
        let mut found = false;
        for dec_tri in &decoded_triangles {
            // Check all rotations
            for rot in 0..3 {
                let matches = (0..3).all(|i| {
                    let d = dec_tri[(i + rot) % 3];
                    let o = orig_tri[i];
                    (d[0] - o[0]).abs() < 0.01
                        && (d[1] - o[1]).abs() < 0.01
                        && (d[2] - o[2]).abs() < 0.01
                });
                if matches {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        assert!(
            found,
            "Original triangle {:?} not found in decoded mesh",
            orig_tri
        );
    }

    // Check attributes
    let decoded_att = decoded_mesh.attribute(0);
    assert_eq!(
        decoded_att.attribute_type(),
        GeometryAttributeType::Position
    );
}
