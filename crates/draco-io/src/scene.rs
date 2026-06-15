//! Scene-graph layer: data model and traits for formats that represent hierarchy.
//!
//! Only hierarchical formats (glTF, FBX) implement [`SceneReader`] /
//! [`SceneWriter`]. Flat formats (OBJ, PLY) are intentionally **not** forced to
//! fabricate a scene graph they do not have. When a caller wants a uniform
//! scene from a flat format, [`flatten_to_scene`] performs the degenerate
//! wrapping explicitly at the call site.

use std::io;

use draco_core::mesh::Mesh;

use crate::traits::{Reader, Writer};

/// Simple transform placeholder (4x4 row-major matrix).
#[derive(Debug, Clone)]
pub struct Transform {
    /// Row-major 4x4 transform matrix.
    pub matrix: [[f32; 4]; 4],
}

/// One mesh instance attached to a scene node.
///
/// The mesh data is owned by the instance for now; `transform` is the optional
/// local transform applied to this specific instance when it cannot be folded
/// into the containing node.
#[derive(Debug, Clone)]
pub struct MeshInstance {
    /// Optional instance name.
    pub name: Option<String>,
    /// Mesh data for this instance.
    pub mesh: Mesh,
    /// Optional local transform for this instance.
    pub transform: Option<Transform>,
}

/// A node in a scene graph. Nodes can contain mesh instances and children.
#[derive(Debug, Clone)]
pub struct SceneNode {
    /// Optional node name.
    pub name: Option<String>,
    /// Optional local transform for this node.
    pub transform: Option<Transform>,
    /// Mesh instances attached directly to this node.
    pub mesh_instances: Vec<MeshInstance>,
    /// Child nodes.
    pub children: Vec<SceneNode>,
}

impl SceneNode {
    /// Create an empty scene node.
    pub fn new(name: Option<String>) -> Self {
        Self {
            name,
            transform: None,
            mesh_instances: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Create a scene node populated with mesh instances.
    pub fn with_mesh_instances(name: Option<String>, mesh_instances: Vec<MeshInstance>) -> Self {
        Self {
            name,
            transform: None,
            mesh_instances,
            children: Vec::new(),
        }
    }
}

/// A simple scene container.
#[derive(Debug, Clone)]
pub struct Scene {
    /// Optional scene name.
    pub name: Option<String>,
    /// Root nodes forming a hierarchy.
    pub root_nodes: Vec<SceneNode>,
}

impl Scene {
    /// Create an empty scene.
    pub fn new(name: Option<String>) -> Self {
        Self {
            name,
            root_nodes: Vec::new(),
        }
    }

    /// Create a flat scene containing one root node with the given mesh instances.
    pub fn from_mesh_instances(name: Option<String>, mesh_instances: Vec<MeshInstance>) -> Self {
        Self {
            root_nodes: vec![SceneNode::with_mesh_instances(name.clone(), mesh_instances)],
            name,
        }
    }
}

/// Trait for readers that can return full scene information (meshes + metadata).
///
/// This is a capability extension over the base [`Reader`] trait and is only
/// implemented by formats that natively carry a scene graph (glTF, FBX). Flat
/// formats deliberately do not implement it; use [`flatten_to_scene`] instead.
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
/// let scene = Scene::new(Some("MyScene".to_string()));
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

/// Wrap the flat mesh list of any [`Reader`] into a single-node [`Scene`].
///
/// This is the honest adapter for formats without a native scene graph (OBJ,
/// PLY): it makes the degenerate wrapping explicit at the call site instead of
/// every flat format pretending it has hierarchy. Hierarchical formats should
/// implement [`SceneReader`] and return their real graph instead.
pub fn flatten_to_scene<R: Reader>(reader: &mut R, name: Option<String>) -> io::Result<Scene> {
    let meshes = reader.read_meshes()?;
    let mesh_instances = meshes
        .into_iter()
        .map(|mesh| MeshInstance {
            name: None,
            mesh,
            transform: None,
        })
        .collect();
    Ok(Scene::from_mesh_instances(name, mesh_instances))
}
