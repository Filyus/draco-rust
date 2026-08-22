//! Decodes a `.drc` file N times and nothing else.
//!
//! The counterpart of `cpp/decode_drc.cpp`, and the Rust half of the
//! callgrind comparison: one decode of one file, no encoder, no harness, no
//! C++ bridge -- so it builds anywhere `draco-core` does, including the WSL
//! side where `valgrind` lives and the Windows-built Draco library does not.
//!
//! Under callgrind use `1` iteration: the tool counts instructions, so
//! repetition buys nothing and costs minutes.
//!
//! ```text
//! cargo run --release --example decode_drc -- grid_s5.drc 1
//! valgrind --tool=callgrind --callgrind-out-file=rust.out ./decode_drc grid_s5.drc 1
//! ```
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect(".drc path");
    let iters: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);

    let encoded = std::fs::read(&path).expect("read .drc");
    let mut shape = (0, 0);
    for _ in 0..iters {
        let mut buffer = DecoderBuffer::new(&encoded);
        let mut mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();
        decoder
            .decode(&mut buffer, &mut mesh)
            .expect("decode failed");
        shape = (mesh.num_points(), mesh.num_faces());
    }
    eprintln!(
        "rust decoded {path}: {} points / {} faces x{iters}",
        shape.0, shape.1
    );
}
