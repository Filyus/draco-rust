//! A `uint32` attribute carrying values above `i32::MAX` is a stream only this
//! encoder writes -- and upstream C++ Draco still reads it byte for byte.
//!
//! Upstream refuses such a mesh at encode time: `PrepareValues` converts every
//! value with `ConvertValue<int32_t>`, and `ConvertComponentValue` returns false
//! for a `uint32` above `INT32_MAX`. Its decoder has no such check --
//! `SequentialIntegerAttributeDecoder::StoreTypedValues` writes the portable
//! `int32` back with a plain `static_cast<uint32_t>` -- so the widening this
//! port allows stays inside what C++ Draco can consume.
//!
//! That asymmetry is what this file pins. Without it the widening is a claim
//! about someone else's decoder that nothing checks. The Rust-side round trip
//! is pinned separately and on stable CI by
//! `a_uint32_attribute_keeps_values_above_i32_max_through_a_round_trip` in
//! `draco-core/tests/attribute_integration_test.rs`; this test needs the C++
//! build and is skipped without it.

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::PointIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;

const FNV_OFFSET: u64 = 1469598103934665603;
const FNV_PRIME: u64 = 1099511628211;

const WIDTH: u32 = 5;
const HEIGHT: u32 = 5;
const NUM_POINTS: usize = (WIDTH * HEIGHT) as usize;
/// Above `i32::MAX`, so the portable `int32` holds these bits as negatives.
const BASE: u32 = 0xFFFF_0000;

fn fnv1a_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn fnv1a_u32(hash: &mut u64, value: u32) {
    fnv1a_bytes(hash, &value.to_le_bytes());
}

fn fnv1a_u64(hash: &mut u64, value: u64) {
    fnv1a_bytes(hash, &value.to_le_bytes());
}

/// The same attribute hash `hash_mesh_attributes` computes in
/// `cpp/test_bridge.cpp`: every value's raw bytes, in point order, behind the
/// attribute's declared shape. Byte-level by construction, which is the point --
/// a float-valued comparison could not tell `0xFFFF_0000` from its neighbours.
fn rust_attribute_hash(mesh: &Mesh) -> u64 {
    let mut hash = FNV_OFFSET;
    fnv1a_u32(&mut hash, mesh.num_attributes() as u32);
    fnv1a_u32(&mut hash, mesh.num_points() as u32);

    for att_id in 0..mesh.num_attributes() {
        let att = mesh.attribute(att_id);
        let stride = att.byte_stride() as u32;
        fnv1a_u32(&mut hash, att.attribute_type() as u32);
        fnv1a_u32(&mut hash, att.data_type() as u32);
        fnv1a_u32(&mut hash, u32::from(att.num_components()));
        fnv1a_u32(&mut hash, u32::from(att.normalized()));
        fnv1a_u32(&mut hash, stride);
        fnv1a_u64(&mut hash, att.size() as u64);

        for point in 0..mesh.num_points() {
            let value_index = att.mapped_index(PointIndex(point as u32));
            fnv1a_u32(&mut hash, value_index.0);
            let offset = value_index.0 as usize * stride as usize;
            let mut bytes = vec![0u8; stride as usize];
            att.buffer().read(offset, &mut bytes);
            fnv1a_bytes(&mut hash, &bytes);
        }
    }

    hash
}

/// A grid whose position components straddle `i32::MAX`, so a signed and an
/// unsigned reading of the same bits are `2^32` apart -- the difference the
/// portable texture-coordinate predictor works on.
fn wide_uint32_mesh() -> Mesh {
    let mut mesh = Mesh::new();
    mesh.set_num_points(NUM_POINTS);

    let mut positions = PointAttribute::new();
    positions.init(
        GeometryAttributeType::Position,
        3,
        DataType::Uint32,
        false,
        NUM_POINTS,
    );
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let index = (y * WIDTH + x) as usize;
            let value = if (x + y) % 2 == 0 {
                [BASE + x * 16, BASE + y * 16, BASE + (x + y) * 4]
            } else {
                [x * 16, y * 16, (x + y) * 4]
            };
            for (component, scalar) in value.iter().enumerate() {
                positions
                    .buffer_mut()
                    .write((index * 3 + component) * 4, &scalar.to_le_bytes());
            }
        }
    }
    mesh.add_attribute(positions);

    let mut tex_coords = PointAttribute::new();
    tex_coords.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Uint16,
        false,
        NUM_POINTS,
    );
    for point in 0..NUM_POINTS {
        let u = (point as u16).wrapping_mul(701);
        let v = (point as u16).wrapping_mul(263);
        tex_coords.buffer_mut().write(point * 4, &u.to_le_bytes());
        tex_coords
            .buffer_mut()
            .write(point * 4 + 2, &v.to_le_bytes());
    }
    mesh.add_attribute(tex_coords);

    let mut faces = Vec::new();
    for y in 0..HEIGHT - 1 {
        for x in 0..WIDTH - 1 {
            let i = y * WIDTH + x;
            faces.push([i, i + 1, i + WIDTH]);
            faces.push([i + 1, i + WIDTH + 1, i + WIDTH]);
        }
    }
    mesh.try_set_num_faces(faces.len()).expect("face count");
    for (index, face) in faces.iter().enumerate() {
        mesh.set_face_from_indices(index, *face);
    }

    mesh
}

#[test]
fn cpp_decodes_a_uint32_attribute_above_i32_max_the_same_way() {
    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    let mut options = EncoderOptions::new();
    // The portable scheme, so the position is read as a number rather than as
    // storage on both sides.
    options.set_attribute_int(1, "prediction_scheme", 5);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(wide_uint32_mesh());
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("a uint32 attribute above i32::MAX encodes");
    let encoded = buffer.data().to_vec();

    let mut rust_decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(&encoded), &mut rust_decoded)
        .expect("and this decoder reads it back");

    let cpp = draco_cpp_test_bridge::decode_cpp_mesh_fingerprint(&encoded)
        .expect("C++ Draco decodes a stream it would not have written");

    assert_eq!(
        cpp.num_points,
        rust_decoded.num_points() as u32,
        "num_points"
    );
    assert_eq!(
        cpp.num_attributes,
        rust_decoded.num_attributes() as u32,
        "num_attributes"
    );
    assert_eq!(
        cpp.attribute_hash,
        rust_attribute_hash(&rust_decoded),
        "C++ and this decoder disagree on the decoded attribute bytes"
    );
}
