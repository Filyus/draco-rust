use std::path::PathBuf;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;

const FNV_OFFSET: u64 = 1469598103934665603;
const FNV_PRIME: u64 = 1099511628211;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RustDecodeFingerprint {
    num_points: u32,
    num_faces: u32,
    num_attributes: u32,
    face_hash: u64,
    attribute_hash: u64,
    canonical_corner_hash: u64,
}

fn testdata_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
}

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

fn hash_mesh_faces(mesh: &Mesh) -> u64 {
    let mut hash = FNV_OFFSET;
    fnv1a_u32(&mut hash, mesh.num_faces() as u32);

    for face_id in 0..mesh.num_faces() {
        let face = mesh.face(FaceIndex(face_id as u32));
        fnv1a_u32(&mut hash, face[0].0);
        fnv1a_u32(&mut hash, face[1].0);
        fnv1a_u32(&mut hash, face[2].0);
    }

    hash
}

fn hash_mesh_attributes(mesh: &Mesh) -> u64 {
    let mut hash = FNV_OFFSET;
    fnv1a_u32(&mut hash, mesh.num_attributes() as u32);
    fnv1a_u32(&mut hash, mesh.num_points() as u32);

    for att_id in 0..mesh.num_attributes() {
        let att = mesh.attribute(att_id);
        let stride = att.byte_stride() as usize;
        fnv1a_u32(&mut hash, att.attribute_type() as u32);
        fnv1a_u32(&mut hash, att.data_type() as u32);
        fnv1a_u32(&mut hash, u32::from(att.num_components()));
        fnv1a_u32(&mut hash, u32::from(att.normalized()));
        fnv1a_u32(&mut hash, stride as u32);
        fnv1a_u64(&mut hash, att.size() as u64);

        for point_id in 0..mesh.num_points() {
            let value_index = att.mapped_index(PointIndex(point_id as u32));
            let offset = value_index.0 as usize * stride;
            fnv1a_u32(&mut hash, value_index.0);
            fnv1a_bytes(&mut hash, &att.buffer().data()[offset..offset + stride]);
        }
    }

    hash
}

fn hash_point_cloud_attributes(point_cloud: &PointCloud) -> u64 {
    let mut hash = FNV_OFFSET;
    fnv1a_u32(&mut hash, point_cloud.num_attributes() as u32);
    fnv1a_u32(&mut hash, point_cloud.num_points() as u32);

    for att_id in 0..point_cloud.num_attributes() {
        let att = point_cloud.attribute(att_id);
        let stride = att.byte_stride() as usize;
        fnv1a_u32(&mut hash, att.attribute_type() as u32);
        fnv1a_u32(&mut hash, att.data_type() as u32);
        fnv1a_u32(&mut hash, u32::from(att.num_components()));
        fnv1a_u32(&mut hash, u32::from(att.normalized()));
        fnv1a_u32(&mut hash, stride as u32);
        fnv1a_u64(&mut hash, att.size() as u64);

        for point_id in 0..point_cloud.num_points() {
            let value_index = att.mapped_index(PointIndex(point_id as u32));
            let offset = value_index.0 as usize * stride;
            fnv1a_u32(&mut hash, value_index.0);
            fnv1a_bytes(&mut hash, &att.buffer().data()[offset..offset + stride]);
        }
    }

    hash
}

fn hash_mesh_canonical_corners(mesh: &Mesh) -> u64 {
    let mut metadata_hash = FNV_OFFSET;
    fnv1a_u32(&mut metadata_hash, mesh.num_attributes() as u32);
    for att_id in 0..mesh.num_attributes() {
        let att = mesh.attribute(att_id);
        fnv1a_u32(&mut metadata_hash, att.attribute_type() as u32);
        fnv1a_u32(&mut metadata_hash, att.data_type() as u32);
        fnv1a_u32(&mut metadata_hash, u32::from(att.num_components()));
        fnv1a_u32(&mut metadata_hash, u32::from(att.normalized()));
        fnv1a_u32(&mut metadata_hash, att.byte_stride() as u32);
    }

    let mut face_hashes = Vec::with_capacity(mesh.num_faces());
    for face_id in 0..mesh.num_faces() {
        let mut face_hash = metadata_hash;
        let face = mesh.face(FaceIndex(face_id as u32));
        for point in face {
            for att_id in 0..mesh.num_attributes() {
                let att = mesh.attribute(att_id);
                let stride = att.byte_stride() as usize;
                let value_index = att.mapped_index(point);
                let offset = value_index.0 as usize * stride;
                fnv1a_bytes(
                    &mut face_hash,
                    &att.buffer().data()[offset..offset + stride],
                );
            }
        }
        face_hashes.push(face_hash);
    }
    face_hashes.sort_unstable();

    let mut hash = FNV_OFFSET;
    fnv1a_u32(&mut hash, mesh.num_faces() as u32);
    fnv1a_u32(&mut hash, mesh.num_attributes() as u32);
    for face_hash in face_hashes {
        fnv1a_u64(&mut hash, face_hash);
    }
    hash
}

fn rust_decode_fingerprint(data: &[u8]) -> RustDecodeFingerprint {
    let mut buffer = DecoderBuffer::new(data);
    let mut mesh = Mesh::new();
    MeshDecoder::new()
        .decode(&mut buffer, &mut mesh)
        .expect("Rust decode failed");

    RustDecodeFingerprint {
        num_points: mesh.num_points() as u32,
        num_faces: mesh.num_faces() as u32,
        num_attributes: mesh.num_attributes() as u32,
        face_hash: hash_mesh_faces(&mesh),
        attribute_hash: hash_mesh_attributes(&mesh),
        canonical_corner_hash: hash_mesh_canonical_corners(&mesh),
    }
}

fn rust_decode_point_cloud_fingerprint(data: &[u8]) -> RustDecodeFingerprint {
    let mut buffer = DecoderBuffer::new(data);
    let mut point_cloud = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut buffer, &mut point_cloud)
        .expect("Rust point-cloud decode failed");

    RustDecodeFingerprint {
        num_points: point_cloud.num_points() as u32,
        num_faces: 0,
        num_attributes: point_cloud.num_attributes() as u32,
        face_hash: 0,
        attribute_hash: hash_point_cloud_attributes(&point_cloud),
        canonical_corner_hash: 0,
    }
}

fn read_varint(data: &[u8], offset: &mut usize) -> u64 {
    let mut value = 0u64;
    let mut shift = 0;
    loop {
        let byte = data[*offset];
        *offset += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if (byte & 0x80) == 0 {
            return value;
        }
        shift += 7;
    }
}

fn sequential_connectivity_method(data: &[u8]) -> Option<u8> {
    if data.len() < 12 || &data[0..5] != b"DRACO" {
        return None;
    }
    let geometry_type = data[7];
    let method = data[8];
    if geometry_type != 1 || method != 0 {
        return None;
    }

    let mut offset = 11;
    let major = data[5];
    let minor = data[6];
    if (major, minor) >= (2, 2) {
        let _num_faces = read_varint(data, &mut offset);
        let _num_points = read_varint(data, &mut offset);
    } else {
        offset += 8;
    }
    data.get(offset).copied()
}

#[test]
fn cpp_and_rust_decode_fingerprints_match_for_mesh_fixtures() {
    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    let base = testdata_dir();
    let cases = [
        "reference_cpp/cpp_encoded_cube_speed_10.drc",
        "reference_cpp/cpp_encoded_cube_speed_0.drc",
        "legacy_draco/cube_att.mesh_seq.1.0.0.drc",
        "legacy_draco/cube_att.mesh_eb.1.1.0.drc",
        "production_draco/cube_att.mesh_eb.v2.2.pos_norm_uv.drc",
        "production_draco/test_pos_color.mesh_eb.v2.2.pos_color.drc",
    ];

    for case in cases {
        let data = std::fs::read(base.join(case)).expect("failed to read fixture");
        let rust = rust_decode_fingerprint(&data);
        let cpp =
            draco_cpp_test_bridge::decode_cpp_mesh_fingerprint(&data).expect("C++ decode failed");

        assert_eq!(rust.num_points, cpp.num_points, "{case}: num_points");
        assert_eq!(rust.num_faces, cpp.num_faces, "{case}: num_faces");
        assert_eq!(
            rust.num_attributes, cpp.num_attributes,
            "{case}: num_attributes"
        );
        assert_eq!(rust.face_hash, cpp.face_hash, "{case}: face_hash");
        assert_eq!(
            rust.canonical_corner_hash, cpp.canonical_corner_hash,
            "{case}: canonical_corner_hash"
        );
        assert_eq!(
            rust.attribute_hash, cpp.attribute_hash,
            "{case}: attribute_hash"
        );
    }
}

#[test]
fn cpp_and_rust_decode_fingerprints_match_for_multi_color_fixture() {
    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    let case = "production_draco/blender_multi_color.mesh_eb.v2.2.pos_norm_uv_color012.drc";
    let data = std::fs::read(testdata_dir().join(case)).expect("failed to read fixture");
    let rust = rust_decode_fingerprint(&data);
    let cpp = draco_cpp_test_bridge::decode_cpp_mesh_fingerprint(&data).expect("C++ decode failed");

    assert_eq!(rust.num_points, cpp.num_points, "{case}: num_points");
    assert_eq!(rust.num_faces, cpp.num_faces, "{case}: num_faces");
    assert_eq!(
        rust.num_attributes, cpp.num_attributes,
        "{case}: num_attributes"
    );
    assert_eq!(rust.face_hash, cpp.face_hash, "{case}: face_hash");
    assert_eq!(
        rust.canonical_corner_hash, cpp.canonical_corner_hash,
        "{case}: canonical_corner_hash"
    );
    assert_eq!(
        rust.attribute_hash, cpp.attribute_hash,
        "{case}: attribute_hash"
    );
}

#[test]
fn cpp_and_rust_decode_fingerprints_match_for_point_cloud_fixtures() {
    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    let base = testdata_dir();
    let cases = [
        "legacy_draco/point_cloud_pos_norm.seq.1.0.0.drc",
        "legacy_draco/point_cloud_pos_norm.seq.1.1.0.drc",
        "production_draco/bpy_point_cloud.seq.v2.3.pos_norm_color.drc",
        "production_draco/bpy_point_cloud.kd.v2.3.pos_norm_color.drc",
    ];

    for case in cases {
        let data = std::fs::read(base.join(case)).expect("failed to read fixture");
        let rust = rust_decode_point_cloud_fingerprint(&data);
        let cpp = draco_cpp_test_bridge::decode_cpp_point_cloud_fingerprint(&data)
            .expect("C++ point-cloud decode failed");

        assert_eq!(rust.num_points, cpp.num_points, "{case}: num_points");
        assert_eq!(
            rust.num_attributes, cpp.num_attributes,
            "{case}: num_attributes"
        );
        assert_eq!(rust.num_faces, cpp.num_faces, "{case}: num_faces");
        assert_eq!(rust.face_hash, cpp.face_hash, "{case}: face_hash");
        assert_eq!(
            rust.canonical_corner_hash, cpp.canonical_corner_hash,
            "{case}: canonical_corner_hash"
        );
        assert_eq!(
            rust.attribute_hash, cpp.attribute_hash,
            "{case}: attribute_hash"
        );
    }
}

#[test]
fn cpp_compressed_sequential_connectivity_matches_rust_decode() {
    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    let positions = [
        0.0f32, 0.0, 0.0, //
        1.0, 0.0, 0.0, //
        1.0, 1.0, 0.0, //
        0.0, 1.0, 0.0, //
        2.0, 0.0, 0.0, //
        2.0, 1.0, 0.0, //
    ];
    let faces = [
        0u32, 2, 1, //
        0, 3, 2, //
        1, 5, 4, //
        1, 2, 5, //
        0, 5, 3, //
    ];

    let data =
        draco_cpp_test_bridge::encode_cpp_mesh_sequential(&positions, &faces, 5, 5, 14, true)
            .expect("C++ sequential compressed encode failed");
    assert_eq!(sequential_connectivity_method(&data), Some(0));

    let rust = rust_decode_fingerprint(&data);
    let cpp = draco_cpp_test_bridge::decode_cpp_mesh_fingerprint(&data).expect("C++ decode failed");

    assert_eq!(rust.num_points, cpp.num_points);
    assert_eq!(rust.num_faces, cpp.num_faces);
    assert_eq!(rust.num_attributes, cpp.num_attributes);
    assert_eq!(rust.face_hash, cpp.face_hash);
    assert_eq!(rust.attribute_hash, cpp.attribute_hash);
    assert_eq!(rust.canonical_corner_hash, cpp.canonical_corner_hash);
}
