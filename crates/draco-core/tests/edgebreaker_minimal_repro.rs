//! Minimal reproduction tests for Edgebreaker vertex counting bug.
//!
//! The issue: E*3 + L + R - S should equal num_encoded_vertices, but we get
//! a mismatch of 21 vertices for complex meshes.

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;

fn create_mesh_with_positions(positions: &[[f32; 3]], faces: &[[u32; 3]]) -> Mesh {
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

fn test_edgebreaker_roundtrip(name: &str, mesh: &Mesh) -> Result<(), String> {
    println!("\n=== Testing: {} ===", name);
    println!(
        "Input: {} faces, {} points",
        mesh.num_faces(),
        mesh.num_points()
    );

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1); // Edgebreaker

    let mut enc_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut enc_buffer)
        .map_err(|e| format!("Encode failed: {:?}", e))?;

    println!("Encoded: {} bytes", enc_buffer.data().len());

    // Decode
    let mut decoder_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();

    decoder
        .decode(&mut decoder_buffer, &mut decoded_mesh)
        .map_err(|e| format!("Decode failed: {:?}", e))?;

    println!(
        "Decoded: {} faces, {} points",
        decoded_mesh.num_faces(),
        decoded_mesh.num_points()
    );

    if decoded_mesh.num_faces() != mesh.num_faces() {
        return Err(format!(
            "Face count mismatch: {} vs {}",
            mesh.num_faces(),
            decoded_mesh.num_faces()
        ));
    }
    if decoded_mesh.num_points() != mesh.num_points() {
        return Err(format!(
            "Point count mismatch: {} vs {}",
            mesh.num_points(),
            decoded_mesh.num_points()
        ));
    }

    println!("PASSED!");
    Ok(())
}

/// Single triangle - simplest case
#[test]
fn test_01_single_triangle() {
    let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 1.0, 0.0]];
    let faces = [[0, 1, 2]];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Single triangle", &mesh).unwrap();
}

/// Two triangles sharing an edge (quad split)
#[test]
fn test_02_two_triangles() {
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let faces = [[0, 1, 2], [0, 2, 3]];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Two triangles (quad)", &mesh).unwrap();
}

/// Tetrahedron - closed mesh, 4 faces
#[test]
fn test_03_tetrahedron() {
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.5, 0.0, 0.866],
        [0.5, 0.816, 0.289],
    ];
    // Proper winding for manifold mesh
    let faces = [
        [0, 2, 1], // bottom
        [0, 1, 3], // front
        [1, 2, 3], // right
        [2, 0, 3], // left
    ];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Tetrahedron (closed)", &mesh).unwrap();
}

/// Triangle fan - all triangles share one vertex
#[test]
fn test_04_triangle_fan() {
    let positions = [
        [0.0, 0.0, 0.0], // center
        [1.0, 0.0, 0.0],
        [0.707, 0.707, 0.0],
        [0.0, 1.0, 0.0],
        [-0.707, 0.707, 0.0],
        [-1.0, 0.0, 0.0],
    ];
    let faces = [[0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 5]];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Triangle fan (4 triangles)", &mesh).unwrap();
}

/// Triangle strip - 4 triangles in a strip
#[test]
fn test_05_triangle_strip() {
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.5, 1.0, 0.0],
        [1.5, 1.0, 0.0],
        [1.0, 2.0, 0.0],
        [2.0, 2.0, 0.0],
    ];
    let faces = [[0, 1, 2], [1, 3, 2], [2, 3, 4], [3, 5, 4]];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Triangle strip", &mesh).unwrap();
}

/// Two separate triangles (two components)
#[test]
fn test_06_two_components() {
    let positions = [
        // Component 1
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.5, 1.0, 0.0],
        // Component 2
        [5.0, 0.0, 0.0],
        [6.0, 0.0, 0.0],
        [5.5, 1.0, 0.0],
    ];
    let faces = [[0, 1, 2], [3, 4, 5]];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Two components (2 triangles)", &mesh).unwrap();
}

/// Three separate triangles (three components)
#[test]
fn test_07_three_components() {
    let positions = [
        // Component 1
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.5, 1.0, 0.0],
        // Component 2
        [5.0, 0.0, 0.0],
        [6.0, 0.0, 0.0],
        [5.5, 1.0, 0.0],
        // Component 3
        [10.0, 0.0, 0.0],
        [11.0, 0.0, 0.0],
        [10.5, 1.0, 0.0],
    ];
    let faces = [[0, 1, 2], [3, 4, 5], [6, 7, 8]];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Three components", &mesh).unwrap();
}

/// Mesh that requires split symbols - two branches from center
#[test]
fn test_08_split_mesh() {
    // Diamond shape with 4 triangles meeting at center
    let positions = [
        [0.0, 0.0, 0.0],  // center
        [1.0, 0.0, 0.0],  // right
        [0.0, 1.0, 0.0],  // top
        [-1.0, 0.0, 0.0], // left
        [0.0, -1.0, 0.0], // bottom
    ];
    let faces = [
        [0, 1, 2], // top-right
        [0, 2, 3], // top-left
        [0, 3, 4], // bottom-left
        [0, 4, 1], // bottom-right
    ];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Split mesh (diamond)", &mesh).unwrap();
}

/// Two tetrahedrons (two closed components)
#[test]
fn test_09_two_tetrahedrons() {
    let positions = [
        // Tetrahedron 1
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.5, 0.0, 0.866],
        [0.5, 0.816, 0.289],
        // Tetrahedron 2
        [5.0, 0.0, 0.0],
        [6.0, 0.0, 0.0],
        [5.5, 0.0, 0.866],
        [5.5, 0.816, 0.289],
    ];
    let faces = [
        // Tetrahedron 1
        [0, 2, 1],
        [0, 1, 3],
        [1, 2, 3],
        [2, 0, 3],
        // Tetrahedron 2
        [4, 6, 5],
        [4, 5, 7],
        [5, 6, 7],
        [6, 4, 7],
    ];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Two tetrahedrons", &mesh).unwrap();
}

/// Cube (closed mesh, 12 triangles)
#[test]
fn test_10_cube() {
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
        // Front
        [0, 2, 1],
        [0, 3, 2],
        // Back
        [4, 5, 6],
        [4, 6, 7],
        // Bottom
        [0, 1, 5],
        [0, 5, 4],
        // Top
        [3, 6, 2],
        [3, 7, 6],
        // Left
        [0, 4, 7],
        [0, 7, 3],
        // Right
        [1, 2, 6],
        [1, 6, 5],
    ];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Cube", &mesh).unwrap();
}

/// Open mesh with a hole (boundary)
#[test]
fn test_11_open_mesh_with_hole() {
    // Simpler: A ring of 8 triangles around a center hole (annulus shape)
    // Outer ring: 0,1,2,3 (square)
    // Inner ring: 4,5,6,7 (square hole)
    // 8 triangles connecting them
    let positions = [
        // Outer square
        [-1.0, -1.0, 0.0], // 0
        [1.0, -1.0, 0.0],  // 1
        [1.0, 1.0, 0.0],   // 2
        [-1.0, 1.0, 0.0],  // 3
        // Inner square (hole boundary)
        [-0.5, -0.5, 0.0], // 4
        [0.5, -0.5, 0.0],  // 5
        [0.5, 0.5, 0.0],   // 6
        [-0.5, 0.5, 0.0],  // 7
    ];
    let faces = [
        // Bottom edge
        [0, 1, 5],
        [0, 5, 4],
        // Right edge
        [1, 2, 6],
        [1, 6, 5],
        // Top edge
        [2, 3, 7],
        [2, 7, 6],
        // Left edge
        [3, 0, 4],
        [3, 4, 7],
    ];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Open mesh with hole", &mesh).unwrap();
}

/// Mesh with multiple boundary loops
#[test]
fn test_12_multiple_boundaries() {
    // Two separate open squares
    let positions = [
        // Square 1 (vertices 0-3)
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        // Square 2 (vertices 4-7)
        [5.0, 0.0, 0.0],
        [6.0, 0.0, 0.0],
        [6.0, 1.0, 0.0],
        [5.0, 1.0, 0.0],
    ];
    let faces = [
        // Square 1
        [0, 1, 2],
        [0, 2, 3],
        // Square 2
        [4, 5, 6],
        [4, 6, 7],
    ];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Multiple boundaries (2 open squares)", &mesh).unwrap();
}

/// Large grid to stress test
#[test]
fn test_13_large_grid() {
    let grid_size = 10;
    let mut positions = Vec::new();
    for y in 0..=grid_size {
        for x in 0..=grid_size {
            positions.push([x as f32, y as f32, 0.0]);
        }
    }

    let mut faces = Vec::new();
    for y in 0..grid_size {
        for x in 0..grid_size {
            let i0 = y * (grid_size + 1) + x;
            let i1 = i0 + 1;
            let i2 = i0 + grid_size + 1;
            let i3 = i2 + 1;
            faces.push([i0 as u32, i1 as u32, i3 as u32]);
            faces.push([i0 as u32, i3 as u32, i2 as u32]);
        }
    }

    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip(
        &format!("{}x{} grid ({} faces)", grid_size, grid_size, faces.len()),
        &mesh,
    )
    .unwrap();
}

/// Mixed closed and open components
#[test]
fn test_14_mixed_components() {
    let positions = [
        // Open triangle (boundary)
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.5, 1.0, 0.0],
        // Tetrahedron (closed)
        [5.0, 0.0, 0.0],
        [6.0, 0.0, 0.0],
        [5.5, 0.0, 0.866],
        [5.5, 0.816, 0.289],
    ];
    let faces = [
        // Open triangle
        [0, 1, 2],
        // Tetrahedron
        [3, 5, 4],
        [3, 4, 6],
        [4, 5, 6],
        [5, 3, 6],
    ];
    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip("Mixed: open + closed", &mesh).unwrap();
}

/// Complex mesh with many splits - wheel pattern
#[test]
fn test_15_wheel() {
    let num_spokes = 12;
    let mut positions = vec![[0.0f32, 0.0, 0.0]]; // center

    for i in 0..num_spokes {
        let angle = (i as f32) * 2.0 * std::f32::consts::PI / (num_spokes as f32);
        positions.push([angle.cos(), angle.sin(), 0.0]);
    }

    let mut faces = Vec::new();
    for i in 0..num_spokes {
        let next = if i + 1 == num_spokes { 1 } else { i + 2 };
        faces.push([0, (i + 1) as u32, next as u32]);
    }

    let mesh = create_mesh_with_positions(&positions, &faces);
    test_edgebreaker_roundtrip(&format!("Wheel ({} spokes)", num_spokes), &mesh).unwrap();
}

/// Run all tests and report which ones fail
#[test]
fn test_all_minimal_repros() {
    // Test data structure contains boxed closures that return meshes.
    // The type Vec<(&str, Box<dyn Fn() -> Mesh>)> is complex but necessary
    // for parameterized test cases with dynamic dispatch.
    #[allow(clippy::type_complexity)]
    let test_cases: Vec<(&str, Box<dyn Fn() -> Mesh>)> = vec![
        (
            "Single triangle",
            Box::new(|| {
                create_mesh_with_positions(
                    &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 1.0, 0.0]],
                    &[[0, 1, 2]],
                )
            }),
        ),
        (
            "Two triangles",
            Box::new(|| {
                create_mesh_with_positions(
                    &[
                        [0.0, 0.0, 0.0],
                        [1.0, 0.0, 0.0],
                        [1.0, 1.0, 0.0],
                        [0.0, 1.0, 0.0],
                    ],
                    &[[0, 1, 2], [0, 2, 3]],
                )
            }),
        ),
        (
            "Two components",
            Box::new(|| {
                create_mesh_with_positions(
                    &[
                        [0.0, 0.0, 0.0],
                        [1.0, 0.0, 0.0],
                        [0.5, 1.0, 0.0],
                        [5.0, 0.0, 0.0],
                        [6.0, 0.0, 0.0],
                        [5.5, 1.0, 0.0],
                    ],
                    &[[0, 1, 2], [3, 4, 5]],
                )
            }),
        ),
    ];

    let mut passed = 0;
    let mut failed = 0;

    for (name, mesh_fn) in test_cases {
        let mesh = mesh_fn();
        match test_edgebreaker_roundtrip(name, &mesh) {
            Ok(_) => passed += 1,
            Err(e) => {
                println!("FAILED: {}: {}", name, e);
                failed += 1;
            }
        }
    }

    println!("\n=== Summary: {} passed, {} failed ===", passed, failed);
    assert_eq!(failed, 0, "Some tests failed");
}

/// Test decoding a C++-encoded annulus file
#[test]
fn test_decode_cpp_annulus() {
    // Use testdata path relative to the crate
    let testdata = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata");
    let path = testdata.join("annulus_eb.drc");
    let annulus_drc = match std::fs::read(&path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "Skipping test_decode_cpp_annulus: file not found at {:?}",
                path
            );
            return; // Skip test if file doesn't exist
        }
        Err(e) => panic!("Failed to read annulus_eb.drc: {:?}", e),
    };

    let mut decoder_buffer = DecoderBuffer::new(&annulus_drc);
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();

    match decoder.decode(&mut decoder_buffer, &mut decoded_mesh) {
        Ok(_) => {
            println!(
                "C++ annulus decoded: {} faces, {} points",
                decoded_mesh.num_faces(),
                decoded_mesh.num_points()
            );
            assert_eq!(decoded_mesh.num_faces(), 8);
            assert_eq!(decoded_mesh.num_points(), 8);
        }
        Err(e) => {
            panic!("Failed to decode C++ annulus: {:?}", e);
        }
    }
}

/// Debug test to analyze annulus corner table structure
#[test]
fn test_debug_annulus_corner_table() {
    use draco_core::corner_table::CornerTable;
    use draco_core::geometry_indices::{CornerIndex, VertexIndex};

    // Create the annulus positions (kept for reference only).
    // The `CornerTable` below is constructed from `faces` and vertex indices;
    // these coordinates exist for human-readable inspection of the test fixture.
    let positions = [
        // Outer square
        [-1.0, -1.0, 0.0], // 0
        [1.0, -1.0, 0.0],  // 1
        [1.0, 1.0, 0.0],   // 2
        [-1.0, 1.0, 0.0],  // 3
        // Inner square (hole boundary)
        [-0.5, -0.5, 0.0], // 4
        [0.5, -0.5, 0.0],  // 5
        [0.5, 0.5, 0.0],   // 6
        [-0.5, 0.5, 0.0],  // 7
    ];
    let faces = [
        // Bottom edge
        [0, 1, 5],
        [0, 5, 4],
        // Right edge
        [1, 2, 6],
        [1, 6, 5],
        // Top edge
        [2, 3, 7],
        [2, 7, 6],
        // Left edge
        [3, 0, 4],
        [3, 4, 7],
    ];

    // Build corner table
    let vertex_faces: Vec<[VertexIndex; 3]> = faces
        .iter()
        .map(|f| [VertexIndex(f[0]), VertexIndex(f[1]), VertexIndex(f[2])])
        .collect();

    let mut corner_table = CornerTable::new(0);
    corner_table.init(&vertex_faces);

    // Sanity check: Ensure vertex count matches our coordinate reference.
    assert_eq!(corner_table.num_vertices(), positions.len());

    println!("\n=== Corner Table Structure for Annulus ===");
    println!(
        "Vertices: {}, Faces: {}, Corners: {}",
        corner_table.num_vertices(),
        corner_table.num_faces(),
        corner_table.num_corners()
    );

    println!("\n=== Face/Corner details ===");
    for f in 0..corner_table.num_faces() {
        let face_id = draco_core::geometry_indices::FaceIndex(f as u32);
        let c0 = corner_table.first_corner(face_id);
        let c1 = corner_table.next(c0);
        let c2 = corner_table.previous(c0);

        let v0 = corner_table.vertex(c0);
        let v1 = corner_table.vertex(c1);
        let v2 = corner_table.vertex(c2);

        let opp0 = corner_table.opposite(c0);
        let opp1 = corner_table.opposite(c1);
        let opp2 = corner_table.opposite(c2);

        let rc0 = corner_table.right_corner(c0);
        let lc0 = corner_table.left_corner(c0);

        println!(
            "Face {}: corners [{},{},{}] vertices [{},{},{}]",
            f, c0.0, c1.0, c2.0, v0.0, v1.0, v2.0
        );
        println!("  Opposites: [{},{},{}]", opp0.0, opp1.0, opp2.0);
        println!("  Corner 0 right_corner={} left_corner={}", rc0.0, lc0.0);
    }

    // Find boundary edges
    println!("\n=== Boundary Edges ===");
    for c in 0..corner_table.num_corners() {
        let corner = CornerIndex(c as u32);
        if corner_table.opposite(corner) == draco_core::geometry_indices::INVALID_CORNER_INDEX {
            let v_from = corner_table.vertex(corner_table.next(corner));
            let v_to = corner_table.vertex(corner_table.previous(corner));
            println!(
                "Boundary edge: corner {} (face {}), edge {}→{}",
                c,
                c / 3,
                v_from.0,
                v_to.0
            );
        }
    }

    println!("\n=== Key Insight ===");
    println!("For annulus, when starting at a face with a boundary edge:");
    println!("- The start_corner should be OPPOSITE to the boundary edge");
    println!("- This means the tip vertex is the one ACROSS from the boundary");
    println!("- For face 0 (verts 0,1,5): if edge 0→1 is boundary, start_corner has vertex 5");
}
