//! Low-level format I/O and glTF binary contracts for Draco.
//!
//! Full glTF documents, scene preservation, compression and compact views live
//! in `draco-gltf`. This crate intentionally owns only container/resource and
//! accessor-to-geometry primitives.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]

#[cfg(feature = "fbx-reader")]
/// FBX binary reader.
pub mod fbx_reader;
#[cfg(feature = "fbx-writer")]
/// FBX binary writer.
pub mod fbx_writer;
#[cfg(feature = "gltf")]
/// glTF/GLB containers and resource resolution.
pub mod gltf_container;
#[cfg(feature = "gltf")]
mod gltf_error;
#[cfg(feature = "gltf-geometry")]
/// Reader-agnostic accessor and Draco geometry contracts.
pub mod gltf_geometry;
#[cfg(feature = "obj-reader")]
/// Wavefront OBJ reader.
pub mod obj_reader;
#[cfg(feature = "obj-writer")]
/// Wavefront OBJ writer.
pub mod obj_writer;
/// PLY format configuration.
pub mod ply_format;
#[cfg(feature = "ply-reader")]
/// PLY reader.
pub mod ply_reader;
#[cfg(feature = "ply-writer")]
/// PLY writer.
pub mod ply_writer;
#[cfg(feature = "scene")]
/// Generic scene graph adapters for format readers and writers.
pub mod scene;
/// Shared reader and writer traits.
pub mod traits;

#[cfg(feature = "fbx-reader")]
pub use fbx_reader::{FbxMemoryReader, FbxReader};
#[cfg(feature = "fbx-writer")]
pub use fbx_writer::FbxWriter;
#[cfg(feature = "gltf")]
pub use gltf_container::{
    decode_data_uri, inspect_glb, parse_glb_json_and_bin, parse_gltf_container,
    resolve_gltf_buffers, resolve_resource_uri, ExternalFilePolicy, FileResourceResolver,
    GlbChunkDescriptor, GlbLayout, GlbRangeReader, GltfBufferReference, GltfContainer,
    GltfContainerFormat, ResourceLimits, ResourceResolver,
};
#[cfg(feature = "gltf")]
pub use gltf_error::GltfError;
#[cfg(feature = "gltf-geometry")]
pub use gltf_geometry::{
    decode_geometry, pack_draco_primitive, AccessorSource, DecodedAccessor, PackedAttribute,
    PackedPrimitive,
};
#[cfg(feature = "obj-reader")]
pub use obj_reader::ObjReader;
#[cfg(feature = "obj-writer")]
pub use obj_writer::ObjWriter;
pub use ply_format::PlyFormat;
#[cfg(feature = "ply-reader")]
pub use ply_reader::PlyReader;
#[cfg(feature = "ply-writer")]
pub use ply_writer::PlyWriter;
#[cfg(feature = "scene")]
pub use scene::{
    flatten_to_scene, MeshInstance, Scene, SceneNode, SceneReader, SceneWriter, Transform,
};
pub use traits::{PointCloudReader, PointCloudWriter, ReadFromBytes, Reader, WriteToBytes, Writer};
