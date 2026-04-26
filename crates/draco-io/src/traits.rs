//! Common traits for readers and writers.
//!
//! These traits define consistent interfaces for all format implementations.
//!
//! # Usage
//!
//! Import the trait to access its methods:
//!
//! ```ignore
//! use draco_io::{Writer, ObjWriter};
//!
//! let mut writer = ObjWriter::new();
//! writer.add_mesh(&mesh, Some("Name"))?;  // Calls trait method
//! writer.write("output.obj")?;
//! ```
//!
//! This enables generic functions:
//!
//! ```ignore
//! fn save<W: Writer>(mut w: W, mesh: &Mesh) -> io::Result<()> {
//!     w.add_mesh(mesh, Some("Model"))?;
//!     w.write("output.ext")
//! }
//! ```

use std::io;
use std::path::Path;

use draco_core::mesh::Mesh;

/// Simple transform placeholder (4x4 row-major matrix).
#[derive(Debug, Clone)]
pub struct Transform {
    pub matrix: [[f32; 4]; 4],
}

/// Represents an object in a scene (Blender-like 'Object').
/// Contains the mesh data and optional transform metadata.
#[derive(Debug, Clone)]
pub struct SceneObject {
    pub name: Option<String>,
    pub mesh: Mesh,
    pub transform: Option<Transform>,
}

/// A node in a scene graph. Nodes can contain parts (meshes) and children.
#[derive(Debug, Clone)]
pub struct SceneNode {
    pub name: Option<String>,
    pub transform: Option<Transform>,
    pub parts: Vec<SceneObject>,
    pub children: Vec<SceneNode>,
}

impl SceneNode {
    pub fn new(name: Option<String>) -> Self {
        Self {
            name,
            transform: None,
            parts: Vec::new(),
            children: Vec::new(),
        }
    }
}

/// A simple scene container.
#[derive(Debug, Clone)]
pub struct Scene {
    pub name: Option<String>,
    /// Flat list of parts (for convenience/backward compatibility).
    pub parts: Vec<SceneObject>,
    /// Root nodes forming a hierarchy.
    pub root_nodes: Vec<SceneNode>,
}

/// Trait for readers that can return full scene information (meshes + metadata).
///
/// This mirrors [`SceneWriter`]: it extends the base [`Reader`] trait with
/// scene-graph specific reading.
pub trait SceneReader: Reader {
    /// Read a single scene from the source/file.
    fn read_scene(&mut self) -> io::Result<Scene>;

    /// Read all scenes (default: single scene wrapper).
    fn read_scenes(&mut self) -> io::Result<Vec<Scene>> {
        Ok(vec![self.read_scene()?])
    }
}

/// Trait for writers that can output full scene graphs (nodes + hierarchy + transforms).
///
/// This mirrors [`SceneReader`]: formats implementing this trait can accept one
/// scene via [`SceneWriter::add_scene`] or many scenes via
/// [`SceneWriter::add_scenes`]. Actual file output is still performed through
/// the base [`Writer`] trait.
///
/// # Example
///
/// ```ignore
/// use draco_io::{SceneWriter, Writer, GltfWriter, Scene};
///
/// let scene = Scene {
///     name: Some("MyScene".to_string()),
///     parts: vec![],
///     root_nodes: vec![/* ... */],
/// };
///
/// let mut writer = GltfWriter::new();
/// writer.add_scene(&scene)?;
/// writer.write("output.glb")?; // Writer::write defaults to GLB for GltfWriter
/// ```
pub trait SceneWriter: Writer {
    /// Add a scene graph to be written.
    fn add_scene(&mut self, scene: &Scene) -> io::Result<()>;

    /// Add all scenes (default: add one scene).
    fn add_scenes(&mut self, scenes: &[Scene]) -> io::Result<()> {
        for scene in scenes {
            self.add_scene(scene)?;
        }
        Ok(())
    }
}

/// Common interface for mesh writers.
///
/// All format writers implement this trait, providing a consistent API:
///
/// ```ignore
/// use draco_io::{Writer, ObjWriter};
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
/// ```ignore
/// use draco_io::{Reader, ObjReader};
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
