//! Draco I/O library for reading and writing 3D mesh formats.
//!
//! This crate provides readers and writers for various 3D mesh formats with
//! Draco compression support and a unified API through common traits.
//!
//! # Supported Formats
//!
//! | Format | Read | Write | Draco Compression |
//! |--------|------|-------|-------------------|
//! | OBJ    | ✓    | ✓     | -                 |
//! | PLY    | ✓    | ✓     | -                 |
//! | FBX    | ✓    | ✓     | -                 |
//! | glTF   | ✓    | ✓     | ✓                 |
//! | GLB    | ✓    | ✓     | ✓                 |
//!
//! # Unified Trait API
//!
//! All readers implement [`Reader`] and all writers implement [`Writer`]:
//!
//! ```ignore
//! use draco_io::{Reader, Writer, ObjReader, ObjWriter};
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
//! ```
//!
//! # Format-Specific Features
//!
//! While the trait provides a common interface, each writer has format-specific methods:
//!
//! ```ignore
//! // OBJ: Named groups
//! let mut obj = ObjWriter::new();
//! obj.add_mesh(&mesh, Some("Cube"));
//!
//! // PLY: Point clouds with colors
//! let mut ply = PlyWriter::new();
//! ply.add_points_with_colors(&points, &colors);
//!
//! // FBX: Optional compression
//! let mut fbx = FbxWriter::new().with_compression(true);
//! fbx.add_mesh(&mesh, Some("Model"));
//!
//! // glTF: Custom quantization, multiple output formats
//! let mut gltf = GltfWriter::new();
//! gltf.add_draco_mesh(&mesh, Some("Model"), None)?;  // Use default quantization
//! gltf.write_glb("output.glb")?;              // Binary GLB
//! gltf.write_gltf("out.gltf", "out.bin")?;   // Separate files
//! gltf.write_gltf_embedded("embedded.gltf")?; // Pure text
//! ```
//!
//! # glTF/GLB with Draco Compression
//!
//! The `gltf_reader` and `gltf_writer` modules provide full support for the
//! `KHR_draco_mesh_compression` extension. Three output formats are available:
//!
//! - **GLB**: Binary container (single .glb file)
//! - **glTF + .bin**: JSON with separate binary file
//! - **glTF (embedded)**: Pure text JSON with base64 data URIs
//!
//! ## Reading Draco-compressed glTF
//!
//! ```ignore
//! use draco_io::gltf_reader::GltfReader;
//!
//! let reader = GltfReader::open("model.glb")?;
//! for (info, mesh) in reader.decode_all_draco_meshes()? {
//!     println!("Mesh '{}' has {} faces",
//!         info.mesh_name.unwrap_or_default(),
//!         mesh.num_faces());
//! }
//! ```
//!
//! ## Writing Draco-compressed GLB
//!
//! ```ignore
//! use draco_io::gltf_writer::GltfWriter;
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
//! ```

// Reader modules (require decoder feature)
#[cfg(feature = "decoder")]
pub mod fbx_reader;
#[cfg(feature = "decoder")]
pub mod gltf_reader;
#[cfg(feature = "decoder")]
pub mod obj_reader;
#[cfg(feature = "decoder")]
pub mod ply_reader;

// Writer modules (require encoder feature)
#[cfg(feature = "encoder")]
pub mod fbx_writer;
#[cfg(feature = "encoder")]
pub mod gltf_writer;
#[cfg(feature = "encoder")]
pub mod obj_writer;
#[cfg(feature = "encoder")]
pub mod ply_writer;

// Traits module is always available
pub mod ply_format;
pub mod traits;

// Re-export main types for convenience
#[cfg(feature = "decoder")]
pub use fbx_reader::FbxReader;
#[cfg(feature = "encoder")]
pub use fbx_writer::FbxWriter;
#[cfg(feature = "decoder")]
pub use gltf_reader::{DracoPrimitiveInfo, GltfError, GltfReader};
#[cfg(feature = "encoder")]
pub use gltf_writer::{GltfWriteError, GltfWriter};
#[cfg(feature = "decoder")]
pub use obj_reader::ObjReader;
#[cfg(feature = "encoder")]
pub use obj_writer::ObjWriter;
pub use ply_format::PlyFormat;
#[cfg(feature = "decoder")]
pub use ply_reader::PlyReader;
#[cfg(feature = "encoder")]
pub use ply_writer::PlyWriter;
pub use traits::{
    PointCloudReader, PointCloudWriter, Reader, Scene, SceneNode, SceneObject, SceneReader,
    SceneWriter, Transform, Writer,
};
