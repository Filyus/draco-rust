//! Compressed size of a `uint32` position attribute carrying values above
//! `i32::MAX`, against a control with the same geometry below the boundary.
//!
//! Sizes are deterministic, so one run per case is the whole measurement --
//! there is no spread to report and nothing to warm up. The control exists
//! because a size on its own says nothing: what is being asked is whether
//! allowing the wider domain costs compression, and only the difference
//! answers that.
//!
//! Run with `cargo run --release -p draco-core --example wide_uint32_ratio`.
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;

const N: u32 = 16;

fn build(base: u32, straddle: bool) -> Mesh {
    let num_points = (N * N) as usize;
    let mut mesh = Mesh::new();
    mesh.set_num_points(num_points);

    let mut positions = PointAttribute::new();
    positions.init(
        GeometryAttributeType::Position,
        3,
        DataType::Uint32,
        false,
        num_points,
    );
    for y in 0..N {
        for x in 0..N {
            let index = (y * N + x) as usize;
            let wide = !straddle || (x + y) % 2 == 0;
            let b = if wide { base } else { 0 };
            let value = [b + x * 16, b + y * 16, b + (x + y) * 4];
            for (c, s) in value.iter().enumerate() {
                positions
                    .buffer_mut()
                    .write((index * 3 + c) * 4, &s.to_le_bytes());
            }
        }
    }
    mesh.add_attribute(positions);

    let mut tex = PointAttribute::new();
    tex.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Uint16,
        false,
        num_points,
    );
    for p in 0..num_points {
        tex.buffer_mut()
            .write(p * 4, &((p as u16).wrapping_mul(701)).to_le_bytes());
        tex.buffer_mut()
            .write(p * 4 + 2, &((p as u16).wrapping_mul(263)).to_le_bytes());
    }
    mesh.add_attribute(tex);

    let mut faces = Vec::new();
    for y in 0..N - 1 {
        for x in 0..N - 1 {
            let i = y * N + x;
            faces.push([i, i + 1, i + N]);
            faces.push([i + 1, i + N + 1, i + N]);
        }
    }
    mesh.try_set_num_faces(faces.len()).unwrap();
    for (i, f) in faces.iter().enumerate() {
        mesh.set_face_from_indices(i, *f);
    }
    mesh
}

fn size(mesh: Mesh, pred: i32) -> usize {
    let mut options = EncoderOptions::new();
    options.set_attribute_int(1, "prediction_scheme", pred);
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encode");
    buffer.data().len()
}

fn main() {
    for pred in [5, -1] {
        let control = size(build(0, false), pred);
        let all_wide = size(build(0xFFFF_0000, false), pred);
        let straddling = size(build(0xFFFF_0000, true), pred);
        println!(
            "pred={pred:>2}  control(low)={control:>6}  all-wide={all_wide:>6} ({:+.1}%)  straddling={straddling:>6} ({:+.1}%)",
            100.0 * (all_wide as f64 - control as f64) / control as f64,
            100.0 * (straddling as f64 - control as f64) / control as f64,
        );
    }
}
