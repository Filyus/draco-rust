//! Common traits for readers and writers.
//!
//! These traits define consistent interfaces for all format implementations.
//!
//! # Usage
//!
//! Import the trait to access its methods:
//!
//! ```no_run
//! # #[cfg(feature = "obj-writer")]
//! # fn main() -> Result<(), std::io::Error> {
//! use draco_io::{Writer, ObjWriter};
//! # let mesh = draco_core::mesh::Mesh::new();
//!
//! let mut writer = ObjWriter::new();
//! writer.add_mesh(&mesh, Some("Name"))?;  // Calls trait method
//! writer.write("output.obj")?;
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "obj-writer"))]
//! # fn main() {}
//! ```
//!
//! This enables generic functions:
//!
//! ```no_run
//! use std::io;
//! use draco_core::mesh::Mesh;
//! use draco_io::Writer;
//!
//! fn save<W: Writer>(mut w: W, mesh: &Mesh) -> io::Result<()> {
//!     w.add_mesh(mesh, Some("Model"))?;
//!     w.write("output.ext")
//! }
//! ```

use std::io::{self, Write};
use std::path::Path;

use draco_core::mesh::Mesh;

/// Common interface for mesh writers.
///
/// All format writers implement this trait, providing a consistent API:
///
/// ```no_run
/// use std::io;
/// use draco_core::mesh::Mesh;
/// use draco_io::Writer;
///
/// fn write_mesh<W: Writer>(mut writer: W, mesh: &Mesh) -> io::Result<()> {
///     writer.add_mesh(mesh, Some("MyMesh"))?;
///     writer.write("output.ext")
/// }
/// ```
pub trait Writer: Sized {
    /// Create a new writer instance.
    fn new() -> Self;

    /// Add a mesh to be written.
    ///
    /// # Arguments
    /// * `mesh` - The mesh to add
    /// * `name` - Optional name for the mesh (if format supports naming)
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err` if the format cannot handle this mesh (e.g., compression failure)
    fn add_mesh(&mut self, mesh: &Mesh, name: Option<&str>) -> io::Result<()>;

    /// Write all added meshes to a file.
    ///
    /// # Arguments
    /// * `path` - Output file path
    fn write<P: AsRef<Path>>(&self, path: P) -> io::Result<()>;

    /// Get the number of meshes/vertices added.
    fn vertex_count(&self) -> usize;

    /// Get the number of faces added (if applicable).
    fn face_count(&self) -> usize {
        0
    }
}

/// Common interface for mesh readers.
///
/// All format readers implement this trait, providing a consistent API:
///
/// ```no_run
/// use std::io;
/// use draco_core::mesh::Mesh;
/// use draco_io::Reader;
///
/// fn load_mesh<R: Reader>(path: &str) -> io::Result<Mesh> {
///     let mut reader = R::open(path)?;
///     reader.read_mesh()
/// }
/// ```
pub trait Reader: Sized {
    /// Open a file for reading.
    ///
    /// # Arguments
    /// * `path` - Input file path
    fn open<P: AsRef<Path>>(path: P) -> io::Result<Self>;

    /// Read multiple meshes (a scene) from the file.
    ///
    /// Formats that represent scenes or multiple mesh primitives should implement
    /// this method and return all meshes in the file or scene.
    fn read_meshes(&mut self) -> io::Result<Vec<Mesh>>;

    /// Read a single mesh from the file.
    ///
    /// Default implementation returns the first mesh from `read_meshes()`.
    fn read_mesh(&mut self) -> io::Result<Mesh> {
        let meshes = self.read_meshes()?;
        if let Some(m) = meshes.into_iter().next() {
            Ok(m)
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidData, "No mesh found"))
        }
    }
}

/// Common interface for readers that can be constructed from in-memory bytes.
///
/// This complements [`Reader::open`] for callers that already have file bytes
/// loaded, or that are working in browser/embedded environments without direct
/// filesystem access.
pub trait ReadFromBytes: Sized {
    /// Create a reader from a complete file payload.
    fn from_bytes(bytes: &[u8]) -> io::Result<Self>;
}

/// Common interface for writers that can emit a complete file payload.
///
/// This complements [`Writer::write`] for callers that need to send bytes over
/// the network, store them in an archive, or run roundtrips without temporary
/// files.
pub trait WriteToBytes: Writer {
    /// Write all added data into a byte vector.
    fn write_to_vec(&self) -> io::Result<Vec<u8>>;

    /// Write all added data into an arbitrary byte sink.
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(&self.write_to_vec()?)
    }
}

/// Extended writer trait for point cloud support.
///
/// Writers that can output point clouds (without faces) implement this trait.
pub trait PointCloudWriter: Writer {
    /// Add raw point positions.
    fn add_points(&mut self, points: &[[f32; 3]]);

    /// Add a single point.
    fn add_point(&mut self, point: [f32; 3]) {
        self.add_points(&[point]);
    }
}

/// Extended reader trait for point cloud support.
///
/// Readers that can read point clouds implement this trait.
pub trait PointCloudReader: Reader {
    /// Read point positions only (no faces or topology).
    fn read_points(&mut self) -> io::Result<Vec<[f32; 3]>>;
}
