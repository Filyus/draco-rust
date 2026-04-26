use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use std::time::{Duration, Instant};

mod common;

fn create_test_mesh_data(grid_size: usize) -> (Vec<f32>, Vec<u32>) {
    let num_points = grid_size * grid_size;
    let num_faces = (grid_size - 1) * (grid_size - 1) * 2;

    let mut positions = Vec::with_capacity(num_points * 3);
    for y in 0..grid_size {
        for x in 0..grid_size {
            positions.push(x as f32);
            positions.push(y as f32);
            positions.push((x as f32 * 0.2).sin() * (y as f32 * 0.2).cos() * 2.0);
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

const BATCHES: usize = 9;

fn decode_iterations(grid_size: usize, speed: i32) -> u32 {
    match (grid_size, speed) {
        (20, 10) => 2_000,
        (50, 10) => 800,
        (100, 10) => 300,
        (20, _) => 40,
        (50, _) => 10,
        (100, _) => 3,
        _ => 10,
    }
}

fn warmup_iterations(iterations: u32) -> u32 {
    iterations.clamp(10, 50)
}

fn median_rust_decode_ns(encoded_data: &[u8], iterations: u32) -> Option<(u128, usize, usize)> {
    if iterations == 0 {
        return None;
    }

    let mut num_points = 0;
    let mut num_faces = 0;

    for _ in 0..warmup_iterations(iterations) {
        let mut decoder_buffer = DecoderBuffer::new(encoded_data);
        let mut out_mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();
        decoder.decode(&mut decoder_buffer, &mut out_mesh).ok()?;
        num_points = out_mesh.num_points();
        num_faces = out_mesh.num_faces();
    }

    let mut batch_ns = Vec::with_capacity(BATCHES);
    for _ in 0..BATCHES {
        let mut elapsed = Duration::ZERO;

        for _ in 0..iterations {
            let mut decoder_buffer = DecoderBuffer::new(encoded_data);
            let mut out_mesh = Mesh::new();
            let mut decoder = MeshDecoder::new();

            let start = Instant::now();
            decoder.decode(&mut decoder_buffer, &mut out_mesh).ok()?;
            elapsed += start.elapsed();

            num_points = out_mesh.num_points();
            num_faces = out_mesh.num_faces();
        }

        batch_ns.push(elapsed.as_nanos() / u128::from(iterations));
    }

    batch_ns.sort_unstable();
    Some((batch_ns[BATCHES / 2], num_points, num_faces))
}

fn ns_to_us(ns: u128) -> f64 {
    ns as f64 / 1_000.0
}

fn signed_delta_us(rust_ns: u128, cpp_ns: u128) -> f64 {
    (cpp_ns as f64 - rust_ns as f64) / 1_000.0
}

fn signed_delta_pct(rust_ns: u128, cpp_ns: u128) -> f64 {
    ((cpp_ns as f64 / rust_ns as f64) - 1.0) * 100.0
}

fn winner(rust_ns: u128, cpp_ns: u128) -> &'static str {
    if rust_ns <= cpp_ns {
        "Rust"
    } else {
        "C++"
    }
}

#[test]
fn bench_decode_cpp_vs_rust() {
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }

    println!("\nComparing C++ vs Rust decode performance on C++-encoded grids");
    println!(
        "{:>7} {:>5} {:>7} {:>10} {:>7} {:>8} {:>10} {:>10} {:>10} {:>9} {:>9} {:>7}",
        "Grid",
        "Speed",
        "Iters",
        "Bytes",
        "Points",
        "Faces",
        "C++ µs",
        "Rust µs",
        "Δ µs",
        "Δ %",
        "Speedup",
        "Winner"
    );
    println!("{}", "-".repeat(119));

    for grid_size in [20, 50, 100] {
        let (positions, faces) = create_test_mesh_data(grid_size);
        let num_faces = (grid_size - 1) * (grid_size - 1) * 2;
        let mut grid_rust_ns = 0u128;
        let mut grid_cpp_ns = 0u128;

        for speed in [0, 1, 5, 10] {
            let iterations = decode_iterations(grid_size, speed);
            let encoded_data =
                draco_cpp_test_bridge::encode_cpp_mesh(&positions, &faces, speed, speed, 10)
                    .expect("C++ encode failed");

            let (rust_ns, rust_points, rust_faces) =
                median_rust_decode_ns(&encoded_data, iterations).expect("Rust decode failed");

            let (cpp_ns_raw, cpp_points, cpp_faces) =
                draco_cpp_test_bridge::benchmark_cpp_decode(&encoded_data, iterations as u32)
                    .expect("C++ benchmark decode failed");
            assert_eq!(rust_points as u32, cpp_points);
            assert_eq!(rust_faces as u32, cpp_faces);

            let cpp_ns = cpp_ns_raw as u128;
            let speedup = cpp_ns as f64 / rust_ns as f64;
            grid_rust_ns += rust_ns;
            grid_cpp_ns += cpp_ns;

            println!(
                "{:>7} {:>5} {:>7} {:>10} {:>7} {:>8} {:>10.1} {:>10.1} {:>+10.1} {:>+8.1}% {:>8.2}x {:>7}",
                format!("{grid_size}x{grid_size}"),
                speed,
                iterations,
                encoded_data.len(),
                rust_points,
                num_faces,
                ns_to_us(cpp_ns),
                ns_to_us(rust_ns),
                signed_delta_us(rust_ns, cpp_ns),
                signed_delta_pct(rust_ns, cpp_ns),
                speedup,
                winner(rust_ns, cpp_ns)
            );
        }

        let grid_speedup = grid_cpp_ns as f64 / grid_rust_ns as f64;
        println!(
            "{:>7} {:>5} {:>7} {:>10} {:>7} {:>8} {:>10.1} {:>10.1} {:>+10.1} {:>+8.1}% {:>8.2}x {:>7}",
            format!("{grid_size}x{grid_size}"),
            "all",
            "-",
            "-",
            grid_size * grid_size,
            num_faces,
            ns_to_us(grid_cpp_ns),
            ns_to_us(grid_rust_ns),
            signed_delta_us(grid_rust_ns, grid_cpp_ns),
            signed_delta_pct(grid_rust_ns, grid_cpp_ns),
            grid_speedup,
            winner(grid_rust_ns, grid_cpp_ns)
        );
        println!("{}", "-".repeat(119));
    }

    println!("Notes:");
    println!("  Δ = C++ time - Rust time; positive values mean Rust was faster.");
    println!("  Speedup = C++ / Rust; values above 1.0 mean Rust was faster.");
    println!("  Times are computed in nanoseconds and printed as median per-iteration µs over 9 batches.");
}
