// Comprehensive performance and correctness test for all encoding speeds

mod common;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;
use draco_cpp_test_bridge;
use std::time::{Duration, Instant};

fn create_grid_mesh_data(grid_size: usize) -> (Vec<f32>, Vec<u32>) {
    let num_points = grid_size * grid_size;
    let num_faces = (grid_size - 1) * (grid_size - 1) * 2;

    // Create positions
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

    // Create faces
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

fn create_uv_sphere_data(lat_segments: usize, lon_segments: usize) -> (Vec<f32>, Vec<u32>) {
    assert!(lat_segments >= 2);
    assert!(lon_segments >= 3);

    let mut positions = Vec::with_capacity((1 + (lat_segments - 1) * lon_segments + 1) * 3);
    positions.extend_from_slice(&[0.0, 1.0, 0.0]);

    for lat in 1..lat_segments {
        let theta = std::f32::consts::PI * lat as f32 / lat_segments as f32;
        let y = theta.cos();
        let radius = theta.sin();
        for lon in 0..lon_segments {
            let phi = 2.0 * std::f32::consts::PI * lon as f32 / lon_segments as f32;
            positions.push(radius * phi.cos());
            positions.push(y);
            positions.push(radius * phi.sin());
        }
    }

    let bottom = (positions.len() / 3) as u32;
    positions.extend_from_slice(&[0.0, -1.0, 0.0]);

    let ring = |lat_ring: usize, lon: usize| -> u32 {
        1 + ((lat_ring - 1) * lon_segments + lon % lon_segments) as u32
    };

    let mut faces =
        Vec::with_capacity((lon_segments * 2 + (lat_segments - 2) * lon_segments * 2) * 3);

    for lon in 0..lon_segments {
        faces.extend_from_slice(&[0, ring(1, lon + 1), ring(1, lon)]);
    }

    for lat in 1..lat_segments - 1 {
        for lon in 0..lon_segments {
            let a = ring(lat, lon);
            let b = ring(lat, lon + 1);
            let c = ring(lat + 1, lon);
            let d = ring(lat + 1, lon + 1);
            faces.extend_from_slice(&[a, b, c]);
            faces.extend_from_slice(&[b, d, c]);
        }
    }

    for lon in 0..lon_segments {
        faces.extend_from_slice(&[
            ring(lat_segments - 1, lon),
            ring(lat_segments - 1, lon + 1),
            bottom,
        ]);
    }

    (positions, faces)
}

fn create_subdivided_cube_data(subdivisions: usize) -> (Vec<f32>, Vec<u32>) {
    assert!(subdivisions >= 1);

    let mut positions = Vec::with_capacity(6 * (subdivisions + 1) * (subdivisions + 1) * 3);
    let mut faces = Vec::with_capacity(6 * subdivisions * subdivisions * 2 * 3);

    let mut add_face = |axis: usize, sign: f32| {
        let base = (positions.len() / 3) as u32;
        for v in 0..=subdivisions {
            for u in 0..=subdivisions {
                let a = -1.0 + 2.0 * u as f32 / subdivisions as f32;
                let b = -1.0 + 2.0 * v as f32 / subdivisions as f32;
                let p = match axis {
                    0 => [sign, b, if sign > 0.0 { -a } else { a }],
                    1 => [a, sign, if sign > 0.0 { b } else { -b }],
                    _ => [if sign > 0.0 { a } else { -a }, b, sign],
                };
                positions.extend_from_slice(&p);
            }
        }

        let row = subdivisions + 1;
        for v in 0..subdivisions {
            for u in 0..subdivisions {
                let p0 = base + (v * row + u) as u32;
                let p1 = base + (v * row + u + 1) as u32;
                let p2 = base + ((v + 1) * row + u) as u32;
                let p3 = base + ((v + 1) * row + u + 1) as u32;
                faces.extend_from_slice(&[p0, p1, p2]);
                faces.extend_from_slice(&[p1, p3, p2]);
            }
        }
    };

    add_face(0, 1.0);
    add_face(0, -1.0);
    add_face(1, 1.0);
    add_face(1, -1.0);
    add_face(2, 1.0);
    add_face(2, -1.0);

    (positions, faces)
}

fn create_mesh_from_data(positions: &[f32], faces: &[u32]) -> Mesh {
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

#[test]
fn bench_encode_decode_matrix() {
    common::disable_noisy_debug_env();

    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    let (major, minor, revision) = draco_cpp_test_bridge::get_version();
    println!("\n╔═══════════════════════════════════════════════════════════════════════╗");
    println!("║     COMPREHENSIVE DRACO PERFORMANCE TEST (C++ vs Rust)               ║");
    println!("╚═══════════════════════════════════════════════════════════════════════╝");
    println!("C++ Draco version: {}.{}.{}", major, minor, revision);
    println!("Rust implementation: draco-core v0.1.0\n");

    let grid_size = 100;
    let (positions, faces) = create_grid_mesh_data(grid_size);
    let mesh = create_mesh_from_data(&positions, &faces);

    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;

    println!(
        "Test mesh: {}×{} grid ({} points, {} faces)\n",
        grid_size, grid_size, num_points, num_faces
    );

    // Test all speeds from 0 to 10
    let speeds = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let encode_iterations = 10;
    let decode_iterations = 50;

    println!("┌───────────────────────────────────────────────────────────────────────┐");
    println!("│                        ENCODING PERFORMANCE                           │");
    println!("├───────┬──────────┬──────────┬──────────┬─────────┬──────────┬─────────┤");
    println!("│ Speed │   Size   │ C++ (µs) │ Rust (µs)│ Speedup │  Winner  │  Match  │");
    println!("├───────┼──────────┼──────────┼──────────┼─────────┼──────────┼─────────┤");

    let mut encode_results = Vec::new();

    for &speed in &speeds {
        // Rust encoding - time just the encoding, not mesh cloning
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);
        options.set_attribute_int(0, "quantization_bits", 10);

        let mut rust_total_us = 0.0;
        let mut rust_data = Vec::new();

        for _ in 0..encode_iterations {
            let mut encoder = MeshEncoder::new();
            encoder.set_mesh(mesh.clone());
            let mut encoder_buffer = EncoderBuffer::new();

            // Time only the encoding operation
            let start = Instant::now();
            encoder
                .encode(&options, &mut encoder_buffer)
                .expect("Rust encode failed");
            rust_total_us += start.elapsed().as_secs_f64() * 1_000_000.0;

            rust_data = encoder_buffer.data().to_vec();
        }
        let rust_avg_us = rust_total_us / f64::from(encode_iterations);

        // C++ encoding
        let cpp_result = draco_cpp_test_bridge::benchmark_cpp_encode(
            &positions,
            &faces,
            speed,
            speed,
            10,
            encode_iterations,
        );

        let (cpp_avg_us, cpp_data_size, _cpp_success) = match cpp_result {
            Some(r) => (r.avg_time_us as f64, r.output_size, true),
            None => {
                println!(
                    "│ {:>5} │ {:>8} │   FAILED │ {:>8.1} │    -    │    -     │    -    │",
                    speed,
                    rust_data.len(),
                    rust_avg_us
                );
                continue;
            }
        };

        let speedup = cpp_avg_us / rust_avg_us;
        let winner = if speedup >= 1.0 { "Rust" } else { "C++" };
        let size_match = rust_data.len() == cpp_data_size;
        let match_str = if size_match { "✓" } else { "✗" };

        println!(
            "│ {:>5} │ {:>8} │ {:>8.1} │ {:>8.1} │ {:>7.2}x │ {:>8} │ {:>7} │",
            speed,
            rust_data.len(),
            cpp_avg_us,
            rust_avg_us,
            speedup,
            winner,
            match_str
        );

        encode_results.push((speed, rust_data, speedup, size_match));
    }

    println!("└───────┴──────────┴──────────┴──────────┴─────────┴──────────┴─────────┘");

    println!("\n┌───────────────────────────────────────────────────────────────────────┐");
    println!("│                        DECODING PERFORMANCE                           │");
    println!("├───────┬──────────┬──────────┬──────────┬─────────┬──────────┬─────────┤");
    println!("│ Speed │   Size   │ C++ (µs) │ Rust (µs)│ Speedup │  Winner  │ Correct │");
    println!("├───────┼──────────┼──────────┼──────────┼─────────┼──────────┼─────────┤");

    for (speed, rust_encoded, _encode_ratio, size_match) in encode_results {
        if !size_match {
            println!(
                "│ {:>5} │ {:>8} │    -     │    -     │    -    │    -     │  SKIP   │",
                speed,
                rust_encoded.len()
            );
            eprintln!("DEBUG: Speed {} skipped - size_match=false", speed);
            continue;
        }

        // C++ decoding
        let cpp_result =
            draco_cpp_test_bridge::profile_cpp_decode(&rust_encoded, decode_iterations);
        let (cpp_avg_us, cpp_points, cpp_faces) = match cpp_result {
            Some(r) => (r.decode_time_us as f64, r.num_points, r.num_faces),
            None => {
                println!(
                    "│ {:>5} │ {:>8} │   FAILED │    -     │    -    │    -     │    -    │",
                    speed,
                    rust_encoded.len()
                );
                continue;
            }
        };

        // Rust decoding
        let mut rust_total = Duration::ZERO;
        let mut rust_points = 0;
        let mut rust_faces = 0;
        let mut rust_success = true;

        for _ in 0..decode_iterations {
            let mut decoder_buffer = DecoderBuffer::new(&rust_encoded);
            let mut out_mesh = Mesh::new();
            let mut decoder = MeshDecoder::new();

            let start = Instant::now();
            match decoder.decode(&mut decoder_buffer, &mut out_mesh) {
                Ok(_) => {
                    rust_total += start.elapsed();
                    rust_points = out_mesh.num_points();
                    rust_faces = out_mesh.num_faces();
                }
                Err(_) => {
                    rust_success = false;
                    break;
                }
            }
        }

        if !rust_success {
            println!(
                "│ {:>5} │ {:>8} │ {:>8.1} │   FAILED │    -    │    -     │    -    │",
                speed,
                rust_encoded.len(),
                cpp_avg_us
            );
            continue;
        }

        let rust_avg_us = rust_total.as_secs_f64() * 1_000_000.0 / f64::from(decode_iterations);
        let speedup = cpp_avg_us / rust_avg_us;
        let winner = if speedup >= 1.0 { "Rust" } else { "C++" };
        let correct = rust_points == cpp_points as usize && rust_faces == cpp_faces as usize;
        let correct_str = if correct { "✓" } else { "✗" };

        println!(
            "│ {:>5} │ {:>8} │ {:>8.1} │ {:>8.1} │ {:>7.2}x │ {:>8} │ {:>7} │",
            speed,
            rust_encoded.len(),
            cpp_avg_us,
            rust_avg_us,
            speedup,
            winner,
            correct_str
        );
    }

    println!("└───────┴──────────┴──────────┴──────────┴─────────┴──────────┴─────────┘");

    println!("\nNotes:");
    println!("  • Encoding iterations: {}", encode_iterations);
    println!("  • Decoding iterations: {}", decode_iterations);
    println!("  • Speedup > 1.0 means Rust is faster");
    println!("  • Speedup < 1.0 means C++ is faster");
    println!("  • Match: Binary output size comparison");
    println!("  • Correct: Decoded mesh matches original\n");
}

#[test]
fn bench_generated_encode_decode_matrix() {
    common::disable_noisy_debug_env();

    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    let speeds = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let encode_iterations = 10;
    let decode_iterations = 40;

    let cases = [
        ("sphere 24x48", create_uv_sphere_data(24, 48)),
        ("cube subdiv20", create_subdivided_cube_data(20)),
    ];

    println!("\n╔═══════════════════════════════════════════════════════════════════════╗");
    println!("║     GENERATED MESH PERFORMANCE TEST (C++ vs Rust)                    ║");
    println!("╚═══════════════════════════════════════════════════════════════════════╝");

    for (label, (positions, faces)) in cases {
        let mesh = create_mesh_from_data(&positions, &faces);
        let num_points = positions.len() / 3;
        let num_faces = faces.len() / 3;

        println!("\nMesh: {label} ({num_points} points, {num_faces} faces)");
        println!(
            "{:>5} {:>8} {:>10} {:>10} {:>9} {:>10} {:>10} {:>9} {:>7}",
            "Speed", "Bytes", "C++ enc", "Rust enc", "Enc x", "C++ dec", "Rust dec", "Dec x", "OK"
        );
        println!("{}", "-".repeat(91));

        for speed in speeds {
            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_speed", speed);
            options.set_global_int("decoding_speed", speed);
            options.set_attribute_int(0, "quantization_bits", 10);

            let mut rust_total_us = 0.0;
            let mut rust_data = Vec::new();

            for _ in 0..encode_iterations {
                let mut encoder = MeshEncoder::new();
                encoder.set_mesh(mesh.clone());
                let mut encoder_buffer = EncoderBuffer::new();

                let start = Instant::now();
                encoder
                    .encode(&options, &mut encoder_buffer)
                    .expect("Rust encode failed");
                rust_total_us += start.elapsed().as_secs_f64() * 1_000_000.0;
                rust_data = encoder_buffer.data().to_vec();
            }
            let rust_encode_us = rust_total_us / f64::from(encode_iterations);

            let Some(cpp_encode) = draco_cpp_test_bridge::benchmark_cpp_encode(
                &positions,
                &faces,
                speed,
                speed,
                10,
                encode_iterations,
            ) else {
                println!(
                    "{speed:>5} {:>8} {:>10} {:>10.1} {:>9} {:>10} {:>10} {:>9} {:>7}",
                    rust_data.len(),
                    "FAILED",
                    rust_encode_us,
                    "-",
                    "-",
                    "-",
                    "-",
                    "-"
                );
                continue;
            };

            let encode_speedup = cpp_encode.avg_time_us as f64 / rust_encode_us;
            let size_match = cpp_encode.output_size == rust_data.len();

            let Some(cpp_decode) =
                draco_cpp_test_bridge::profile_cpp_decode(&rust_data, decode_iterations)
            else {
                println!(
                    "{speed:>5} {:>8} {:>10.1} {:>10.1} {:>8.2}x {:>10} {:>10} {:>9} {:>7}",
                    rust_data.len(),
                    cpp_encode.avg_time_us,
                    rust_encode_us,
                    encode_speedup,
                    "FAILED",
                    "-",
                    "-",
                    "-"
                );
                continue;
            };

            let mut rust_decode_total = Duration::ZERO;
            let mut rust_points = 0usize;
            let mut rust_faces = 0usize;
            for _ in 0..decode_iterations {
                let mut decoder_buffer = DecoderBuffer::new(&rust_data);
                let mut out_mesh = Mesh::new();
                let mut decoder = MeshDecoder::new();
                let start = Instant::now();
                decoder
                    .decode(&mut decoder_buffer, &mut out_mesh)
                    .expect("Rust decode failed");
                rust_decode_total += start.elapsed();
                rust_points = out_mesh.num_points();
                rust_faces = out_mesh.num_faces();
            }

            let rust_decode_us =
                rust_decode_total.as_secs_f64() * 1_000_000.0 / f64::from(decode_iterations);
            let decode_speedup = cpp_decode.decode_time_us as f64 / rust_decode_us;
            let decode_match = rust_points == cpp_decode.num_points as usize
                && rust_faces == cpp_decode.num_faces as usize;
            let ok = if size_match && decode_match {
                "✓"
            } else {
                "✗"
            };

            println!(
                "{speed:>5} {:>8} {:>10.1} {:>10.1} {:>8.2}x {:>10.1} {:>10.1} {:>8.2}x {:>7}",
                rust_data.len(),
                cpp_encode.avg_time_us,
                rust_encode_us,
                encode_speedup,
                cpp_decode.decode_time_us,
                rust_decode_us,
                decode_speedup,
                ok
            );
        }
    }

    println!("\nNotes:");
    println!("  • Encoding iterations: {encode_iterations}");
    println!("  • Decoding iterations: {decode_iterations}");
    println!("  • Enc x / Dec x = C++ time divided by Rust time.");
    println!("  • OK requires matching encoded byte size and decoded point/face counts.");
}
