//! Corner-table construction, C++ against Rust, on identical faces.
//!
//! Corner-table construction is about 45% of a position-only Rust encode, as
//! timed by a Rust-only construction benchmark. Nothing had compared it against
//! the C++ it was ported from: a whole-encode benchmark times this stage
//! together with everything around it, so a gap here reads as a few percent of
//! the total and a gap elsewhere reads the same way.
//!
//! Both sides build the face array once, outside the timed loop, and both are
//! timed around `Create`/`init` only. The vertex and degenerated-face counts
//! are printed so a run that built two different tables is visible rather than
//! quietly comparable.
//!
//! ```text
//! cargo run --release --example corner_table_loop -- <mesh.obj> [iters]
//! ```
use draco_core::corner_table::CornerTable;
use draco_core::geometry_indices::{FaceIndex, VertexIndex};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("mesh path");
    let iters: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);

    let bytes = std::fs::read(&path).expect("read mesh");
    let mesh = draco_io::obj_reader::ObjReader::read_from_bytes(&bytes).expect("parse obj");

    // The faces exactly as `MeshEncoder` hands them to the table: position
    // attribute value indices, one triple per face.
    let pos = mesh.attribute(0);
    let flat: Vec<u32> = (0..mesh.num_faces())
        .flat_map(|f| {
            let face = mesh.face(FaceIndex(f as u32));
            [
                pos.mapped_index(face[0]).0,
                pos.mapped_index(face[1]).0,
                pos.mapped_index(face[2]).0,
            ]
        })
        .collect();
    let faces: Vec<[VertexIndex; 3]> = flat
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| [VertexIndex(c[0]), VertexIndex(c[1]), VertexIndex(c[2])])
        .collect();

    println!("{} faces, {iters} iterations", faces.len());

    match draco_cpp_test_bridge::profile_cpp_corner_table(&flat, iters) {
        Some((us, verts, degen)) => {
            println!("cpp:  {us} us/create, {verts} vertices, {degen} degenerated");
        }
        None => println!("cpp:  unavailable (bridge disabled or Create failed)"),
    }

    for _ in 0..3 {
        let mut ct = CornerTable::new(0);
        assert!(ct.init(&faces));
    }
    let start = std::time::Instant::now();
    let mut verts = 0;
    let mut degen = 0;
    for _ in 0..iters {
        let mut ct = CornerTable::new(0);
        assert!(ct.init(&faces));
        verts = ct.num_vertices();
        degen = ct.num_degenerated_faces();
    }
    let us = start.elapsed().as_secs_f64() * 1e6 / f64::from(iters);
    println!("rust: {us:.0} us/init, {verts} vertices, {degen} degenerated");
}
