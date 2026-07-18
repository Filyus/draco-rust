//! Low-level format I/O and glTF binary contracts for Draco.
//!
//! Full glTF documents, scene preservation, compression and compact views live
//! in `draco-gltf`. This crate intentionally owns only container/resource and
//! accessor-to-geometry primitives.

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "fbx-reader")]
pub mod fbx_reader;
#[cfg(feature = "fbx-writer")]
pub mod fbx_writer;
#[cfg(feature = "gltf")]
pub mod gltf_container;
#[cfg(feature = "gltf")]
pub mod gltf_geometry;
#[cfg(feature = "obj-reader")]
pub mod obj_reader;
#[cfg(feature = "obj-writer")]
pub mod obj_writer;
pub mod ply_format;
#[cfg(feature = "ply-reader")]
pub mod ply_reader;
#[cfg(feature = "ply-writer")]
pub mod ply_writer;
#[cfg(feature = "scene")]
pub mod scene;
pub mod traits;

#[cfg(feature = "fbx-reader")]
pub use fbx_reader::{FbxMemoryReader, FbxReader};
#[cfg(feature = "fbx-writer")]
pub use fbx_writer::FbxWriter;
#[cfg(feature = "gltf")]
pub use gltf_container::{
    decode_data_uri, inspect_glb, parse_glb_json_and_bin, parse_gltf_container,
    resolve_gltf_buffers, resolve_resource_uri, ExternalFilePolicy, FileResourceResolver,
    GlbChunkDescriptor, GlbLayout, GltfBufferReference, GltfContainer, GltfContainerFormat,
    ResourceLimits, ResourceResolver,
};
#[cfg(feature = "gltf")]
pub use gltf_geometry::{decode_geometry, AccessorSource, DecodedAccessor, GltfError};
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
