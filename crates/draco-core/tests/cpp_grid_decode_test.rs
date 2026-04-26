//! Test to decode C++ encoded grid mesh and compare

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use std::fs;

#[test]
fn test_rust_decode_cpp_encoded_grid() {
    // Read the C++ encoded file
    let cpp_encoded_path = "../../testdata/grid5x5_cpp.drc";
    let cpp_encoded = match fs::read(cpp_encoded_path) {
        Ok(data) => data,
        Err(e) => {
            println!("Could not read {}: {}. Skipping test.", cpp_encoded_path, e);
            return;
        }
    };

    println!("Read {} bytes from C++ encoded file", cpp_encoded.len());

    // Decode with Rust
    let mut decoder = MeshDecoder::new();
    let mut decoded_mesh = Mesh::new();
    let mut decoder_buffer = DecoderBuffer::new(&cpp_encoded);

    decoder
        .decode(&mut decoder_buffer, &mut decoded_mesh)
        .expect("Rust decode of C++ encoded file failed");

    println!(
        "Decoded mesh: {} points, {} faces",
        decoded_mesh.num_points(),
        decoded_mesh.num_faces()
    );

    // Verify all expected grid positions exist
    let pos_attr = decoded_mesh.attribute(0);
    let pos_data = pos_attr.buffer().data();

    // Collect decoded positions
    let mut decoded_positions: Vec<[f32; 3]> = Vec::new();
    for i in 0..decoded_mesh.num_points() {
        let offset = i * 12;
        let x = f32::from_le_bytes(pos_data[offset..offset + 4].try_into().unwrap());
        let y = f32::from_le_bytes(pos_data[offset + 4..offset + 8].try_into().unwrap());
        let z = f32::from_le_bytes(pos_data[offset + 8..offset + 12].try_into().unwrap());
        decoded_positions.push([x, y, z]);
    }

    println!("\nDecoded positions:");
    for (i, pos) in decoded_positions.iter().enumerate() {
        println!(
            "  Point {}: ({:.3}, {:.3}, {:.3})",
            i, pos[0], pos[1], pos[2]
        );
    }

    // Verify all 5x5 grid positions exist
    let mut missing = Vec::new();
    for y in 0..5 {
        for x in 0..5 {
            let target = [x as f32, y as f32, 0.0f32];
            let found = decoded_positions.iter().any(|p| {
                (p[0] - target[0]).abs() < 0.1
                    && (p[1] - target[1]).abs() < 0.1
                    && (p[2] - target[2]).abs() < 0.1
            });
            if !found {
                missing.push((x, y));
            }
        }
    }

    if !missing.is_empty() {
        println!("\nMissing grid positions:");
        for (x, y) in &missing {
            println!("  ({}, {})", x, y);
        }
        panic!("{} grid positions are missing!", missing.len());
    }

    println!("\nAll 25 grid positions found!");
}
