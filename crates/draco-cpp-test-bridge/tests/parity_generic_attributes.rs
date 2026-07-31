//! Byte parity with the C++ encoder for `GENERIC` attributes and for the
//! scalar types the rest of the parity suite never carries: `Int64`,
//! `Uint64`, `Float64`, `Bool`. Positions/normals/colors/UVs cover
//! `Float32` and `Uint8` thoroughly elsewhere; this is what is left of the
//! `DataType` enum and the one `GeometryAttributeType` -- application-defined
//! attributes -- that never appears in any of them.
//!
//! `select_sequential_encoder` (`sequential_attribute_encoder.rs`) is the
//! single place this crate decides which encoder handles an attribute, from
//! its data type. Everything here is chosen to land on a different arm of
//! that decision: the six-type integer list, `Float32` with and without
//! quantization, and the fallback that every other type -- including the
//! four this file is for -- takes.

use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_encoder::PointCloudEncoder;
use draco_cpp_test_bridge::{encode_cpp_generic, CppGenericAttribute};

/// Every speed is asserted. Kept as a named constant so that narrowing the
/// guarantee again is a visible edit rather than a quiet one.
const ASSERTED_FROM_SPEED: i32 = 0;
const POSITION_BITS: i32 = 14;

/// An 8x8 grid, triangulated -- 64 points, comfortably over the 40-point
/// threshold `SelectPredictionMethod` checks before choosing constrained
/// multi-parallelogram over plain parallelogram at speeds 0-1.
fn grid_positions_and_faces() -> (Vec<f32>, Vec<u32>) {
    const N: usize = 8;
    let mut positions = Vec::new();
    for y in 0..N {
        for x in 0..N {
            positions.extend([x as f32, y as f32, ((x + y) % 3) as f32 * 0.1]);
        }
    }
    let mut faces = Vec::new();
    for y in 0..N - 1 {
        for x in 0..N - 1 {
            let i = (y * N + x) as u32;
            faces.extend([i, i + 1, i + N as u32]);
            faces.extend([i + 1, i + N as u32 + 1, i + N as u32]);
        }
    }
    (positions, faces)
}

/// A generic attribute's values, already packed to the byte layout both
/// sides read: little-endian, `num_components` scalars per point.
struct GenericSample {
    name: &'static str,
    data_type: DataType,
    num_components: i32,
    quantization_bits: i32,
    bytes: Vec<u8>,
}

fn float32_sample(num_points: usize, quantize: bool) -> GenericSample {
    let mut bytes = Vec::with_capacity(num_points * 4);
    for i in 0..num_points {
        let v = (i as f32) * 0.37 - 5.0;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    GenericSample {
        name: if quantize {
            "f32 quantized"
        } else {
            "f32 unquantized"
        },
        data_type: DataType::Float32,
        num_components: 1,
        quantization_bits: if quantize { 12 } else { -1 },
        bytes,
    }
}

fn int32_sample(num_points: usize) -> GenericSample {
    let mut bytes = Vec::with_capacity(num_points * 4);
    for i in 0..num_points {
        let v: i32 = (i as i32) * 131 - 4000;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    GenericSample {
        name: "i32",
        data_type: DataType::Int32,
        num_components: 1,
        quantization_bits: -1,
        bytes,
    }
}

fn uint8_sample(num_points: usize) -> GenericSample {
    let bytes: Vec<u8> = (0..num_points).map(|i| (i * 37 % 256) as u8).collect();
    GenericSample {
        name: "u8",
        data_type: DataType::Uint8,
        num_components: 2,
        quantization_bits: -1,
        bytes: bytes.iter().flat_map(|&v| [v, v.wrapping_add(1)]).collect(),
    }
}

fn int64_sample(num_points: usize) -> GenericSample {
    let mut bytes = Vec::with_capacity(num_points * 8);
    for i in 0..num_points {
        let v: i64 = (i as i64) * 1_000_000_007 - 3_000_000_000;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    GenericSample {
        name: "i64",
        data_type: DataType::Int64,
        num_components: 1,
        quantization_bits: -1,
        bytes,
    }
}

fn uint64_sample(num_points: usize) -> GenericSample {
    let mut bytes = Vec::with_capacity(num_points * 8);
    for i in 0..num_points {
        let v: u64 = (i as u64) * 9_223_372_036 + 5;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    GenericSample {
        name: "u64",
        data_type: DataType::Uint64,
        num_components: 1,
        quantization_bits: -1,
        bytes,
    }
}

fn float64_sample(num_points: usize, request_quantization: bool) -> GenericSample {
    let mut bytes = Vec::with_capacity(num_points * 8);
    for i in 0..num_points {
        let v = (i as f64) * 0.001 - 1.5;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    GenericSample {
        // Quantization only ever applies to Float32; requesting it here must
        // be silently ignored rather than treated as a signal to quantize --
        // exactly the fault this crate's point-cloud encoder had for Uint8
        // colors, checked here for a type nothing has tested it against.
        name: if request_quantization {
            "f64 (quantization requested, must be ignored)"
        } else {
            "f64"
        },
        data_type: DataType::Float64,
        num_components: 1,
        quantization_bits: if request_quantization { 10 } else { -1 },
        bytes,
    }
}

fn bool_sample(num_points: usize) -> GenericSample {
    let bytes: Vec<u8> = (0..num_points).map(|i| (i % 2) as u8).collect();
    GenericSample {
        name: "bool",
        data_type: DataType::Bool,
        num_components: 1,
        quantization_bits: -1,
        bytes,
    }
}

fn build_rust_mesh(positions: &[f32], faces: &[u32], sample: &GenericSample) -> Mesh {
    let num_points = positions.len() / 3;
    let mut mesh = Mesh::new();
    mesh.set_num_points(num_points);
    mesh.set_num_faces(faces.len() / 3);

    let mut position_attribute = PointAttribute::new();
    position_attribute.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    for (i, chunk) in positions.chunks(3).enumerate() {
        for (c, &v) in chunk.iter().enumerate() {
            position_attribute
                .buffer_mut()
                .update(&v.to_le_bytes(), Some(i * 12 + c * 4));
        }
    }
    mesh.add_attribute(position_attribute);

    let mut generic_attribute = PointAttribute::new();
    generic_attribute.init(
        GeometryAttributeType::Generic,
        sample.num_components as u8,
        sample.data_type,
        false,
        num_points,
    );
    generic_attribute
        .buffer_mut()
        .update(&sample.bytes, Some(0));
    mesh.add_attribute(generic_attribute);

    for (i, face) in faces.chunks(3).enumerate() {
        mesh.set_face(
            FaceIndex(i as u32),
            [
                PointIndex(face[0]),
                PointIndex(face[1]),
                PointIndex(face[2]),
            ],
        );
    }
    mesh
}

fn build_rust_point_cloud(positions: &[f32], sample: &GenericSample) -> PointCloud {
    let num_points = positions.len() / 3;
    let mut pc = PointCloud::new();
    pc.set_num_points(num_points);

    let mut position_attribute = PointAttribute::new();
    position_attribute.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    for (i, chunk) in positions.chunks(3).enumerate() {
        for (c, &v) in chunk.iter().enumerate() {
            position_attribute
                .buffer_mut()
                .update(&v.to_le_bytes(), Some(i * 12 + c * 4));
        }
    }
    pc.add_attribute(position_attribute);

    let mut generic_attribute = PointAttribute::new();
    generic_attribute.init(
        GeometryAttributeType::Generic,
        sample.num_components as u8,
        sample.data_type,
        false,
        num_points,
    );
    generic_attribute
        .buffer_mut()
        .update(&sample.bytes, Some(0));
    pc.add_attribute(generic_attribute);

    pc
}

fn first_difference(left: &[u8], right: &[u8]) -> Option<usize> {
    let shared = left.len().min(right.len());
    (0..shared)
        .find(|&i| left[i] != right[i])
        .or(if left.len() == right.len() {
            None
        } else {
            Some(shared)
        })
}

#[test]
fn mesh_generic_attributes_match_cpp() {
    if !draco_cpp_test_bridge::is_available() {
        println!("SKIP generic attribute parity: no C++ bridge");
        return;
    }

    let (positions, faces) = grid_positions_and_faces();
    let num_points = positions.len() / 3;
    let samples = vec![
        float32_sample(num_points, true),
        float32_sample(num_points, false),
        int32_sample(num_points),
        uint8_sample(num_points),
        int64_sample(num_points),
        uint64_sample(num_points),
        float64_sample(num_points, false),
        float64_sample(num_points, true),
        bool_sample(num_points),
    ];

    let mut compared = 0;
    let mut mismatches = Vec::new();

    for sample in &samples {
        for speed in 0..=10 {
            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_speed", speed);
            options.set_global_int("decoding_speed", speed);
            options.set_attribute_int(0, "quantization_bits", POSITION_BITS);
            if sample.quantization_bits > 0 {
                options.set_attribute_int(1, "quantization_bits", sample.quantization_bits);
            }

            let mesh = build_rust_mesh(&positions, &faces, sample);
            let mut encoder = MeshEncoder::new();
            encoder.set_mesh(mesh);
            let mut buffer = EncoderBuffer::new();
            let Ok(()) = encoder.encode(&options, &mut buffer) else {
                mismatches.push(format!("{} speed {speed}: Rust encode failed", sample.name));
                continue;
            };
            let rust = buffer.data().to_vec();

            let Some(cpp) = encode_cpp_generic(
                true,
                &positions,
                &faces,
                CppGenericAttribute {
                    data_type: sample.data_type as i32,
                    num_components: sample.num_components,
                    bytes: &sample.bytes,
                    quantization_bits: sample.quantization_bits,
                },
                None,
                speed,
                speed,
                POSITION_BITS,
            ) else {
                mismatches.push(format!("{} speed {speed}: C++ encode failed", sample.name));
                continue;
            };
            compared += 1;

            if let Some(offset) = first_difference(&rust, &cpp) {
                if speed < ASSERTED_FROM_SPEED {
                    continue;
                }
                mismatches.push(format!(
                    "{} speed {speed}: C++ {} bytes, Rust {} bytes, first difference at {offset}",
                    sample.name,
                    cpp.len(),
                    rust.len(),
                ));
            }
        }
    }

    println!(
        "compared {compared} mesh generic-attribute encodes across {} samples",
        samples.len()
    );
    assert!(
        mismatches.is_empty(),
        "encoder output differs from C++ Draco on generic mesh attributes:\n{}",
        mismatches.join("\n")
    );
}

#[test]
fn point_cloud_generic_attributes_match_cpp() {
    if !draco_cpp_test_bridge::is_available() {
        println!("SKIP generic attribute parity: no C++ bridge");
        return;
    }

    let (positions, _faces) = grid_positions_and_faces();
    let num_points = positions.len() / 3;
    let samples = vec![
        float32_sample(num_points, true),
        float32_sample(num_points, false),
        int32_sample(num_points),
        uint8_sample(num_points),
        int64_sample(num_points),
        uint64_sample(num_points),
        float64_sample(num_points, false),
        float64_sample(num_points, true),
        bool_sample(num_points),
    ];

    let mut compared = 0;
    let mut mismatches = Vec::new();
    let mut methods_seen = std::collections::BTreeSet::new();

    for sample in &samples {
        // `None` exercises the selection rule: a Float32 attribute needs
        // quantization to make the cloud kd-tree eligible, everything else in
        // this file's list keeps it sequential except the plain integer types.
        for method in [None, Some(0), Some(1)] {
            for speed in 0..=10 {
                let mut options = EncoderOptions::new();
                options.set_global_int("encoding_speed", speed);
                options.set_global_int("decoding_speed", speed);
                if let Some(method) = method {
                    options.set_encoding_method(method);
                }
                options.set_attribute_int(0, "quantization_bits", POSITION_BITS);
                if sample.quantization_bits > 0 {
                    options.set_attribute_int(1, "quantization_bits", sample.quantization_bits);
                }

                let pc = build_rust_point_cloud(&positions, sample);
                let mut encoder = PointCloudEncoder::new();
                encoder.set_point_cloud(pc);
                let mut buffer = EncoderBuffer::new();
                let rust_ok = encoder.encode(&options, &mut buffer).is_ok();
                let rust = buffer.data().to_vec();

                let cpp = encode_cpp_generic(
                    false,
                    &positions,
                    &[],
                    CppGenericAttribute {
                        data_type: sample.data_type as i32,
                        num_components: sample.num_components,
                        bytes: &sample.bytes,
                        quantization_bits: sample.quantization_bits,
                    },
                    method,
                    speed,
                    speed,
                    POSITION_BITS,
                );

                match (rust_ok, cpp) {
                    (false, None) => continue, // both correctly reject this combination
                    (false, Some(_)) => {
                        mismatches.push(format!(
                            "{} method {method:?} speed {speed}: Rust rejected, C++ accepted",
                            sample.name
                        ));
                        continue;
                    }
                    (true, None) => {
                        mismatches.push(format!(
                            "{} method {method:?} speed {speed}: C++ rejected, Rust accepted",
                            sample.name
                        ));
                        continue;
                    }
                    (true, Some(cpp)) => {
                        compared += 1;
                        if let Some(byte) =
                            (cpp.len() > 8 && &cpp[0..5] == b"DRACO" && cpp[7] == 0).then(|| cpp[8])
                        {
                            methods_seen.insert(byte);
                        }
                        let Some(offset) = first_difference(&rust, &cpp) else {
                            continue;
                        };
                        if speed < ASSERTED_FROM_SPEED {
                            continue;
                        }
                        mismatches.push(format!(
                            "{} method {method:?} speed {speed}: C++ {} bytes, Rust {} bytes, first difference at {offset}",
                            sample.name,
                            cpp.len(),
                            rust.len(),
                        ));
                    }
                }
            }
        }
    }

    println!(
        "compared {compared} point-cloud generic-attribute encodes across {} samples",
        samples.len()
    );
    assert!(
        mismatches.is_empty(),
        "encoder output differs from C++ Draco on generic point-cloud attributes:\n{}",
        mismatches.join("\n")
    );
    assert!(
        methods_seen.contains(&0) && methods_seen.contains(&1),
        "expected both point-cloud encoders to be reached, saw methods {methods_seen:?}"
    );
}
