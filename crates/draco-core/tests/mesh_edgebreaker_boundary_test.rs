use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;

/// Create a mesh that's a triangle strip with boundaries on both ends.
/// This creates a linear sequence of faces where the encoder will generate
/// L symbols for most faces, but the start and end are boundaries.
fn create_triangle_strip(num_triangles: u32) -> Mesh {
    let mut mesh = Mesh::new();
    // A strip of n triangles needs n+2 vertices
    let num_vertices = num_triangles + 2;
    mesh.set_num_points(num_vertices as usize);
    mesh.set_num_faces(num_triangles as usize);

    for i in 0..num_triangles {
        if i % 2 == 0 {
            mesh.set_face(
                FaceIndex(i),
                [PointIndex(i), PointIndex(i + 1), PointIndex(i + 2)],
            );
        } else {
            mesh.set_face(
                FaceIndex(i),
                [PointIndex(i), PointIndex(i + 2), PointIndex(i + 1)],
            );
        }
    }

    mesh
}

/// Create a mesh with a fan topology (one central vertex connected to all others).
/// This creates a boundary mesh with all edges incident to the center being interior.
fn create_triangle_fan(num_triangles: u32) -> Mesh {
    let mut mesh = Mesh::new();
    let num_vertices = num_triangles + 1; // center + outer vertices
    mesh.set_num_points(num_vertices as usize);
    mesh.set_num_faces(num_triangles as usize);

    for i in 0..num_triangles {
        let next_i = (i + 1) % num_triangles;
        mesh.set_face(
            FaceIndex(i),
            [PointIndex(0), PointIndex(i + 1), PointIndex(next_i + 1)],
        );
    }

    mesh
}

/// Create a grid mesh with boundaries on all four edges.
/// This is a more complex boundary mesh that may generate Split symbols.
fn create_grid_mesh(rows: u32, cols: u32) -> Mesh {
    let mut mesh = Mesh::new();
    let num_vertices = (rows + 1) * (cols + 1);
    let num_faces = rows * cols * 2;
    mesh.set_num_points(num_vertices as usize);
    mesh.set_num_faces(num_faces as usize);

    let vertex_index = |r: u32, c: u32| -> u32 { r * (cols + 1) + c };

    let mut face_idx = 0;
    for r in 0..rows {
        for c in 0..cols {
            let v00 = vertex_index(r, c);
            let v10 = vertex_index(r + 1, c);
            let v01 = vertex_index(r, c + 1);
            let v11 = vertex_index(r + 1, c + 1);

            mesh.set_face(
                FaceIndex(face_idx),
                [PointIndex(v00), PointIndex(v10), PointIndex(v01)],
            );
            face_idx += 1;
            mesh.set_face(
                FaceIndex(face_idx),
                [PointIndex(v10), PointIndex(v11), PointIndex(v01)],
            );
            face_idx += 1;
        }
    }

    mesh
}

#[test]
fn test_triangle_strip_roundtrip() {
    for num_triangles in [2, 3, 5, 10] {
        let mesh = create_triangle_strip(num_triangles);

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_method", 1); // Edgebreaker

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh);
        let mut encoder_buffer = EncoderBuffer::new();

        let encode_result = encoder.encode(&options, &mut encoder_buffer);
        assert!(
            encode_result.is_ok(),
            "Encode failed for strip with {} triangles: {:?}",
            num_triangles,
            encode_result.err()
        );

        let mut decoder = MeshDecoder::new();
        let mut decoder_buffer = DecoderBuffer::new(encoder_buffer.data());

        let mut decoded_mesh = Mesh::new();
        let decode_result = decoder.decode(&mut decoder_buffer, &mut decoded_mesh);
        assert!(
            decode_result.is_ok(),
            "Decode failed for strip with {} triangles: {:?}",
            num_triangles,
            decode_result.err()
        );

        assert_eq!(
            decoded_mesh.num_faces(),
            num_triangles as usize,
            "Face count mismatch for strip with {} triangles",
            num_triangles
        );
    }
}

#[test]
fn test_triangle_fan_roundtrip() {
    for num_triangles in [3, 4, 6, 10] {
        let mesh = create_triangle_fan(num_triangles);

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_method", 1); // Edgebreaker

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh);
        let mut encoder_buffer = EncoderBuffer::new();

        let encode_result = encoder.encode(&options, &mut encoder_buffer);
        assert!(
            encode_result.is_ok(),
            "Encode failed for fan with {} triangles: {:?}",
            num_triangles,
            encode_result.err()
        );

        let mut decoder = MeshDecoder::new();
        let mut decoder_buffer = DecoderBuffer::new(encoder_buffer.data());

        let mut decoded_mesh = Mesh::new();
        let decode_result = decoder.decode(&mut decoder_buffer, &mut decoded_mesh);
        assert!(
            decode_result.is_ok(),
            "Decode failed for fan with {} triangles: {:?}",
            num_triangles,
            decode_result.err()
        );

        assert_eq!(
            decoded_mesh.num_faces(),
            num_triangles as usize,
            "Face count mismatch for fan with {} triangles",
            num_triangles
        );
    }
}

#[test]
fn test_grid_mesh_roundtrip() {
    for (rows, cols) in [(2, 2), (3, 3), (4, 4), (5, 5)] {
        let mesh = create_grid_mesh(rows, cols);
        let expected_faces = (rows * cols * 2) as usize;

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_method", 1); // Edgebreaker

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh);
        let mut encoder_buffer = EncoderBuffer::new();

        let encode_result = encoder.encode(&options, &mut encoder_buffer);
        assert!(
            encode_result.is_ok(),
            "Encode failed for grid {}x{}: {:?}",
            rows,
            cols,
            encode_result.err()
        );

        let mut decoder = MeshDecoder::new();
        let mut decoder_buffer = DecoderBuffer::new(encoder_buffer.data());

        let mut decoded_mesh = Mesh::new();
        let decode_result = decoder.decode(&mut decoder_buffer, &mut decoded_mesh);
        assert!(
            decode_result.is_ok(),
            "Decode failed for grid {}x{}: {:?}",
            rows,
            cols,
            decode_result.err()
        );

        assert_eq!(
            decoded_mesh.num_faces(),
            expected_faces,
            "Face count mismatch for grid {}x{}",
            rows,
            cols
        );
    }
}

/// Test with a specific mesh structure that creates nested Splits.
/// This tests the case where:
/// - S1 splits into two branches
/// - Each branch contains another S
/// - The topology events must be correctly generated
#[test]
fn test_nested_splits_boundary() {
    // Create a mesh with structure that generates nested splits
    //
    //       3
    //      /|\
    //     / | \
    //    4--0--2
    //     \ | /
    //      \|/
    //       1
    //
    // This creates 4 triangles with vertex 0 in the center.
    // Faces: (0,1,2), (0,2,3), (0,3,4), (0,4,1)
    // With boundaries on the outer edges.

    let mut mesh = Mesh::new();
    mesh.set_num_points(5);
    mesh.set_num_faces(4);

    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);
    mesh.set_face(FaceIndex(2), [PointIndex(0), PointIndex(3), PointIndex(4)]);
    mesh.set_face(FaceIndex(3), [PointIndex(0), PointIndex(4), PointIndex(1)]);

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1); // Edgebreaker

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut encoder_buffer = EncoderBuffer::new();

    let encode_result = encoder.encode(&options, &mut encoder_buffer);
    assert!(
        encode_result.is_ok(),
        "Encode failed: {:?}",
        encode_result.err()
    );

    let mut decoder = MeshDecoder::new();
    let mut decoder_buffer = DecoderBuffer::new(encoder_buffer.data());

    let mut decoded_mesh = Mesh::new();
    let decode_result = decoder.decode(&mut decoder_buffer, &mut decoded_mesh);
    assert!(
        decode_result.is_ok(),
        "Decode failed: {:?}",
        decode_result.err()
    );

    assert_eq!(decoded_mesh.num_faces(), 4);
}

/// Test with a large grid to stress test boundary handling
#[test]
fn test_large_grid_roundtrip() {
    let mesh = create_grid_mesh(10, 10);
    let expected_faces = 200;

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1); // Edgebreaker

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut encoder_buffer = EncoderBuffer::new();

    let encode_result = encoder.encode(&options, &mut encoder_buffer);
    assert!(
        encode_result.is_ok(),
        "Encode failed: {:?}",
        encode_result.err()
    );

    let mut decoder = MeshDecoder::new();
    let mut decoder_buffer = DecoderBuffer::new(encoder_buffer.data());

    let mut decoded_mesh = Mesh::new();
    let decode_result = decoder.decode(&mut decoder_buffer, &mut decoded_mesh);
    assert!(
        decode_result.is_ok(),
        "Decode failed: {:?}",
        decode_result.err()
    );

    assert_eq!(decoded_mesh.num_faces(), expected_faces);
}
