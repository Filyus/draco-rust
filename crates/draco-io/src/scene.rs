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

    pub fn with_parts(name: Option<String>, parts: Vec<SceneObject>) -> Self {
        Self {
            name,
            transform: None,
            parts,
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

impl Scene {
    pub fn new(name: Option<String>) -> Self {
        Self {
            name,
            parts: Vec::new(),
            root_nodes: Vec::new(),
        }
    }

    pub fn from_parts(name: Option<String>, parts: Vec<SceneObject>) -> Self {
        Self {
            name,
            parts,
            root_nodes: Vec::new(),
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

/// Wrap the flat mesh list of any [`Reader`] into a single-node [`Scene`].
///
/// This is the honest adapter for formats without a native scene graph (OBJ,
/// PLY): it makes the degenerate wrapping explicit at the call site instead of
/// every flat format pretending it has hierarchy. Hierarchical formats should
/// implement [`SceneReader`] and return their real graph instead.
pub fn flatten_to_scene<R: Reader>(reader: &mut R, name: Option<String>) -> io::Result<Scene> {
    let meshes = reader.read_meshes()?;
    let mut root = SceneNode::new(name.clone());
    let mut parts = Vec::with_capacity(meshes.len());
    for mesh in meshes {
        let part = SceneObject {
            name: None,
            mesh,
            transform: None,
        };
        root.parts.push(part.clone());
        parts.push(part);
    }
    Ok(Scene {
        name,
        parts,
        root_nodes: vec![root],
    })
}
