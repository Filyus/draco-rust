// Byte-by-byte comparison of C++ vs Rust encoder output

mod common;

use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;
use draco_cpp_test_bridge;
use std::sync::Mutex;

static OUTPUT_LOCK: Mutex<()> = Mutex::new(());

fn print_size_table_header(title: &str) {
    println!("\n=== {title} ===");
    println!(
        "{:<18} {:>11} {:>11} {:>10}",
        "Case", "C++ bytes", "Rust bytes", "Status"
    );
    println!("{}", "-".repeat(55));
}

fn print_size_row(case: &str, cpp_size: usize, rust_size: usize, status: &str) {
    println!(
        "{:<18} {:>11} {:>11} {:>10}",
        case, cpp_size, rust_size, status
    );
}

fn create_simple_mesh_data() -> (Vec<f32>, Vec<u32>) {
    // Simple cube
    let positions = vec![
        // Front face
        -0.5, -0.5, 0.5, 0.5, -0.5, 0.5, 0.5, 0.5, 0.5, -0.5, 0.5, 0.5, // Back face
        -0.5, -0.5, -0.5, 0.5, -0.5, -0.5, 0.5, 0.5, -0.5, -0.5, 0.5, -0.5,
    ];

    let faces = vec![
        // Front
        0, 1, 2, 0, 2, 3, // Back
        5, 4, 7, 5, 7, 6, // Top
        3, 2, 6, 3, 6, 7, // Bottom
        4, 5, 1, 4, 1, 0, // Right
        1, 5, 6, 1, 6, 2, // Left
        4, 0, 3, 4, 3, 7,
    ];

    (positions, faces)
}

fn create_grid_mesh_data(grid_size: usize) -> (Vec<f32>, Vec<u32>) {
    let num_points = grid_size * grid_size;
    let num_faces = (grid_size - 1) * (grid_size - 1) * 2;

    let mut positions = Vec::with_capacity(num_points * 3);
    for y in 0..grid_size {
        for x in 0..grid_size {
            let px = x as f32;
            let py = y as f32;
            let pz = (x as f32 * 0.2).sin() * (y as f32 * 0.2).cos() * 2.0;
            positions.push(px);
            positions.push(py);
            positions.push(pz);
        }
    }

    let mut faces = Vec::with_capacity(num_faces * 3);
    for y in 0..grid_size - 1 {
        for x in 0..grid_size - 1 {
            let p0 = (y * grid_size + x) as u32;
            let p1 = (y * grid_size + x + 1) as u32;
            let p2 = ((y + 1) * grid_size + x) as u32;
            let p3 = ((y + 1) * grid_size + x + 1) as u32;

            faces.push(p0);
            faces.push(p1);
            faces.push(p2);

            faces.push(p1);
            faces.push(p3);
            faces.push(p2);
        }
    }

    (positions, faces)
}

fn create_rust_mesh(positions: &[f32], faces: &[u32]) -> Mesh {
    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;

    let mut mesh = Mesh::new();
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

    for i in 0..num_points {
        let offset = i * 3 * 4;
        pos_attr
            .buffer_mut()
            .update(&positions[i * 3].to_le_bytes(), Some(offset));
        pos_attr
            .buffer_mut()
            .update(&positions[i * 3 + 1].to_le_bytes(), Some(offset + 4));
        pos_attr
            .buffer_mut()
            .update(&positions[i * 3 + 2].to_le_bytes(), Some(offset + 8));
    }
    mesh.add_attribute(pos_attr);

    for i in 0..num_faces {
        mesh.set_face(
            FaceIndex(i as u32),
            [
                PointIndex(faces[i * 3]),
                PointIndex(faces[i * 3 + 1]),
                PointIndex(faces[i * 3 + 2]),
            ],
        );
    }

    mesh
}

fn encode_rust(mesh: &Mesh, speed: i32, quantization_bits: i32) -> Vec<u8> {
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", speed);
    options.set_global_int("decoding_speed", speed);
    options.set_attribute_int(0, "quantization_bits", quantization_bits);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut encoder_buffer = EncoderBuffer::new();
    let _ = encoder.encode(&options, &mut encoder_buffer);

    encoder_buffer.data().to_vec()
}

fn find_first_difference(rust: &[u8], cpp: &[u8]) -> Option<usize> {
    let min_len = rust.len().min(cpp.len());
    for i in 0..min_len {
        if rust[i] != cpp[i] {
            return Some(i);
        }
    }
    if rust.len() != cpp.len() {
        return Some(min_len);
    }
    None
}

fn print_bytes_around(data: &[u8], center: usize, context: usize, label: &str) {
    let start = center.saturating_sub(context);
    let end = (center + context + 1).min(data.len());

    print!("{}: ", label);
    for i in start..end {
        if i == center {
            print!("[{:02x}] ", data[i]);
        } else {
            print!("{:02x} ", data[i]);
        }
    }
    println!();
}

#[test]
fn parity_encode_bytes_speed_10_simple() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    print_size_table_header("C++ vs Rust Byte Comparison: Speed 10 Simple Cube");

    let (positions, faces) = create_simple_mesh_data();
    let mesh = create_rust_mesh(&positions, &faces);

    let rust_bytes = encode_rust(&mesh, 10, 10);
    let cpp_bytes = draco_cpp_test_bridge::encode_cpp_mesh(&positions, &faces, 10, 10, 10)
        .expect("C++ encoding failed");

    print_size_row(
        "simple cube",
        cpp_bytes.len(),
        rust_bytes.len(),
        if cpp_bytes == rust_bytes {
            "MATCH"
        } else {
            "DIFF"
        },
    );

    if let Some(diff_pos) = find_first_difference(&rust_bytes, &cpp_bytes) {
        println!("\nFirst difference at byte offset: {}", diff_pos);
        println!("Context (5 bytes before/after):");
        print_bytes_around(&cpp_bytes, diff_pos, 5, "C++ ");
        print_bytes_around(&rust_bytes, diff_pos, 5, "Rust");

        // Print hex dump of first 100 bytes
        println!("\n--- First 100 bytes hex dump ---");
        println!("C++:");
        for (i, chunk) in cpp_bytes
            .iter()
            .take(100)
            .collect::<Vec<_>>()
            .chunks(16)
            .enumerate()
        {
            print!("{:04x}: ", i * 16);
            for b in chunk {
                print!("{:02x} ", b);
            }
            println!();
        }
        println!("\nRust:");
        for (i, chunk) in rust_bytes
            .iter()
            .take(100)
            .collect::<Vec<_>>()
            .chunks(16)
            .enumerate()
        {
            print!("{:04x}: ", i * 16);
            for b in chunk {
                print!("{:02x} ", b);
            }
            println!();
        }

        panic!("C++ and Rust outputs differ at byte {}", diff_pos);
    } else {
        println!("\n✓ C++ and Rust outputs are IDENTICAL!");
    }
}

#[test]
fn parity_encode_bytes_speed_10_grid() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    print_size_table_header("C++ vs Rust Byte Comparison: Speed 10 10x10 Grid");

    let (positions, faces) = create_grid_mesh_data(10);
    let mesh = create_rust_mesh(&positions, &faces);

    let rust_bytes = encode_rust(&mesh, 10, 10);
    let cpp_bytes = draco_cpp_test_bridge::encode_cpp_mesh(&positions, &faces, 10, 10, 10)
        .expect("C++ encoding failed");

    print_size_row(
        "10x10 grid",
        cpp_bytes.len(),
        rust_bytes.len(),
        if cpp_bytes == rust_bytes {
            "MATCH"
        } else {
            "DIFF"
        },
    );

    if let Some(diff_pos) = find_first_difference(&rust_bytes, &cpp_bytes) {
        println!("\nFirst difference at byte offset: {}", diff_pos);
        println!("Context (10 bytes before/after):");
        print_bytes_around(&cpp_bytes, diff_pos, 10, "C++ ");
        print_bytes_around(&rust_bytes, diff_pos, 10, "Rust");

        // Analyze header
        println!("\n--- Header analysis ---");
        println!("Bytes 0-4: Magic 'DRACO'");
        println!("Byte 5: Major version");
        println!("Byte 6: Minor version");
        println!("Byte 7: Encoder type (0=PC, 1=mesh)");
        println!("Byte 8: Encoding method (0=sequential, 1=edgebreaker)");
        println!("Bytes 9-10: Flags");

        println!(
            "\nC++  header: {:?}",
            &cpp_bytes[0..11.min(cpp_bytes.len())]
        );
        println!(
            "Rust header: {:?}",
            &rust_bytes[0..11.min(rust_bytes.len())]
        );

        panic!("C++ and Rust outputs differ at byte {}", diff_pos);
    } else {
        println!("\n✓ C++ and Rust outputs are IDENTICAL!");
    }
}

#[test]
fn parity_encode_bytes_all_speeds() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    print_size_table_header("C++ vs Rust Byte Comparison: All Speeds 10x10 Grid");

    let (positions, faces) = create_grid_mesh_data(10);
    let mesh = create_rust_mesh(&positions, &faces);

    let speeds = [0, 1, 5, 10];

    for speed in speeds {
        let rust_bytes = encode_rust(&mesh, speed, 10);
        let cpp_bytes =
            draco_cpp_test_bridge::encode_cpp_mesh(&positions, &faces, speed, speed, 10)
                .expect("C++ encoding failed");

        let status = if rust_bytes == cpp_bytes {
            "✓ MATCH"
        } else if let Some(diff_pos) = find_first_difference(&rust_bytes, &cpp_bytes) {
            &format!("✗ DIFF at byte {}", diff_pos)
        } else {
            "✗ SIZE DIFF"
        };

        print_size_row(
            &format!("speed {speed:2}"),
            cpp_bytes.len(),
            rust_bytes.len(),
            status,
        );
    }
}
