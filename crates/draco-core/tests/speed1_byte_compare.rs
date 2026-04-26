use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
/// Detailed byte comparison for speed=1 mismatches
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

fn dump_bytes(data: &[u8], start: usize, end: usize, label: &str) {
    println!("{} bytes {} to {}:", label, start, end.min(data.len() - 1));
    for i in start..end.min(data.len()) {
        print!("{:3}: 0x{:02X}", i, data[i]);
        if i < data.len() - 1 {
            print!("  ");
        }
        if (i - start + 1) % 8 == 0 {
            println!();
        }
    }
    println!();
}

fn compare_bytes(rust: &[u8], cpp: &[u8]) {
    println!("\n=== HEADER COMPARISON (bytes 0-30) ===");
    dump_bytes(rust, 0, 31, "Rust");
    dump_bytes(cpp, 0, 31, "C++");

    // Find first difference
    for i in 0..rust.len().min(cpp.len()) {
        if rust[i] != cpp[i] {
            println!("\n=== FIRST DIFFERENCE AT BYTE {} ===", i);
            println!("Rust: 0x{:02X} ({})", rust[i], rust[i]);
            println!("C++:  0x{:02X} ({})", cpp[i], cpp[i]);

            // Show context
            let start = if i >= 10 { i - 10 } else { 0 };
            let end = i + 20;
            println!("\n=== CONTEXT AROUND DIFFERENCE ===");
            dump_bytes(rust, start, end, "Rust");
            dump_bytes(cpp, start, end, "C++");

            // Show differences in a range
            println!("\n=== ALL DIFFERENCES ===");
            let mut diff_count = 0;
            for j in 0..rust.len().min(cpp.len()) {
                if rust[j] != cpp[j] {
                    println!("byte {}: rust=0x{:02X}, cpp=0x{:02X}", j, rust[j], cpp[j]);
                    diff_count += 1;
                    if diff_count >= 20 {
                        println!("... (truncated at 20 differences)");
                        break;
                    }
                }
            }
            return;
        }
    }
}

#[test]
fn test_speed1_detailed_comparison() {
    let tools_path = match get_cpp_tools_path() {
        Some(p) => p,
        None => {
            eprintln!("SKIPPING: C++ tools not found");
            return;
        }
    };
    let encoder_path = tools_path.join("draco_encoder.exe");

    if !encoder_path.exists() {
        eprintln!("SKIPPING: C++ encoder not found at {:?}", encoder_path);
        return;
    }

    // Test the specific failing case: grid=10, z_var=0.2, qp=10, speed=1
    let grid_size = 10;
    let z_var = 0.2f32;
    let qp = 10;
    let speed = 1;

    let mesh = create_grid_mesh(grid_size, z_var);
    println!(
        "Mesh has {} faces, {} points",
        mesh.num_faces(),
        mesh.num_points()
    );

    // Write to OBJ
    let obj_path = Path::new("temp_speed1_test.obj");
    draco_io::obj_writer::write_obj_mesh(obj_path, &mesh).expect("Failed to write obj");

    // Re-read through OBJ to ensure consistent processing
    let mut obj_reader = draco_io::ObjReader::open(obj_path).expect("Failed to open obj");
    let mesh = obj_reader.read_mesh().expect("Failed to read obj");

    // Rust encode
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", speed);
    options.set_global_int("decoding_speed", speed);
    options.set_attribute_int(0, "quantization_bits", qp);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("Rust encode failed");
    let rust_data = buffer.data().to_vec();

    // C++ encode
    let cpp_cl = 10 - speed; // cl=9
    let cpp_drc_path = "temp_cpp_speed1_test.drc";
    let output = Command::new(&encoder_path)
        .arg("-i")
        .arg(obj_path)
        .arg("-o")
        .arg(cpp_drc_path)
        .arg("-method")
        .arg("edgebreaker")
        .arg("-cl")
        .arg(cpp_cl.to_string())
        .arg("-qp")
        .arg(qp.to_string())
        .output()
        .expect("Failed to run C++ encoder");

    if !output.status.success() {
        eprintln!(
            "C++ encoder failed: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    // Print C++ debug output
    let cpp_stderr = String::from_utf8_lossy(&output.stderr);
    if !cpp_stderr.is_empty() {
        eprintln!("=== C++ encoder stderr ===");
        eprintln!("{}", cpp_stderr);
        eprintln!("=== end C++ stderr ===");
    }

    let mut cpp_file = File::open(cpp_drc_path).expect("Failed to open cpp drc");
    let mut cpp_data = Vec::new();
    cpp_file
        .read_to_end(&mut cpp_data)
        .expect("Failed to read cpp drc");

    println!("\nRust output: {} bytes", rust_data.len());
    println!("C++ output:  {} bytes", cpp_data.len());

    compare_bytes(&rust_data, &cpp_data);

    // Keep files for debugging
    // let _ = std::fs::remove_file(obj_path);
    // let _ = std::fs::remove_file(cpp_drc_path);
}
