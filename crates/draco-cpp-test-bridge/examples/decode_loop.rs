//! One decoder, one payload, one loop -- and a count of what it allocates.
//!
//! `bench_decode_cpp_vs_rust` times both sides in one process, which is right
//! for wall clock and wrong for anything a profiler attributes per process: a
//! sampler or a counting allocator cannot tell the two apart there. This runs
//! exactly one side, so a profile or a counter belongs to it alone.
//!
//! The Rust side also reports allocations and bytes per decode, and
//! `SAMPLE_ALLOC=1` prints a backtrace for every allocation of 64 KB or more
//! from a single decode. That is how the corner table's growth was found: ten
//! reallocations totalling 3.9 MB to reach a 1.7 MB table.
//!
//! Point the C++ side at a reference build with
//! `DRACO_CPP_BUILD_DIR`/`DRACO_CPP_SOURCE_DIR`, and rebuild whenever it
//! changes -- the linked library is whatever the last build used, which is
//! easy to get wrong when another `cargo` command relinks it in between.
//!
//! ```text
//! cargo run --release --example decode_loop -- <mesh.obj> cpp|rust <speed> <iters>
//! SAMPLE_ALLOC=1 cargo run --release --example decode_loop -- <mesh.obj> rust 5 1
//! ```
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;

use draco_cpp_test_bridge::counting;

#[global_allocator]
static ALLOC: counting::Counting = counting::Counting;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("mesh path");
    let side = args.next().unwrap_or_else(|| "rust".to_string());
    let speed: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    let iters: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);

    let bytes = std::fs::read(&path).expect("read mesh");
    let mesh = draco_io::obj_reader::ObjReader::read_from_bytes(&bytes).expect("parse obj");
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", speed);
    options.set_global_int("decoding_speed", speed);
    options.set_attribute_int(0, "quantization_bits", 11);
    if mesh.num_attributes() > 1 {
        options.set_attribute_int(1, "quantization_bits", 8);
    }
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encode");
    let encoded = buffer.data().to_vec();
    eprintln!(
        "payload: {} bytes, {} faces, side={side}, speed={speed}, iters={iters}",
        encoded.len(),
        mesh.num_faces()
    );

    match side.as_str() {
        "cpp" => {
            let result = draco_cpp_test_bridge::profile_cpp_decode(&encoded, iters)
                .expect("C++ decode failed");
            eprintln!(
                "cpp: {} points, {} faces, {} us/decode",
                result.num_points, result.num_faces, result.decode_time_us
            );
        }
        _ => {
            if std::env::var("SAMPLE_ALLOC").is_ok() {
                use std::sync::atomic::Ordering::Relaxed;
                counting::COUNT.store(0, Relaxed);
                counting::SAMPLING.store(true, Relaxed);
                let mut b = DecoderBuffer::new(&encoded);
                let mut m = Mesh::new();
                let mut d = MeshDecoder::new();
                d.decode(&mut b, &mut m).expect("rust decode failed");
                counting::SAMPLING.store(false, Relaxed);
                let samples = counting::SAMPLES.lock().unwrap().clone();
                eprintln!(
                    "SAMPLED {} stacks of {} allocations",
                    samples.len(),
                    counting::COUNT.load(Relaxed)
                );
                for sample in samples {
                    println!("=== {sample}");
                }
                return;
            }
            counting::reset();
            let start = std::time::Instant::now();
            let mut points = 0;
            let mut faces = 0;
            for _ in 0..iters {
                let mut b = DecoderBuffer::new(&encoded);
                let mut m = Mesh::new();
                let mut d = MeshDecoder::new();
                d.decode(&mut b, &mut m).expect("rust decode failed");
                points = m.num_points();
                faces = m.num_faces();
            }
            let us = start.elapsed().as_secs_f64() * 1e6 / f64::from(iters);
            use std::sync::atomic::Ordering::Relaxed;
            let n = counting::COUNT.swap(0, Relaxed) as f64 / f64::from(iters);
            let b = counting::BYTES.swap(0, Relaxed) as f64 / f64::from(iters);
            let l = counting::LARGE.swap(0, Relaxed) as f64 / f64::from(iters);
            eprintln!("rust: {points} points, {faces} faces, {us:.1} us/decode");
            eprintln!(
                "alloc: {n:.0} allocations/decode, {:.2} MB/decode, {l:.0} of them >= 64 KB",
                b / (1024.0 * 1024.0)
            );
        }
    }
}
