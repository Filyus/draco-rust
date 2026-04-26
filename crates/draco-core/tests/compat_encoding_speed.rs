use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

fn print_speed_table_header() {
    println!(
        "{:>5} {:>5} {:>11} {:>11} {:>12}",
        "Speed", "cl", "C++ bytes", "Rust bytes", "Status"
    );
    println!("{}", "-".repeat(52));
}

fn get_cpp_tools_path() -> Option<PathBuf> {
    // Corrected path based on previous exploration
    let path = Path::new("../../build-original/src/draco/Release");
    if path.exists() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

fn create_complex_mesh() -> Mesh {
    let mut mesh = Mesh::new();
    let grid_size = 50;
    let num_points = grid_size * grid_size;
    let num_faces = (grid_size - 1) * (grid_size - 1) * 2;

    mesh.set_num_points(num_points);
    mesh.set_num_faces(num_faces);

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
            // Add sinusoidal variation to Z to make it non-planar and interesting for quantization
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

    let mut face_idx = 0;
    for y in 0..grid_size - 1 {
        for x in 0..grid_size - 1 {
            let p0 = y * grid_size + x;
            let p1 = y * grid_size + x + 1;
            let p2 = (y + 1) * grid_size + x;
            let p3 = (y + 1) * grid_size + x + 1;

            // Triangle 1
            mesh.set_face(
                FaceIndex(face_idx),
                [
                    PointIndex(p0 as u32),
                    PointIndex(p1 as u32),
                    PointIndex(p2 as u32),
                ],
            );
            face_idx += 1;

            // Triangle 2
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

#[test]
fn compat_encoding_speed() {
    let tools_path = match get_cpp_tools_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "SKIPPING comparison: C++ tools not found at {:?}",
                Path::new("../../build-original/src/draco/Release").canonicalize()
            );
            return;
        }
    };
    eprintln!("Using C++ tools from: {:?}", tools_path.canonicalize());
    let encoder_path = tools_path.join("draco_encoder.exe");
    let decoder_path = tools_path.join("draco_decoder.exe");

    if !encoder_path.exists() {
        eprintln!("draco_encoder.exe not found");
        return;
    }

    let mesh = create_complex_mesh();

    let obj_path = Path::new("temp_compatibility.obj");
    // Ensure cleanup of this file
    if obj_path.exists() {
        let _ = std::fs::remove_file(obj_path);
    }

    // We need obj writer separate or we can assume there is one available in tests or core
    // Based on debug_comparison.rs, it seems `draco_io::obj_writer` is available.
    draco_io::obj_writer::write_obj_mesh(obj_path, &mesh).expect("Failed to write obj");

    // IMPORTANT: Read the mesh back from OBJ to ensure we use the same precision as C++
    // The OBJ format limits float precision, so both encoders must use the OBJ-parsed values.
    // The OBJ reader automatically deduplicates point IDs to match C++ OBJ loader behavior.
    let mut obj_reader = draco_io::ObjReader::open(obj_path).expect("Failed to open obj");
    let mesh = obj_reader.read_mesh().expect("Failed to read obj back");

    println!("\n=== C++ vs Rust Speed Compatibility ===");
    print_speed_table_header();

    for speed in 0..=10 {
        // Rust Encode
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);
        options.set_attribute_int(0, "quantization_bits", 10); // Match C++ -qp 10

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut encoder_buffer = EncoderBuffer::new();
        let res = encoder.encode(&options, &mut encoder_buffer);
        assert!(res.is_ok(), "Rust encode failed at speed {}", speed);
        let rust_data = encoder_buffer.data().to_vec();

        // Verify: Can C++ decoder decode Rust output?
        let rust_drc_path = format!("temp_rust_s{}.drc", speed);
        std::fs::write(&rust_drc_path, &rust_data)
            .expect("Failed to write rust output for checking");

        let decode_output = Command::new(&decoder_path)
            .arg("-i")
            .arg(&rust_drc_path)
            .output()
            .expect("Failed to run draco_decoder on Rust output");

        let decode_success = decode_output.status.success();
        if !decode_success {
            eprintln!(
                "C++ decoder failed to decode Rust output at speed {}: {}",
                speed,
                String::from_utf8_lossy(&decode_output.stderr)
            );
            println!("STDOUT: {}", String::from_utf8_lossy(&decode_output.stdout));
        }
        assert!(
            decode_success,
            "C++ decoder could not decode Rust generated drc at speed {}",
            speed
        );

        // Optional: Compare with C++ output (heuristic mapping)
        // Heuristic: C++ cl (0..10) often maps inversely to speed or directly depending on implementation.
        // Usually cl=10 is max compression (slowest), cl=0 is min compression (fastest).
        // Rust speed=10 is fastest, speed=0 is slowest.
        // So we try cl = 10 - speed.
        let cpp_cl = 10 - speed;

        let cpp_drc_path = format!("temp_cpp_s{}.drc", speed);
        let mut cmd = Command::new(&encoder_path);
        cmd.arg("-i")
            .arg(obj_path)
            .arg("-o")
            .arg(&cpp_drc_path)
            .arg("-method")
            .arg("edgebreaker")
            .arg("-cl")
            .arg(cpp_cl.to_string())
            .arg("-qp")
            .arg("10");
        // Enable sparse CMP diagnostics for speed 0 runs so we can capture C++ divergence logs.
        if speed == 0 {
            cmd.env("DRACO_DEBUG_CMP_CPP", "1");
        }
        let output = cmd.output();

        if let Ok(output) = output {
            if output.status.success() {
                // Print C++ encoder stderr/stdout when diagnostics enabled (speed 0)
                if speed == 0 {
                    eprintln!(
                        "C++ encoder stderr:\n{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    eprintln!(
                        "C++ encoder stdout:\n{}",
                        String::from_utf8_lossy(&output.stdout)
                    );
                }

                let mut file = File::open(&cpp_drc_path).expect("Failed to open cpp drc");
                let mut cpp_data = Vec::new();
                file.read_to_end(&mut cpp_data)
                    .expect("Failed to read cpp drc");

                let status = if rust_data == cpp_data {
                    "MATCH"
                } else {
                    "MISMATCH"
                };
                println!(
                    "{:>5} {:>5} {:>11} {:>11} {:>12}",
                    speed,
                    cpp_cl,
                    cpp_data.len(),
                    rust_data.len(),
                    status
                );
                // let _ = std::fs::remove_file(&cpp_drc_path); // Keep for debugging
            } else {
                if speed == 0 {
                    eprintln!(
                        "C++ encoder failed. stderr:\n{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    eprintln!(
                        "C++ encoder stdout:\n{}",
                        String::from_utf8_lossy(&output.stdout)
                    );
                }
            }
        }

        // Cleanup per loop
        // let _ = std::fs::remove_file(&rust_drc_path); // Keep for debugging
    }

    // let _ = std::fs::remove_file(obj_path); // Keep for debugging
}
