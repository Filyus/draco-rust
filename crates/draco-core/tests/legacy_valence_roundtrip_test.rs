#![allow(clippy::needless_range_loop)]

//! Round-trip coverage for the pre-2.2 ("legacy") EdgeBreaker valence encoder.
//! The decoder reads valence streams from Draco 0.10.0-1.1.0 (see
//! drc_edge_cases_test::legacy_valence_...); these tests drive the encode side
//! and confirm a < 2.2 valence stream decodes back to the same geometry the
//! modern (2.2) path produces.

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;

/// Build an `n` x `n` vertex grid (>= 1000 faces for n >= 24), positions only.
/// Large enough that the encoder selects the valence traversal at speed < 5.
fn build_grid(n: usize) -> Mesh {
    let mut mesh = Mesh::new();
    let mut pos = PointAttribute::new();
    let num_points = n * n;
    pos.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    {
        let buf = pos.buffer_mut();
        for y in 0..n {
            for x in 0..n {
                let i = y * n + x;
                let p = [x as f32, y as f32, (((x * 7 + y * 13) % 5) as f32) * 0.25];
                for k in 0..3 {
                    buf.write(i * 12 + k * 4, &p[k].to_le_bytes());
                }
            }
        }
    }
    mesh.add_attribute(pos);

    let faces = (n - 1) * (n - 1) * 2;
    mesh.set_num_faces(faces);
    let mut f = 0u32;
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let v00 = (y * n + x) as u32;
            let v10 = (y * n + x + 1) as u32;
            let v01 = ((y + 1) * n + x) as u32;
            let v11 = ((y + 1) * n + x + 1) as u32;
            mesh.set_face(FaceIndex(f), [v00.into(), v10.into(), v11.into()]);
            f += 1;
            mesh.set_face(FaceIndex(f), [v00.into(), v11.into(), v01.into()]);
            f += 1;
        }
    }
    mesh
}

/// Encode `mesh` targeting bitstream `major.minor` with valence traversal
/// (speed 0), then decode it back.
fn encode_decode(mesh: &Mesh, major: u8, minor: u8) -> Result<Mesh, String> {
    encode_decode_q(mesh, major, minor, None)
}

/// As [`encode_decode`], optionally quantizing positions to `qp` bits (which, at
/// speed 0, selects the constrained-multi-parallelogram predictor).
fn encode_decode_q(mesh: &Mesh, major: u8, minor: u8, qp: Option<u32>) -> Result<Mesh, String> {
    let mut opts = EncoderOptions::new();
    opts.set_version(major, minor);
    opts.set_encoding_method(1); // Edgebreaker
    opts.set_global_int("encoding_speed", 0);
    opts.set_global_int("decoding_speed", 0);
    if let Some(q) = qp {
        opts.set_attribute_int(0, "quantization_bits", q as i32);
    }

    let mut enc = MeshEncoder::new();
    enc.set_mesh(mesh.clone());
    let mut out = EncoderBuffer::new();
    enc.encode(&opts, &mut out)
        .map_err(|e| format!("encode v{major}.{minor}: {e:?}"))?;

    let mut buf = DecoderBuffer::new(out.data());
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut buf, &mut decoded)
        .map_err(|e| format!("decode v{major}.{minor}: {e:?}"))?;
    Ok(decoded)
}

/// Decoded vertex positions as raw bit patterns, sorted — comparable regardless
/// of vertex ordering. Both versions quantize identically, so an exact match
/// means the geometry round-tripped.
fn sorted_positions(mesh: &Mesh) -> Vec<[u32; 3]> {
    let att = mesh.attribute(mesh.named_attribute_id(GeometryAttributeType::Position));
    let buffer = att.buffer();
    let stride = att.byte_stride() as usize;
    let n = att.size();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let off = i * stride;
        let mut v = [0u32; 3];
        for k in 0..3 {
            let mut b = [0u8; 4];
            buffer.read(off + k * 4, &mut b);
            v[k] = u32::from_le_bytes(b);
        }
        out.push(v);
    }
    out.sort_unstable();
    out
}

#[test]
fn legacy_valence_roundtrip_preserves_geometry() {
    let mesh = build_grid(33); // 2048 faces -> valence at speed < 5

    // Trusted reference: the modern (2.2) valence round-trip.
    let reference = encode_decode(&mesh, 2, 2).expect("modern 2.2 round-trip");
    assert_eq!(reference.num_faces(), mesh.num_faces());
    let reference_positions = sorted_positions(&reference);

    // v2.1, v2.0 and v1.2 are all pre-2.2 layouts this encoder mirrors (the
    // valence/connectivity block split; v2.0 adds an empty hole-event section;
    // v1.2 additionally uses fixed-u32 connectivity counts and the always-present
    // header flags field). Each must decode back to the same geometry as modern.
    for (major, minor) in [(2u8, 1u8), (2, 0), (1, 2)] {
        let decoded = encode_decode(&mesh, major, minor)
            .unwrap_or_else(|e| panic!("v{major}.{minor} round-trip failed: {e}"));
        assert_eq!(
            decoded.num_faces(),
            mesh.num_faces(),
            "v{major}.{minor}: face count"
        );
        assert_eq!(
            sorted_positions(&decoded),
            reference_positions,
            "v{major}.{minor}: geometry differs from the modern round-trip"
        );
    }
}

/// At speed 0 with quantization the encoder selects the constrained-multi-
/// parallelogram position predictor, which pre-2.2 prefixes a mode byte. A v2.1
/// valence stream using it must round-trip to the same geometry as 2.2.
#[test]
fn legacy_valence_roundtrip_high_compression() {
    let mesh = build_grid(33);
    let reference = encode_decode_q(&mesh, 2, 2, Some(14)).expect("2.2 quantized round-trip");
    let decoded = encode_decode_q(&mesh, 2, 1, Some(14)).expect("v2.1 quantized round-trip");
    assert_eq!(decoded.num_faces(), mesh.num_faces(), "v2.1: face count");
    assert_eq!(
        sorted_positions(&decoded),
        sorted_positions(&reference),
        "v2.1 constrained-multi geometry differs from the modern round-trip"
    );
}

