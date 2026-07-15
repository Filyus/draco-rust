//! Test decoding C++ encoded files with Rust decoder

#![cfg(feature = "test")]

use std::fs;
use std::path::Path;

#[test]
fn test_decode_cpp_encoded_bunny() {
    use draco_core::decoder_buffer::DecoderBuffer;
    use draco_core::mesh::Mesh as DracoMesh;
    use draco_core::mesh_decoder::MeshDecoder;

    let cpp_encoded_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
        .join("bunny_cpp_standard.drc");
    assert!(
        cpp_encoded_path.is_file(),
        "committed C++ fixture is missing: {}",
        cpp_encoded_path.display()
    );

    let data = fs::read(&cpp_encoded_path).expect("Failed to read C++ encoded file");
    println!("C++ encoded file size: {} bytes", data.len());

    let mut decoder_buffer = DecoderBuffer::new(&data);
    let mut decoder = MeshDecoder::new();
    let mut mesh = DracoMesh::new();

    match decoder.decode(&mut decoder_buffer, &mut mesh) {
        Ok(_) => {
            println!("Decoding successful!");
            println!("  num_points: {}", mesh.num_points());
            println!("  num_faces: {}", mesh.num_faces());

            // Expected: bunny has 35947 vertices, 69451 faces (or similar after compression)
            assert!(mesh.num_points() > 30000, "Expected > 30000 vertices");
            assert!(mesh.num_faces() > 60000, "Expected > 60000 faces");
        }
        Err(e) => {
            panic!("Failed to decode C++ encoded file: {:?}", e);
        }
    }
}
