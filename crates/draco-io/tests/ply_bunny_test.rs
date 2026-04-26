//! Test PLY bunny file parsing and encoding.

use std::fs;

use std::path::Path;

use draco_io::ply_reader::PlyReader;

/// Parse the bun_zipper.ply file to verify the structure  
#[test]
fn test_ply_bunny_structure() {
    let ply_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/bun_zipper.ply");
    println!("Reading: {:?}", ply_path);

    let content = fs::read_to_string(&ply_path).expect("Failed to read PLY file");
    let mut lines = content.lines();

    // Parse header
    let first_line = lines.next().unwrap();
    assert_eq!(first_line, "ply", "Expected PLY header");

    let mut vertex_count = 0;
    let mut face_count = 0;
    let mut vertex_properties: Vec<String> = Vec::new();
    let mut in_vertex_element = false;

    for line in &mut lines {
        let line = line.trim();
        if line == "end_header" {
            break;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "element" => {
                if parts.len() >= 3 {
                    if parts[1] == "vertex" {
                        vertex_count = parts[2].parse().unwrap_or(0);
                        in_vertex_element = true;
                    } else {
                        in_vertex_element = false;
                        if parts[1] == "face" {
                            face_count = parts[2].parse().unwrap_or(0);
                        }
                    }
                }
            }
            "property" => {
                if in_vertex_element && parts.len() >= 3 {
                    vertex_properties.push(parts[2].to_string());
                }
            }
            _ => {}
        }
    }

    println!("\n=== PLY Header Info ===");
    println!("Vertex count: {}", vertex_count);
    println!("Face count: {}", face_count);
    println!("Vertex properties: {:?}", vertex_properties);

    // Count actual data lines (vertices + faces)
    let mut vertex_data_count = 0;
    let mut face_data_count = 0;
    let mut total_indices = 0;

    // Read vertex data
    for _ in 0..vertex_count {
        if lines.next().is_some() {
            vertex_data_count += 1;
        }
    }

    // Read face data
    for _ in 0..face_count {
        if let Some(line) = lines.next() {
            face_data_count += 1;
            // Face format: n i0 i1 i2 ...
            let parts: Vec<&str> = line.split_whitespace().collect();
            if !parts.is_empty() {
                let n: usize = parts[0].parse().unwrap_or(0);
                total_indices += n;
            }
        }
    }

    println!("\n=== Data Counts ===");
    println!("Vertex data lines: {}", vertex_data_count);
    println!("Face data lines: {}", face_data_count);
    println!("Total face indices: {}", total_indices);

    // For triangles: face_count * 3 = total_indices
    let triangulated_count = (total_indices as f64 - 2.0 * face_count as f64) as usize;
    println!(
        "Estimated triangulated faces: {} (for 69451 tris should be ~69451 if all tris)",
        triangulated_count
    );

    // Verify expectations
    assert_eq!(vertex_count, 35947, "Expected 35947 vertices in header");
    assert_eq!(face_count, 69451, "Expected 69451 faces in header");

    // Properties should NOT include nx, ny, nz (normals)
    assert!(
        !vertex_properties.contains(&"nx".to_string()),
        "File should not have normals"
    );
    assert!(
        !vertex_properties.contains(&"ny".to_string()),
        "File should not have normals"
    );
    assert!(
        !vertex_properties.contains(&"nz".to_string()),
        "File should not have normals"
    );

    // Expected properties: x, y, z, confidence, intensity
    assert!(
        vertex_properties.contains(&"x".to_string()),
        "Should have x"
    );
    assert!(
        vertex_properties.contains(&"y".to_string()),
        "Should have y"
    );
    assert!(
        vertex_properties.contains(&"z".to_string()),
        "Should have z"
    );

    println!("\n=== Test passed! ===");
}

/// Test reading the bunny PLY through PlyReader and encoding it with Draco
#[test]
fn test_ply_bunny_encode_draco() {
    use draco_core::encoder_buffer::EncoderBuffer;
    use draco_core::encoder_options::EncoderOptions;
    use draco_core::mesh::Mesh as DracoMesh;
    use draco_core::mesh_encoder::MeshEncoder;

    let ply_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/bun_zipper.ply");
    let mut positions_reader = PlyReader::open(&ply_path).expect("Failed to open PLY file");
    let raw_positions = positions_reader
        .read_positions()
        .expect("Failed to read PLY positions");
    let mut reader = PlyReader::open(&ply_path).expect("Failed to open PLY file");
    let draco_mesh = reader.read_mesh().expect("Failed to parse PLY mesh");

    println!("\n=== Parsed PLY Data ===");
    let positions_len = draco_mesh.attribute(0).buffer().data().len() / 4;
    let indices_len = draco_mesh.num_faces() * 3;
    println!("Positions length: {}", positions_len);
    println!("Indices length: {}", indices_len);
    println!("Normals length: 0");

    let vertex_count = draco_mesh.num_points();
    let face_count = draco_mesh.num_faces();

    println!("Vertex count: {}", vertex_count);
    println!("Face count: {}", face_count);

    // Verify counts match expectations
    assert_eq!(
        raw_positions.len(),
        35947,
        "Expected 35947 raw PLY vertices"
    );
    assert!(
        vertex_count <= raw_positions.len(),
        "Mesh point count should not exceed raw PLY vertices"
    );
    assert!(
        vertex_count > 34000,
        "Expected substantial point count after deduplication"
    );
    assert_eq!(
        positions_len,
        vertex_count * 3,
        "Position buffer should match deduplicated mesh point count"
    );
    assert_eq!(face_count, 69451, "Expected 69451 faces");
    assert_eq!(
        indices_len,
        69451 * 3,
        "Expected 69451 * 3 = 208353 indices"
    );

    // Verify index range
    let mut max_index = 0u32;
    let mut min_index = u32::MAX;
    for face_id in 0..face_count {
        for point_index in draco_mesh.face(draco_core::geometry_indices::FaceIndex(face_id as u32))
        {
            max_index = max_index.max(point_index.0);
            min_index = min_index.min(point_index.0);
        }
    }
    println!("Index range: {} to {}", min_index, max_index);
    assert!(
        max_index < vertex_count as u32,
        "Max index should be < vertex count"
    );

    println!("Draco mesh created:");
    println!("  num_points: {}", draco_mesh.num_points());
    println!("  num_faces: {}", draco_mesh.num_faces());

    // Encode
    println!("\n=== Encoding with Draco ===");
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(draco_mesh);
    let mut encoder_buffer = EncoderBuffer::new();
    let mut enc_options = EncoderOptions::default();
    enc_options.set_attribute_int(0, "quantization_bits", 0);

    encoder
        .encode(&enc_options, &mut encoder_buffer)
        .expect("Draco encoding should succeed");

    let encoded_data = encoder_buffer.data();
    println!("Encoded size: {} bytes", encoded_data.len());

    assert!(!encoded_data.is_empty(), "Encoded data should not be empty");

    // Now decode and verify
    println!("\n=== Decoding Draco ===");
    use draco_core::decoder_buffer::DecoderBuffer;
    use draco_core::mesh_decoder::MeshDecoder;

    let mut decoder_buffer = DecoderBuffer::new(encoded_data);

    let mut decoder = MeshDecoder::new();
    let mut decoded_mesh = DracoMesh::new();
    decoder
        .decode(&mut decoder_buffer, &mut decoded_mesh)
        .expect("Decoding should succeed");

    println!("Decoded mesh:");
    println!("  num_points: {}", decoded_mesh.num_points());
    println!("  num_faces: {}", decoded_mesh.num_faces());

    // Verify decoded counts match original
    // Verify decoded counts (Draco may deduplicate vertices, so <= is expected)
    assert!(
        decoded_mesh.num_points() <= vertex_count,
        "Decoded vertex count mismatch: {} > {}",
        decoded_mesh.num_points(),
        vertex_count
    );
    // Sanity check
    assert!(
        decoded_mesh.num_points() > 34000,
        "Decoded vertex count too low: {}",
        decoded_mesh.num_points()
    );
    assert_eq!(
        decoded_mesh.num_faces(),
        face_count,
        "Decoded face count mismatch"
    );

    println!("\n=== Test passed! ===");
}

/// Test that bunny encoded with Rust can be decoded by C++ decoder
#[test]
fn test_bunny_cpp_interop() {
    use draco_core::encoder_buffer::EncoderBuffer;
    use draco_core::encoder_options::EncoderOptions;
    use draco_core::mesh_encoder::MeshEncoder;
    use std::process::Command;

    let ply_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/bun_zipper.ply");
    println!("Reading PLY: {:?}", ply_path);

    let mut positions_reader = PlyReader::open(&ply_path).expect("Failed to open PLY file");
    let raw_positions = positions_reader
        .read_positions()
        .expect("Failed to read PLY positions");
    let mut reader = PlyReader::open(&ply_path).expect("Failed to open PLY file");
    let mesh = reader.read_mesh().expect("Failed to parse PLY mesh");

    let vertex_count = mesh.num_points();
    let face_count = mesh.num_faces();

    println!("Loaded mesh: {} points, {} faces", vertex_count, face_count);
    assert_eq!(
        raw_positions.len(),
        35947,
        "Expected 35947 raw PLY vertices"
    );
    assert!(
        vertex_count <= raw_positions.len(),
        "Mesh point count should not exceed raw PLY vertices"
    );
    assert!(
        vertex_count > 34000,
        "Expected substantial point count after deduplication"
    );
    assert_eq!(face_count, 69451, "Expected 69451 faces");

    // Encode with Draco
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut encoder_buffer = EncoderBuffer::new();
    let enc_options = EncoderOptions::default();

    encoder
        .encode(&enc_options, &mut encoder_buffer)
        .expect("Draco encoding should succeed");

    let encoded_data = encoder_buffer.data();
    println!("Rust encoded size: {} bytes", encoded_data.len());

    // Save to temp file
    let output_path = std::env::temp_dir().join("bunny_rust_encoded.drc");
    fs::write(&output_path, encoded_data).expect("Failed to write file");
    println!("Saved to: {:?}", output_path);

    // Try to decode with C++ decoder (if available via env var or default paths)
    let cpp_decoder_path = std::env::var("DRACO_CPP_DECODER").ok().or_else(|| {
        let candidates = [
            "../../build-original/src/draco/Release/draco_decoder.exe",
            "../../build/src/draco/Release/draco_decoder.exe",
            "../../build/src/draco/Debug/draco_decoder.exe",
        ];
        candidates
            .iter()
            .find(|p| Path::new(p).exists())
            .map(|s| s.to_string())
    });

    if let Some(decoder_path) = cpp_decoder_path {
        if Path::new(&decoder_path).exists() {
            let output = Command::new(&decoder_path)
                .args(["-i", output_path.to_string_lossy().as_ref()])
                .output()
                .expect("Failed to run C++ decoder");

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("C++ decoder stdout:\n{}", stdout);
            println!("C++ decoder stderr:\n{}", stderr);

            // Check for success
            if stdout.contains("Failed") || stderr.contains("Failed") {
                panic!("C++ decoder failed to decode Rust-encoded bunny!");
            } else {
                println!("SUCCESS: C++ decoder can decode Rust-encoded bunny!");
            }
        } else {
            println!(
                "C++ decoder not found at {:?}, skipping interop test",
                decoder_path
            );
        }
    } else {
        println!("C++ decoder not found, skipping interop test");
    }
}
