//! GLB Round-trip test using IridescenceLamp.glb
//!
//! This test verifies that GLB files can be decoded and re-encoded correctly.

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_io::gltf_reader::GltfReader;
use std::path::Path;

#[test]
fn test_glb_decode_and_inspect() {
    // Path: crates/draco-io -> crates -> Draco root -> testdata
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
        .join("IridescenceLamp.glb");

    if !test_file.exists() {
        eprintln!("Test file not found: {:?}, skipping", test_file);
        return;
    }

    println!("Loading GLB file: {:?}", test_file);

    // Read the GLB file
    let reader = GltfReader::open(&test_file).expect("Failed to open GLB");
    let meshes = reader.decode_all_meshes().expect("Failed to decode meshes");

    println!("Decoded {} meshes:", meshes.len());
    for (i, mesh) in meshes.iter().enumerate() {
        let pos_att_id = mesh.named_attribute_id(GeometryAttributeType::Position);
        let norm_att_id = mesh.named_attribute_id(GeometryAttributeType::Normal);
        let uv_att_id = mesh.named_attribute_id(GeometryAttributeType::TexCoord);

        let num_points = mesh.num_points();
        let num_faces = mesh.num_faces();

        println!("  Mesh {}: {} points, {} faces", i, num_points, num_faces);
        println!("    Position attr: {}", pos_att_id);
        println!("    Normal attr: {}", norm_att_id);
        println!("    TexCoord attr: {}", uv_att_id);

        // Extract a few sample positions
        if pos_att_id >= 0 {
            let pos_attr = mesh.attribute(pos_att_id);
            let num_entries = pos_attr.size();
            println!("    Position entries: {}", num_entries);
            let buffer = pos_attr.buffer();
            let byte_stride = pos_attr.byte_stride() as usize;
            for j in 0..3.min(num_entries) {
                let offset = j * byte_stride;
                let mut bytes = [0u8; 12];
                if offset + 12 <= buffer.data_size() {
                    buffer.read(offset, &mut bytes);
                    let x = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                    let y = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                    let z = f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
                    println!("      Pos[{}]: [{}, {}, {}]", j, x, y, z);
                }
            }
        }

        // Extract a few sample faces
        for j in 0..3.min(num_faces) {
            let face = mesh.face(FaceIndex(j as u32));
            println!(
                "    Face[{}]: [{}, {}, {}]",
                j, face[0].0, face[1].0, face[2].0
            );
        }
    }

    assert!(!meshes.is_empty(), "Should decode at least one mesh");
}

#[test]
fn test_glb_roundtrip_with_draco() {
    // Path: crates/draco-io -> crates -> Draco root -> testdata
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
        .join("IridescenceLamp.glb");

    if !test_file.exists() {
        eprintln!("Test file not found: {:?}, skipping", test_file);
        return;
    }

    // Read the original GLB
    let reader = GltfReader::open(&test_file).expect("Failed to open GLB");
    let original_meshes = reader.decode_all_meshes().expect("Failed to decode meshes");

    println!("Original meshes: {}", original_meshes.len());
    for (i, mesh) in original_meshes.iter().enumerate() {
        println!(
            "  Mesh {}: {} faces, {} points, {} attrs",
            i,
            mesh.num_faces(),
            mesh.num_points(),
            mesh.num_attributes()
        );
    }

    // Try encoding just the first mesh to isolate the issue
    if let Some(first_mesh) = original_meshes.first() {
        println!("\nTrying to encode first mesh...");

        // Encode with Draco
        use draco_core::encoder_buffer::EncoderBuffer;
        use draco_core::encoder_options::EncoderOptions;
        use draco_core::mesh_encoder::MeshEncoder;

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(first_mesh.clone());

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_method", 1); // Edgebreaker

        let mut enc_buffer = EncoderBuffer::new();
        match encoder.encode(&options, &mut enc_buffer) {
            Ok(_) => println!("Encoding succeeded! {} bytes", enc_buffer.data().len()),
            Err(e) => {
                println!("Encoding failed: {:?}", e);
                panic!("Failed to encode first mesh: {:?}", e);
            }
        }

        // Peek at the encoded data to count split symbols
        let data = enc_buffer.data();
        println!(
            "First 100 bytes of encoded data: {:?}",
            &data[..100.min(data.len())]
        );

        // Now try to decode
        println!("\nTrying to decode...");
        use draco_core::decoder_buffer::DecoderBuffer;
        use draco_core::mesh_decoder::MeshDecoder;

        let mut decoder_buffer = DecoderBuffer::new(enc_buffer.data());
        let mut decoded_mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();

        match decoder.decode(&mut decoder_buffer, &mut decoded_mesh) {
            Ok(_) => println!(
                "Decoding succeeded! {} faces, {} points",
                decoded_mesh.num_faces(),
                decoded_mesh.num_points()
            ),
            Err(e) => {
                println!("Decoding failed: {:?}", e);

                // Save to a file for analysis
                let debug_path = std::env::temp_dir().join("debug_draco.drc");
                std::fs::write(&debug_path, enc_buffer.data()).expect("Failed to write debug file");
                println!("Saved encoded data to {:?}", debug_path);

                panic!("Failed to decode: {:?}", e);
            }
        }
    }
}

#[test]
fn test_decode_cpp_encoded_bunny() {
    // Try to decode a file encoded by C++ Draco
    let drc_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
        .join("bunny_cpp_standard.drc");

    if !drc_path.exists() {
        println!("Skipping test - bunny_cpp.drc not found at {:?}", drc_path);
        return;
    }

    let data = std::fs::read(&drc_path).expect("Failed to read bunny_cpp.drc");
    println!("Read {} bytes from {:?}", data.len(), drc_path);

    let mut decoder = MeshDecoder::new();
    let mut mesh = Mesh::new();
    let mut buffer = DecoderBuffer::new(&data);

    match decoder.decode(&mut buffer, &mut mesh) {
        Ok(_) => {
            println!("Successfully decoded C++ encoded mesh:");
            println!("  Faces: {}", mesh.num_faces());
            println!("  Points: {}", mesh.num_points());
            println!("  Attributes: {}", mesh.num_attributes());
        }
        Err(e) => {
            panic!("Failed to decode C++ encoded file: {:?}", e);
        }
    }
}

#[test]
fn test_glb_mesh_topology() {
    // Analyze the GLB mesh topology
    let glb_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
        .join("IridescenceLamp.glb");

    if !glb_path.exists() {
        println!(
            "Skipping test - IridescenceLamp.glb not found at {:?}",
            glb_path
        );
        return;
    }

    let reader = GltfReader::open(&glb_path).expect("Failed to open GLB");
    let meshes = reader.decode_all_meshes().expect("Failed to decode meshes");

    let mesh = &meshes[0];
    println!(
        "Mesh 0: {} faces, {} points",
        mesh.num_faces(),
        mesh.num_points()
    );

    // Build edge count to detect boundaries
    use std::collections::HashMap;
    let mut edge_counts: HashMap<(u32, u32), u32> = HashMap::new();

    for fi in 0..mesh.num_faces() {
        let f = mesh.face(FaceIndex(fi as u32));
        for i in 0..3 {
            let v0 = f[i].0;
            let v1 = f[(i + 1) % 3].0;
            let edge = if v0 < v1 { (v0, v1) } else { (v1, v0) };
            *edge_counts.entry(edge).or_insert(0) += 1;
        }
    }

    let boundary_edges: Vec<_> = edge_counts
        .iter()
        .filter(|(_, &count)| count == 1)
        .collect();
    let non_manifold_edges: Vec<_> = edge_counts.iter().filter(|(_, &count)| count > 2).collect();

    println!("Total edges: {}", edge_counts.len());
    println!("Boundary edges (used once): {}", boundary_edges.len());
    println!(
        "Non-manifold edges (used >2 times): {}",
        non_manifold_edges.len()
    );

    // Euler characteristic: V - E + F = 2 - 2g (for closed surface)
    // For surface with b boundary components: V - E + F = 2 - 2g - b
    let v = mesh.num_points();
    let e = edge_counts.len();
    let f = mesh.num_faces();
    let chi = v as i64 - e as i64 + f as i64;
    println!(
        "Euler characteristic (V - E + F): {} - {} + {} = {}",
        v, e, f, chi
    );

    if boundary_edges.is_empty() {
        println!("Mesh is CLOSED (no boundaries)");
    } else {
        println!("Mesh has BOUNDARIES");
    }
}

#[test]
fn test_glb_roundtrip_sequential() {
    // Test with sequential encoding (simpler, should work)
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("testdata")
        .join("IridescenceLamp.glb");

    if !test_file.exists() {
        eprintln!("Test file not found: {:?}, skipping", test_file);
        return;
    }

    let reader = GltfReader::open(&test_file).expect("Failed to open GLB");
    let original_meshes = reader.decode_all_meshes().expect("Failed to decode meshes");

    if let Some(first_mesh) = original_meshes.first() {
        println!("Testing with SEQUENTIAL encoding...");
        println!(
            "Original mesh: {} faces, {} points",
            first_mesh.num_faces(),
            first_mesh.num_points()
        );

        use draco_core::encoder_buffer::EncoderBuffer;
        use draco_core::encoder_options::EncoderOptions;
        use draco_core::mesh_encoder::MeshEncoder;

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(first_mesh.clone());

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_method", 0); // Sequential

        let mut enc_buffer = EncoderBuffer::new();
        encoder
            .encode(&options, &mut enc_buffer)
            .expect("Sequential encoding failed");
        println!("Encoding succeeded! {} bytes", enc_buffer.data().len());

        // Now decode
        use draco_core::decoder_buffer::DecoderBuffer;
        use draco_core::mesh_decoder::MeshDecoder;

        let mut decoder_buffer = DecoderBuffer::new(enc_buffer.data());
        let mut decoded_mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();

        decoder
            .decode(&mut decoder_buffer, &mut decoded_mesh)
            .expect("Sequential decoding failed");
        println!(
            "Decoding succeeded! {} faces, {} points",
            decoded_mesh.num_faces(),
            decoded_mesh.num_points()
        );

        // Verify mesh properties
        assert_eq!(
            first_mesh.num_faces(),
            decoded_mesh.num_faces(),
            "Face count mismatch"
        );
        assert_eq!(
            first_mesh.num_points(),
            decoded_mesh.num_points(),
            "Point count mismatch"
        );
        println!("Sequential roundtrip PASSED!");
    }
}
