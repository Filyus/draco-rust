//! Decodes a `.drc` and writes the mesh out as `.obj`, so the encode/decode
//! matrices -- which take `.obj` -- can be pointed at a model that only exists
//! as a compressed stream.
//!
//! ```text
//! cargo run --release -p draco-cpp-test-bridge --example drc_to_obj -- in.drc out.obj
//! ```
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("usage: drc_to_obj <in.drc> <out.obj>");
    let output = args.next().expect("usage: drc_to_obj <in.drc> <out.obj>");

    let bytes = std::fs::read(&input).expect("read .drc");
    let mut buffer = DecoderBuffer::new(&bytes);
    let mut mesh = Mesh::new();
    MeshDecoder::new()
        .decode(&mut buffer, &mut mesh)
        .expect("decode");

    eprintln!(
        "{input}: {} points, {} faces, {} attributes",
        mesh.num_points(),
        mesh.num_faces(),
        mesh.num_attributes()
    );
    for index in 0..mesh.num_attributes() {
        let attribute = mesh.attribute(index);
        eprintln!(
            "  attribute {index}: {:?}, {} components, {:?}",
            attribute.attribute_type(),
            attribute.num_components(),
            attribute.data_type()
        );
    }

    draco_io::obj_writer::write_obj_mesh(&output, &mesh).expect("write .obj");
    eprintln!("wrote {output}");
}
