// Simple test to verify corner table structure
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use std::fs::File;
use std::io::Read;

fn main() {
    let drc_path = std::env::args()
        .nth(1)
        .expect("Usage: verify_corner_table <drc_path>");

    let mut f = File::open(&drc_path).expect("Failed to open DRC file");
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).expect("Failed to read DRC file");

    let mut decoder_buffer = DecoderBuffer::new(&buf);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder
        .decode(&mut decoder_buffer, &mut mesh)
        .expect("Failed to decode");

    println!(
        "Decoded mesh: {} vertices, {} faces",
        mesh.num_points(),
        mesh.num_faces()
    );

    // Print first 10 faces
    let num_to_print = mesh.num_faces().min(10);
    println!("First {} faces (point indices):", num_to_print);
    for f in 0..num_to_print {
        let face = mesh.face(draco_core::geometry_indices::FaceIndex(f as u32));
        println!(
            "  Face {}: [{}, {}, {}]",
            f, face[0].0, face[1].0, face[2].0
        );
    }
}
