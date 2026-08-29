//! Low-level format I/O and glTF binary contracts for Draco.
//!
//! Full glTF documents, scene preservation, compression and compact views live
//! in `draco-gltf`. This crate intentionally owns only container/resource and
//! accessor-to-geometry primitives.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]

#[cfg(feature = "fbx-reader")]
/// FBX ASCII container reader.
pub mod fbx_ascii;
#[cfg(any(feature = "fbx-reader", feature = "fbx-writer"))]
mod fbx_ascii_syntax;
#[cfg(feature = "fbx-writer")]
mod fbx_ascii_writer;
#[cfg(feature = "fbx-reader")]
/// FBX binary container decoder.
pub mod fbx_container;
#[cfg(feature = "fbx-writer")]
mod fbx_encoder;
#[cfg(any(feature = "fbx-reader", feature = "fbx-writer"))]
pub mod fbx_node;
#[cfg(feature = "fbx-reader")]
/// Byte order, resource limits, and read options for the FBX reader.
pub mod fbx_options;
#[cfg(feature = "fbx-reader")]
/// FBX binary reader.
pub mod fbx_reader;
#[cfg(any(feature = "fbx-reader", feature = "fbx-writer"))]
/// Polygon-corner-domain expansion of FBX geometry.
pub mod fbx_render_mesh;

// Shared by the two readers that intern corners into points; see the module.
#[cfg(any(feature = "fbx-reader", feature = "fbx-writer"))]
mod fbx_scene;
#[cfg(feature = "fbx-reader")]
mod fbx_templates;
#[cfg(any(feature = "fbx-reader", feature = "fbx-writer"))]
/// Composition of the FBX transform stack into a local matrix.
mod fbx_transform;
#[cfg(feature = "fbx-writer")]
/// FBX binary writer.
pub mod fbx_writer;
#[cfg(feature = "fbx-writer")]
mod fbx_writer_6100;
#[cfg(feature = "gltf-container")]
/// glTF/GLB containers and resource resolution.
pub mod gltf_container;
#[cfg(feature = "gltf-container")]
mod gltf_error;
#[cfg(feature = "gltf-geometry")]
/// Reader-agnostic accessor and Draco geometry contracts.
pub mod gltf_geometry;
#[cfg(any(feature = "fbx-reader", feature = "fbx-writer", feature = "obj-reader"))]
mod mesh_weld;
#[cfg(feature = "gltf-container")]
/// `EXT_meshopt_compression` bitstream decoders.
pub mod meshopt;
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
#[cfg(feature = "stl-reader")]
/// STL reader.
pub mod stl_reader;
#[cfg(feature = "stl-writer")]
/// STL writer.
pub mod stl_writer;
/// Shared reader and writer traits.
pub mod traits;

#[cfg(feature = "fbx-reader")]
pub use fbx_options::{FbxByteOrder, FbxDecodeLimits, FbxReadOptions};
#[cfg(feature = "fbx-reader")]
pub use fbx_reader::{FbxMemoryReader, FbxReader};
#[cfg(any(feature = "fbx-reader", feature = "fbx-writer"))]
pub use fbx_render_mesh::{FbxGeometryLayers, FbxRenderLayer, FbxRenderMesh};
#[cfg(any(feature = "fbx-reader", feature = "fbx-writer"))]
pub use fbx_scene::{
    FbxAnimChannel, FbxAnimChannelPath, FbxAnimInterpolation, FbxAnimSampler, FbxAnimation,
    FbxBinormalSet, FbxCamera, FbxColorSet, FbxCreaseKind, FbxCreaseLayer, FbxGeometricTransform,
    FbxGlobalSettings, FbxLayerSet, FbxLight, FbxMaterial, FbxMeshInstance, FbxMeshLayers,
    FbxMorphTarget, FbxNodeAttribute, FbxNodeId, FbxNodeKind, FbxNormalSet, FbxScene, FbxSceneNode,
    FbxSkin, FbxSkinCluster, FbxSmoothingLayer, FbxTangentSet, FbxTexture, FbxTextureBinding,
    FbxTextureSlot, FbxTransform, FbxTransformStack, FbxUvSet, FbxWarning, FbxWarningCode,
};
#[cfg(feature = "fbx-writer")]
pub use fbx_writer::{FbxFormat, FbxWriteStats, FbxWriter};
#[cfg(feature = "gltf-container")]
pub use gltf_container::{
    decode_data_uri, inspect_glb, parse_glb_json_and_bin, parse_gltf_container,
    resolve_gltf_buffers, resolve_resource_uri, ExternalFilePolicy, FileResourceResolver,
    GlbChunkDescriptor, GlbLayout, GlbRangeReader, GltfBufferReference, GltfContainer,
    GltfContainerFormat, ResourceLimits, ResourceResolver,
};
#[cfg(feature = "gltf-container")]
pub use gltf_error::GltfError;
#[cfg(feature = "gltf-geometry")]
pub use gltf_geometry::{decode_geometry, AccessorSource, DecodedAccessor};
#[cfg(feature = "gltf-container")]
pub use meshopt::{MeshoptFilter, MeshoptMode};
#[cfg(feature = "obj-reader")]
pub use obj_reader::ObjReader;
#[cfg(feature = "obj-writer")]
pub use obj_writer::ObjWriter;
pub use ply_format::PlyFormat;
#[cfg(feature = "ply-reader")]
pub use ply_reader::PlyReader;
#[cfg(feature = "ply-writer")]
pub use ply_writer::PlyWriter;
#[cfg(feature = "stl-reader")]
pub use stl_reader::StlReader;
#[cfg(feature = "stl-writer")]
pub use stl_writer::{StlFormat, StlWriter};
pub use traits::{PointCloudReader, PointCloudWriter, ReadFromBytes, Reader, WriteToBytes, Writer};
