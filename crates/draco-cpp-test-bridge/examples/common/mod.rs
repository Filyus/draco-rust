//! Pieces the matrix harnesses share: a payload, and how a set of samples is
//! reported.
//!
//! Included by each example with `#[path = "common/mod.rs"] mod common;` --
//! it cannot live in the library, which does not depend on `draco-core`.
//!
//! Both matrices answer the same question in different directions, so the
//! parts that decide what is being compared -- how a mesh is loaded, what a
//! median means, what spread is printed beside it -- live here rather than
//! being written twice. A harness that measures the two sides differently
//! from its sibling is a harness whose numbers cannot be put in one table.

use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;

/// One payload, parsed once and held in both sides' input forms so neither
/// pays a conversion the other does not.
///
/// Each example uses a subset -- the decode matrix never needs the flat
/// position and face arrays, which only the C++ encoder entry point takes.
#[allow(dead_code)]
pub struct Payload {
    pub name: String,
    pub mesh: Mesh,
    pub positions: Vec<f32>,
    pub faces: Vec<u32>,
}

/// Read an `.obj` into a payload, named by its file stem.
pub fn load(path: &str) -> Payload {
    let bytes = std::fs::read(path).expect("read mesh");
    let mesh = draco_io::obj_reader::ObjReader::read_from_bytes(&bytes).expect("parse obj");
    let positions: Vec<f32> = mesh.attribute(0).read_f32s(mesh.num_points(), 3);
    let faces: Vec<u32> = (0..mesh.num_faces())
        .flat_map(|f| {
            let face = mesh.face(FaceIndex(f as u32));
            [face[0].0, face[1].0, face[2].0]
        })
        .collect();
    let name = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    Payload {
        name,
        mesh,
        positions,
        faces,
    }
}

/// Median, and the spread as a percentage of it -- the number that says
/// whether the median means anything.
///
/// Printed beside every figure on purpose: a cell whose spread is wider than
/// the gap it claims resolved nothing, and that should be visible rather than
/// assumed.
pub fn median_and_spread(samples: &mut [f64]) -> (f64, f64) {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    let median = samples[samples.len() / 2];
    let spread = if median > 0.0 {
        (samples[samples.len() - 1] - samples[0]) / median * 100.0
    } else {
        0.0
    };
    (median, spread)
}

/// The encoder options both matrices use, matching the `production_draco`
/// fixtures: position at 11 bits, a second attribute at 8.
pub fn options_for(mesh: &Mesh, speed: i32) -> draco_core::EncoderOptions {
    let mut options = draco_core::EncoderOptions::new();
    options.set_global_int("encoding_speed", speed);
    options.set_global_int("decoding_speed", speed);
    options.set_attribute_int(0, "quantization_bits", 11);
    if mesh.num_attributes() > 1 {
        options.set_attribute_int(1, "quantization_bits", 8);
    }
    options
}
