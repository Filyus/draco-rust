//! Encode a generated mesh and decode it back, printing what survived.
//!
//! No input file and no arguments: the mesh is built in code so the example is
//! a self-contained check that both halves of the crate work together. It is
//! also what `tests/prose_to_code_ratio.rs` links to measure error prose
//! against machine code, which is why it exercises the encoder *and* the
//! decoder -- a decode-only program leaves every encoder message unlinked, and
//! the ratio would then describe the example as much as the crate.
//!
//! ```text
//! cargo run --release --example encode_decode
//! ```

use draco_core::{
    DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, FaceIndex, GeometryAttributeType, Mesh,
    MeshDecoder, MeshEncoder, PointAttribute,
};

/// Side length in vertices. Large enough that the connectivity coder has
/// interior vertices to traverse rather than a single fan.
const SIDE: usize = 16;

/// A `SIDE * SIDE` grid of quads split into triangles, with positions and
/// normals -- two attributes, so the encoder picks a prediction scheme per
/// attribute instead of taking the single-attribute path.
fn grid_mesh() -> Mesh {
    let num_points = SIDE * SIDE;
    let mut mesh = Mesh::new();

    let mut positions = PointAttribute::new();
    positions.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    let mut normals = PointAttribute::new();
    normals.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        num_points,
    );

    for y in 0..SIDE {
        for x in 0..SIDE {
            let i = y * SIDE + x;
            let fx = x as f32 / (SIDE - 1) as f32;
            let fy = y as f32 / (SIDE - 1) as f32;
            // A gentle bump, so the surface is not planar and the normals
            // differ from point to point.
            let fz = (fx * 6.0).sin() * (fy * 6.0).cos() * 0.25;
            write_vec3(&mut positions, i, [fx, fy, fz]);
            write_vec3(&mut normals, i, normal_at(fx, fy));
        }
    }

    mesh.add_attribute(positions);
    mesh.add_attribute(normals);

    let quads = (SIDE - 1) * (SIDE - 1);
    mesh.set_num_faces(quads * 2);
    let mut face = 0;
    for y in 0..SIDE - 1 {
        for x in 0..SIDE - 1 {
            let tl = (y * SIDE + x) as u32;
            let tr = tl + 1;
            let bl = tl + SIDE as u32;
            let br = bl + 1;
            mesh.set_face(FaceIndex(face), [tl.into(), bl.into(), tr.into()]);
            mesh.set_face(FaceIndex(face + 1), [tr.into(), bl.into(), br.into()]);
            face += 2;
        }
    }
    mesh
}

/// The analytic normal of the surface `grid_mesh` builds, normalised.
fn normal_at(fx: f32, fy: f32) -> [f32; 3] {
    let dx = 6.0 * (fx * 6.0).cos() * (fy * 6.0).cos() * 0.25;
    let dy = -6.0 * (fx * 6.0).sin() * (fy * 6.0).sin() * 0.25;
    let len = (dx * dx + dy * dy + 1.0).sqrt();
    [-dx / len, -dy / len, 1.0 / len]
}

fn write_vec3(attribute: &mut PointAttribute, index: usize, value: [f32; 3]) {
    for (component, v) in value.iter().enumerate() {
        attribute
            .buffer_mut()
            .write((index * 3 + component) * 4, &v.to_le_bytes());
    }
}

fn main() {
    let mesh = grid_mesh();
    println!(
        "input: {} points, {} faces, {} attributes",
        mesh.num_points(),
        mesh.num_faces(),
        mesh.num_attributes()
    );

    // Both ends of the speed range: speed 0 picks the slowest, densest coders
    // and speed 10 the fastest, so the two together reach both the EdgeBreaker
    // and sequential connectivity paths and both prediction families.
    for speed in [0, 10] {
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);
        options.set_attribute_int(0, "quantization_bits", 11);
        options.set_attribute_int(1, "quantization_bits", 8);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut buffer = EncoderBuffer::new();
        match encoder.encode(&options, &mut buffer) {
            Ok(()) => {}
            Err(error) => {
                println!("speed {speed}: encode failed: {error}");
                continue;
            }
        }
        let encoded = buffer.data().len();

        let mut decoded = Mesh::new();
        let mut decoder_buffer = DecoderBuffer::new(buffer.data());
        match MeshDecoder::new().decode(&mut decoder_buffer, &mut decoded) {
            Ok(()) => println!(
                "speed {speed}: {encoded} bytes -> {} points, {} faces, {} attributes",
                decoded.num_points(),
                decoded.num_faces(),
                decoded.num_attributes()
            ),
            Err(error) => println!("speed {speed}: {encoded} bytes, decode failed: {error}"),
        }
    }
}
