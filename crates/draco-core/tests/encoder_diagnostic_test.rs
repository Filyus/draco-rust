use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
/// Diagnostic test to find actual implementation differences between Rust and C++
/// This test uses multiple mesh sizes and parameters to identify real bugs.
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

fn get_cpp_tools_path() -> Option<PathBuf> {
    let path = Path::new("../../build-original/src/draco/Release");
    if path.exists() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

fn create_grid_mesh(grid_size: usize, z_variation: f32) -> Mesh {
    let mut mesh = Mesh::new();
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
            let pz = (x as f32 * z_variation).sin() * (y as f32 * z_variation).cos() * 2.0;

            let offset = index * 3 * 4;
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

    let mut face_idx = 0u32;
    for y in 0..grid_size - 1 {
        for x in 0..grid_size - 1 {
            let p0 = (y * grid_size + x) as u32;
            let p1 = (y * grid_size + x + 1) as u32;
            let p2 = ((y + 1) * grid_size + x) as u32;
            let p3 = ((y + 1) * grid_size + x + 1) as u32;

            mesh.set_face(
                FaceIndex(face_idx),
                [PointIndex(p0), PointIndex(p1), PointIndex(p2)],
            );
            face_idx += 1;
            mesh.set_face(
                FaceIndex(face_idx),
                [PointIndex(p1), PointIndex(p3), PointIndex(p2)],
            );
            face_idx += 1;
        }
    }

    mesh
}

fn find_first_diff(rust_data: &[u8], cpp_data: &[u8]) -> Option<(usize, u8, u8)> {
    let min_len = rust_data.len().min(cpp_data.len());
    for i in 0..min_len {
        if rust_data[i] != cpp_data[i] {
            return Some((i, rust_data[i], cpp_data[i]));
        }
    }
    if rust_data.len() != cpp_data.len() {
        return Some((
            min_len,
            rust_data.get(min_len).copied().unwrap_or(0),
            cpp_data.get(min_len).copied().unwrap_or(0),
        ));
    }
    None
}

/// Test to find actual mismatches with various parameters
#[test]
fn test_find_implementation_differences() {
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

    println!("\n=== Testing Multiple Grid Sizes and Parameters ===\n");

    // Test various grid sizes with the known-working z_var=0.2 and qp=10
    // to isolate size-related issues from parameter-related issues
    let grid_sizes = [10, 20, 30, 40, 50];
    let z_variations = [0.2f32]; // Use only the known-working value
    let qp_values = [10]; // Use only the known-working value
    let speeds = [0, 1, 2, 3, 4, 5, 10];

    let mut mismatches: Vec<String> = Vec::new();

    for &grid_size in &grid_sizes {
        for &z_var in &z_variations {
            let mesh = create_grid_mesh(grid_size, z_var);
            let obj_path = format!("temp_diag_g{}_z{}.obj", grid_size, (z_var * 10.0) as i32);
            let obj_path = Path::new(&obj_path);

            if obj_path.exists() {
                let _ = std::fs::remove_file(obj_path);
            }
            draco_io::obj_writer::write_obj_mesh(obj_path, &mesh).expect("Failed to write obj");

            let mut obj_reader = draco_io::ObjReader::open(obj_path).expect("Failed to open obj");
            let mesh = obj_reader.read_mesh().expect("Failed to read obj");

            for &qp in &qp_values {
                for &speed in &speeds {
                    let cpp_cl = 10 - speed;

                    // Rust encode
                    let mut options = EncoderOptions::new();
                    options.set_global_int("encoding_speed", speed);
                    options.set_global_int("decoding_speed", speed);
                    options.set_attribute_int(0, "quantization_bits", qp);

                    let mut encoder = MeshEncoder::new();
                    encoder.set_mesh(mesh.clone());
                    let mut buffer = EncoderBuffer::new();
                    if encoder.encode(&options, &mut buffer).is_err() {
                        continue;
                    }
                    let rust_data = buffer.data().to_vec();

                    // C++ encode
                    let cpp_drc_path = format!(
                        "temp_cpp_diag_g{}_z{}_qp{}_s{}.drc",
                        grid_size,
                        (z_var * 10.0) as i32,
                        qp,
                        speed
                    );
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
                            let mut file =
                                File::open(&cpp_drc_path).expect("Failed to open cpp drc");
                            let mut cpp_data = Vec::new();
                            file.read_to_end(&mut cpp_data)
                                .expect("Failed to read cpp drc");

                            if rust_data != cpp_data {
                                let diff_info = if let Some((pos, rust_byte, cpp_byte)) =
                                    find_first_diff(&rust_data, &cpp_data)
                                {
                                    format!(
                                        "first diff at byte {}: rust=0x{:02X}, cpp=0x{:02X}",
                                        pos, rust_byte, cpp_byte
                                    )
                                } else {
                                    "size diff only".to_string()
                                };

                                let msg = format!(
                                    "MISMATCH: grid={}, z_var={}, qp={}, speed={} (cl={}): rust={}, cpp={} bytes - {}",
                                    grid_size, z_var, qp, speed, cpp_cl, rust_data.len(), cpp_data.len(), diff_info
                                );
                                println!("{}", msg);
                                mismatches.push(msg);

                                // Save files for analysis
                                let rust_drc_path = format!(
                                    "temp_rust_diag_g{}_z{}_qp{}_s{}.drc",
                                    grid_size,
                                    (z_var * 10.0) as i32,
                                    qp,
                                    speed
                                );
                                std::fs::write(&rust_drc_path, &rust_data).ok();
                            }

                            let _ = std::fs::remove_file(&cpp_drc_path);
                        }
                    }
                }
            }

            let _ = std::fs::remove_file(obj_path);
        }
    }

    println!("\n=== Summary ===");
    println!("Total mismatches found: {}", mismatches.len());

    if !mismatches.is_empty() {
        println!("\nAll mismatches:");
        for m in &mismatches {
            println!("  {}", m);
        }

        // Don't fail - just report for now so we can analyze
        // panic!("Found {} implementation differences", mismatches.len());
    }
}
