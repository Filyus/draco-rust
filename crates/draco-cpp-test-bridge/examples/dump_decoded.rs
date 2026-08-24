//! Decodes a `.drc` and writes what came out, so two builds can be `cmp`-ed.
//!
//! The counterpart of `decode_drc.rs`, which decodes and reports only a point
//! and face count. A count is not the output: a prediction round that got the
//! arithmetic wrong on one component of one entry still decodes the same number
//! of points. This writes every face and every attribute value, in decode
//! order, as bytes -- so "the output did not change" is one `cmp` rather than an
//! argument about which tests would have caught it.
//!
//! ```text
//! cargo run --release --example dump_decoded -- grid_s5.drc out.bin
//! ```
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect(".drc path");
    let out = args.next().expect("output path");

    let encoded = std::fs::read(&path).expect("read .drc");
    let mut buffer = DecoderBuffer::new(&encoded);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder
        .decode(&mut buffer, &mut mesh)
        .expect("decode failed");

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(mesh.num_points() as u64).to_le_bytes());
    bytes.extend_from_slice(&(mesh.num_faces() as u64).to_le_bytes());
    bytes.extend_from_slice(&(mesh.num_attributes() as u64).to_le_bytes());
    for f in 0..mesh.num_faces() {
        for corner in mesh.face(FaceIndex(f as u32)) {
            bytes.extend_from_slice(&corner.0.to_le_bytes());
        }
    }
    for a in 0..mesh.num_attributes() {
        let attribute = mesh.attribute(a);
        let components = attribute.num_components() as usize;
        for value in attribute.read_f32s(mesh.num_points(), components) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(&out, &bytes).expect("write dump");
    eprintln!(
        "{path} -> {out}: {} points / {} faces / {} attributes, {} bytes",
        mesh.num_points(),
        mesh.num_faces(),
        mesh.num_attributes(),
        bytes.len()
    );
}
