//! Per-site heap totals for one Rust decode, via `dhat`.
//!
//! The counting allocator answers "how many bytes total"; this answers
//! "which line allocated them". One payload, one speed, one decode inside
//! the profiled region -- encode and OBJ loading happen before the profiler
//! starts, so `dhat-heap.json` holds the decode alone. View the file in
//! dhat's viewer, or reduce it with the summary script the round writeup
//! names.
//!
//! ```text
//! cargo run --release --example decode_dhat -- seeded_ribbon_0.obj 5
//! ```
//!
//! A dhat build records a backtrace per allocation: never time this binary.
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;

#[path = "common/mod.rs"]
mod common;
use common::{load, options_for};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("mesh path");
    let speed: i32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .expect("encode speed");

    let payload = load(&path);
    let options = options_for(&payload.mesh, speed);
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(payload.mesh.clone());
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("rust encode failed");
    let encoded = buffer.data().to_vec();

    let _profiler = dhat::Profiler::new_heap();
    let mut in_buffer = DecoderBuffer::new(&encoded);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder
        .decode(&mut in_buffer, &mut mesh)
        .expect("rust decode failed");
    eprintln!(
        "decoded {} points / {} faces at speed {speed}; totals in dhat-heap.json",
        mesh.num_points(),
        mesh.num_faces()
    );
}
