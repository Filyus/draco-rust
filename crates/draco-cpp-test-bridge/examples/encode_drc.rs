//! Encodes an `.obj` N times and nothing else.
//!
//! The counterpart of `cpp/encode_drc.cpp`, and the encode half of the
//! callgrind comparison -- the same shape as `decode_drc`, for the same
//! reason: no bridge, no harness, so it builds anywhere `draco-core` does,
//! including the WSL side where `valgrind` lives.
//!
//! Both sides read the same `.obj` rather than a `.drc`, because encode has
//! no encoded input to share. What makes the comparison honest is the byte
//! count each side prints: two encoders that agree on the output size to the
//! byte were given the same mesh under the same options. Check it before
//! reading any per-stage figure.
//!
//! Under callgrind use `1` iteration: the tool counts instructions, so
//! repetition buys nothing and costs minutes.
//!
//! ```text
//! cargo run --release --example encode_drc -- seeded_grid_0.obj 5 1
//! valgrind --tool=callgrind --callgrind-out-file=rust.out ./encode_drc grid.obj 5 1
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
    let iters: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);

    let payload = load(&path);
    let options = options_for(&payload.mesh, speed);

    // The mesh is handed over once, outside the loop. `set_mesh` takes
    // ownership, so a per-iteration clone would charge this side a copy the
    // C++ entry point does not make -- and `encode_matrix` brackets it the
    // same way, outside its timer. `encode` resets its derived state, so the
    // encoder is reusable across iterations exactly as `draco::Encoder` is.
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(payload.mesh.clone());

    let mut bytes = 0;
    for _ in 0..iters {
        let mut buffer = EncoderBuffer::new();
        encoder
            .encode(&options, &mut buffer)
            .expect("rust encode failed");
        bytes = buffer.data().len();
    }
    eprintln!(
        "rust encoded {path} at speed {speed}: {bytes} bytes from {} points / {} faces x{iters}",
        payload.mesh.num_points(),
        payload.mesh.num_faces()
    );
}
