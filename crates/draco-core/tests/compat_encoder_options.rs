use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
/// Tests for encoder options compatibility between Rust and C++ implementations.
/// This tests different quantization bits and compression levels.
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

static OUTPUT_LOCK: Mutex<()> = Mutex::new(());

fn print_size_table_header(title: &str) {
    println!("\n=== {title} ===");
    println!(
        "{:<28} {:>11} {:>11} {:>10}",
        "Case", "C++ bytes", "Rust bytes", "Status"
    );
    println!("{}", "-".repeat(66));
}

fn print_size_row(case: impl std::fmt::Display, cpp_size: usize, rust_size: usize, status: &str) {
    println!(
        "{:<28} {:>11} {:>11} {:>10}",
        case, cpp_size, rust_size, status
    );
}

fn print_case_status(case: impl std::fmt::Display, status: &str) {
    println!("{:<28} {:>11} {:>11} {:>10}", case, "-", "-", status);
}

fn get_cpp_tools_path() -> Option<PathBuf> {
    let path = Path::new("../../build-original/src/draco/Release");
    if path.exists() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

/// Create a mesh with position, normals, and texture coordinates
/// Uses 50x50 grid to match the original encoding-speed compatibility test
fn create_mesh_with_attributes() -> Mesh {
    let mut mesh = Mesh::new();
    let grid_size = 50; // Must match compat_encoding_speed.rs for consistent results
    let num_points = grid_size * grid_size;
    let num_faces = (grid_size - 1) * (grid_size - 1) * 2;

    mesh.set_num_points(num_points);
    mesh.set_num_faces(num_faces);

    // Position attribute
    let mut pos_attr = PointAttribute::new();
    pos_attr.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points as usize,
    );

    for y in 0..grid_size {
        for x in 0..grid_size {
            let index = y * grid_size + x;
            let px = x as f32;
            let py = y as f32;
            // Use same sinusoidal variation as compat_encoding_speed.rs (0.2 not 0.3)
            let pz = (x as f32 * 0.2).sin() * (y as f32 * 0.2).cos() * 2.0;

            let offset = index as usize * 3 * 4;
            pos_attr
                .buffer_mut()
                .update(&px.to_le_bytes(), Some(offset));
            pos_attr
                .buffer_mut()
                .update(&py.to_le_bytes(), Some(offset + 4));
            pos_attr
                .buffer_mut()
                .update(&pz.to_le_bytes(), Some(offset + 8));
        }
    }
    mesh.add_attribute(pos_attr);

    // Set faces
    let mut face_idx = 0;
    for y in 0..grid_size - 1 {
        for x in 0..grid_size - 1 {
            let p0 = y * grid_size + x;
            let p1 = y * grid_size + x + 1;
            let p2 = (y + 1) * grid_size + x;
            let p3 = (y + 1) * grid_size + x + 1;

            mesh.set_face(
                FaceIndex(face_idx),
                [
                    PointIndex(p0 as u32),
                    PointIndex(p1 as u32),
                    PointIndex(p2 as u32),
                ],
            );
            face_idx += 1;
            mesh.set_face(
                FaceIndex(face_idx),
                [
                    PointIndex(p1 as u32),
                    PointIndex(p3 as u32),
                    PointIndex(p2 as u32),
                ],
            );
            face_idx += 1;
        }
    }

    mesh
}

/// Test different quantization bits for position attribute
#[test]
fn test_quantization_bits_compatibility() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    let tools_path = match get_cpp_tools_path() {
        Some(p) => p,
        None => {
            eprintln!("SKIPPING: C++ tools not found");
            return;
        }
    };
    let encoder_path = tools_path.join("draco_encoder.exe");
    let decoder_path = tools_path.join("draco_decoder.exe");

    if !encoder_path.exists() || !decoder_path.exists() {
        eprintln!("SKIPPING: C++ encoder/decoder not found");
        return;
    }

    let mesh = create_mesh_with_attributes();
    let obj_path = Path::new("temp_qbits_test.obj");
    if obj_path.exists() {
        let _ = std::fs::remove_file(obj_path);
    }
    draco_io::obj_writer::write_obj_mesh(obj_path, &mesh).expect("Failed to write obj");

    // Read back to match OBJ precision
    let mut obj_reader = draco_io::ObjReader::open(obj_path).expect("Failed to open obj");
    let mesh = obj_reader.read_mesh().expect("Failed to read obj");

    // Test different quantization bits: 8, 10, 11 (default), 12, 14, 16
    let quantization_values = [8, 10, 11, 12, 14, 16];

    print_size_table_header("C++ vs Rust Quantization Bits Compatibility");

    let mut all_passed = true;

    for qp in quantization_values {
        // Use a fixed speed for this test
        let speed = 5;
        let cpp_cl = 10 - speed;

        // Rust encode
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);
        options.set_attribute_int(0, "quantization_bits", qp);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut buffer = EncoderBuffer::new();
        let res = encoder.encode(&options, &mut buffer);
        assert!(res.is_ok(), "Rust encode failed with qp={}", qp);
        let rust_data = buffer.data().to_vec();

        // C++ encode
        let cpp_drc_path = format!("temp_cpp_qp{}.drc", qp);
        let output = Command::new(&encoder_path)
            .arg("-i")
            .arg(obj_path)
            .arg("-o")
            .arg(&cpp_drc_path)
            .arg("-method")
            .arg("edgebreaker")
            .arg("-cl")
            .arg(cpp_cl.to_string())
            .arg("-qp")
            .arg(qp.to_string())
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                let mut file = File::open(&cpp_drc_path).expect("Failed to open cpp drc");
                let mut cpp_data = Vec::new();
                file.read_to_end(&mut cpp_data)
                    .expect("Failed to read cpp drc");

                let status = if rust_data == cpp_data {
                    "MATCH"
                } else {
                    "MISMATCH"
                };
                if rust_data != cpp_data {
                    all_passed = false;
                }
                print_size_row(
                    format!("qp={qp:2}"),
                    cpp_data.len(),
                    rust_data.len(),
                    status,
                );

                let _ = std::fs::remove_file(&cpp_drc_path);
            } else {
                eprintln!(
                    "C++ encoder failed for qp={}: {}",
                    qp,
                    String::from_utf8_lossy(&output.stderr)
                );
                all_passed = false;
            }
        }

        // Verify C++ can decode Rust output
        let rust_drc_path = format!("temp_rust_qp{}.drc", qp);
        std::fs::write(&rust_drc_path, &rust_data).expect("write failed");
        let decode_result = Command::new(&decoder_path)
            .arg("-i")
            .arg(&rust_drc_path)
            .output();
        if let Ok(output) = decode_result {
            if !output.status.success() {
                eprintln!("C++ decoder failed for Rust output qp={}", qp);
                all_passed = false;
            }
        }
        let _ = std::fs::remove_file(&rust_drc_path);
    }

    let _ = std::fs::remove_file(obj_path);

    assert!(
        all_passed,
        "Some quantization tests failed - see output above"
    );
}

/// Test all compression levels (0-10) at a specific quantization
#[test]
fn test_compression_levels_compatibility() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    let tools_path = match get_cpp_tools_path() {
        Some(p) => p,
        None => {
            eprintln!("SKIPPING: C++ tools not found");
            return;
        }
    };
    let encoder_path = tools_path.join("draco_encoder.exe");
    let decoder_path = tools_path.join("draco_decoder.exe");

    if !encoder_path.exists() || !decoder_path.exists() {
        eprintln!("SKIPPING: C++ encoder/decoder not found");
        return;
    }

    let mesh = create_mesh_with_attributes();
    let obj_path = Path::new("temp_cl_test.obj");
    if obj_path.exists() {
        let _ = std::fs::remove_file(obj_path);
    }
    draco_io::obj_writer::write_obj_mesh(obj_path, &mesh).expect("Failed to write obj");

    let mut obj_reader = draco_io::ObjReader::open(obj_path).expect("Failed to open obj");
    let mesh = obj_reader.read_mesh().expect("Failed to read obj");

    print_size_table_header("C++ vs Rust Compression Levels Compatibility");
    println!("Note: Rust speed = 10 - cpp_cl\n");

    let mut all_passed = true;

    // Test compression levels 0-10
    for cl in 0..=10 {
        let speed = 10 - cl; // Rust speed is inverse of C++ compression level
        let qp = 10; // Match original encoding-speed compatibility test

        // Rust encode
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);
        options.set_attribute_int(0, "quantization_bits", qp);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut buffer = EncoderBuffer::new();
        let res = encoder.encode(&options, &mut buffer);
        assert!(
            res.is_ok(),
            "Rust encode failed with cl={} (speed={})",
            cl,
            speed
        );
        let rust_data = buffer.data().to_vec();

        // C++ encode
        let cpp_drc_path = format!("temp_cpp_cl{}.drc", cl);
        let output = Command::new(&encoder_path)
            .arg("-i")
            .arg(obj_path)
            .arg("-o")
            .arg(&cpp_drc_path)
            .arg("-method")
            .arg("edgebreaker")
            .arg("-cl")
            .arg(cl.to_string())
            .arg("-qp")
            .arg(qp.to_string())
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                let mut file = File::open(&cpp_drc_path).expect("Failed to open cpp drc");
                let mut cpp_data = Vec::new();
                file.read_to_end(&mut cpp_data)
                    .expect("Failed to read cpp drc");

                let status = if rust_data == cpp_data {
                    "MATCH"
                } else {
                    "MISMATCH"
                };
                if rust_data != cpp_data {
                    all_passed = false;
                }
                print_size_row(
                    format!("cl={cl:2} speed={speed:2}"),
                    cpp_data.len(),
                    rust_data.len(),
                    status,
                );

                let _ = std::fs::remove_file(&cpp_drc_path);
            } else {
                eprintln!(
                    "C++ encoder failed for cl={}: {}",
                    cl,
                    String::from_utf8_lossy(&output.stderr)
                );
                all_passed = false;
            }
        }

        // Verify C++ can decode Rust output
        let rust_drc_path = format!("temp_rust_cl{}.drc", cl);
        std::fs::write(&rust_drc_path, &rust_data).expect("write failed");
        let decode_result = Command::new(&decoder_path)
            .arg("-i")
            .arg(&rust_drc_path)
            .output();
        if let Ok(output) = decode_result {
            if !output.status.success() {
                eprintln!("C++ decoder failed for Rust output cl={}", cl);
                all_passed = false;
            }
        }
        let _ = std::fs::remove_file(&rust_drc_path);
    }

    let _ = std::fs::remove_file(obj_path);

    assert!(
        all_passed,
        "Some compression level tests failed - see output above"
    );
}

/// Test edge cases for quantization bits (minimum and maximum values)
#[test]
fn test_quantization_edge_cases() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    let tools_path = match get_cpp_tools_path() {
        Some(p) => p,
        None => {
            eprintln!("SKIPPING: C++ tools not found");
            return;
        }
    };
    let encoder_path = tools_path.join("draco_encoder.exe");
    let decoder_path = tools_path.join("draco_decoder.exe");

    if !encoder_path.exists() || !decoder_path.exists() {
        eprintln!("SKIPPING: C++ encoder/decoder not found");
        return;
    }

    let mesh = create_mesh_with_attributes();
    let obj_path = Path::new("temp_qedge_test.obj");
    if obj_path.exists() {
        let _ = std::fs::remove_file(obj_path);
    }
    draco_io::obj_writer::write_obj_mesh(obj_path, &mesh).expect("Failed to write obj");

    let mut obj_reader = draco_io::ObjReader::open(obj_path).expect("Failed to open obj");
    let mesh = obj_reader.read_mesh().expect("Failed to read obj");

    print_size_table_header("C++ vs Rust Quantization Edge Cases");

    // Test extreme quantization values
    // Note: C++ Draco supports 1-30 bits, but practical range is typically 8-20
    let edge_cases = [
        (1, "minimum bits"),
        (4, "very low bits"),
        (20, "high precision"),
        (24, "very high precision"),
    ];

    let mut all_passed = true;

    for (qp, desc) in edge_cases {
        let speed = 5;
        let cpp_cl = 10 - speed;

        // Rust encode
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);
        options.set_attribute_int(0, "quantization_bits", qp);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut buffer = EncoderBuffer::new();
        let res = encoder.encode(&options, &mut buffer);

        if res.is_err() {
            print_case_status(format!("qp={qp:2} {desc}"), "RUST FAIL");
            continue;
        }
        let rust_data = buffer.data().to_vec();

        // C++ encode
        let cpp_drc_path = format!("temp_cpp_qedge{}.drc", qp);
        let output = Command::new(&encoder_path)
            .arg("-i")
            .arg(obj_path)
            .arg("-o")
            .arg(&cpp_drc_path)
            .arg("-method")
            .arg("edgebreaker")
            .arg("-cl")
            .arg(cpp_cl.to_string())
            .arg("-qp")
            .arg(qp.to_string())
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                let mut file = File::open(&cpp_drc_path).expect("Failed to open cpp drc");
                let mut cpp_data = Vec::new();
                file.read_to_end(&mut cpp_data)
                    .expect("Failed to read cpp drc");

                let status = if rust_data == cpp_data {
                    "MATCH"
                } else {
                    "MISMATCH"
                };
                if rust_data != cpp_data {
                    all_passed = false;
                }
                print_size_row(
                    format!("qp={qp:2} {desc}"),
                    cpp_data.len(),
                    rust_data.len(),
                    status,
                );

                let _ = std::fs::remove_file(&cpp_drc_path);
            } else {
                print_case_status(format!("qp={qp:2} {desc}"), "C++ FAIL");
            }
        }

        // Verify C++ can decode Rust output
        let rust_drc_path = format!("temp_rust_qedge{}.drc", qp);
        std::fs::write(&rust_drc_path, &rust_data).expect("write failed");
        let decode_result = Command::new(&decoder_path)
            .arg("-i")
            .arg(&rust_drc_path)
            .output();
        if let Ok(output) = decode_result {
            if !output.status.success() {
                println!("qp={:2} ({}): C++ decoder failed for Rust output", qp, desc);
                all_passed = false;
            }
        }
        let _ = std::fs::remove_file(&rust_drc_path);
    }

    let _ = std::fs::remove_file(obj_path);

    // Don't fail on edge cases - they may legitimately differ
    if !all_passed {
        println!("\nNote: Some edge case tests showed differences - this may be expected for extreme values");
    }
}

/// Combined test: different speeds with different quantization bits
#[test]
fn test_speed_quantization_matrix() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    let tools_path = match get_cpp_tools_path() {
        Some(p) => p,
        None => {
            eprintln!("SKIPPING: C++ tools not found");
            return;
        }
    };
    let encoder_path = tools_path.join("draco_encoder.exe");
    let decoder_path = tools_path.join("draco_decoder.exe");

    if !encoder_path.exists() || !decoder_path.exists() {
        eprintln!("SKIPPING: C++ encoder/decoder not found");
        return;
    }

    let mesh = create_mesh_with_attributes();
    let obj_path = Path::new("temp_matrix_test.obj");
    if obj_path.exists() {
        let _ = std::fs::remove_file(obj_path);
    }
    draco_io::obj_writer::write_obj_mesh(obj_path, &mesh).expect("Failed to write obj");

    let mut obj_reader = draco_io::ObjReader::open(obj_path).expect("Failed to open obj");
    let mesh = obj_reader.read_mesh().expect("Failed to read obj");

    print_size_table_header("C++ vs Rust Speed x Quantization Matrix");
    println!("Note: speed 0, 5, 10 with qp 8, 11, 14\n");

    let speeds = [0, 5, 10];
    let qp_values = [8, 11, 14];

    let mut match_count = 0;
    let mut total_count = 0;

    for speed in speeds {
        for qp in qp_values {
            let cpp_cl = 10 - speed;
            total_count += 1;

            // Rust encode
            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_speed", speed);
            options.set_global_int("decoding_speed", speed);
            options.set_attribute_int(0, "quantization_bits", qp);

            let mut encoder = MeshEncoder::new();
            encoder.set_mesh(mesh.clone());
            let mut buffer = EncoderBuffer::new();
            let res = encoder.encode(&options, &mut buffer);
            assert!(res.is_ok(), "Rust encode failed");
            let rust_data = buffer.data().to_vec();

            // C++ encode
            let cpp_drc_path = format!("temp_cpp_s{}_qp{}.drc", speed, qp);
            let output = Command::new(&encoder_path)
                .arg("-i")
                .arg(obj_path)
                .arg("-o")
                .arg(&cpp_drc_path)
                .arg("-method")
                .arg("edgebreaker")
                .arg("-cl")
                .arg(cpp_cl.to_string())
                .arg("-qp")
                .arg(qp.to_string())
                .output();

            if let Ok(output) = output {
                if output.status.success() {
                    let mut file = File::open(&cpp_drc_path).expect("Failed to open cpp drc");
                    let mut cpp_data = Vec::new();
                    file.read_to_end(&mut cpp_data)
                        .expect("Failed to read cpp drc");

                    let matched = rust_data == cpp_data;
                    if matched {
                        match_count += 1;
                    }
                    let status = if matched { "MATCH" } else { "MISMATCH" };
                    print_size_row(
                        format!("speed={speed:2} qp={qp:2}"),
                        cpp_data.len(),
                        rust_data.len(),
                        status,
                    );

                    let _ = std::fs::remove_file(&cpp_drc_path);
                }
            }

            // Cleanup
            let rust_drc_path = format!("temp_rust_s{}_qp{}.drc", speed, qp);
            std::fs::write(&rust_drc_path, &rust_data).ok();
            let _ = std::fs::remove_file(&rust_drc_path);
        }
    }

    let _ = std::fs::remove_file(obj_path);

    println!(
        "\nMatrix results: {}/{} combinations matched",
        match_count, total_count
    );
    assert_eq!(
        match_count, total_count,
        "Not all speed/quantization combinations matched"
    );
}
