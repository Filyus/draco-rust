//! One encoder, one payload, one loop -- and a count of what it allocates.
//!
//! The encode-side sibling of `decode_loop`: exactly one side per process, so
//! a profiler or the counting allocator attributes to it alone. The Rust side
//! reports allocations and bytes per encode; `SAMPLE_ALLOC=1` prints a
//! backtrace for every allocation of 64 KB or more from a single encode. The
//! C++ side goes through `profile_cpp_encode`, which is position-only -- pass
//! a position-only mesh when comparing sides.
//!
//! `REUSE_ENCODER=1` keeps one `MeshEncoder` across the loop instead of
//! building one per iteration, which is the difference between a converter
//! walking a glTF's primitives and a caller encoding a single mesh. On a small
//! mesh most of the encode is per-call cost, so which of the two is being
//! measured decides the number.
//!
//! ```text
//! cargo run --release --example encode_loop -- <mesh.obj> cpp|rust <speed> <iters>
//! SAMPLE_ALLOC=1 cargo run --release --example encode_loop -- <mesh.obj> rust 5 1
//! REUSE_ENCODER=1 cargo run --release --example encode_loop -- <mesh.obj> rust 5 20000
//! ```
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;

use draco_cpp_test_bridge::counting;

#[global_allocator]
static ALLOC: counting::Counting<std::alloc::System> = counting::Counting(std::alloc::System);

/// One encode, with the setup the C++ side also keeps outside its timed region
/// charged separately.
///
/// `set_mesh` takes the mesh by value, so a loop has to hand it a fresh one
/// each iteration -- but `draco_profile_encode` builds its `draco::Mesh` under
/// a *separate* timer and times `EncodeMeshToBuffer` alone. Timing the clone
/// with the encode would compare a Rust encode plus a 1.2 MB copy against a
/// C++ encode without one. Returns `(bytes, setup_us, encode_us)`.
///
/// The `encoder` is passed in rather than made here so the caller decides
/// whether each iteration gets a fresh one. Both are real callers: a converter
/// walking a glTF's primitives can keep one encoder across all of them, and
/// what that is worth is a property of `reset_derived_state` -- which `clear`s
/// some buffers and drops others -- rather than something the API documents.
fn rust_encode(
    encoder: &mut MeshEncoder,
    mesh: &draco_core::mesh::Mesh,
    options: &EncoderOptions,
) -> (Vec<u8>, f64, f64) {
    let setup_start = std::time::Instant::now();
    encoder.set_mesh(mesh.clone());
    let mut buffer = EncoderBuffer::new();
    let setup_us = setup_start.elapsed().as_secs_f64() * 1e6;

    let encode_start = std::time::Instant::now();
    encoder
        .encode(options, &mut buffer)
        .expect("rust encode failed");
    let encode_us = encode_start.elapsed().as_secs_f64() * 1e6;

    (buffer.data().to_vec(), setup_us, encode_us)
}

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
    eprintln!(
        "payload: {} faces, side={side}, speed={speed}, iters={iters}",
        mesh.num_faces()
    );

    match side.as_str() {
        "cpp" => {
            let positions: Vec<f32> = mesh.attribute(0).read_f32s(mesh.num_points(), 3);
            let faces: Vec<u32> = (0..mesh.num_faces())
                .flat_map(|f| {
                    let face = mesh.face(draco_core::geometry_indices::FaceIndex(f as u32));
                    [face[0].0, face[1].0, face[2].0]
                })
                .collect();
            let result = draco_cpp_test_bridge::profile_cpp_encode(
                &positions, &faces, speed, speed, 11, iters,
            )
            .expect("C++ encode failed");
            eprintln!(
                "cpp: {} bytes, {} us/encode (encode only), {} us total",
                result.output_size, result.encode_time_us, result.total_time_us
            );
        }
        _ => {
            if std::env::var("SAMPLE_ALLOC").is_ok() {
                use std::sync::atomic::Ordering::Relaxed;
                counting::reset();
                counting::SAMPLING.store(true, Relaxed);
                let (encoded, _, _) = rust_encode(&mut MeshEncoder::new(), &mesh, &options);
                counting::SAMPLING.store(false, Relaxed);
                let samples = counting::SAMPLES.lock().unwrap().clone();
                eprintln!(
                    "SAMPLED {} stacks of {} allocations, {} bytes encoded",
                    samples.len(),
                    counting::COUNT.load(Relaxed),
                    encoded.len()
                );
                for sample in samples {
                    println!("=== {sample}");
                }
                return;
            }
            use std::sync::atomic::Ordering::Relaxed;
            // `REUSE_ENCODER=1` keeps one encoder across the loop, which is the
            // converter walking a glTF's primitives; the default builds a fresh
            // one per iteration, which is the caller encoding one mesh. The two
            // differ only in what `reset_derived_state` manages to retain.
            let reuse = std::env::var("REUSE_ENCODER").is_ok();
            let mut kept = reuse.then(MeshEncoder::new);
            counting::reset();
            let mut size = 0;
            let mut total_setup = 0.0;
            let mut total_encode = 0.0;
            for _ in 0..iters {
                let mut fresh;
                let encoder = match kept.as_mut() {
                    Some(kept) => kept,
                    None => {
                        fresh = MeshEncoder::new();
                        &mut fresh
                    }
                };
                let (bytes, setup, encode) = rust_encode(encoder, &mesh, &options);
                size = bytes.len();
                total_setup += setup;
                total_encode += encode;
            }
            let us = total_encode / f64::from(iters);
            let setup_us = total_setup / f64::from(iters);
            let n = counting::COUNT.swap(0, Relaxed) as f64 / f64::from(iters);
            let b = counting::BYTES.swap(0, Relaxed) as f64 / f64::from(iters);
            let l = counting::LARGE.swap(0, Relaxed) as f64 / f64::from(iters);
            eprintln!(
                "rust: {size} bytes, {us:.1} us/encode (encode only), {setup_us:.1} us setup"
            );
            eprintln!(
                "alloc: {n:.0} allocations/encode, {:.2} MB/encode, {l:.0} of them >= 64 KB",
                b / (1024.0 * 1024.0)
            );
        }
    }
}
