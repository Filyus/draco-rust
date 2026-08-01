#![allow(clippy::needless_range_loop)]

//! Round-trip coverage for the pre-2.2 ("legacy") EdgeBreaker valence encoder.
//! The decoder reads valence streams from Draco 0.10.0-1.1.0 (see
//! drc_edge_cases_test::legacy_valence_...); these tests drive the encode side
//! and confirm a < 2.2 valence stream decodes back to the same geometry the
//! modern (2.2) path produces.

use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
#[cfg(not(feature = "legacy_bitstream_encode"))]
use draco_core::DracoError;
#[cfg(feature = "legacy_bitstream_encode")]
use draco_core::{decoder_buffer::DecoderBuffer, mesh_decoder::MeshDecoder};

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

#[cfg(feature = "legacy_bitstream_encode")]
fn build_grid_with_normals(n: usize) -> Mesh {
    let mut mesh = build_grid(n);
    let num_points = n * n;
    let mut normal = PointAttribute::new();
    normal.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    {
        let buf = normal.buffer_mut();
        for i in 0..num_points {
            let n = [0.0f32, 0.0, 1.0];
            for k in 0..3 {
                buf.write(i * 12 + k * 4, &n[k].to_le_bytes());
            }
        }
    }
    mesh.add_attribute(normal);
    mesh
}

/// Encode `mesh` targeting bitstream `major.minor` with valence traversal
/// (speed 0), then decode it back.
#[cfg(feature = "legacy_bitstream_encode")]
fn encode_decode(mesh: &Mesh, major: u8, minor: u8) -> Result<Mesh, String> {
    encode_decode_q(mesh, major, minor, None)
}

/// As [`encode_decode`], optionally quantizing positions to `qp` bits (which, at
/// speed 0, selects the constrained-multi-parallelogram predictor).
#[cfg(feature = "legacy_bitstream_encode")]
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
#[cfg(feature = "legacy_bitstream_encode")]
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

#[cfg(feature = "legacy_bitstream_encode")]
fn decoded_normals_point_up(mesh: &Mesh) -> bool {
    let normal_id = mesh.named_attribute_id(GeometryAttributeType::Normal);
    if normal_id < 0 {
        return false;
    }
    let att = mesh.attribute(normal_id);
    let stride = att.byte_stride() as usize;
    let buffer = att.buffer();
    for i in 0..att.size() {
        let off = i * stride;
        let mut v = [0.0f32; 3];
        for k in 0..3 {
            let mut b = [0u8; 4];
            buffer.read(off + k * 4, &mut b);
            v[k] = f32::from_le_bytes(b);
        }
        if v[2] <= 0.99 {
            return false;
        }
    }
    true
}

#[test]
#[cfg(feature = "legacy_bitstream_encode")]
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
#[cfg(feature = "legacy_bitstream_encode")]
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

/// The legacy predictive (type-1) traversal, forced via the encoder option, must
/// round-trip to the same geometry as the modern path. It targets a pre-2.0
/// stream (Draco 0.9.1-style); the decoder handles type-1 behind
/// legacy_bitstream_decode.
#[test]
#[cfg(feature = "legacy_bitstream_encode")]
fn legacy_predictive_roundtrip_preserves_geometry() {
    let mesh = build_grid(33);
    let reference = encode_decode(&mesh, 2, 2).expect("modern reference round-trip");
    let reference_positions = sorted_positions(&reference);

    let mut opts = EncoderOptions::new();
    opts.set_version(1, 2);
    opts.set_encoding_method(1); // Edgebreaker
    opts.set_global_int("encoding_speed", 0);
    opts.set_global_int("decoding_speed", 0);
    opts.set_global_int("force_predictive_traversal", 1);

    let mut enc = MeshEncoder::new();
    enc.set_mesh(mesh.clone());
    let mut out = EncoderBuffer::new();
    enc.encode(&opts, &mut out).expect("predictive encode");

    let mut buf = DecoderBuffer::new(out.data());
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut buf, &mut decoded)
        .expect("predictive decode");

    assert_eq!(
        decoded.num_faces(),
        mesh.num_faces(),
        "predictive: face count"
    );
    assert_eq!(
        sorted_positions(&decoded),
        reference_positions,
        "predictive geometry differs from the modern round-trip"
    );
}

#[test]
#[cfg(feature = "legacy_bitstream_encode")]
fn legacy_normal_octahedron_transform_roundtrip() {
    let mesh = build_grid_with_normals(33);

    let mut opts = EncoderOptions::new();
    // 1.2, not 1.1. This round-trips at 1.1 too, but only for a mesh without
    // topology splits: below 1.2 the encoder writes split events as delta
    // varints while the decoder reads absolute u32 ids with an explicit edge
    // byte. A claim that holds for some meshes and not others is not one the
    // version table can make, so 1.2 is the floor and this test sits on it.
    // The two versions differ only in that split coding, and this grid has no
    // splits, so the transform under test is unaffected.
    opts.set_version(1, 2);
    opts.set_encoding_method(1); // Edgebreaker
    opts.set_global_int("encoding_speed", 0);
    opts.set_global_int("decoding_speed", 0);
    opts.set_global_int("force_predictive_traversal", 1);
    opts.set_attribute_int(1, "quantization_bits", 10);

    let mut enc = MeshEncoder::new();
    enc.set_mesh(mesh.clone());
    let mut out = EncoderBuffer::new();
    enc.encode(&opts, &mut out)
        .expect("legacy normal octahedron encode");

    let mut buf = DecoderBuffer::new(out.data());
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut buf, &mut decoded)
        .expect("legacy normal octahedron decode");

    assert_eq!(decoded.num_faces(), mesh.num_faces(), "face count");
    assert!(decoded_normals_point_up(&decoded));
}

#[test]
#[cfg(not(feature = "legacy_bitstream_encode"))]
fn legacy_edgebreaker_version_requires_legacy_encode_feature() {
    let mesh = build_grid(4);
    let mut opts = EncoderOptions::new();
    opts.set_version(2, 1);
    opts.set_encoding_method(1);

    let mut enc = MeshEncoder::new();
    enc.set_mesh(mesh);
    let mut out = EncoderBuffer::new();
    let err = enc
        .encode(&opts, &mut out)
        .expect_err("pre-2.2 EdgeBreaker encode should require legacy feature");

    match err {
        DracoError::unsupported_version(message) => {
            assert!(message.contains("legacy_bitstream_encode"));
        }
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
    assert!(
        out.data().is_empty(),
        "encode should fail before writing a partial header"
    );
}

#[test]
#[cfg(not(feature = "legacy_bitstream_encode"))]
fn force_predictive_traversal_requires_legacy_encode_feature() {
    let mesh = build_grid(4);
    let mut opts = EncoderOptions::new();
    opts.set_version(2, 2);
    opts.set_encoding_method(1);
    opts.set_global_int("force_predictive_traversal", 1);

    let mut enc = MeshEncoder::new();
    enc.set_mesh(mesh);
    let mut out = EncoderBuffer::new();
    let err = enc
        .encode(&opts, &mut out)
        .expect_err("predictive traversal encode should require legacy feature");

    match err {
        DracoError::unsupported_feature(message) => {
            assert!(message.contains("legacy_bitstream_encode"));
        }
        other => panic!("expected UnsupportedFeature, got {other:?}"),
    }
    assert!(
        out.data().is_empty(),
        "encode should fail before writing a partial header"
    );
}
