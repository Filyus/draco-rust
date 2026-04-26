use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;

fn get_cpp_encoder_path() -> Option<PathBuf> {
    let path = Path::new("../../build-original/src/draco/Release/draco_encoder.exe");
    if path.exists() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

#[allow(dead_code)]
fn get_cpp_decoder_path() -> Option<PathBuf> {
    let path = Path::new("../../build-original/src/draco/Release/draco_decoder.exe");
    if path.exists() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

#[allow(dead_code)]
fn create_grid_mesh(grid_size: usize) -> Mesh {
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
        num_points,
    );

    for y in 0..grid_size {
        for x in 0..grid_size {
            let index = y * grid_size + x;
            let px = x as f32;
            let py = y as f32;
            let pz = (x as f32 * 0.2).sin() * (y as f32 * 0.2).cos() * 2.0;

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

    let mut face_idx = 0;
    for y in 0..grid_size - 1 {
        for x in 0..grid_size - 1 {
            let p0 = y * grid_size + x;
            let p1 = y * grid_size + x + 1;
            let p2 = (y + 1) * grid_size + x;
            let p3 = (y + 1) * grid_size + x + 1;

            mesh.set_face(
                FaceIndex(face_idx as u32),
                [
                    PointIndex(p0 as u32),
                    PointIndex(p1 as u32),
                    PointIndex(p2 as u32),
                ],
            );
            face_idx += 1;
            mesh.set_face(
                FaceIndex(face_idx as u32),
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

fn write_obj(path: &Path, grid_size: usize) {
    let mut obj_content = String::new();
    for y in 0..grid_size {
        for x in 0..grid_size {
            let pz = (x as f32 * 0.2).sin() * (y as f32 * 0.2).cos() * 2.0;
            obj_content.push_str(&format!("v {} {} {}\n", x, y, pz));
        }
    }
    for y in 0..(grid_size - 1) {
        for x in 0..(grid_size - 1) {
            let v0 = y * grid_size + x + 1;
            let v1 = y * grid_size + x + 2;
            let v2 = (y + 1) * grid_size + x + 1;
            let v3 = (y + 1) * grid_size + x + 2;
            obj_content.push_str(&format!("f {} {} {}\n", v0, v1, v2));
            obj_content.push_str(&format!("f {} {} {}\n", v1, v3, v2));
        }
    }
    std::fs::write(path, &obj_content).expect("Failed to write OBJ");
}

fn benchmark_rust_encoding(mesh: &Mesh, speed: i32, iterations: u32) -> (f64, usize) {
    let mut total_time = 0.0;
    let mut output_size = 0;

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", speed);
    options.set_global_int("decoding_speed", speed);
    options.set_attribute_int(0, "quantization_bits", 10);

    for _ in 0..iterations {
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut encoder_buffer = EncoderBuffer::new();

        let start = Instant::now();
        let _ = encoder.encode(&options, &mut encoder_buffer);
        let elapsed = start.elapsed();

        total_time += elapsed.as_secs_f64();
        output_size = encoder_buffer.data().len();
    }

    (total_time / iterations as f64, output_size)
}

#[allow(dead_code)]
fn benchmark_rust_decoding(encoded_data: &[u8], iterations: u32) -> f64 {
    let mut total_time = 0.0;

    for _ in 0..iterations {
        let mut decoder_buffer = DecoderBuffer::new(encoded_data);
        let mut out_mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();

        let start = Instant::now();
        let _ = decoder.decode(&mut decoder_buffer, &mut out_mesh);
        let elapsed = start.elapsed();

        total_time += elapsed.as_secs_f64();
    }

    total_time / iterations as f64
}

#[allow(dead_code)]
fn benchmark_cpp_decoding(decoder_path: &Path, drc_path: &Path, iterations: u32) -> f64 {
    let obj_out = drc_path.with_extension("decoded.obj");
    let mut total_time = 0.0;

    for _ in 0..iterations {
        let _ = std::fs::remove_file(&obj_out);

        let start = Instant::now();
        let output = Command::new(decoder_path)
            .args([
                "-i",
                drc_path.to_str().unwrap(),
                "-o",
                obj_out.to_str().unwrap(),
            ])
            .output()
            .expect("Failed to run C++ decoder");
        let elapsed = start.elapsed();

        if !output.status.success() {
            eprintln!(
                "C++ decoder failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return 0.0;
        }

        total_time += elapsed.as_secs_f64();
    }

    let _ = std::fs::remove_file(&obj_out);

    total_time / iterations as f64
}

fn benchmark_cpp_encoding(
    encoder_path: &Path,
    obj_path: &Path,
    speed: i32,
    iterations: u32,
) -> (f64, usize) {
    let drc_path = obj_path.with_extension("drc");
    let cl = 10 - speed;

    let mut total_time = 0.0;
    let mut output_size = 0;

    for _ in 0..iterations {
        let _ = std::fs::remove_file(&drc_path);

        let start = Instant::now();
        let output = Command::new(encoder_path)
            .args([
                "-i",
                obj_path.to_str().unwrap(),
                "-o",
                drc_path.to_str().unwrap(),
                "-method",
                "edgebreaker",
                "-cl",
                &cl.to_string(),
                "-qp",
                "10",
            ])
            .output()
            .expect("Failed to run C++ encoder");
        let elapsed = start.elapsed();

        if !output.status.success() {
            eprintln!(
                "C++ encoder failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return (0.0, 0);
        }

        total_time += elapsed.as_secs_f64();
        output_size = std::fs::metadata(&drc_path)
            .map(|m| m.len() as usize)
            .unwrap_or(0);
    }

    let _ = std::fs::remove_file(&drc_path);

    (total_time / iterations as f64, output_size)
}

#[test]
fn bench_external_cpp_encode() {
    let encoder_path = match get_cpp_encoder_path() {
        Some(p) => p,
        None => {
            eprintln!("SKIPPING: C++ encoder not found");
            return;
        }
    };

    let grid_sizes = [50, 100, 200];
    let speeds = [0, 1, 5, 10];
    let iterations = 5;

    println!("\n=== C++ vs Rust Encoding Performance ===\n");
    println!(
        "{:>6} {:>6} {:>12} {:>12} {:>10} {:>11} {:>11} {:>10}",
        "Grid", "Speed", "C++ (ms)", "Rust (ms)", "Speedup", "C++ bytes", "Rust bytes", "Status"
    );
    println!("{}", "-".repeat(92));

    for &grid_size in &grid_sizes {
        let num_faces = (grid_size - 1) * (grid_size - 1) * 2;
        let num_points = grid_size * grid_size;

        // Write OBJ for C++ encoder
        let obj_path = PathBuf::from(format!("temp_bench_{}.obj", grid_size));
        write_obj(&obj_path, grid_size);

        // Read back through OBJ to match C++ precision
        let mut obj_reader = draco_io::ObjReader::open(&obj_path).expect("Failed to open obj");
        let mesh = obj_reader.read_mesh().expect("Failed to read obj");

        println!(
            "\nGrid {}x{} ({} points, {} faces):",
            grid_size, grid_size, num_points, num_faces
        );

        for &speed in &speeds {
            let (rust_time, rust_size) = benchmark_rust_encoding(&mesh, speed, iterations);
            let (cpp_time, cpp_size) =
                benchmark_cpp_encoding(&encoder_path, &obj_path, speed, iterations);

            let rust_ms = rust_time * 1000.0;
            let cpp_ms = cpp_time * 1000.0;
            let speedup = if rust_ms > 0.0 { cpp_ms / rust_ms } else { 0.0 };

            let status = if rust_size == cpp_size {
                "MATCH"
            } else {
                "MISMATCH"
            };

            println!(
                "{:>6} {:>6} {:>10.2}ms {:>10.2}ms {:>9.2}x {:>11} {:>11} {:>10}",
                grid_size, speed, cpp_ms, rust_ms, speedup, cpp_size, rust_size, status
            );
        }

        let _ = std::fs::remove_file(&obj_path);
    }

    println!("\nNote: C++ times include process startup overhead (~10-20ms)");
    println!("      Speedup > 1.0 means Rust is faster, < 1.0 means C++ is faster\n");
}
