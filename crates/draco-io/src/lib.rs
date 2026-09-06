//! Mesh interchange formats for the `draco-core` geometry model: OBJ, PLY,
//! STL and FBX, read and written end to end.
//!
//! Nothing here touches the Draco codec. These formats carry geometry in their
//! own encodings, so a reader ends at a [`draco_core::mesh::Mesh`] and a writer
//! starts from one; whether that mesh is ever Draco-compressed is the caller's
//! business. glTF is the one format that carries a Draco bitstream itself, and
//! it lives in `draco-gltf` together with its containers and accessors.

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
// Reader-level regression tests for `Mesh::finalize`, which every reader here
// that builds a mesh from scratch ends with. The operation itself is
// `draco-core`'s -- it composes three of that crate's own passes and the order
// is load-bearing -- but what it buys is only visible through a reader, so the
// tests that pin it sit on this side.
#[cfg(test)]
mod mesh_finalize;
// The only remaining user of the hashable-tuple weld: OBJ interns `(v, vt,
// vn)` index triples into points during parsing. FBX used to key one on
// resolved attribute *values* here too, before it moved onto the same
// `Mesh::finalize` pass every other from-scratch reader ends with.
#[cfg(feature = "obj-reader")]
mod mesh_weld;
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
// Shared by the readers that pack typed component arrays into an attribute
// buffer themselves -- OBJ, PLY and STL. glTF decodes accessors straight into
// native byte layout and never needs this.
#[cfg(any(feature = "obj-reader", feature = "ply-reader", feature = "stl-reader"))]
mod raw_attribute;
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
