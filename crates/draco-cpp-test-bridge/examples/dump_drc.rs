//! Writes the exact bytes the matrices decode to a `.drc` file.
//!
//! The seeded corpus lives as `.obj` and is encoded in-process by every
//! harness here, so nothing on disk holds what the decoders actually read.
//! A cross-platform comparison needs that: the callgrind runs under WSL
//! decode a file, and it has to be the same file both sides see, produced by
//! the same Rust encoder at the same speed as the in-process matrices.
//!
//! ```text
//! cargo run --release --example dump_drc -- seeded_grid_0.obj 5 out/grid_s5.drc
//! ```
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::mesh_encoder::MeshEncoder;

#[path = "common/mod.rs"]
mod common;
use common::{load, options_for};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("mesh path");
    let speed: i32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .expect("encode speed");
    let out = args.next().expect("output .drc path");

    let payload = load(&path);
    let options = options_for(&payload.mesh, speed);
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(payload.mesh.clone());
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("rust encode failed");
    let bytes = buffer.data();
    std::fs::write(&out, bytes).expect("write .drc");
    eprintln!(
        "{} -> {out}: {} bytes at speed {speed} ({} points / {} faces)",
        payload.name,
        bytes.len(),
        payload.mesh.num_points(),
        payload.mesh.num_faces()
    );
}
