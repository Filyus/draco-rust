//! Format I/O layer for Draco geometry.
//!
//! `draco-io` maps external 3D formats onto the geometry types from
//! `draco-core`. It handles file/container concerns such as OBJ, PLY, FBX,
//! glTF, GLB, scene hierarchy, and `KHR_draco_mesh_compression`; raw `.drc`
//! bitstream encoding and decoding stays in `draco-core`.
//!
//! # Supported Formats
//!
//! | Format | Read | Write | Draco compression |
//! |--------|------|-------|-------------------|
//! | OBJ    | yes  | yes   | no                |
//! | PLY    | yes  | yes   | no                |
//! | FBX    | yes  | yes   | no                |
//! | glTF   | yes  | yes   | yes               |
//! | GLB    | yes  | yes   | yes               |
//!
//! # Feature Model
//!
//! The default feature set enables all readers, all writers, optional
//! compression support, and point-cloud Draco decoding. For smaller builds, use
//! `default-features = false` and enable only the format features needed, such
//! as `gltf-reader`, `gltf-writer`, `obj-reader`, or `ply-writer`.
//!
//! # Geometry Contract
//!
//! `draco-io` maps source formats onto the Draco geometry model. Meshes use
//! triangle faces, `Position` is the required attribute, and `Normal`, `Color`,
//! `TexCoord`, and `Generic` are preserved when the file format can represent
//! them as Draco attributes. Scene support in the *geometry model* is limited to
//! names, hierarchy, transforms, and mesh parts; materials, textures, cameras,
//! lights, animation, skinning, structural metadata, and arbitrary format extras
//! are not represented in that model.
//!
//! # Document-preserving glTF compression
//!
//! Decoding into the geometry model and re-emitting a fresh glTF necessarily
//! drops anything the model does not represent (materials, textures, and so on).
//! When the goal is to Draco-compress an existing glTF/GLB **in place**, use
//! [`compress_gltf_bytes`]: it rewrites only the compressible mesh geometry and
//! carries the rest of the document through untouched — materials, textures,
//! images, samplers, cameras, nodes, animations, skins, `extras`, and unknown
//! JSON extensions survive. Unknown extension content that appears to contain
//! binary buffer/view/offset references is rejected because it cannot be
//! remapped safely. Default attribute quantization is lossy. Embedding glTF
//! buffers does not embed external image URIs.
//!
//! # Unified Trait API
//!
//! All readers implement [`Reader`] and all writers implement [`Writer`]:
//!
//! ```no_run
//! # #[cfg(all(feature = "obj-reader", feature = "obj-writer", feature = "ply-writer"))]
//! # fn main() -> Result<(), std::io::Error> {
//! use std::io;
//! use draco_core::mesh::Mesh;
//! use draco_io::{ObjReader, ObjWriter, PlyWriter, Reader, Writer};
//!
//! // Generic read function
//! fn load<R: Reader>(path: &str) -> io::Result<Mesh> {
//!     let mut reader = R::open(path)?;
//!     reader.read_mesh()
//! }
//!
//! // Generic write function
//! fn save<W: Writer>(mut writer: W, mesh: &Mesh) -> io::Result<()> {
//!     writer.add_mesh(mesh, Some("Model"))?;
//!     writer.write("output.ext")
//! }
//!
//! // Works with any format
//! let mesh = load::<ObjReader>("input.obj")?;
//! save(ObjWriter::new(), &mesh)?;
//! save(PlyWriter::new(), &mesh)?;
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "obj-reader", feature = "obj-writer", feature = "ply-writer")))]
//! # fn main() {}
//! ```
//!
//! # Format-Specific Features
//!
//! While the trait provides a common interface, each writer has format-specific methods:
//!
//! ```no_run
//! # #[cfg(all(feature = "fbx-writer", feature = "gltf-writer", feature = "obj-writer", feature = "ply-writer"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use draco_io::{FbxWriter, GltfWriter, ObjWriter, PlyWriter};
//! use draco_io::{PointCloudWriter, Writer};
//! # let mesh = draco_core::mesh::Mesh::new();
//! # let points = [[0.0, 0.0, 0.0]];
//! # let colors = [[255, 255, 255, 255]];
//!
//! // OBJ: Named groups
//! let mut obj = ObjWriter::new();
//! obj.add_mesh(&mesh, Some("Cube"))?;
//!
//! // PLY: Point clouds with colors
//! let mut ply = PlyWriter::new();
//! ply.add_points_with_colors(&points, &colors);
//!
//! // FBX: Optional compression
//! let mut fbx = FbxWriter::new().with_compression(true);
//! fbx.add_mesh(&mesh, Some("Model"))?;
//!
//! // glTF: Custom quantization, multiple output formats
//! let mut gltf = GltfWriter::new();
//! gltf.add_draco_mesh(&mesh, Some("Model"), None)?;  // Use default quantization
//! gltf.write_glb("output.glb")?;              // Binary GLB
//! gltf.write_gltf("out.gltf", "out.bin")?;   // Separate files
//! gltf.write_gltf_embedded("embedded.gltf")?; // Pure text
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "fbx-writer", feature = "gltf-writer", feature = "obj-writer", feature = "ply-writer")))]
//! # fn main() {}
//! ```
//!
//! # glTF/GLB with Draco Compression
//!
//! The `gltf_reader` and `gltf_writer` modules provide focused support for
//! Draco triangle meshes through `KHR_draco_mesh_compression`. They validate
//! the container enough to avoid silently dropping required glTF features; they
//! are not a full glTF SDK.
//!
//! Three output formats are available:
//!
//! - **GLB**: Binary container (single .glb file)
//! - **glTF + .bin**: JSON with separate binary file
//! - **glTF (embedded)**: Pure text JSON with base64 data URIs
//!
//! ## Reading Draco-compressed glTF
//!
//! ```no_run
//! # #[cfg(feature = "gltf-reader")]
//! # fn main() -> Result<(), draco_io::GltfError> {
//! use draco_io::gltf_reader::GltfReader;
//!
//! let reader = GltfReader::open("model.glb")?;
//! for (info, mesh) in reader.decode_all_draco_meshes()? {
//!     println!("Mesh '{}' has {} faces",
//!         info.mesh_name.unwrap_or_default(),
//!         mesh.num_faces());
//! }
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "gltf-reader"))]
//! # fn main() {}
//! ```
//!
//! ## Writing Draco-compressed GLB
//!
//! ```no_run
//! # #[cfg(feature = "gltf-writer")]
//! # fn main() -> Result<(), draco_io::GltfWriteError> {
//! use draco_io::gltf_writer::GltfWriter;
//! # let mesh = draco_core::mesh::Mesh::new();
//!
//! let mut writer = GltfWriter::new();
//! writer.add_draco_mesh(&mesh, Some("MyMesh"), None)?;  // Use default quantization
//!
//! // Option 1: Binary GLB (most compact)
//! writer.write_glb("output.glb")?;
//!
//! // Option 2: Separate JSON and binary
//! writer.write_gltf("output.gltf", "output.bin")?;
//!
//! // Option 3: Pure text with embedded data (no external files)
//! writer.write_gltf_embedded("output.gltf")?;
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "gltf-writer"))]
//! # fn main() {}
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]

// Reader modules.
#[cfg(feature = "fbx-reader")]
#[cfg_attr(docsrs, doc(cfg(feature = "fbx-reader")))]
pub mod fbx_reader;
// Reader-agnostic glTF geometry decode + shared error type. Available with the
// reader, the writer, or the compact reader, so the compressor and the
// serde-free front end reuse it without the serde-backed reader.
#[cfg(any(
    feature = "gltf-reader",
    feature = "gltf-writer",
    feature = "gltf-compact"
))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(
        feature = "gltf-reader",
        feature = "gltf-writer",
        feature = "gltf-compact"
    )))
)]
pub mod gltf_container;
#[cfg(any(
    feature = "gltf-reader",
    feature = "gltf-writer",
    feature = "gltf-compact"
))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(
        feature = "gltf-reader",
        feature = "gltf-writer",
        feature = "gltf-compact"
    )))
)]
pub mod gltf_geometry;
#[cfg(any(feature = "gltf-reader", feature = "gltf-writer"))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "gltf-reader", feature = "gltf-writer")))
)]
pub mod gltf_khr_draco;
#[cfg(feature = "gltf-reader")]
#[cfg_attr(docsrs, doc(cfg(feature = "gltf-reader")))]
pub mod gltf_reader;
// serde-free compact glTF front end: reuses the strict container + shared
// error type without pulling the serde document model into the binary.
#[cfg(feature = "gltf-compact")]
#[cfg_attr(docsrs, doc(cfg(feature = "gltf-compact")))]
pub mod gltf_compact;
#[cfg(feature = "obj-reader")]
#[cfg_attr(docsrs, doc(cfg(feature = "obj-reader")))]
pub mod obj_reader;
#[cfg(feature = "ply-reader")]
#[cfg_attr(docsrs, doc(cfg(feature = "ply-reader")))]
pub mod ply_reader;

// Writer modules.
#[cfg(feature = "fbx-writer")]
#[cfg_attr(docsrs, doc(cfg(feature = "fbx-writer")))]
pub mod fbx_writer;
#[cfg(feature = "gltf-writer")]
#[cfg_attr(docsrs, doc(cfg(feature = "gltf-writer")))]
/// Document-preserving glTF Draco compression (keeps materials, textures, etc.).
///
/// The in-memory [`gltf_compress::compress_gltf_value`] needs only the writer;
/// the byte API ([`gltf_compress::compress_gltf_bytes`]) also needs the reader.
pub mod gltf_compress;
#[cfg(feature = "gltf-writer")]
#[cfg_attr(docsrs, doc(cfg(feature = "gltf-writer")))]
/// glTF/GLB writer with Draco mesh compression support.
pub mod gltf_writer;
#[cfg(feature = "obj-writer")]
#[cfg_attr(docsrs, doc(cfg(feature = "obj-writer")))]
pub mod obj_writer;
#[cfg(feature = "ply-writer")]
#[cfg_attr(docsrs, doc(cfg(feature = "ply-writer")))]
pub mod ply_writer;

/// Shared PLY storage-format enum.
pub mod ply_format;
// Traits module is always available
pub mod traits;

// Scene-graph layer (data model + traits) is only compiled for hierarchical
// formats (glTF, FBX) that actually carry a scene.
#[cfg(feature = "scene")]
#[cfg_attr(docsrs, doc(cfg(feature = "scene")))]
pub mod scene;

// Re-export main types for convenience
#[cfg(feature = "fbx-reader")]
#[cfg_attr(docsrs, doc(cfg(feature = "fbx-reader")))]
pub use fbx_reader::{FbxMemoryReader, FbxReader};
#[cfg(feature = "fbx-writer")]
#[cfg_attr(docsrs, doc(cfg(feature = "fbx-writer")))]
pub use fbx_writer::FbxWriter;
// Reader-agnostic geometry decode + shared error type (reader or writer).
#[cfg(feature = "gltf-compact")]
#[cfg_attr(docsrs, doc(cfg(feature = "gltf-compact")))]
pub use gltf_compact::{
    parse_compact_document, parse_compact_document_with_limits, CompactDocument, CompactLimits,
    CompactMeshData, CompactNode, CompactScene,
};
#[cfg(feature = "gltf-writer")]
pub use gltf_container::{
    build_glb_container, encode_data_uri, serialize_gltf_document, OutputFormat,
};
#[cfg(any(
    feature = "gltf-reader",
    feature = "gltf-writer",
    feature = "gltf-compact"
))]
pub use gltf_container::{
    decode_data_uri, parse_glb_json_and_bin, parse_gltf_container, resolve_gltf_buffers,
    resolve_resource_uri, ExternalFilePolicy, FileResourceResolver, GltfBufferReference,
    GltfContainer, GltfContainerFormat, ResourceLimits, ResourceResolver,
};
#[cfg(any(
    feature = "gltf-reader",
    feature = "gltf-writer",
    feature = "gltf-compact"
))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(
        feature = "gltf-reader",
        feature = "gltf-writer",
        feature = "gltf-compact"
    )))
)]
pub use gltf_geometry::{decode_geometry, AccessorSource, DecodedAccessor, GltfError};
#[cfg(any(feature = "gltf-reader", feature = "gltf-writer"))]
pub use gltf_khr_draco::{
    parse_khr_draco_extension_value, parse_khr_draco_mesh_compression, validate_khr_draco_document,
    KhrDracoExtension, KhrDracoMeshCompression, KHR_DRACO_MESH_COMPRESSION,
};
// In-memory compressor core: writer only.
#[cfg(feature = "gltf-writer")]
#[cfg_attr(docsrs, doc(cfg(feature = "gltf-writer")))]
pub use gltf_compress::{
    compress_gltf_value, consolidate_gltf_buffers, validate_gltf_document_binary_layout,
    validate_gltf_document_for_repacking, CompressionOutput, CompressionReport, EncodingMethod,
    GltfCompressionOptions, PreserveReason, PreservedPrimitive, PrimitiveLocation,
    QuantizationOptions,
};
// Byte compressor API: needs the reader to parse + resolve buffers.
#[cfg(all(feature = "gltf-reader", feature = "gltf-writer"))]
#[cfg_attr(
    docsrs,
    doc(cfg(all(feature = "gltf-reader", feature = "gltf-writer")))
)]
pub use gltf_compress::{
    compress_gltf_bytes, compress_gltf_bytes_with_base_path, compress_gltf_bytes_with_options,
    compress_gltf_bytes_with_resolver,
};
#[cfg(feature = "gltf-reader")]
#[cfg_attr(docsrs, doc(cfg(feature = "gltf-reader")))]
pub use gltf_reader::{
    DracoPrimitiveInfo, GltfDocumentMetadata, GltfNodeMetadata, GltfReader, GltfSceneMetadata,
};
#[cfg(feature = "gltf-writer")]
#[cfg_attr(docsrs, doc(cfg(feature = "gltf-writer")))]
pub use gltf_writer::{GltfWriteError, GltfWriter};
#[cfg(feature = "obj-reader")]
#[cfg_attr(docsrs, doc(cfg(feature = "obj-reader")))]
pub use obj_reader::ObjReader;
#[cfg(feature = "obj-writer")]
#[cfg_attr(docsrs, doc(cfg(feature = "obj-writer")))]
pub use obj_writer::ObjWriter;
pub use ply_format::PlyFormat;
#[cfg(feature = "ply-reader")]
#[cfg_attr(docsrs, doc(cfg(feature = "ply-reader")))]
pub use ply_reader::PlyReader;
#[cfg(feature = "ply-writer")]
#[cfg_attr(docsrs, doc(cfg(feature = "ply-writer")))]
pub use ply_writer::PlyWriter;
#[cfg(feature = "scene")]
#[cfg_attr(docsrs, doc(cfg(feature = "scene")))]
pub use scene::{
    flatten_to_scene, MeshInstance, Scene, SceneNode, SceneReader, SceneWriter, Transform,
};
pub use traits::{PointCloudReader, PointCloudWriter, ReadFromBytes, Reader, WriteToBytes, Writer};
