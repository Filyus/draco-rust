//! The decode fingerprint the C++ bridge reports, taken of this crate's decode
//! the same way: counts, faces in order, every attribute's bytes per point,
//! and the faces as an unordered set. Shared by the parity tests and the
//! corpus probe.
#![allow(dead_code)]

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;

pub const FNV_OFFSET: u64 = 1469598103934665603;
pub const FNV_PRIME: u64 = 1099511628211;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RustDecodeFingerprint {
    pub num_points: u32,
    pub num_faces: u32,
    pub num_attributes: u32,
    pub face_hash: u64,
    pub attribute_hash: u64,
    pub canonical_corner_hash: u64,
}

pub fn fnv1a_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

pub fn fnv1a_u32(hash: &mut u64, value: u32) {
    fnv1a_bytes(hash, &value.to_le_bytes());
}

pub fn fnv1a_u64(hash: &mut u64, value: u64) {
    fnv1a_bytes(hash, &value.to_le_bytes());
}

pub fn hash_mesh_faces(mesh: &Mesh) -> u64 {
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

pub fn hash_mesh_attributes(mesh: &Mesh) -> u64 {
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

pub fn hash_point_cloud_attributes(point_cloud: &PointCloud) -> u64 {
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

pub fn hash_mesh_canonical_corners(mesh: &Mesh) -> u64 {
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

pub fn rust_decode_fingerprint(data: &[u8]) -> RustDecodeFingerprint {
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

pub fn rust_decode_point_cloud_fingerprint(data: &[u8]) -> RustDecodeFingerprint {
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
