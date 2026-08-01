// Test uses array indexing patterns that mirror C++ test structure for clarity.
// needless_range_loop: for i in 0..n { arr[i] } makes index-based operations explicit
#![allow(clippy::needless_range_loop)]

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;
use draco_core::version::{DEFAULT_MESH_VERSION, DEFAULT_POINT_CLOUD_VERSION};
#[cfg(feature = "legacy_bitstream_encode")]
use draco_core::DracoError;

/// The oldest point-cloud bitstream this crate still writes when asked for it
/// explicitly. It is no longer any method's default -- upstream writes 2.3 for
/// every point cloud -- so this test names it rather than importing a constant.
const LEGACY_POINT_CLOUD_VERSION: (u8, u8) = (1, 3);

fn create_test_pc() -> PointCloud {
    let mut pc = PointCloud::new();
    let mut pos_att = PointAttribute::new();
    let num_points = 3;
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let buffer = pos_att.buffer_mut();
    for i in 0..9 {
        buffer.write(i * 4, &positions[i].to_le_bytes());
    }
    pc.add_attribute(pos_att);
    pc
}

fn create_test_mesh() -> Mesh {
    let mut mesh = Mesh::new();
    let mut pos_att = PointAttribute::new();
    let num_points = 3;
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let buffer = pos_att.buffer_mut();
    for i in 0..9 {
        buffer.write(i * 4, &positions[i].to_le_bytes());
    }
    mesh.add_attribute(pos_att);
    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [0u32.into(), 1u32.into(), 2u32.into()]);
    mesh
}

#[test]
fn test_mesh_roundtrip_v1_3() {
    let mesh = create_test_mesh();
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    // Use legacy v1.3 header for testing sequential encoding with legacy version.
    // We do NOT set quantization_bits here: the Rust encoder never writes the old
    // v < 2.0 quantization-params-before-symbols layout, so round-tripping
    // quantized attributes at v1.3 is not supported (matches current C++ behavior).
    options.set_version(1, 3);
    options.set_encoding_method(0); // Sequential

    let mut enc_buffer = EncoderBuffer::new();
    assert!(encoder.encode(&options, &mut enc_buffer).is_ok());

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    assert!(decoder.decode(&mut dec_buffer, &mut decoded_mesh).is_ok());

    assert_eq!(decoded_mesh.num_points(), 3);
    assert_eq!(decoded_mesh.num_faces(), 1);
}

#[test]
fn test_mesh_roundtrip_v2_2() {
    let mesh = create_test_mesh();
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    options.set_version(DEFAULT_MESH_VERSION.0, DEFAULT_MESH_VERSION.1);
    options.set_encoding_method(1); // Edgebreaker
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut enc_buffer = EncoderBuffer::new();
    assert!(encoder.encode(&options, &mut enc_buffer).is_ok());

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    assert!(decoder.decode(&mut dec_buffer, &mut decoded_mesh).is_ok());

    assert_eq!(decoded_mesh.num_points(), 3);
    assert_eq!(decoded_mesh.num_faces(), 1);
}

/// The decoded positions, sorted, so a comparison does not depend on the point
/// order — the KD-tree coder reorders points by construction.
fn sorted_positions(pc: &PointCloud) -> Vec<[i32; 3]> {
    let att = pc.attribute(pc.named_attribute_id(GeometryAttributeType::Position));
    let stride = att.byte_stride() as usize;
    let mut out = Vec::with_capacity(att.size());
    for i in 0..att.size() {
        let mut position = [0i32; 3];
        for (k, component) in position.iter_mut().enumerate() {
            let mut bytes = [0u8; 4];
            att.buffer().read(i * stride + k * 4, &mut bytes);
            // 14-bit quantization over a unit range leaves an error near 6e-5;
            // rounding to 1e-3 keeps the comparison exact-valued while staying
            // far above it.
            *component = (f32::from_le_bytes(bytes) * 1000.0).round() as i32;
        }
        out.push(position);
    }
    out.sort_unstable();
    out
}

/// Every point cloud version this crate claims must return the geometry it was
/// given, not merely the right number of points.
///
/// These three tests used to assert `num_points == 3` and nothing else, which a
/// stream whose quantization parameters land in the wrong place survives
/// unchanged — the count is written in the header, well before anything that
/// could go wrong with the values.
fn assert_point_cloud_round_trips(major: u8, minor: u8, method: i32) {
    let pc = create_test_pc();
    let expected = sorted_positions(&pc);

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);

    let mut options = EncoderOptions::new();
    options.set_version(major, minor);
    options.set_encoding_method(method);
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut enc_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut enc_buffer)
        .unwrap_or_else(|e| panic!("v{major}.{minor} method {method} encode: {e:?}"));

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_pc = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut dec_buffer, &mut decoded_pc)
        .unwrap_or_else(|e| panic!("v{major}.{minor} method {method} decode: {e:?}"));

    assert_eq!(
        decoded_pc.num_points(),
        3,
        "v{major}.{minor} method {method}: point count"
    );
    assert_eq!(
        sorted_positions(&decoded_pc),
        expected,
        "v{major}.{minor} method {method}: positions"
    );
}

#[test]
fn test_point_cloud_roundtrip_v1_3() {
    assert_point_cloud_round_trips(LEGACY_POINT_CLOUD_VERSION.0, LEGACY_POINT_CLOUD_VERSION.1, 0);
}

#[test]
fn test_point_cloud_roundtrip_v2_3() {
    assert_point_cloud_round_trips(
        DEFAULT_POINT_CLOUD_VERSION.0,
        DEFAULT_POINT_CLOUD_VERSION.1,
        1, // KD-tree
    );
}

/// The sequential point-cloud coder at its own newest claimed version, which
/// neither of the two tests above reached: 1.3 exercises sequential and 2.3
/// exercised the KD-tree.
#[test]
fn a_sequential_point_cloud_round_trips_at_2_3() {
    assert_point_cloud_round_trips(
        DEFAULT_POINT_CLOUD_VERSION.0,
        DEFAULT_POINT_CLOUD_VERSION.1,
        0,
    );
}

/// A claimed version must survive the index-width boundary it straddles.
///
/// Sequential connectivity picks its index width from the point count: u8 below
/// 256, u16 below 65536, then varints — but the decoder only reads varints from
/// 2.2, so at 1.3 the encoder wrote varints the decoder read as u32. The stream
/// decoded without error and with the right face count, and the faces were
/// different ones. Counts are exactly what a test must not stop at here.
#[test]
#[cfg(feature = "legacy_bitstream_encode")]
fn a_sequential_mesh_at_1_3_survives_the_varint_index_boundary() {
    use draco_core::geometry_indices::FaceIndex;

    // 257 x 257 = 66,049 points, just past the 65,536 boundary.
    let mesh = grid_mesh(257);
    assert!(
        mesh.num_points() > 65536,
        "the boundary is the point of this"
    );

    let mut options = EncoderOptions::new();
    options.set_version(1, 3);
    options.set_encoding_method(0);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("1.3 encode");

    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(buffer.data()), &mut decoded)
        .expect("1.3 decode");

    assert_eq!(decoded.num_faces(), mesh.num_faces(), "face count");
    for face_id in 0..mesh.num_faces() {
        let index = FaceIndex(face_id as u32);
        assert_eq!(
            decoded.face(index),
            mesh.face(index),
            "face {face_id} differs, so the index width disagreed"
        );
    }
}

/// A pre-2.2 EdgeBreaker target that would take the standard traversal is
/// refused, and one that takes the valence traversal still encodes.
///
/// The traversal is not a caller-visible option — it follows from the speed and
/// the face count — so the same version request succeeds or fails depending on
/// the mesh. Every refused case here decoded with "Failed to start start-face
/// bit decoding" before the guard: `encode_traversal_buffer` writes the block
/// in a buffer pinned to 2.2, and only the valence and predictive traversals
/// have a pre-2.2 path.
#[test]
#[cfg(feature = "legacy_bitstream_encode")]
fn a_pre_2_2_edgebreaker_target_refuses_the_standard_traversal() {
    // 2048 faces, over the 1000-face floor below which no speed selects valence.
    let large = grid_mesh(33);
    // 162 faces, under it: the traversal is standard at every speed.
    let tiny = grid_mesh(10);

    let encode = |mesh: &Mesh, speed: i32, major: u8, minor: u8| {
        let mut options = EncoderOptions::new();
        options.set_version(major, minor);
        options.set_encoding_method(1); // EdgeBreaker
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut buffer = EncoderBuffer::new();
        encoder.encode(&options, &mut buffer).map(|()| buffer)
    };

    for (major, minor) in [(2u8, 1u8), (2, 0), (1, 2)] {
        for (mesh, speed, label) in [
            (&large, 5, "2048 faces at speed 5"),
            (&tiny, 5, "162 faces at speed 5"),
            (&tiny, 0, "162 faces at speed 0"),
        ] {
            let error = encode(mesh, speed, major, minor)
                .err()
                .unwrap_or_else(|| panic!("v{major}.{minor}, {label}: encode should be refused"));
            assert!(
                matches!(error, draco_core::DracoError::UnsupportedVersion(_)),
                "v{major}.{minor}, {label}: got {error:?}"
            );
        }

        // Speed 0 on a mesh over the floor selects valence, which does have a
        // pre-2.2 layout, so the guard must not reach it.
        let buffer = encode(&large, 0, major, minor)
            .unwrap_or_else(|e| panic!("v{major}.{minor} valence encode: {e:?}"));
        let mut decoded = Mesh::new();
        MeshDecoder::new()
            .decode(&mut DecoderBuffer::new(buffer.data()), &mut decoded)
            .unwrap_or_else(|e| panic!("v{major}.{minor} valence decode: {e:?}"));
        assert_eq!(
            decoded.num_faces(),
            large.num_faces(),
            "v{major}.{minor}: face count"
        );
    }

    // 2.2 itself is the standard traversal's own version and stays encodable.
    let buffer = encode(&tiny, 5, DEFAULT_MESH_VERSION.0, DEFAULT_MESH_VERSION.1)
        .expect("2.2 standard encode");
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(buffer.data()), &mut decoded)
        .expect("2.2 standard decode");
    assert_eq!(decoded.num_faces(), tiny.num_faces());
}

/// Attribute coding a claimed version cannot carry is refused, and the cases
/// next to it that do work are not.
///
/// The version table alone cannot express these: they depend on the attribute's
/// encoder, so the same version request succeeds or fails per attribute. Each
/// refused row here decoded to an error before the guard, and each accepted row
/// still round-trips — which is what keeps the guard from being drawn wider
/// than the defect.
#[test]
#[cfg(feature = "legacy_bitstream_encode")]
fn attribute_coding_without_a_legacy_layout_is_refused() {
    let quantized_position = |major, minor, method, speed| {
        let mut options = EncoderOptions::new();
        options.set_version(major, minor);
        options.set_encoding_method(method);
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);
        options.set_attribute_int(0, "quantization_bits", 12);
        options
    };

    // A quantized non-normal attribute: the encoder writes the quantization
    // parameters trailing at every version, the decoder reads them inline below
    // 2.0. Both claimed sub-2.0 mesh versions are affected.
    for (major, minor, method, speed) in [(1u8, 2u8, 1i32, 0i32), (1, 3, 0, 5)] {
        let error = round_trip(&grid_mesh(33), &quantized_position(major, minor, method, speed))
            .expect_err(&format!("v{major}.{minor}: quantization should be refused"));
        assert!(
            matches!(error, RoundTripError::Encode(DracoError::UnsupportedVersion(_))),
            "v{major}.{minor}: got {error:?}"
        );
    }
    // 2.0 is where the trailing layout starts, and it still round-trips.
    round_trip(&grid_mesh(33), &quantized_position(2, 0, 1, 0)).expect("v2.0 quantized");

    // A quantized *normal* takes the octahedron transform, which has its own
    // pre-2.0 path, so the same version must keep accepting it.
    let mut normal_options = EncoderOptions::new();
    normal_options.set_version(1, 2);
    normal_options.set_encoding_method(1);
    normal_options.set_global_int("encoding_speed", 0);
    normal_options.set_global_int("decoding_speed", 0);
    normal_options.set_attribute_int(1, "quantization_bits", 10);
    round_trip(&grid_mesh_with_normals(33), &normal_options).expect("v1.2 quantized normal");

    // Prediction schemes 3 and 6 have no pre-2.2 layout on the encode side: the
    // deprecated tex-coords scheme writes a varint orientation count where the
    // decoder reads a u32, and the geometric-normal scheme's pre-2.2 prediction
    // data carries a mode byte the encoder never writes.
    for scheme in [3i32, 6] {
        // Each scheme predicts one attribute kind, and forcing it onto another
        // is already refused by `validate_prediction_schemes`.
        let mesh = if scheme == 6 {
            grid_mesh_with_normals(33)
        } else {
            grid_mesh_with_texcoords(33)
        };
        let scheme_options = |major, minor, quantize| {
            let mut options = EncoderOptions::new();
            options.set_version(major, minor);
            options.set_encoding_method(1);
            options.set_global_int("encoding_speed", 0);
            options.set_global_int("decoding_speed", 0);
            options.set_attribute_int(1, "prediction_scheme", scheme);
            if quantize {
                options.set_attribute_int(1, "quantization_bits", 10);
            }
            options
        };

        // At 2.2 the scheme is what it is meant to be.
        round_trip(&mesh, &scheme_options(2, 2, true))
            .unwrap_or_else(|e| panic!("scheme {scheme} at 2.2: {e:?}"));

        for (major, minor) in [(2u8, 1u8), (2, 0), (1, 2)] {
            let error = round_trip(&mesh, &scheme_options(major, minor, true))
                .expect_err(&format!("scheme {scheme} at {major}.{minor} should be refused"));
            assert!(
                matches!(error, RoundTripError::Encode(DracoError::UnsupportedVersion(_))),
                "scheme {scheme} at {major}.{minor}: got {error:?}"
            );

            // Unquantized, the attribute goes to the generic encoder and never
            // meets a prediction scheme at all, so the refusal must not fire.
            round_trip(&mesh, &scheme_options(major, minor, false)).unwrap_or_else(|e| {
                panic!("scheme {scheme} at {major}.{minor}, unquantized: {e:?}")
            });
        }
    }
}

#[cfg(feature = "legacy_bitstream_encode")]
#[derive(Debug)]
enum RoundTripError {
    Encode(DracoError),
    /// Carried for the panic message; no assertion matches on it, because a
    /// decode failure here is always a plain failure of the case under test.
    Decode(#[allow(dead_code)] DracoError),
}

/// Encode then decode, reporting which half failed.
#[cfg(feature = "legacy_bitstream_encode")]
fn round_trip(mesh: &Mesh, options: &EncoderOptions) -> Result<Mesh, RoundTripError> {
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(options, &mut buffer)
        .map_err(RoundTripError::Encode)?;
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(buffer.data()), &mut decoded)
        .map_err(RoundTripError::Decode)?;
    Ok(decoded)
}

/// [`grid_mesh`] plus a tex coordinate per point.
#[cfg(feature = "legacy_bitstream_encode")]
fn grid_mesh_with_texcoords(n: usize) -> Mesh {
    let mut mesh = grid_mesh(n);
    let num_points = n * n;
    let mut uv = PointAttribute::new();
    uv.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Float32,
        false,
        num_points,
    );
    for y in 0..n {
        for x in 0..n {
            let offset = (y * n + x) * 8;
            uv.buffer_mut()
                .write(offset, &(x as f32 / n as f32).to_le_bytes());
            uv.buffer_mut()
                .write(offset + 4, &(y as f32 / n as f32).to_le_bytes());
        }
    }
    mesh.add_attribute(uv);
    mesh
}

/// [`grid_mesh`] plus a unit normal per point.
#[cfg(feature = "legacy_bitstream_encode")]
fn grid_mesh_with_normals(n: usize) -> Mesh {
    let mut mesh = grid_mesh(n);
    let num_points = n * n;
    let mut normal = PointAttribute::new();
    normal.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    for i in 0..num_points {
        for (k, component) in [0.0f32, 0.0, 1.0].iter().enumerate() {
            normal
                .buffer_mut()
                .write(i * 12 + k * 4, &component.to_le_bytes());
        }
    }
    mesh.add_attribute(normal);
    mesh
}

/// A square grid of `n` by `n` points with two triangles per cell.
#[cfg(feature = "legacy_bitstream_encode")]
fn grid_mesh(n: usize) -> Mesh {
    use draco_core::draco_types::DataType;
    use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};

    let mut mesh = Mesh::new();
    let num_points = n * n;
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    for y in 0..n {
        for x in 0..n {
            let offset = (y * n + x) * 12;
            position
                .buffer_mut()
                .write(offset, &(x as f32).to_le_bytes());
            position
                .buffer_mut()
                .write(offset + 4, &(y as f32).to_le_bytes());
            position
                .buffer_mut()
                .write(offset + 8, &(((x + y) % 5) as f32).to_le_bytes());
        }
    }
    mesh.set_num_points(num_points);
    mesh.add_attribute(position);

    mesh.set_num_faces((n - 1) * (n - 1) * 2);
    let mut face_id = 0;
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let p0 = (y * n + x) as u32;
            let p1 = (y * n + x + 1) as u32;
            let p2 = ((y + 1) * n + x) as u32;
            let p3 = ((y + 1) * n + x + 1) as u32;
            mesh.set_face_from_indices(face_id, [p0, p1, p2]);
            face_id += 1;
            mesh.set_face_from_indices(face_id, [p1, p3, p2]);
            face_id += 1;
        }
    }
    mesh
}
