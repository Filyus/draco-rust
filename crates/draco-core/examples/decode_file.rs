use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use std::fs::File;
use std::io::Read;

fn main() {
    let path = std::env::args().nth(1).expect("Usage: decode_file <path>");
    let mut f = File::open(&path).expect("Failed to open file");
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).expect("Failed to read file");

    let mut decoder_buffer = DecoderBuffer::new(&buf);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    match decoder.decode(&mut decoder_buffer, &mut mesh) {
        Ok(()) => {
            println!(
                "Decoded mesh: num_points={}, num_faces={}",
                mesh.num_points(),
                mesh.num_faces()
            );
        }
        Err(e) => {
            println!("Failed to decode: {:?}", e);
        }
    }
}
