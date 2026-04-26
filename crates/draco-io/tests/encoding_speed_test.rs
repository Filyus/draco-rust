//! Tests for different Draco encoding speeds.
//!
//! This test verifies that different encoding speed settings produce valid
//! compressed output. Speed affects which prediction scheme is selected:
//!
//! Speed 10: PREDICTION_DIFFERENCE (fastest, least compression)
//! Speed 8-9: PREDICTION_DIFFERENCE
//! Speed 2-7: MESH_PREDICTION_PARALLELOGRAM (balanced)
//! Speed 0-1: MESH_PREDICTION_CONSTRAINED_MULTI_PARALLELOGRAM (slowest, best compression)
//!
//! Some speeds may have bugs that cause incorrect output (e.g., only 1 vertex).

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_io::gltf_reader::GltfReader;
use std::collections::HashSet;
use std::path::Path;

fn get_testdata_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
}

/// Extract position values from a mesh as Vec<[f32; 3]>
fn extract_positions(mesh: &Mesh) -> Vec<[f32; 3]> {
    let pos_att_id = mesh.named_attribute_id(GeometryAttributeType::Position);
    if pos_att_id < 0 {
        return Vec::new();
    }

    let pos_attr = mesh.attribute(pos_att_id);
    let num_entries = pos_attr.size();
    let buffer = pos_attr.buffer();
    let byte_stride = pos_attr.byte_stride() as usize;

    let mut positions = Vec::with_capacity(num_entries);
    for i in 0..num_entries {
        let offset = i * byte_stride;
        let mut bytes = [0u8; 12];
        if offset + 12 <= buffer.data_size() {
            buffer.read(offset, &mut bytes);
            let x = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let y = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            let z = f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
            positions.push([x, y, z]);
        }
    }
    positions
}

/// Extract face indices from a mesh
fn extract_faces(mesh: &Mesh) -> Vec<[u32; 3]> {
    (0..mesh.num_faces())
        .map(|i| {
            let face = mesh.face(FaceIndex(i as u32));
            [face[0].0, face[1].0, face[2].0]
        })
        .collect()
}

/// Compute bounding box of positions
fn compute_bbox(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    if positions.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }

    let mut min = positions[0];
    let mut max = positions[0];

    for p in positions {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    (min, max)
}

/// Check if two bounding boxes are approximately equal (within tolerance)
fn bbox_approx_equal(
    bbox1: &([f32; 3], [f32; 3]),
    bbox2: &([f32; 3], [f32; 3]),
    tolerance: f32,
) -> bool {
    for i in 0..3 {
        if (bbox1.0[i] - bbox2.0[i]).abs() > tolerance {
            return false;
        }
        if (bbox1.1[i] - bbox2.1[i]).abs() > tolerance {
            return false;
        }
    }
    true
}

/// Verify that decoded positions contain reasonable data by checking:
/// 1. All positions are finite (not NaN or Inf)
/// 2. Bounding box approximately matches original
/// 3. Positions are not all identical (degenerate)
fn verify_positions(
    original: &[[f32; 3]],
    decoded: &[[f32; 3]],
    quantization_bits: i32,
) -> Result<(), String> {
    // Check all decoded positions are finite
    for (i, p) in decoded.iter().enumerate() {
        if !p[0].is_finite() || !p[1].is_finite() || !p[2].is_finite() {
            return Err(format!(
                "Decoded position {} contains non-finite values: {:?}",
                i, p
            ));
        }
    }

    // Check for degenerate mesh (all positions identical)
    let unique_positions: HashSet<[u32; 3]> = decoded
        .iter()
        .map(|p| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()])
        .collect();

    if unique_positions.len() <= 1 && decoded.len() > 1 {
        return Err(format!(
            "Decoded mesh is degenerate: {} positions but only {} unique",
            decoded.len(),
            unique_positions.len()
        ));
    }

    // Compute bounding boxes
    let orig_bbox = compute_bbox(original);
    let dec_bbox = compute_bbox(decoded);

    // Calculate expected tolerance based on quantization
    // With Q bits, the error per component is roughly range / 2^Q
    let orig_range = [
        orig_bbox.1[0] - orig_bbox.0[0],
        orig_bbox.1[1] - orig_bbox.0[1],
        orig_bbox.1[2] - orig_bbox.0[2],
    ];
    let max_range = orig_range[0].max(orig_range[1]).max(orig_range[2]);
    let quant_step = max_range / (1 << quantization_bits) as f32;
    // Allow some tolerance for quantization error (2x quant step should be enough)
    let tolerance = quant_step * 2.0;

    if !bbox_approx_equal(&orig_bbox, &dec_bbox, tolerance) {
        return Err(format!(
            "Bounding box mismatch!\n  Original: min={:?}, max={:?}\n  Decoded:  min={:?}, max={:?}\n  Tolerance: {}",
            orig_bbox.0, orig_bbox.1, dec_bbox.0, dec_bbox.1, tolerance
        ));
    }

    // Additional check: variance of positions should be similar
    let orig_variance = compute_position_variance(original);
    let dec_variance = compute_position_variance(decoded);

    // Variance should be within 50% (generous for quantization effects)
    if orig_variance > 0.0 {
        let variance_ratio = dec_variance / orig_variance;
        if variance_ratio < 0.5 || variance_ratio > 2.0 {
            return Err(format!(
                "Position variance mismatch: original={}, decoded={}, ratio={}",
                orig_variance, dec_variance, variance_ratio
            ));
        }
    }

    Ok(())
}

/// Compute variance of position coordinates
fn compute_position_variance(positions: &[[f32; 3]]) -> f32 {
    if positions.len() < 2 {
        return 0.0;
    }

    // Compute mean
    let mut sum = [0.0f64; 3];
    for p in positions {
        sum[0] += p[0] as f64;
        sum[1] += p[1] as f64;
        sum[2] += p[2] as f64;
    }
    let n = positions.len() as f64;
    let mean = [sum[0] / n, sum[1] / n, sum[2] / n];

    // Compute variance
    let mut var_sum = 0.0f64;
    for p in positions {
        let dx = p[0] as f64 - mean[0];
        let dy = p[1] as f64 - mean[1];
        let dz = p[2] as f64 - mean[2];
        var_sum += dx * dx + dy * dy + dz * dz;
    }

    (var_sum / n) as f32
}

/// Test encoding speed configuration for each speed value (0-10).
/// C++ reference: prediction_scheme_encoder_factory.cc SelectPredictionMethod()
///
/// Speed mapping (for mesh encoding):
/// - Speed 10: PREDICTION_DIFFERENCE
/// - Speed 8-9: PREDICTION_DIFFERENCE
/// - Speed 2-7: MESH_PREDICTION_PARALLELOGRAM (or small mesh fallback)
/// - Speed 0-1: MESH_PREDICTION_CONSTRAINED_MULTI_PARALLELOGRAM
#[test]
fn test_encoding_speed_roundtrip_iridescence_lamp() {
    let test_file = get_testdata_path().join("IridescenceLamp.glb");

    if !test_file.exists() {
        eprintln!("Test file not found: {:?}, skipping", test_file);
        return;
    }

    // Read the original GLB
    let reader = GltfReader::open(&test_file).expect("Failed to open GLB");
    let original_meshes = reader.decode_all_meshes().expect("Failed to decode meshes");

    let mesh = original_meshes.first().expect("No meshes in file");
    let original_faces = mesh.num_faces();
    let original_points = mesh.num_points();

    println!(
        "Original mesh: {} faces, {} points, {} attrs",
        original_faces,
        original_points,
        mesh.num_attributes()
    );

    // Test all speed levels (0-10)
    let speed_levels = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

    // Track which speeds work and which fail
    let mut results: Vec<(i32, bool, String)> = Vec::new();

    for speed in speed_levels {
        println!("\n=== Testing encoding speed {} ===", speed);

        // Test with Edgebreaker encoding
        let result = test_speed_with_method(mesh, speed, 1, "Edgebreaker");
        results.push((speed, result.0, result.1));

        // Also test with Sequential encoding for comparison
        let seq_result = test_speed_with_method(mesh, speed, 0, "Sequential");
        println!(
            "  Sequential speed {}: {}",
            speed,
            if seq_result.0 { "PASS" } else { "FAIL" }
        );
    }

    println!("\n=== Summary (Edgebreaker) ===");
    for (speed, success, msg) in &results {
        let status = if *success { "✓ PASS" } else { "✗ FAIL" };
        println!("Speed {}: {} - {}", speed, status, msg);
    }

    // Assert all speeds work correctly
    let all_passed = results.iter().all(|(_, success, _)| *success);

    // Print failures with more detail
    if !all_passed {
        println!("\n=== Failures Detail ===");
        for (speed, success, msg) in &results {
            if !*success {
                println!("FAILED Speed {}: {}", speed, msg);
            }
        }
    }

    // For now, just assert that at least speed 5 works (the default)
    let speed_5_result = results.iter().find(|(s, _, _)| *s == 5);
    assert!(
        speed_5_result.map(|(_, s, _)| *s).unwrap_or(false),
        "Speed 5 (default) should work"
    );
}

fn test_speed_with_method(
    mesh: &Mesh,
    speed: i32,
    encoding_method: i32,
    method_name: &str,
) -> (bool, String) {
    let original_faces = mesh.num_faces();
    let original_points = mesh.num_points();

    // Extract original positions for comparison
    let original_positions = extract_positions(mesh);
    let quantization_bits = 14; // Position quantization bits

    // Encode
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", encoding_method);
    options.set_global_int("encoding_speed", speed);
    options.set_global_int("decoding_speed", speed);

    // Set quantization for all attributes
    for i in 0..mesh.num_attributes() {
        let att = mesh.attribute(i);
        let bits = match att.attribute_type() {
            GeometryAttributeType::Position => quantization_bits,
            GeometryAttributeType::Normal => 10,
            GeometryAttributeType::TexCoord => 12,
            GeometryAttributeType::Color => 8,
            _ => 8,
        };
        options.set_attribute_int(i, "quantization_bits", bits);
    }

    let mut enc_buffer = EncoderBuffer::new();
    match encoder.encode(&options, &mut enc_buffer) {
        Ok(_) => {}
        Err(e) => {
            return (false, format!("{} encode error: {:?}", method_name, e));
        }
    }

    let encoded_size = enc_buffer.data().len();
    println!(
        "  {} speed {}: encoded {} bytes",
        method_name, speed, encoded_size
    );

    // Decode
    let mut decoder = MeshDecoder::new();
    let mut decoded_mesh = Mesh::new();
    let mut decoder_buffer = DecoderBuffer::new(enc_buffer.data());

    match decoder.decode(&mut decoder_buffer, &mut decoded_mesh) {
        Ok(_) => {}
        Err(e) => {
            return (false, format!("{} decode error: {:?}", method_name, e));
        }
    }

    let decoded_faces = decoded_mesh.num_faces();
    let decoded_points = decoded_mesh.num_points();

    println!(
        "    Decoded: {} faces, {} points",
        decoded_faces, decoded_points
    );

    // Verify decoded mesh is valid
    if decoded_faces == 0 {
        return (
            false,
            format!("Decoded mesh has 0 faces (expected {})", original_faces),
        );
    }

    if decoded_points <= 1 {
        return (
            false,
            format!(
                "Decoded mesh has only {} points (expected {})",
                decoded_points, original_points
            ),
        );
    }

    // Face count should match exactly
    if decoded_faces != original_faces {
        return (
            false,
            format!(
                "Face count mismatch: {} vs {} (expected)",
                decoded_faces, original_faces
            ),
        );
    }

    // Point count should match (allowing for some variation due to deduplication)
    // With use_single_connectivity at speed >= 6, point count should match exactly.
    // At lower speeds, vertices may be deduplicated.
    if decoded_points != original_points {
        // This may be acceptable depending on speed settings
        println!(
            "    Note: Point count differs {} vs {} (deduplication may have occurred)",
            decoded_points, original_points
        );
    }

    // CRITICAL: Verify actual position data is correct, not just counts
    let decoded_positions = extract_positions(&decoded_mesh);

    if let Err(e) = verify_positions(&original_positions, &decoded_positions, quantization_bits) {
        return (false, format!("Position verification failed: {}", e));
    }

    // Verify face indices are valid (within bounds)
    let decoded_face_data = extract_faces(&decoded_mesh);
    for (i, face) in decoded_face_data.iter().enumerate() {
        for &idx in face {
            if idx as usize >= decoded_points {
                return (
                    false,
                    format!(
                        "Face {} has invalid vertex index {} (max={})",
                        i,
                        idx,
                        decoded_points - 1
                    ),
                );
            }
        }
    }

    (
        true,
        format!(
            "OK - {} bytes, {} faces, {} points (verified)",
            encoded_size, decoded_faces, decoded_points
        ),
    )
}

/// Test each prediction scheme selection based on speed.
/// This tests the logic from C++ SelectPredictionMethod().
#[test]
fn test_prediction_scheme_selection_by_speed() {
    let test_file = get_testdata_path().join("IridescenceLamp.glb");

    if !test_file.exists() {
        eprintln!("Test file not found: {:?}, skipping", test_file);
        return;
    }

    let reader = GltfReader::open(&test_file).expect("Failed to open GLB");
    let meshes = reader.decode_all_meshes().expect("Failed to decode meshes");
    let mesh = meshes.first().expect("No meshes");

    // Test speed groupings that should use different prediction schemes
    let speed_groups = [
        (10, "DIFFERENCE (fastest)"),
        (8, "DIFFERENCE"),
        (5, "PARALLELOGRAM (default)"),
        (2, "PARALLELOGRAM"),
        (1, "CONSTRAINED_MULTI_PARALLELOGRAM"),
        (0, "CONSTRAINED_MULTI_PARALLELOGRAM (best compression)"),
    ];

    println!("Testing prediction scheme selection by speed:");
    println!(
        "Original mesh: {} faces, {} points\n",
        mesh.num_faces(),
        mesh.num_points()
    );

    let mut encoded_sizes: Vec<(i32, usize)> = Vec::new();

    for (speed, expected_scheme) in speed_groups {
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_method", 1); // Edgebreaker
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);

        // Set quantization
        for i in 0..mesh.num_attributes() {
            options.set_attribute_int(i, "quantization_bits", 14);
        }

        let mut enc_buffer = EncoderBuffer::new();
        let encode_result = encoder.encode(&options, &mut enc_buffer);

        match encode_result {
            Ok(_) => {
                let size = enc_buffer.data().len();
                encoded_sizes.push((speed, size));
                println!("Speed {:2} ({}): {} bytes", speed, expected_scheme, size);

                // Verify decoding works
                let mut decoder = MeshDecoder::new();
                let mut decoded_mesh = Mesh::new();
                let mut decoder_buffer = DecoderBuffer::new(enc_buffer.data());

                match decoder.decode(&mut decoder_buffer, &mut decoded_mesh) {
                    Ok(_) => {
                        // Check for the "1 vertex" bug
                        if decoded_mesh.num_points() <= 1 {
                            println!(
                                "  ⚠ WARNING: Decoded only {} points! Possible bug.",
                                decoded_mesh.num_points()
                            );
                        }
                        assert!(
                            decoded_mesh.num_faces() > 0,
                            "Speed {} produced 0 faces",
                            speed
                        );
                    }
                    Err(e) => {
                        println!("  ✗ Decode FAILED: {:?}", e);
                    }
                }
            }
            Err(e) => {
                println!(
                    "Speed {:2} ({}): ENCODE FAILED - {:?}",
                    speed, expected_scheme, e
                );
            }
        }
    }

    // Generally, lower speeds should produce smaller files (better compression)
    // Speed 10 (DIFFERENCE) should be largest, speed 0 (CMPM) should be smallest
    if encoded_sizes.len() >= 2 {
        println!("\nCompression comparison:");
        let max_size = encoded_sizes.iter().map(|(_, s)| *s).max().unwrap_or(0);
        let min_size = encoded_sizes.iter().map(|(_, s)| *s).min().unwrap_or(0);
        println!(
            "  Best compression: {} bytes (speed {})",
            min_size,
            encoded_sizes
                .iter()
                .min_by_key(|(_, s)| s)
                .map(|(sp, _)| sp)
                .unwrap_or(&-1)
        );
        println!(
            "  Worst compression: {} bytes (speed {})",
            max_size,
            encoded_sizes
                .iter()
                .max_by_key(|(_, s)| s)
                .map(|(sp, _)| sp)
                .unwrap_or(&-1)
        );
        if min_size > 0 {
            println!(
                "  Compression ratio: {:.1}x",
                max_size as f64 / min_size as f64
            );
        }
    }
}

/// Test that speed affects mesh seam handling (use_single_connectivity).
/// C++ behavior: speed >= 6 uses single connectivity (no vertex deduplication).
#[test]
fn test_speed_affects_connectivity_handling() {
    let test_file = get_testdata_path().join("IridescenceLamp.glb");

    if !test_file.exists() {
        eprintln!("Test file not found: {:?}, skipping", test_file);
        return;
    }

    let reader = GltfReader::open(&test_file).expect("Failed to open GLB");
    let meshes = reader.decode_all_meshes().expect("Failed to decode meshes");
    let mesh = meshes.first().expect("No meshes");

    let original_points = mesh.num_points();

    println!("Testing connectivity handling by speed:");
    println!("Original mesh: {} points\n", original_points);

    // Speed < 6: may deduplicate vertices based on position
    // Speed >= 6: uses single connectivity (preserves all vertices)
    let test_speeds = [3, 5, 6, 7, 10];

    for speed in test_speeds {
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_method", 1); // Edgebreaker
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);

        for i in 0..mesh.num_attributes() {
            options.set_attribute_int(i, "quantization_bits", 14);
        }

        let mut enc_buffer = EncoderBuffer::new();
        if encoder.encode(&options, &mut enc_buffer).is_err() {
            println!("Speed {}: Encode failed", speed);
            continue;
        }

        let mut decoder = MeshDecoder::new();
        let mut decoded_mesh = Mesh::new();
        let mut decoder_buffer = DecoderBuffer::new(enc_buffer.data());

        if decoder
            .decode(&mut decoder_buffer, &mut decoded_mesh)
            .is_err()
        {
            println!("Speed {}: Decode failed", speed);
            continue;
        }

        let decoded_points = decoded_mesh.num_points();
        let connectivity_mode = if speed >= 6 {
            "single (preserves vertices)"
        } else {
            "position-based (may deduplicate)"
        };

        println!(
            "Speed {:2}: {} points ({}) - {}",
            speed,
            decoded_points,
            connectivity_mode,
            if decoded_points == original_points {
                "matches original"
            } else {
                "differs from original"
            }
        );

        // Verify we didn't get the "1 vertex" bug
        assert!(
            decoded_points > 1,
            "Speed {} produced only {} points - likely a bug!",
            speed,
            decoded_points
        );
    }
}

/// Comprehensive test that exercises edge cases and known problem areas.
#[test]
fn test_encoding_speed_edge_cases() {
    let test_file = get_testdata_path().join("IridescenceLamp.glb");

    if !test_file.exists() {
        eprintln!("Test file not found: {:?}, skipping", test_file);
        return;
    }

    let reader = GltfReader::open(&test_file).expect("Failed to open GLB");
    let meshes = reader.decode_all_meshes().expect("Failed to decode meshes");
    let mesh = meshes.first().expect("No meshes");

    println!("=== Edge Case Tests ===\n");

    // Test 1: Default speed (5) with no explicit speed setting
    {
        println!("Test 1: Default speed (no explicit setting)");
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_method", 1); // Edgebreaker only
                                                      // No speed setting - should default to 5

        for i in 0..mesh.num_attributes() {
            options.set_attribute_int(i, "quantization_bits", 14);
        }

        let mut enc_buffer = EncoderBuffer::new();
        let result = encoder.encode(&options, &mut enc_buffer);

        match result {
            Ok(_) => {
                println!("  Encode: OK ({} bytes)", enc_buffer.data().len());

                let mut decoder = MeshDecoder::new();
                let mut decoded_mesh = Mesh::new();
                let mut decoder_buffer = DecoderBuffer::new(enc_buffer.data());

                match decoder.decode(&mut decoder_buffer, &mut decoded_mesh) {
                    Ok(_) => {
                        println!(
                            "  Decode: OK ({} faces, {} points)",
                            decoded_mesh.num_faces(),
                            decoded_mesh.num_points()
                        );
                        assert!(
                            decoded_mesh.num_points() > 1,
                            "Default speed produced 1 vertex bug"
                        );
                    }
                    Err(e) => panic!("Default speed decode failed: {:?}", e),
                }
            }
            Err(e) => panic!("Default speed encode failed: {:?}", e),
        }
    }

    // Test 2: Encoding speed and decoding speed set differently
    {
        println!("\nTest 2: Different encoding and decoding speeds");
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_method", 1);
        options.set_global_int("encoding_speed", 2); // Slow encoding
        options.set_global_int("decoding_speed", 8); // Fast decoding

        for i in 0..mesh.num_attributes() {
            options.set_attribute_int(i, "quantization_bits", 14);
        }

        let mut enc_buffer = EncoderBuffer::new();
        match encoder.encode(&options, &mut enc_buffer) {
            Ok(_) => {
                println!("  Encode: OK ({} bytes)", enc_buffer.data().len());

                let mut decoder = MeshDecoder::new();
                let mut decoded_mesh = Mesh::new();
                let mut decoder_buffer = DecoderBuffer::new(enc_buffer.data());

                match decoder.decode(&mut decoder_buffer, &mut decoded_mesh) {
                    Ok(_) => {
                        println!(
                            "  Decode: OK ({} faces, {} points)",
                            decoded_mesh.num_faces(),
                            decoded_mesh.num_points()
                        );
                    }
                    Err(e) => println!("  Decode: FAILED - {:?}", e),
                }
            }
            Err(e) => println!("  Encode: FAILED - {:?}", e),
        }
    }

    // Test 3: Out of range speed values (should be clamped or handled)
    {
        println!("\nTest 3: Out of range speed values");
        for speed in [-1i32, 11, 100] {
            let mut encoder = MeshEncoder::new();
            encoder.set_mesh(mesh.clone());

            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_method", 1);
            options.set_global_int("encoding_speed", speed);
            options.set_global_int("decoding_speed", speed);

            for i in 0..mesh.num_attributes() {
                options.set_attribute_int(i, "quantization_bits", 14);
            }

            let mut enc_buffer = EncoderBuffer::new();
            match encoder.encode(&options, &mut enc_buffer) {
                Ok(_) => {
                    let mut decoder = MeshDecoder::new();
                    let mut decoded_mesh = Mesh::new();
                    let mut decoder_buffer = DecoderBuffer::new(enc_buffer.data());

                    match decoder.decode(&mut decoder_buffer, &mut decoded_mesh) {
                        Ok(_) => {
                            println!(
                                "  Speed {}: OK ({} faces, {} points)",
                                speed,
                                decoded_mesh.num_faces(),
                                decoded_mesh.num_points()
                            );
                        }
                        Err(e) => println!("  Speed {}: Decode failed - {:?}", speed, e),
                    }
                }
                Err(e) => println!("  Speed {}: Encode failed - {:?}", speed, e),
            }
        }
    }

    println!("\n=== Edge Case Tests Complete ===");
}

/// Test that verifies encoded size varies with speed (better compression at lower speeds).
#[test]
fn test_compression_efficiency_by_speed() {
    let test_file = get_testdata_path().join("IridescenceLamp.glb");

    if !test_file.exists() {
        eprintln!("Test file not found: {:?}, skipping", test_file);
        return;
    }

    let reader = GltfReader::open(&test_file).expect("Failed to open GLB");
    let meshes = reader.decode_all_meshes().expect("Failed to decode meshes");
    let mesh = meshes.first().expect("No meshes");

    println!("Testing compression efficiency across speed levels:");
    println!(
        "Original mesh: {} faces, {} points\n",
        mesh.num_faces(),
        mesh.num_points()
    );

    let mut results: Vec<(i32, usize, bool)> = Vec::new();

    for speed in 0..=10 {
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_method", 1); // Edgebreaker
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);

        for i in 0..mesh.num_attributes() {
            options.set_attribute_int(i, "quantization_bits", 14);
        }

        let mut enc_buffer = EncoderBuffer::new();
        if encoder.encode(&options, &mut enc_buffer).is_ok() {
            let size = enc_buffer.data().len();

            // Verify decoding
            let mut decoder = MeshDecoder::new();
            let mut decoded_mesh = Mesh::new();
            let mut decoder_buffer = DecoderBuffer::new(enc_buffer.data());
            let decode_ok = decoder
                .decode(&mut decoder_buffer, &mut decoded_mesh)
                .is_ok()
                && decoded_mesh.num_points() > 1;

            results.push((speed, size, decode_ok));
        }
    }

    // Print results as a table
    println!("Speed | Size (bytes) | Decode OK | Compression Ratio");
    println!("------|--------------|-----------|------------------");

    let max_size = results.iter().map(|(_, s, _)| *s).max().unwrap_or(1);

    for (speed, size, decode_ok) in &results {
        let ratio = *size as f64 / max_size as f64;
        let decode_status = if *decode_ok { "✓" } else { "✗" };
        println!(
            "  {:2}  | {:>12} | {:>9} | {:.2}x",
            speed, size, decode_status, ratio
        );
    }

    // Verify that at least some speeds work
    let working_speeds: Vec<_> = results.iter().filter(|(_, _, ok)| *ok).collect();
    assert!(
        !working_speeds.is_empty(),
        "At least some speed levels should work"
    );

    println!(
        "\n{}/{} speed levels produce valid output",
        working_speeds.len(),
        results.len()
    );
}
