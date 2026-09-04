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
//! `REUSE_DECODE=1` decodes into one `Mesh` through one `MeshDecoder` for the
//! whole loop instead of building both per iteration -- the caller decoding
//! many files against the caller decoding one. The allocation counts are what
//! say whether the second decode skips anything.
//!
//! ```text
//! cargo run --release --example decode_loop -- <mesh.obj> cpp|rust <speed> <iters>
//! SAMPLE_ALLOC=1 cargo run --release --example decode_loop -- <mesh.obj> rust 5 1
//! REUSE_DECODE=1 cargo run --release --example decode_loop -- <mesh.obj> rust 5 2000
//! ```
//!
//! `DROP_SPARE=1` alongside `REUSE_DECODE=1` clears the mesh and releases the
//! storage it retained from the previous decode before every decode -- what a
//! `clear` before this retention existed did -- so the same binary measures
//! reuse with and without it.
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;

use draco_cpp_test_bridge::counting;

#[global_allocator]
static ALLOC: counting::Counting<std::alloc::System> = counting::Counting(std::alloc::System);

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("mesh path");
    let side = args.next().unwrap_or_else(|| "rust".to_string());
    let speed: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    let iters: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);

    // A comma-separated path list is decoded round-robin, which is what a
    // caller decoding many files does. It matters to `REUSE_DECODE`: one
    // payload repeated hands the reused `Mesh` buffers that are already
    // exactly the right size, and that is the best case rather than the case.
    let streams: Vec<(Vec<u8>, usize)> = path
        .split(',')
        .map(|p| {
            let bytes = std::fs::read(p).unwrap_or_else(|e| panic!("read mesh {p}: {e}"));
            let mesh = draco_io::obj_reader::ObjReader::read_from_bytes(&bytes).expect("parse obj");
            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_speed", speed);
            options.set_global_int("decoding_speed", speed);
            options.set_attribute_int(0, "quantization_bits", 11);
            if mesh.num_attributes() > 1 {
                options.set_attribute_int(1, "quantization_bits", 8);
            }
            let mut encoder = MeshEncoder::new();
            let faces = mesh.num_faces();
            encoder.set_mesh(mesh);
            let mut buffer = EncoderBuffer::new();
            encoder.encode(&options, &mut buffer).expect("encode");
            (buffer.data().to_vec(), faces)
        })
        .collect();
    let encoded = streams[0].0.clone();
    eprintln!(
        "payload: {}, side={side}, speed={speed}, iters={iters}",
        streams
            .iter()
            .map(|(s, faces)| format!("{} bytes / {faces} faces", s.len()))
            .collect::<Vec<_>>()
            .join(", ")
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
            // `REUSE_DECODE=1` decodes into the same `Mesh` through the same
            // `MeshDecoder` every iteration, which is the caller decoding many
            // files in a row; the default builds both fresh, which is the
            // caller decoding one. `decode` takes `&mut Mesh`, so the reuse the
            // question is about is already expressible -- what it is worth is
            // the measurement, and the counts below are what answers it.
            let reuse = std::env::var("REUSE_DECODE").is_ok();
            let drop_spare = std::env::var("DROP_SPARE").is_ok();
            let mut kept = reuse.then(|| (Mesh::new(), MeshDecoder::new()));
            counting::reset();
            let start = std::time::Instant::now();
            let mut points = 0;
            let mut faces = 0;
            let mut attributes = 0;
            // One point count per stream, so a reused mesh that appended
            // instead of replacing is caught even when the streams differ.
            let mut expected = vec![0usize; streams.len()];
            for i in 0..iters as usize {
                let which = i % streams.len();
                let mut b = DecoderBuffer::new(&streams[which].0);
                let mut fresh;
                let (m, d) = match kept.as_mut() {
                    Some(kept) => (&mut kept.0, &mut kept.1),
                    None => {
                        fresh = (Mesh::new(), MeshDecoder::new());
                        (&mut fresh.0, &mut fresh.1)
                    }
                };
                if drop_spare {
                    m.clear();
                    m.release_spare_storage();
                }
                d.decode(&mut b, m).expect("rust decode failed");
                assert!(
                    expected[which] == 0 || expected[which] == m.num_points(),
                    "decode {} produced {} points for stream {which} after {}",
                    if reuse { "reusing" } else { "fresh" },
                    m.num_points(),
                    expected[which]
                );
                expected[which] = m.num_points();
                points = m.num_points();
                attributes = m.num_attributes();
                faces = m.num_faces();
            }
            let us = start.elapsed().as_secs_f64() * 1e6 / f64::from(iters);
            use std::sync::atomic::Ordering::Relaxed;
            let n = counting::COUNT.swap(0, Relaxed) as f64 / f64::from(iters);
            let b = counting::BYTES.swap(0, Relaxed) as f64 / f64::from(iters);
            let l = counting::LARGE.swap(0, Relaxed) as f64 / f64::from(iters);
            eprintln!(
                "rust: {points} points, {faces} faces, {attributes} attributes, {us:.3} us/decode"
            );
            eprintln!(
                "alloc: {n:.0} allocations/decode, {:.2} MB/decode, {l:.0} of them >= 64 KB",
                b / (1024.0 * 1024.0)
            );
        }
    }
}
