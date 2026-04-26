use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;

fn main() {
    let data = std::fs::read("testdata/cube_att.drc").unwrap();
    println!("File size: {}", data.len());

    let mut buffer = DecoderBuffer::new(&data);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder.decode(&mut buffer, &mut mesh).unwrap();
    println!(
        "Rust decode OK: {} pts, {} faces, {} attrs",
        mesh.num_points(),
        mesh.num_faces(),
        mesh.num_attributes()
    );
    for att_id in 0..mesh.num_attributes() {
        let att = mesh.attribute(att_id);
        println!(
            "  Rust attr {att_id}: {:?}, size={}",
            att.attribute_type(),
            att.size()
        );
    }

    match draco_cpp_test_bridge::profile_cpp_decode(&data, 1) {
        Some(r) => println!(
            "C++ decode OK: {} pts, {} faces, {} us",
            r.num_points, r.num_faces, r.decode_time_us
        ),
        None => println!("C++ decode FAILED"),
    }
}
