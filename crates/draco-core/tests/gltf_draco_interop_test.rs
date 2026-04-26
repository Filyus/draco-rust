//! Test Draco encode/decode interoperability with C++ decoder for glTF-style meshes.
//!
//! This test replicates the exact flow used by the web app gltf-writer:
//! 1. Create mesh with Position, Normal, TexCoord attributes
//! 2. Encode with Sequential encoding and quantization
//! 3. Decode with C++ draco_decoder
//! 4. Verify positions match

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::PointIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;

fn create_temp_dir(test_name: &str) -> PathBuf {
    let tmp = std::env::temp_dir().join(format!("draco_test_{}", test_name));
    if tmp.exists() {
        fs::remove_dir_all(&tmp).ok();
    }
    fs::create_dir_all(&tmp).expect("failed to create temp dir");
    tmp
}

fn cpp_decoder() -> Option<PathBuf> {
    // Check environment variable first
    if let Ok(build_dir) = std::env::var("DRACO_CPP_BUILD_DIR") {
        let dec = PathBuf::from(&build_dir).join("draco_decoder.exe");
        if dec.exists() {
            return Some(dec);
        }
    }

    // Check common locations
    let build_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .join("build")
        .join("Debug");
    let dec = build_dir.join("draco_decoder.exe");
    if dec.exists() {
        return Some(dec);
    }
    None
}

fn encode_mesh_simple(
    positions: &[f32],
    normals: &[f32],
    indices: &[u32],
    position_quantization: i32,
    normal_quantization: i32,
) -> Result<Vec<u8>, String> {
    let vertex_count = positions.len() / 3;
    let face_count = indices.len() / 3;

    let mut draco_mesh = Mesh::new();

    // Add position attribute
    let mut pos_attr = PointAttribute::new();
    pos_attr.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        vertex_count,
    );
    let pos_buffer = pos_attr.buffer_mut();
    for (i, chunk) in positions.chunks(3).enumerate() {
        let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
        pos_buffer.write(i * 12, &bytes);
    }
    draco_mesh.add_attribute(pos_attr);

    // Add normal attribute
    let mut norm_attr = PointAttribute::new();
    norm_attr.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        vertex_count,
    );
    let norm_buffer = norm_attr.buffer_mut();
    for (i, chunk) in normals.chunks(3).enumerate() {
        let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
        norm_buffer.write(i * 12, &bytes);
    }
    draco_mesh.add_attribute(norm_attr);

    // Add faces
    for i in 0..face_count {
        let i0 = PointIndex(indices[i * 3]);
        let i1 = PointIndex(indices[i * 3 + 1]);
        let i2 = PointIndex(indices[i * 3 + 2]);
        draco_mesh.add_face([i0, i1, i2]);
    }

    // Encode
    let mut encoder = MeshEncoder::new();
    let mut encoder_buffer = EncoderBuffer::new();

    let mut enc_options = EncoderOptions::default();

    // Set quantization bits
    enc_options.set_attribute_int(0, "quantization_bits", position_quantization);
    enc_options.set_attribute_int(1, "quantization_bits", normal_quantization);

    encoder.set_mesh(draco_mesh);
    encoder
        .encode(&enc_options, &mut encoder_buffer)
        .map_err(|e| format!("{:?}", e))?;

    Ok(encoder_buffer.data().to_vec())
}

#[test]
fn dump_rust_vs_cpp_bytes() {
    // Same as test_normals.obj:
    // v 0 0 0
    // v 1 0 0
    // v 0.5 1 0
    // vn 1 0 0
    // vn -1 0 0
    // vn 0 1 0
    // f 1//1 2//2 3//3

    let positions: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0];

    let normals: Vec<f32> = vec![1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

    let indices: Vec<u32> = vec![0, 1, 2];

    let draco_bytes =
        encode_mesh_simple(&positions, &normals, &indices, 14, 10).expect("Encoding failed");

    println!("Rust encoded {} bytes", draco_bytes.len());
    println!("Hex dump:");
    for (i, byte) in draco_bytes.iter().enumerate() {
        print!("{:02X} ", byte);
        if (i + 1) % 16 == 0 {
            println!();
        }
    }
    println!();

    // Save to file for comparison
    let tmp = create_temp_dir("dump_bytes");
    let drc_path = tmp.join("rust_encoded.drc");
    fs::write(&drc_path, &draco_bytes).expect("Failed to write DRC");
    println!("Saved to: {}", drc_path.display());

    // Try to decode with C++ decoder
    if let Some(decoder) = cpp_decoder() {
        let obj_path = tmp.join("decoded.obj");
        let output = Command::new(&decoder)
            .args([
                "-i",
                drc_path.to_string_lossy().as_ref(),
                "-o",
                obj_path.to_string_lossy().as_ref(),
            ])
            .output()
            .expect("Failed to run draco_decoder");

        if output.status.success() {
            println!("\nC++ decode SUCCESS!");
            let obj_content = fs::read_to_string(&obj_path).expect("Failed to read OBJ");
            println!("Decoded OBJ:\n{}", obj_content);
        } else {
            println!("\nC++ decode FAILED!");
            println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
            println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        }
    } else {
        println!("C++ decoder not found, skipping decode test");
    }
}

fn encode_mesh_like_gltf_writer(
    positions: &[f32],
    normals: Option<&[f32]>,
    uvs: Option<&[f32]>,
    indices: &[u32],
    position_quantization: i32,
    normal_quantization: i32,
    texcoord_quantization: i32,
) -> Result<Vec<u8>, String> {
    let vertex_count = positions.len() / 3;
    let face_count = indices.len() / 3;

    let mut draco_mesh = Mesh::new();

    // Add position attribute
    let mut pos_attr = PointAttribute::new();
    pos_attr.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        vertex_count,
    );
    let pos_buffer = pos_attr.buffer_mut();
    for (i, chunk) in positions.chunks(3).enumerate() {
        let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
        pos_buffer.write(i * 12, &bytes);
    }
    draco_mesh.add_attribute(pos_attr);

    // Add normal attribute if present
    if let Some(normals) = normals {
        if !normals.is_empty() {
            let mut norm_attr = PointAttribute::new();
            norm_attr.init(
                GeometryAttributeType::Normal,
                3,
                DataType::Float32,
                false,
                vertex_count,
            );
            let norm_buffer = norm_attr.buffer_mut();
            for (i, chunk) in normals.chunks(3).enumerate() {
                let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
                norm_buffer.write(i * 12, &bytes);
            }
            draco_mesh.add_attribute(norm_attr);
        }
    }

    // Add UV attribute if present
    if let Some(uvs) = uvs {
        if !uvs.is_empty() {
            let mut uv_attr = PointAttribute::new();
            uv_attr.init(
                GeometryAttributeType::TexCoord,
                2,
                DataType::Float32,
                false,
                vertex_count,
            );
            let uv_buffer = uv_attr.buffer_mut();
            for (i, chunk) in uvs.chunks(2).enumerate() {
                let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
                uv_buffer.write(i * 8, &bytes);
            }
            draco_mesh.add_attribute(uv_attr);
        }
    }

    // Add faces
    for i in 0..face_count {
        let i0 = PointIndex(indices[i * 3]);
        let i1 = PointIndex(indices[i * 3 + 1]);
        let i2 = PointIndex(indices[i * 3 + 2]);
        draco_mesh.add_face([i0, i1, i2]);
    }

    let quant = draco_io::gltf_writer::QuantizationBits {
        position: position_quantization,
        normal: normal_quantization,
        texcoord: texcoord_quantization,
        ..Default::default()
    };

    draco_io::gltf_writer::encode_draco_mesh(&draco_mesh, quant).map_err(|e| e.to_string())
}

fn parse_obj_positions(obj_content: &str) -> Vec<[f32; 3]> {
    let mut positions = Vec::new();
    for line in obj_content.lines() {
        if line.starts_with("v ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let x: f32 = parts[1].parse().unwrap_or(0.0);
                let y: f32 = parts[2].parse().unwrap_or(0.0);
                let z: f32 = parts[3].parse().unwrap_or(0.0);
                positions.push([x, y, z]);
            }
        }
    }
    positions
}

#[test]
fn rust_encode_gltf_style_mesh_cpp_decode() {
    let Some(decoder) = cpp_decoder() else {
        eprintln!("Skipping: C++ draco_decoder not found. Set DRACO_CPP_BUILD_DIR.");
        return;
    };

    let tmp = create_temp_dir("gltf_interop");
    let drc_path = tmp.join("mesh.drc");
    let obj_path = tmp.join("mesh.obj");

    // Create a cube mesh like what comes from glTF
    let positions: Vec<f32> = vec![
        // Front face
        -1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0, 1.0, 1.0, -1.0, 1.0, 1.0, // Back face
        -1.0, -1.0, -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0,
    ];

    let normals: Vec<f32> = vec![
        0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, -1.0,
        0.0, 0.0, -1.0, 0.0, 0.0, -1.0,
    ];

    let uvs: Vec<f32> = vec![
        0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0,
    ];

    let indices: Vec<u32> = vec![
        0, 1, 2, 2, 3, 0, // Front
        4, 5, 6, 6, 7, 4, // Back
    ];

    // Encode with same settings as web app (14-bit position, 10-bit normal, 12-bit UV)
    let draco_bytes = encode_mesh_like_gltf_writer(
        &positions,
        Some(&normals),
        Some(&uvs),
        &indices,
        14, // position_quantization
        10, // normal_quantization
        12, // texcoord_quantization
    )
    .expect("Encoding failed");

    println!("Draco encoded {} bytes", draco_bytes.len());
    fs::write(&drc_path, &draco_bytes).expect("Failed to write DRC");

    // Decode with C++ decoder
    let output = Command::new(&decoder)
        .args([
            "-i",
            drc_path.to_string_lossy().as_ref(),
            "-o",
            obj_path.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("Failed to run draco_decoder");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "draco_decoder failed!\nstdout: {}\nstderr: {}",
            stdout, stderr
        );
    }

    // Parse OBJ and verify positions
    let obj_content = fs::read_to_string(&obj_path).expect("Failed to read OBJ");
    let decoded_positions = parse_obj_positions(&obj_content);

    println!(
        "Original positions: {:?}",
        positions.chunks(3).collect::<Vec<_>>()
    );
    println!("Decoded positions: {:?}", decoded_positions);

    assert_eq!(
        decoded_positions.len(),
        positions.len() / 3,
        "Position count mismatch: expected {}, got {}",
        positions.len() / 3,
        decoded_positions.len()
    );

    // Compare positions with tolerance for quantization error
    // 14-bit quantization over range 2.0 gives resolution of 2.0 / 16384 ≈ 0.000122
    let tolerance = 0.001; // Be generous for quantization
    for (i, orig_chunk) in positions.chunks(3).enumerate() {
        let orig = [orig_chunk[0], orig_chunk[1], orig_chunk[2]];
        let decoded = decoded_positions[i];

        let dx = (orig[0] - decoded[0]).abs();
        let dy = (orig[1] - decoded[1]).abs();
        let dz = (orig[2] - decoded[2]).abs();

        assert!(
            dx <= tolerance && dy <= tolerance && dz <= tolerance,
            "Position {} mismatch: original {:?}, decoded {:?}, delta ({}, {}, {})",
            i,
            orig,
            decoded,
            dx,
            dy,
            dz
        );
    }

    println!("All positions match within tolerance!");
}

#[test]
fn rust_encode_mesh_with_uvs_only_cpp_decode() {
    // Test the case where normals are absent but UVs are present
    // This is where the hardcoded attribute ID bug would manifest
    let Some(decoder) = cpp_decoder() else {
        eprintln!("Skipping: C++ draco_decoder not found. Set DRACO_CPP_BUILD_DIR.");
        return;
    };

    let tmp = create_temp_dir("gltf_uvs_only");
    let drc_path = tmp.join("mesh.drc");
    let obj_path = tmp.join("mesh.obj");

    let positions: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0];

    let uvs: Vec<f32> = vec![0.0, 0.0, 1.0, 0.0, 0.5, 1.0];

    let indices: Vec<u32> = vec![0, 1, 2];

    // Encode with UVs but NO normals
    let draco_bytes = encode_mesh_like_gltf_writer(
        &positions,
        None, // No normals!
        Some(&uvs),
        &indices,
        14,
        10,
        12,
    )
    .expect("Encoding failed");

    println!("Draco encoded {} bytes (no normals)", draco_bytes.len());
    fs::write(&drc_path, &draco_bytes).expect("Failed to write DRC");

    // Decode with C++ decoder
    let output = Command::new(&decoder)
        .args([
            "-i",
            drc_path.to_string_lossy().as_ref(),
            "-o",
            obj_path.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("Failed to run draco_decoder");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "draco_decoder failed!\nstdout: {}\nstderr: {}",
            stdout, stderr
        );
    }

    let obj_content = fs::read_to_string(&obj_path).expect("Failed to read OBJ");
    let decoded_positions = parse_obj_positions(&obj_content);

    println!("Decoded positions (UVs only): {:?}", decoded_positions);

    assert_eq!(decoded_positions.len(), 3);

    // Verify positions are correct
    let tolerance = 0.001;
    for (i, orig_chunk) in positions.chunks(3).enumerate() {
        let decoded = decoded_positions[i];
        let dx = (orig_chunk[0] - decoded[0]).abs();
        let dy = (orig_chunk[1] - decoded[1]).abs();
        let dz = (orig_chunk[2] - decoded[2]).abs();
        assert!(
            dx <= tolerance && dy <= tolerance && dz <= tolerance,
            "Position {} mismatch",
            i
        );
    }

    println!("UVs-only mesh positions correct!");
}

fn parse_obj_normals(obj_content: &str) -> Vec<[f32; 3]> {
    let mut normals = Vec::new();
    for line in obj_content.lines() {
        if line.starts_with("vn ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let x: f32 = parts[1].parse().unwrap_or(0.0);
                let y: f32 = parts[2].parse().unwrap_or(0.0);
                let z: f32 = parts[3].parse().unwrap_or(0.0);
                normals.push([x, y, z]);
            }
        }
    }
    normals
}

fn read_attr_vec3(attr: &PointAttribute, point_index: usize) -> [f32; 3] {
    let mapped_index = attr.mapped_index(PointIndex(point_index as u32)).0 as usize;
    let offset = mapped_index * attr.byte_stride() as usize;
    let data = attr.buffer().data();
    [
        f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()),
        f32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()),
        f32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap()),
    ]
}

fn nearest_position_index(position: [f32; 3], positions: &[f32]) -> usize {
    positions
        .chunks(3)
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = squared_distance(position, [a[0], a[1], a[2]]);
            let db = squared_distance(position, [b[0], b[1], b[2]]);
            da.partial_cmp(&db).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap()
}

fn squared_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

#[test]
fn rust_encode_gltf_style_mesh_verify_normals() {
    let Some(decoder) = cpp_decoder() else {
        eprintln!("Skipping: C++ draco_decoder not found. Set DRACO_CPP_BUILD_DIR.");
        return;
    };

    let tmp = create_temp_dir("gltf_normals");
    let drc_path = tmp.join("mesh.drc");
    let obj_path = tmp.join("mesh.obj");

    // Create a mesh with distinct normals in all directions
    // This tests both right hemisphere (x >= 0) and left hemisphere (x < 0)
    let positions: Vec<f32> = vec![
        // Vertex 0: normal +X
        0.0, 0.0, 0.0, // Vertex 1: normal -X (left hemisphere!)
        1.0, 0.0, 0.0, // Vertex 2: normal +Y
        0.5, 1.0, 0.0, // Vertex 3: normal -Y
        2.0, 0.0, 0.0, // Vertex 4: normal +Z
        2.5, 1.0, 0.0, // Vertex 5: normal -Z
        1.5, 1.0, 0.0, // Vertex 6: normal (+X+Y+Z)/sqrt(3)
        3.0, 0.0, 0.0, // Vertex 7: normal (-X+Y+Z)/sqrt(3) -- left hemisphere!
        3.5, 1.0, 0.0,
    ];

    let normals: Vec<f32> = vec![
        1.0, 0.0, 0.0, // +X
        -1.0, 0.0, 0.0, // -X (LEFT)
        0.0, 1.0, 0.0, // +Y
        0.0, -1.0, 0.0, // -Y
        0.0, 0.0, 1.0, // +Z
        0.0, 0.0, -1.0, // -Z
        0.57735, 0.57735, 0.57735, // +X+Y+Z
        -0.57735, 0.57735, 0.57735, // -X+Y+Z (LEFT)
    ];

    let uvs: Vec<f32> = vec![
        0.0, 0.0, 1.0, 0.0, 0.5, 1.0, 0.0, 0.0, 1.0, 0.0, 0.5, 1.0, 0.0, 0.0, 0.5, 1.0,
    ];

    // Two triangles and two more triangles
    let indices: Vec<u32> = vec![
        0, 1, 2, // First triangle
        3, 4, 5, // Second triangle
        0, 6, 7, // Third triangle (reuse vertex 0)
    ];

    let draco_bytes = encode_mesh_like_gltf_writer(
        &positions,
        Some(&normals),
        Some(&uvs),
        &indices,
        14, // position_quantization
        10, // normal_quantization
        12, // texcoord_quantization
    )
    .expect("Encoding failed");

    println!("Draco encoded {} bytes for normal test", draco_bytes.len());
    fs::write(&drc_path, &draco_bytes).expect("Failed to write DRC");

    // Decode with C++ decoder
    let output = Command::new(&decoder)
        .args([
            "-i",
            drc_path.to_string_lossy().as_ref(),
            "-o",
            obj_path.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("Failed to run draco_decoder");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "draco_decoder failed!\nstdout: {}\nstderr: {}",
            stdout, stderr
        );
    }

    let obj_content = fs::read_to_string(&obj_path).expect("Failed to read OBJ");
    println!("OBJ content:\n{}", obj_content);

    let decoded_normals = parse_obj_normals(&obj_content);

    println!("\n=== NORMAL COMPARISON ===");
    println!("Original normals count: {}", normals.len() / 3);
    println!("Decoded normals count: {}", decoded_normals.len());

    // Compare normals using dot product (should be > 0.98 for similar direction)
    let tolerance = 0.02; // For 10-bit quantization
    for (i, orig_chunk) in normals.chunks(3).enumerate() {
        if i >= decoded_normals.len() {
            println!("Normal {}: MISSING in decoded", i);
            continue;
        }
        let orig = [orig_chunk[0], orig_chunk[1], orig_chunk[2]];
        let decoded = decoded_normals[i];

        // Normalize both
        let orig_len = (orig[0] * orig[0] + orig[1] * orig[1] + orig[2] * orig[2]).sqrt();
        let decoded_len =
            (decoded[0] * decoded[0] + decoded[1] * decoded[1] + decoded[2] * decoded[2]).sqrt();

        let orig_normalized = [orig[0] / orig_len, orig[1] / orig_len, orig[2] / orig_len];
        let decoded_normalized = if decoded_len > 0.0001 {
            [
                decoded[0] / decoded_len,
                decoded[1] / decoded_len,
                decoded[2] / decoded_len,
            ]
        } else {
            decoded
        };

        let dot = orig_normalized[0] * decoded_normalized[0]
            + orig_normalized[1] * decoded_normalized[1]
            + orig_normalized[2] * decoded_normalized[2];

        let is_left = orig[0] < 0.0;
        println!(
            "Normal {}: {} orig {:?} -> decoded {:?}, dot={:.4}",
            i,
            if is_left { "LEFT " } else { "RIGHT" },
            orig_normalized,
            decoded_normalized,
            dot
        );

        assert!(
            dot > 0.98 - tolerance,
            "Normal {} mismatch: original {:?}, decoded {:?}, dot={}",
            i,
            orig_normalized,
            decoded_normalized,
            dot
        );
    }

    println!("\nAll normals match within tolerance!");
}

#[test]
fn rust_encode_decode_normals_roundtrip() {
    // Test pure Rust encode/decode without C++ decoder
    use draco_core::decoder_buffer::DecoderBuffer;
    use draco_core::mesh::Mesh;
    use draco_core::mesh_decoder::MeshDecoder;

    let positions: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0];

    let normals: Vec<f32> = vec![
        1.0, 0.0, 0.0, // +X
        -1.0, 0.0, 0.0, // -X (left hemisphere)
        0.0, 1.0, 0.0, // +Y
    ];

    let uvs: Vec<f32> = vec![0.0, 0.0, 1.0, 0.0, 0.5, 1.0];

    let indices: Vec<u32> = vec![0, 1, 2];

    let draco_bytes =
        encode_mesh_like_gltf_writer(&positions, Some(&normals), Some(&uvs), &indices, 14, 10, 12)
            .expect("Encoding failed");

    println!("Draco encoded {} bytes", draco_bytes.len());

    // Decode with Rust
    let mut decoder_buffer = DecoderBuffer::new(&draco_bytes);
    let mut decoder = MeshDecoder::new();
    let mut decoded_mesh = Mesh::new();
    let result = decoder.decode(&mut decoder_buffer, &mut decoded_mesh);

    if let Err(e) = result {
        panic!("Rust decode failed: {:?}", e);
    }

    println!(
        "\nDecoded mesh: {} points, {} attributes",
        decoded_mesh.num_points(),
        decoded_mesh.num_attributes()
    );

    let pos_attr = decoded_mesh
        .named_attribute(GeometryAttributeType::Position)
        .expect("No position attribute found in decoded mesh");

    // Find normal attribute
    let norm_att = decoded_mesh.named_attribute(GeometryAttributeType::Normal);
    if let Some(norm_attr) = norm_att {
        println!(
            "Normal attribute: {} values, {} components",
            norm_attr.size(),
            norm_attr.num_components()
        );

        for point_index in 0..decoded_mesh.num_points() {
            let decoded_position = read_attr_vec3(pos_attr, point_index);
            let original_index = nearest_position_index(decoded_position, &positions);
            let orig = [
                normals[original_index * 3],
                normals[original_index * 3 + 1],
                normals[original_index * 3 + 2],
            ];
            let decoded = read_attr_vec3(norm_attr, point_index);

            let dot = orig[0] * decoded[0] + orig[1] * decoded[1] + orig[2] * decoded[2];

            println!(
                "Point {}: position {:?}, original vertex {}, normal {:?} -> decoded {:?}, dot={:.4}",
                point_index, decoded_position, original_index, orig, decoded, dot
            );

            assert!(
                dot > 0.98,
                "Point {} normal mismatch: orig {:?}, decoded {:?}, dot={}",
                point_index,
                orig,
                decoded,
                dot
            );
        }

        println!("\nRust encode/decode roundtrip: All normals match!");
    } else {
        panic!("No normal attribute found in decoded mesh");
    }
}

#[test]
fn compare_cpp_vs_rust_encoded_bytes() {
    use std::fs;
    use std::process::Command;

    let tmp = create_temp_dir("compare_encoders");

    // Create an OBJ file with normals
    let obj_content = r#"# Simple triangle with normals
v 0 0 0
v 1 0 0
v 0.5 1 0
vn 0 0 1
vn 0 0 1
vn 0 0 1
f 1//1 2//2 3//3
"#;
    let obj_path = tmp.join("input.obj");
    fs::write(&obj_path, obj_content).expect("Failed to write OBJ");

    // Check for C++ encoder
    let encoder_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("build/Debug/draco_encoder.exe");

    if !encoder_path.exists() {
        println!("C++ encoder not found, skipping test");
        return;
    }

    // Encode with C++ - using sequential encoding (-cl 0) and quantization
    let cpp_drc_path = tmp.join("cpp_encoded.drc");
    let output = Command::new(&encoder_path)
        .args([
            "-i",
            obj_path.to_string_lossy().as_ref(),
            "-o",
            cpp_drc_path.to_string_lossy().as_ref(),
            "-cl",
            "0", // Sequential encoding
            "-qp",
            "14", // Position quantization
            "-qn",
            "10", // Normal quantization
        ])
        .output()
        .expect("Failed to run draco_encoder");

    if !output.status.success() {
        println!(
            "C++ encode failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let cpp_bytes = fs::read(&cpp_drc_path).expect("Failed to read C++ DRC");
    println!("C++ encoded {} bytes", cpp_bytes.len());

    // Now encode the same mesh with Rust
    let positions: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0];

    let normals: Vec<f32> = vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];

    let indices: Vec<u32> = vec![0, 1, 2];

    let rust_bytes =
        encode_mesh_simple(&positions, &normals, &indices, 14, 10).expect("Rust encoding failed");

    println!("Rust encoded {} bytes", rust_bytes.len());

    // Compare bytes
    println!("\n=== Byte comparison ===");
    println!("C++ bytes:");
    for (i, byte) in cpp_bytes.iter().enumerate() {
        print!("{:02X} ", byte);
        if (i + 1) % 16 == 0 {
            println!();
        }
    }
    println!();

    println!("Rust bytes:");
    for (i, byte) in rust_bytes.iter().enumerate() {
        print!("{:02X} ", byte);
        if (i + 1) % 16 == 0 {
            println!();
        }
    }
    println!();

    // Save Rust file for decoding comparison
    let rust_drc_path = tmp.join("rust_encoded.drc");
    fs::write(&rust_drc_path, &rust_bytes).expect("Failed to write Rust DRC");

    // Decode both with C++ decoder
    let decoder_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("build/Debug/draco_decoder.exe");

    if decoder_path.exists() {
        let cpp_obj_path = tmp.join("cpp_decoded.obj");
        let rust_obj_path = tmp.join("rust_decoded.obj");

        let _ = Command::new(&decoder_path)
            .args([
                "-i",
                cpp_drc_path.to_string_lossy().as_ref(),
                "-o",
                cpp_obj_path.to_string_lossy().as_ref(),
            ])
            .output();

        let _ = Command::new(&decoder_path)
            .args([
                "-i",
                rust_drc_path.to_string_lossy().as_ref(),
                "-o",
                rust_obj_path.to_string_lossy().as_ref(),
            ])
            .output();

        println!("\n=== C++ decoder results ===");
        if cpp_obj_path.exists() {
            println!("From C++ encoded:");
            println!("{}", fs::read_to_string(&cpp_obj_path).unwrap_or_default());
        }
        if rust_obj_path.exists() {
            println!("From Rust encoded:");
            println!("{}", fs::read_to_string(&rust_obj_path).unwrap_or_default());
        }
    }

    // Find first difference
    println!("\n=== First byte differences ===");
    let min_len = cpp_bytes.len().min(rust_bytes.len());
    let mut diff_count = 0;
    for i in 0..min_len {
        if cpp_bytes[i] != rust_bytes[i] && diff_count < 10 {
            println!(
                "Offset {}: C++={:02X}, Rust={:02X}",
                i, cpp_bytes[i], rust_bytes[i]
            );
            diff_count += 1;
        }
    }
    if cpp_bytes.len() != rust_bytes.len() {
        println!(
            "Length differs: C++={}, Rust={}",
            cpp_bytes.len(),
            rust_bytes.len()
        );
    }
}
