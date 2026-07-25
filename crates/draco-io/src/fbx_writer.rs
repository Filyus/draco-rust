//! FBX binary format writer for meshes, materials, textures, and TRS animation.
//!
//! Supports writing:
//! - Binary FBX format (version 7.5 with 64-bit headers)
//! - Vertex positions, normals, and texture coordinates
//! - Triangle faces
//! - Optional zlib compression for arrays (with `compression` feature)
//! - Phong/Lambert materials, textures, and per-mesh material indices
//! - Node-TRS animation (`AnimationStack` / `AnimationLayer` /
//!   `AnimationCurveNode` / `AnimationCurve`)
//!
//! FBX pivots, cameras, and arbitrary metadata are not written. Skin clusters,
//! bind poses, and sparse blend-shape deltas are emitted. Mesh attributes
//! other than `Position`, `Normal`, and `TexCoord`
//! produce an explicit `InvalidInput` error so geometry data is not dropped
//! silently.
//!
//! # Example
//!
//! ```no_run
//! use draco_io::fbx_writer::FbxWriter;
//! use draco_io::Writer;
//!
//! let mesh = draco_core::mesh::Mesh::new();
//! let mut writer = FbxWriter::new();
//! writer.add_mesh(&mesh, Some("MyMesh"))?;
//! writer.write("output.fbx")?;
//!
//! // With compression (requires 'compression' feature)
//! let mut writer = FbxWriter::new().with_compression(true);
//! writer.add_mesh(&mesh, Some("MyMesh"))?;
//! writer.write("output_compressed.fbx")?;
//! # Ok::<(), std::io::Error>(())
//! ```

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufWriter, Cursor, Write};
use std::path::Path;

use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;

use crate::fbx_ascii_syntax::{name_class, FBX_VERSION};
use crate::fbx_encoder::{encode_node, write_footer, write_null_record, WriterOptions, FBX_MAGIC};
use crate::fbx_node::{FbxNode, FbxProperty};
use crate::traits::{WriteToBytes, Writer};

/// FBX binary format writer.
///
/// This struct provides a builder-style API for writing FBX files.
/// Meshes are added via `add_mesh()`, then written with `write()`.
///
/// # Example
///
/// ```no_run
/// use draco_io::fbx_writer::FbxWriter;
/// use draco_io::Writer;
/// # let mesh = draco_core::mesh::Mesh::new();
///
/// let mut writer = FbxWriter::new()
///     .with_compression(true)
///     .with_compression_threshold(64);
///
/// writer.add_mesh(&mesh, Some("CubeMesh"))?;
/// writer.write("output.fbx")?;
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct FbxWriter {
    /// Whether to compress arrays using zlib (requires `compression` feature).
    compress: bool,
    /// Minimum array size (in bytes) to consider for compression.
    compression_threshold: usize,
    /// Meshes to write, with optional names.
    meshes: Vec<MeshData>,
    /// FBX model nodes, including nodes without attached geometry.
    models: Vec<ModelData>,
    /// Material objects to write.
    materials: Vec<MaterialData>,
    /// Texture objects to write (one per embedded/external image).
    textures: Vec<TextureData>,
    /// Animation stacks/layers/curvenodes/curves to write.
    anim: Vec<AnimStackData>,
    /// Skin objects and their clusters.
    skins: Vec<SkinData>,
    morphs: Vec<MorphData>,
    /// Source-only settings used for semantic FBX re-export.
    global_settings: Option<crate::fbx_scene::FbxGlobalSettings>,
    /// Scene node ids that must be emitted as FBX LimbNode models.
    joint_scene_ids: HashSet<crate::fbx_scene::FbxNodeId>,
    /// Pending object-property connections emitted after `write_objects`.
    connections: Vec<PendingConnection>,
    /// ID allocator for generating unique object IDs.
    next_id: i64,
}

/// Internal mesh data storage.
#[derive(Debug, Clone)]
struct MeshData {
    vertices: Vec<f64>,
    indices: Vec<i32>,
    name: String,
    geometry_id: i64,
    model_id: i64,
    /// Optional normals (3 floats per control point), when present on the
    /// source Draco mesh.
    normals: Option<Vec<f64>>,
    /// Optional UVs (2 floats per control point).
    uvs: Option<Vec<f64>>,
    /// Per-triangle indices into the materials connected to the model.
    material_indices: Vec<i32>,
    control_points: Option<Vec<f64>>,
    polygon_vertex_indices: Option<Vec<i32>>,
    uv_sets: Vec<crate::fbx_scene::FbxUvSet>,
    normal_sets: Vec<crate::fbx_scene::FbxNormalSet>,
    color_sets: Vec<crate::fbx_scene::FbxColorSet>,
    tangent_sets: Vec<crate::fbx_scene::FbxTangentSet>,
    binormal_sets: Vec<crate::fbx_scene::FbxBinormalSet>,
    smoothing_layers: Vec<crate::fbx_scene::FbxSmoothingLayer>,
    crease_layers: Vec<crate::fbx_scene::FbxCreaseLayer>,
    edges: Vec<i32>,
}

/// Internal FBX Model data.
#[derive(Debug, Clone)]
struct ModelData {
    name: String,
    model_id: i64,
    /// Stable document-local id supplied by FbxScene, when available.
    scene_node_id: Option<crate::fbx_scene::FbxNodeId>,
    parent_id: Option<i64>,
    transform: Option<crate::fbx_scene::FbxTransform>,
    transform_stack: Option<crate::fbx_scene::FbxTransformStack>,
    /// Material ids connected to this model via `OO`.
    material_ids: Vec<i64>,
    class: &'static str,
}

#[derive(Debug, Clone)]
struct SkinData {
    skin_id: i64,
    pose_id: i64,
    geometry_id: i64,
    clusters: Vec<SkinClusterData>,
    bind_pose: Vec<(crate::fbx_scene::FbxNodeId, crate::fbx_scene::FbxTransform)>,
}

#[derive(Debug, Clone)]
struct SkinClusterData {
    cluster_id: i64,
    source: crate::fbx_scene::FbxSkinCluster,
}

#[derive(Debug, Clone)]
struct MorphData {
    blend_shape_id: i64,
    geometry_id: i64,
    model_id: i64,
    targets: Vec<MorphTargetData>,
}

#[derive(Debug, Clone)]
struct MorphTargetData {
    channel_id: i64,
    shape_geometry_id: i64,
    source: crate::fbx_scene::FbxMorphTarget,
}

/// Internal FBX Material data.
#[derive(Debug, Clone)]
struct MaterialData {
    material_id: i64,
    source: crate::fbx_scene::FbxMaterial,
}

/// Internal FBX Texture data.
#[derive(Debug, Clone)]
struct TextureData {
    texture_id: i64,
    video_id: i64,
    source: crate::fbx_scene::FbxTexture,
}

/// Internal animation container for one stack.
#[derive(Debug, Clone)]
struct AnimStackData {
    stack_id: i64,
    layer_id: i64,
    name: Option<String>,
    duration: f32,
    channels: Vec<crate::fbx_scene::FbxAnimChannel>,
}

/// A connection that will be emitted in the `Connections` section.
#[derive(Debug, Clone)]
struct PendingConnection {
    kind: &'static str,
    child: i64,
    parent: i64,
    property: Option<String>,
}

impl Default for FbxWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl FbxWriter {
    /// Create a new FBX writer with default settings.
    pub fn new() -> Self {
        Self {
            compress: false,
            compression_threshold: 128,
            meshes: Vec::new(),
            models: Vec::new(),
            materials: Vec::new(),
            textures: Vec::new(),
            anim: Vec::new(),
            skins: Vec::new(),
            morphs: Vec::new(),
            global_settings: None,
            joint_scene_ids: HashSet::new(),
            connections: Vec::new(),
            next_id: 1000, // Start at 1000 to avoid reserved IDs (0 = root)
        }
    }

    /// Enable or disable zlib compression for arrays.
    ///
    /// Compression is only applied if the `compression` feature is enabled
    /// and the array size exceeds the compression threshold.
    pub fn with_compression(mut self, compress: bool) -> Self {
        self.compress = compress;
        self
    }

    /// Set the minimum byte size for arrays to be compressed.
    ///
    /// Arrays smaller than this threshold will not be compressed even
    /// if compression is enabled. Default is 128 bytes.
    pub fn with_compression_threshold(mut self, threshold: usize) -> Self {
        self.compression_threshold = threshold;
        self
    }

    /// Allocate a unique ID for an object.
    fn allocate_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn add_model(
        &mut self,
        name: String,
        parent_id: Option<i64>,
        transform: Option<crate::fbx_scene::FbxTransform>,
        material_ids: Vec<i64>,
    ) -> i64 {
        self.add_model_with_scene_id(name, parent_id, transform, None, material_ids, None)
    }

    fn add_model_with_scene_id(
        &mut self,
        name: String,
        parent_id: Option<i64>,
        transform: Option<crate::fbx_scene::FbxTransform>,
        transform_stack: Option<crate::fbx_scene::FbxTransformStack>,
        material_ids: Vec<i64>,
        scene_node_id: Option<crate::fbx_scene::FbxNodeId>,
    ) -> i64 {
        let model_id = self.allocate_id();
        self.models.push(ModelData {
            name,
            model_id,
            scene_node_id,
            parent_id,
            transform,
            transform_stack,
            material_ids,
            class: if scene_node_id
                .map(|id| self.joint_scene_ids.contains(&id))
                .unwrap_or(false)
            {
                "LimbNode"
            } else {
                "Mesh"
            },
        });
        model_id
    }

    #[allow(clippy::too_many_arguments)]
    fn add_mesh_to_model(
        &mut self,
        mesh: &Mesh,
        name: &str,
        model_id: i64,
        material_indices: &[i32],
        skin: Option<crate::fbx_scene::FbxSkin>,
        morph_targets: &[crate::fbx_scene::FbxMorphTarget],
        layers: crate::fbx_render_mesh::FbxGeometryLayers<'_>,
        edges: &[i32],
    ) -> io::Result<()> {
        let crate::fbx_render_mesh::FbxGeometryLayers {
            control_points,
            polygon_vertex_indices,
            uv_sets,
            normal_sets,
            color_sets,
            tangent_sets,
            binormal_sets,
            smoothing_layers,
            crease_layers,
        } = layers;
        validate_supported_fbx_attributes(mesh)?;
        let geometry_id = self.allocate_id();
        // `LayerElementMaterial` indexes polygons. This writer emits one
        // triangle per polygon, so retain the corresponding prefix directly.
        let material_indices = material_indices
            .iter()
            .copied()
            .take(mesh.num_faces())
            .collect();
        self.meshes.push(MeshData {
            vertices: extract_vertices(mesh),
            indices: extract_polygon_indices(mesh),
            name: name.to_string(),
            geometry_id,
            model_id,
            normals: extract_normals(mesh),
            uvs: extract_uvs(mesh),
            material_indices,
            control_points: (!control_points.is_empty()).then(|| {
                control_points
                    .iter()
                    .flat_map(|point| point.iter().map(|value| f64::from(*value)))
                    .collect()
            }),
            polygon_vertex_indices: (!polygon_vertex_indices.is_empty())
                .then(|| polygon_vertex_indices.to_vec()),
            uv_sets: uv_sets.to_vec(),
            normal_sets: normal_sets.to_vec(),
            color_sets: color_sets.to_vec(),
            tangent_sets: tangent_sets.to_vec(),
            binormal_sets: binormal_sets.to_vec(),
            edges: edges.to_vec(),
            smoothing_layers: smoothing_layers.to_vec(),
            crease_layers: crease_layers.to_vec(),
        });
        if let Some(skin) = skin {
            let skin_id = self.allocate_id();
            let clusters = skin
                .clusters
                .iter()
                .cloned()
                .map(|source| SkinClusterData {
                    cluster_id: self.allocate_id(),
                    source,
                })
                .collect();
            let pose_id = self.allocate_id();
            self.skins.push(SkinData {
                skin_id,
                pose_id,
                geometry_id,
                clusters,
                bind_pose: skin.bind_pose,
            });
        }
        if !morph_targets.is_empty() {
            let blend_shape_id = self.allocate_id();
            let targets = morph_targets
                .iter()
                .cloned()
                .map(|source| MorphTargetData {
                    channel_id: self.allocate_id(),
                    shape_geometry_id: self.allocate_id(),
                    source,
                })
                .collect();
            self.morphs.push(MorphData {
                blend_shape_id,
                geometry_id,
                model_id,
                targets,
            });
        }
        Ok(())
    }

    /// Adds a hierarchy, materials, textures, and animation read by
    /// [`crate::FbxReader::read_scene`].
    ///
    /// Mesh geometry, model names, parent-child relationships, local affine TRS
    /// transforms, Phong/Lambert materials, textures, per-mesh material
    /// indices, and node-TRS animation are written. FBX pivots and inheritance
    /// rules are not represented by [`crate::FbxTransform`] and are therefore
    /// not emitted.
    pub fn add_scene(&mut self, scene: &crate::FbxScene) -> io::Result<()> {
        self.global_settings = scene.global_settings.clone();
        fn collect_joint_ids(
            node: &crate::fbx_scene::FbxSceneNode,
            ids: &mut HashSet<crate::fbx_scene::FbxNodeId>,
        ) {
            for mesh in &node.mesh_instances {
                if let Some(skin) = &mesh.skin {
                    ids.extend(skin.clusters.iter().map(|cluster| cluster.joint_node_id));
                }
            }
            for child in &node.children {
                collect_joint_ids(child, ids);
            }
        }
        for node in &scene.root_nodes {
            collect_joint_ids(node, &mut self.joint_scene_ids);
        }
        // Allocate stable ids for every material and texture first so the
        // scene traversal can resolve mesh material indices.
        let material_ids: Vec<i64> = scene
            .materials
            .iter()
            .map(|material| {
                let id = self.allocate_id();
                self.materials.push(MaterialData {
                    material_id: id,
                    source: material.clone(),
                });
                id
            })
            .collect();

        for texture in &scene.textures {
            let texture_id = self.allocate_id();
            let video_id = self.allocate_id();
            self.textures.push(TextureData {
                texture_id,
                video_id,
                source: texture.clone(),
            });
        }
        // Map scene texture index -> (texture_id, video_id) for material links.
        let texture_ids: Vec<(i64, i64)> = self
            .textures
            .iter()
            .map(|t| (t.texture_id, t.video_id))
            .collect();

        // Walk the scene node tree. Each node is emitted as a Model; its mesh
        // instances become Geometry nodes connected to the Model.
        for node in &scene.root_nodes {
            self.add_scene_node(node, None, &material_ids, &texture_ids)?;
        }

        // Emit OP connections for material->texture bindings now that all
        // material and texture ids are known.
        for (mat_data, &mat_id) in self.materials.iter().zip(material_ids.iter()) {
            for binding in &mat_data.source.textures {
                if let Some(&(tex_id, _video_id)) = texture_ids.get(binding.texture_index) {
                    self.connections.push(PendingConnection {
                        kind: "OP",
                        child: tex_id,
                        parent: mat_id,
                        property: Some(binding.slot.property_name().to_string()),
                    });
                }
            }
        }

        // Animation. Each FbxAnimation becomes one stack + one layer; each
        // channel becomes a curve node with up to three curves.
        for animation in &scene.animations {
            let stack_id = self.allocate_id();
            let layer_id = self.allocate_id();
            self.anim.push(AnimStackData {
                stack_id,
                layer_id,
                name: animation.name.clone(),
                duration: animation.duration,
                channels: animation.channels.clone(),
            });
        }

        Ok(())
    }

    fn add_scene_node(
        &mut self,
        node: &crate::fbx_scene::FbxSceneNode,
        parent_id: Option<i64>,
        material_ids: &[i64],
        texture_ids: &[(i64, i64)],
    ) -> io::Result<()> {
        // Resolve which materials apply to this node. The FBX writer connects
        // materials to the model; we attach the unique set referenced by any
        // mesh instance under this node.
        // FBX LayerElementMaterial indices address the material slots on the
        // owning Model, not the document-wide material table. Build that
        // stable per-model slot table first, then remap each polygon index.
        let referenced_material_indices: std::collections::BTreeSet<usize> = node
            .mesh_instances
            .iter()
            .flat_map(|mesh| mesh.material_indices.iter())
            .filter_map(|&idx| usize::try_from(idx).ok())
            .filter(|&idx| idx < material_ids.len())
            .collect();
        let referenced_material_indices: Vec<usize> =
            referenced_material_indices.into_iter().collect();
        let referenced_material_ids: Vec<i64> = referenced_material_indices
            .iter()
            .map(|&idx| material_ids[idx])
            .collect();
        let model_id = self.add_model_with_scene_id(
            node.name.clone().unwrap_or_else(|| "Node".to_string()),
            parent_id,
            node.transform,
            node.transform_stack.clone(),
            referenced_material_ids.clone(),
            Some(node.id),
        );
        // We need a mutable borrow of self.connections but add_mesh_to_model
        // borrows self mutably too; collect geometry first.
        let mesh_count = node.mesh_instances.len();
        for mesh_instance in &node.mesh_instances {
            // This names the `Geometry` object only; the `Model` keeps its own
            // name. An unnamed Geometry stays unnamed rather than borrowing
            // the model's, which a read/write cycle would otherwise invent.
            let name = mesh_instance.name.as_deref().unwrap_or("");
            // `material_indices` addresses the scene material list; the
            // written layer addresses the slots connected to this Model.
            //
            // A file can carry a material layer with no `Material` objects at
            // all (Revit exports do). There is nothing to map onto then, so
            // pass the slots through instead of collapsing them to zero, which
            // would erase the face grouping the layer encodes.
            let local_material_indices = if referenced_material_indices.is_empty() {
                mesh_instance.material_indices.clone()
            } else {
                mesh_instance
                    .material_indices
                    .iter()
                    .map(|&index| {
                        usize::try_from(index)
                            .ok()
                            .and_then(|global| {
                                referenced_material_indices
                                    .iter()
                                    .position(|&slot| slot == global)
                            })
                            .map(|local| local as i32)
                            .unwrap_or(0)
                    })
                    .collect::<Vec<_>>()
            };
            self.add_mesh_to_model(
                &mesh_instance.mesh,
                name,
                model_id,
                &local_material_indices,
                mesh_instance.skin.clone(),
                &mesh_instance.morph_targets,
                crate::fbx_render_mesh::FbxGeometryLayers::from_instance(mesh_instance),
                &mesh_instance.edges,
            )?;
        }
        let _ = mesh_count;
        let _ = texture_ids;
        for child in &node.children {
            self.add_scene_node(child, Some(model_id), material_ids, texture_ids)?;
        }
        Ok(())
    }

    /// Add a mesh to be written.
    /// Write the FBX file to the given path.
    pub fn write<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        self.write_to(&mut writer)
    }

    /// Assembles the whole document as a tree of records.
    ///
    /// Everything about what this file contains is decided here; nothing in
    /// it knows how a record is spelled in bytes. `encode_node` is the only
    /// thing that does, which is what makes a second container backend a
    /// matter of writing one printer rather than a second document builder.
    fn build_document(&self) -> io::Result<Vec<FbxNode>> {
        let mut document = vec![
            header_extension_node(),
            global_settings_node(self.global_settings.as_ref()),
            documents_node(),
            definitions_node(
                &self.meshes,
                &self.models,
                &self.materials,
                &self.textures,
                &self.anim,
                &self.skins,
                &self.morphs,
            ),
        ];
        document.push(objects_node(
            &self.meshes,
            &self.models,
            &self.materials,
            &self.textures,
            &self.anim,
            &self.skins,
            &self.morphs,
        )?);
        document.push(connections_node(
            &self.models,
            &self.meshes,
            &self.textures,
            &self.anim,
            &self.connections,
            &self.skins,
            &self.morphs,
        ));
        Ok(document)
    }

    /// Write the FBX data to a writer.
    ///
    /// Takes only `Write`: the backpatched node header needs to seek, but it
    /// does that inside a buffer of its own rather than in the caller's sink.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(&self.write_to_vec()?)
    }

    /// Write the FBX data into a byte vector.
    pub fn write_to_vec(&self) -> io::Result<Vec<u8>> {
        let options = WriterOptions {
            compress: self.compress,
            compression_threshold: self.compression_threshold,
        };
        let is_64 = FBX_VERSION >= 7500;

        let mut cursor = Cursor::new(Vec::new());
        cursor.write_all(FBX_MAGIC)?;
        cursor.write_all(&[0x1A, 0x00])?; // Reserved bytes
        cursor.write_all(&FBX_VERSION.to_le_bytes())?;
        for node in self.build_document()? {
            encode_node(&mut cursor, &node, is_64, &options)?;
        }
        // Marks the end of the top-level nodes.
        write_null_record(&mut cursor, is_64)?;
        write_footer(&mut cursor)?;
        Ok(cursor.into_inner())
    }

    /// Get the number of meshes added.
    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    /// Check if compression is enabled.
    pub fn is_compression_enabled(&self) -> bool {
        self.compress
    }
}

// ============================================================================
// Trait Implementations
// ============================================================================

impl Writer for FbxWriter {
    fn new() -> Self {
        Self::default()
    }

    fn add_mesh(&mut self, mesh: &Mesh, name: Option<&str>) -> io::Result<()> {
        let name = name.unwrap_or("Mesh").to_string();
        let model_id = self.add_model(name.clone(), None, None, Vec::new());
        self.add_mesh_to_model(
            mesh,
            &name,
            model_id,
            &[],
            None,
            &[],
            // A flat Draco mesh carries no FBX layer elements of its own; the
            // writer derives normals and UVs from its attributes instead.
            crate::fbx_render_mesh::FbxGeometryLayers::default(),
            &[],
        )
    }

    fn write<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        self.write(path)
    }

    fn vertex_count(&self) -> usize {
        self.meshes.iter().map(|m| m.vertices.len() / 3).sum()
    }

    fn face_count(&self) -> usize {
        self.meshes.iter().map(|m| m.indices.len() / 3).sum()
    }
}

impl WriteToBytes for FbxWriter {
    fn write_to_vec(&self) -> io::Result<Vec<u8>> {
        FbxWriter::write_to_vec(self)
    }
}

// ============================================================================
// Convenience Functions (for backward compatibility)
// ============================================================================

/// Write a mesh to a binary FBX file.
///
/// This is a convenience function. For more control, use `FbxWriter` directly.
pub fn write_fbx_mesh<P: AsRef<Path>>(path: P, mesh: &Mesh) -> io::Result<()> {
    let mut writer = FbxWriter::new();
    Writer::add_mesh(&mut writer, mesh, None)?;
    writer.write(path)
}

/// Write a mesh to a binary FBX file with compression.
///
/// This is a convenience function. For more control, use `FbxWriter` directly.
#[cfg(feature = "compression")]
pub fn write_fbx_mesh_compressed<P: AsRef<Path>>(path: P, mesh: &Mesh) -> io::Result<()> {
    let mut writer = FbxWriter::new().with_compression(true);
    Writer::add_mesh(&mut writer, mesh, None)?;
    writer.write(path)
}

// ============================================================================
// FBX Section Writers
// ============================================================================

fn header_extension_node() -> FbxNode {
    FbxNode {
        name: "FBXHeaderExtension".to_string(),
        properties: Vec::new(),
        children: vec![
            value_node("FBXHeaderVersion", FbxProperty::I32(1003)),
            value_node("FBXVersion", FbxProperty::I32(FBX_VERSION as i32)),
            value_node("Creator", FbxProperty::String("draco-io-rs".to_string())),
        ],
    }
}

fn global_settings_node(source: Option<&crate::fbx_scene::FbxGlobalSettings>) -> FbxNode {
    let source = source.cloned().unwrap_or_default();
    let mut properties = vec![
        int_property_node("UpAxis", "int", "Integer", "", source.up_axis.unwrap_or(2)),
        int_property_node(
            "UpAxisSign",
            "int",
            "Integer",
            "",
            source.up_axis_sign.unwrap_or(1),
        ),
        int_property_node(
            "FrontAxis",
            "int",
            "Integer",
            "",
            source.front_axis.unwrap_or(1),
        ),
        int_property_node(
            "FrontAxisSign",
            "int",
            "Integer",
            "",
            source.front_axis_sign.unwrap_or(-1),
        ),
        int_property_node(
            "CoordAxis",
            "int",
            "Integer",
            "",
            source.coord_axis.unwrap_or(0),
        ),
        int_property_node(
            "CoordAxisSign",
            "int",
            "Integer",
            "",
            source.coord_axis_sign.unwrap_or(1),
        ),
        // FBX `UnitScaleFactor = 1.0` is documented to mean *centimeters*
        // (Blender io_scene_fbx comment: "FBX default base unit seems to be
        // the centimeter"). Most authoring tools therefore write the
        // number of centimeters-per-meter (100) here, and Blender reads
        // the file with `multiplier = UnitScaleFactor / 100`. A value of
        // 100 makes the file round-trip at its true (meter) scale; the
        // legacy value of 1.0 caused every imported scene to come in
        // 100x too small.
        f64_property_node(
            "UnitScaleFactor",
            "double",
            "Number",
            "",
            source.unit_scale_factor.unwrap_or(100.0),
        ),
        f64_property_node(
            "OriginalUnitScaleFactor",
            "double",
            "Number",
            "",
            source.original_unit_scale_factor.unwrap_or(100.0),
        ),
    ];
    if let Some(time_mode) = source.time_mode {
        properties.push(int_property_node("TimeMode", "enum", "", "", time_mode));
    }

    FbxNode {
        name: "GlobalSettings".to_string(),
        properties: Vec::new(),
        children: vec![
            value_node("Version", FbxProperty::I32(1000)),
            properties70_node(properties),
        ],
    }
}

/// Wraps `P` records in the `Properties70` node that holds them.
fn properties70_node(properties: Vec<FbxNode>) -> FbxNode {
    FbxNode {
        name: "Properties70".to_string(),
        properties: Vec::new(),
        children: properties,
    }
}

/// Builds one `Properties70` `P` record.
///
/// Every `P` node has the same shape: four strings naming the property, its
/// declared FBX type, a secondary type and a flag string, followed by as many
/// value properties as the type calls for.
fn property_node(
    name: &str,
    type1: &str,
    type2: &str,
    flags: &str,
    values: Vec<FbxProperty>,
) -> FbxNode {
    let mut properties = vec![
        FbxProperty::String(name.to_string()),
        FbxProperty::String(type1.to_string()),
        FbxProperty::String(type2.to_string()),
        FbxProperty::String(flags.to_string()),
    ];
    properties.extend(values);
    FbxNode {
        name: "P".to_string(),
        properties,
        children: Vec::new(),
    }
}

fn int_property_node(name: &str, type1: &str, type2: &str, flags: &str, value: i32) -> FbxNode {
    property_node(name, type1, type2, flags, vec![FbxProperty::I32(value)])
}

fn bool_property_node(name: &str, value: bool) -> FbxNode {
    // Blender's FBX property helper expects `RotationActive` to carry an
    // INT32 scalar even though its declared property type is `bool`.
    property_node(
        name,
        "bool",
        "",
        "",
        vec![FbxProperty::I32(i32::from(value))],
    )
}

fn f64_property_node(name: &str, type1: &str, type2: &str, flags: &str, value: f64) -> FbxNode {
    property_node(name, type1, type2, flags, vec![FbxProperty::F64(value)])
}

/// A three-component property, whose declared type is its own name.
///
/// That is right for `Lcl Translation` and the rest of the transform stack,
/// and wrong for anything else -- a real `Vector3D` property would say so.
/// Nothing but transform properties goes through here today.
fn vec3_property_node(name: &str, values: [f64; 3]) -> FbxNode {
    property_node(
        name,
        name,
        "",
        "A",
        values.into_iter().map(FbxProperty::F64).collect(),
    )
}

fn decompose_transform(
    transform: crate::fbx_scene::FbxTransform,
) -> io::Result<([f64; 3], [f64; 3], [f64; 3])> {
    let matrix = transform.matrix.map(|row| row.map(f64::from));
    if !matrix.iter().flatten().all(|value| value.is_finite()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "FBX transform contains a non-finite value",
        ));
    }
    if matrix[0][3].abs() > 1e-5
        || matrix[1][3].abs() > 1e-5
        || matrix[2][3].abs() > 1e-5
        || (matrix[3][3] - 1.0).abs() > 1e-5
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "FBX scene export requires an affine transform matrix",
        ));
    }

    // FbxTransform is packed column-major. Convert its linear part to the
    // conventional row-major form used by the decomposition below.
    let mut rotation = [
        [matrix[0][0], matrix[1][0], matrix[2][0]],
        [matrix[0][1], matrix[1][1], matrix[2][1]],
        [matrix[0][2], matrix[1][2], matrix[2][2]],
    ];
    let mut scaling = [
        (rotation[0][0].powi(2) + rotation[1][0].powi(2) + rotation[2][0].powi(2)).sqrt(),
        (rotation[0][1].powi(2) + rotation[1][1].powi(2) + rotation[2][1].powi(2)).sqrt(),
        (rotation[0][2].powi(2) + rotation[1][2].powi(2) + rotation[2][2].powi(2)).sqrt(),
    ];
    if scaling.iter().any(|value| *value < 1e-8) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "FBX scene export cannot decompose a zero-scale transform",
        ));
    }
    for column in 0..3 {
        for row in &mut rotation {
            row[column] /= scaling[column];
        }
    }

    let determinant = rotation[0][0]
        * (rotation[1][1] * rotation[2][2] - rotation[1][2] * rotation[2][1])
        - rotation[0][1] * (rotation[1][0] * rotation[2][2] - rotation[1][2] * rotation[2][0])
        + rotation[0][2] * (rotation[1][0] * rotation[2][1] - rotation[1][1] * rotation[2][0]);
    if determinant < 0.0 {
        scaling[0] = -scaling[0];
        for row in &mut rotation {
            row[0] = -row[0];
        }
    }

    let dot01 = rotation[0][0] * rotation[0][1]
        + rotation[1][0] * rotation[1][1]
        + rotation[2][0] * rotation[2][1];
    let dot02 = rotation[0][0] * rotation[0][2]
        + rotation[1][0] * rotation[1][2]
        + rotation[2][0] * rotation[2][2];
    let dot12 = rotation[0][1] * rotation[0][2]
        + rotation[1][1] * rotation[1][2]
        + rotation[2][1] * rotation[2][2];
    if dot01.abs() > 1e-4 || dot02.abs() > 1e-4 || dot12.abs() > 1e-4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "FBX scene export cannot represent transform shear",
        ));
    }

    let y = (-rotation[2][0]).asin();
    let (x, z) = if y.cos().abs() > 1e-6 {
        (
            rotation[2][1].atan2(rotation[2][2]),
            rotation[1][0].atan2(rotation[0][0]),
        )
    } else {
        ((-rotation[1][2]).atan2(rotation[1][1]), 0.0)
    };
    Ok((
        [matrix[3][0], matrix[3][1], matrix[3][2]],
        [x.to_degrees(), y.to_degrees(), z.to_degrees()],
        scaling,
    ))
}

fn documents_node() -> FbxNode {
    FbxNode {
        name: "Documents".to_string(),
        properties: Vec::new(),
        children: vec![
            value_node("Count", FbxProperty::I32(1)),
            FbxNode {
                name: "Document".to_string(),
                properties: vec![
                    FbxProperty::I64(0), // Document ID (0 for root)
                    FbxProperty::String(String::new()),
                    FbxProperty::String("Scene".to_string()),
                ],
                children: Vec::new(),
            },
        ],
    }
}

/// Declares how many objects of each type the document holds.
///
/// The `Count` node is the number of `ObjectType` blocks, which used to be a
/// hand-maintained literal that had to be kept in step with the block list by
/// eye. Building the list first makes it the list's length.
// Takes one slice per FBX object type so it can emit an accurate `Count`.
#[allow(clippy::too_many_arguments)]
fn definitions_node(
    meshes: &[MeshData],
    models: &[ModelData],
    materials: &[MaterialData],
    textures: &[TextureData],
    anim: &[AnimStackData],
    skins: &[SkinData],
    morphs: &[MorphData],
) -> FbxNode {
    let limb_nodes = models
        .iter()
        .filter(|model| model.class == "LimbNode")
        .count();
    let shape_count = morphs
        .iter()
        .map(|morph| morph.targets.len())
        .sum::<usize>();
    let curve_nodes: usize = anim.iter().map(|stack| stack.channels.len()).sum();
    // Each channel produces one curve node and its path's component curves.
    let curve_count: usize = anim
        .iter()
        .flat_map(|stack| &stack.channels)
        .map(|channel| channel.path.component_count())
        .sum();

    let mut object_types = vec![
        object_type_node("Geometry", (meshes.len() + shape_count) as i32),
        object_type_node("Model", models.len() as i32),
    ];
    if limb_nodes > 0 {
        // Blender 5's native importer uses this explicit companion to
        // identify a Model::LimbNode as a skeleton bone.
        object_types.push(object_type_node("NodeAttribute", limb_nodes as i32));
    }
    object_types.push(object_type_node("Material", materials.len() as i32));
    object_types.push(object_type_node("Texture", textures.len() as i32));
    object_types.push(object_type_node("Video", textures.len() as i32));
    object_types.push(object_type_node("AnimationStack", anim.len() as i32));
    // One layer per stack.
    object_types.push(object_type_node("AnimationLayer", anim.len() as i32));
    object_types.push(object_type_node("AnimationCurveNode", curve_nodes as i32));
    object_types.push(object_type_node("AnimationCurve", curve_count as i32));
    if !skins.is_empty() || !morphs.is_empty() {
        let deformer_count = skins.len()
            + skins.iter().map(|skin| skin.clusters.len()).sum::<usize>()
            + morphs.len()
            + morphs
                .iter()
                .map(|morph| morph.targets.len())
                .sum::<usize>();
        object_types.push(object_type_node("Deformer", deformer_count as i32));
        object_types.push(object_type_node("Pose", skins.len() as i32));
    }

    let mut children = vec![
        value_node("Version", FbxProperty::I32(100)),
        value_node("Count", FbxProperty::I32(object_types.len() as i32)),
    ];
    children.extend(object_types);
    FbxNode {
        name: "Definitions".to_string(),
        properties: Vec::new(),
        children,
    }
}

fn object_type_node(type_name: &str, count: i32) -> FbxNode {
    FbxNode {
        name: "ObjectType".to_string(),
        properties: vec![FbxProperty::String(type_name.to_string())],
        children: vec![value_node("Count", FbxProperty::I32(count))],
    }
}

// Takes one slice per FBX object type; see `definitions_node`.
#[allow(clippy::too_many_arguments)]
fn objects_node(
    meshes: &[MeshData],
    models: &[ModelData],
    materials: &[MaterialData],
    textures: &[TextureData],
    anim: &[AnimStackData],
    skins: &[SkinData],
    morphs: &[MorphData],
) -> io::Result<FbxNode> {
    let mut children = Vec::new();
    for mesh_data in meshes {
        children.push(geometry_node(mesh_data));
    }
    for model_data in models {
        children.push(model_node(model_data)?);
    }
    for model_data in models.iter().filter(|model| model.class == "LimbNode") {
        children.push(limb_node_attribute_node(model_data));
    }
    for material_data in materials {
        children.push(material_node(material_data));
    }
    for texture_data in textures {
        children.push(texture_node(texture_data));
        children.push(video_node(texture_data));
    }
    for stack in anim {
        children.extend(animation_stack_nodes(stack));
    }
    for skin in skins {
        children.extend(skin_nodes(skin, models));
    }
    for morph in morphs {
        children.extend(morph_nodes(morph));
    }

    Ok(FbxNode {
        name: "Objects".to_string(),
        properties: Vec::new(),
        children,
    })
}

fn model_node(model_data: &ModelData) -> io::Result<FbxNode> {
    let mut properties = Vec::new();
    if let Some(transform) = model_data.transform {
        let (matrix_translation, matrix_rotation, matrix_scaling) = decompose_transform(transform)?;
        let as_f64 = |value: [f32; 3]| value.map(f64::from);
        if let Some(stack) = model_data.transform_stack.as_ref() {
            // A source stack deliberately retains property presence: an
            // omitted Lcl property is an authored FBX default, not an
            // invitation to synthesize a decomposition from the semantic local
            // matrix (which may already include pre/post rotation).
            for (name, value) in [
                ("Lcl Translation", stack.translation),
                ("Lcl Rotation", stack.rotation),
                ("Lcl Scaling", stack.scaling),
            ] {
                if let Some(value) = value {
                    properties.push(vec3_property_node(name, as_f64(value)));
                }
            }
            if let Some(value) = stack.rotation_order {
                properties.push(int_property_node("RotationOrder", "enum", "", "", value));
            }
            if let Some(value) = stack.rotation_active {
                properties.push(bool_property_node("RotationActive", value));
            }
            for (name, value) in [
                ("PreRotation", stack.pre_rotation),
                ("PostRotation", stack.post_rotation),
                ("RotationOffset", stack.rotation_offset),
                ("RotationPivot", stack.rotation_pivot),
                ("ScalingOffset", stack.scaling_offset),
                ("ScalingPivot", stack.scaling_pivot),
            ] {
                if let Some(value) = value {
                    properties.push(vec3_property_node(name, as_f64(value)));
                }
            }
            if let Some(value) = stack.inherit_type {
                properties.push(int_property_node("InheritType", "enum", "", "", value));
            }
        } else {
            properties.push(vec3_property_node("Lcl Translation", matrix_translation));
            properties.push(vec3_property_node("Lcl Rotation", matrix_rotation));
            properties.push(vec3_property_node("Lcl Scaling", matrix_scaling));
        }
    }

    Ok(FbxNode {
        name: "Model".to_string(),
        properties: vec![
            FbxProperty::I64(model_data.model_id),
            FbxProperty::String(name_class(&model_data.name, "Model")),
            FbxProperty::String(model_data.class.to_string()),
        ],
        children: vec![
            value_node("Version", FbxProperty::I32(232)),
            properties70_node(properties),
            value_node("Shading", FbxProperty::I16(1)),
            value_node("Culling", FbxProperty::String("CullingOff".to_string())),
        ],
    })
}

/// FBX separates a limb's Model transform from its Skeleton node attribute.
/// Without this object Blender 5 imports animated joints as plain empties and
/// cannot create an armature modifier for their skin clusters.
fn limb_node_attribute_node(model_data: &ModelData) -> FbxNode {
    FbxNode {
        name: "NodeAttribute".to_string(),
        properties: vec![
            FbxProperty::I64(limb_node_attribute_id(model_data.model_id)),
            FbxProperty::String(name_class(&model_data.name, "NodeAttribute")),
            FbxProperty::String("LimbNode".to_string()),
        ],
        children: vec![value_node(
            "TypeFlags",
            FbxProperty::String("Skeleton".to_string()),
        )],
    }
}

fn limb_node_attribute_id(model_id: i64) -> i64 {
    // Writer-allocated object ids are small positive integers; animation ids
    // use a separate 2-million-plus hash range. Reserve a distinct stable
    // range for synthetic skeleton attributes.
    1_000_000_000i64.saturating_add(model_id)
}

/// Flatten a transform to the 16-value FBX matrix layout.
///
/// `FbxTransform` already uses the packed 16-value FBX matrix layout.
fn flatten_fbx_transform(transform: crate::fbx_scene::FbxTransform) -> Vec<f64> {
    transform
        .matrix
        .into_iter()
        .flatten()
        .map(f64::from)
        .collect()
}

/// Builds a Skin, its Clusters and an explicit BindPose -- siblings, not one
/// node.
///
/// Blender's native importer resolves clusters through the geometry and joint
/// Model connections. BindPose is emitted independently so rest transforms do
/// not depend on a frame-zero animation sample.
fn skin_nodes(skin: &SkinData, models: &[ModelData]) -> Vec<FbxNode> {
    let mut nodes = vec![FbxNode {
        name: "Deformer".to_string(),
        properties: vec![
            FbxProperty::I64(skin.skin_id),
            FbxProperty::String(name_class("Skin", "Deformer")),
            FbxProperty::String("Skin".to_string()),
        ],
        children: vec![
            value_node("Version", FbxProperty::I32(101)),
            value_node("Link_DeformAcuracy", FbxProperty::F64(50.0)),
        ],
    }];

    for cluster in &skin.clusters {
        let source = &cluster.source;
        let mut children = vec![
            value_node("Version", FbxProperty::I32(100)),
            FbxNode {
                name: "UserData".to_string(),
                properties: vec![
                    FbxProperty::String(String::new()),
                    FbxProperty::String(String::new()),
                ],
                children: Vec::new(),
            },
            value_node(
                "Indexes",
                FbxProperty::I32Array(
                    source
                        .control_point_indices
                        .iter()
                        .map(|&index| index as i32)
                        .collect(),
                ),
            ),
            value_node(
                "Weights",
                FbxProperty::F64Array(source.weights.iter().copied().map(f64::from).collect()),
            ),
            value_node(
                "Transform",
                FbxProperty::F64Array(flatten_fbx_transform(source.mesh_bind_transform)),
            ),
            value_node(
                "TransformLink",
                FbxProperty::F64Array(flatten_fbx_transform(source.joint_bind_transform)),
            ),
        ];
        if let Some(armature_bind_transform) = source.armature_bind_transform {
            children.push(value_node(
                "TransformAssociateModel",
                FbxProperty::F64Array(flatten_fbx_transform(armature_bind_transform)),
            ));
        }
        nodes.push(FbxNode {
            name: "Deformer".to_string(),
            properties: vec![
                FbxProperty::I64(cluster.cluster_id),
                FbxProperty::String(name_class("Cluster", "SubDeformer")),
                FbxProperty::String("Cluster".to_string()),
            ],
            children,
        });
    }

    let mut pose_children = vec![
        value_node("Type", FbxProperty::String("BindPose".to_string())),
        value_node("Version", FbxProperty::I32(100)),
    ];
    for (node_id, transform) in &skin.bind_pose {
        let Some(model) = models
            .iter()
            .find(|model| model.scene_node_id == Some(*node_id))
        else {
            continue;
        };
        pose_children.push(FbxNode {
            name: "PoseNode".to_string(),
            properties: Vec::new(),
            children: vec![
                value_node("Node", FbxProperty::I64(model.model_id)),
                value_node(
                    "Matrix",
                    FbxProperty::F64Array(flatten_fbx_transform(*transform)),
                ),
            ],
        });
    }
    nodes.push(FbxNode {
        name: "Pose".to_string(),
        properties: vec![
            FbxProperty::I64(skin.pose_id),
            FbxProperty::String(name_class("BindPose", "Pose")),
            FbxProperty::String("BindPose".to_string()),
        ],
        children: pose_children,
    });

    nodes
}

/// Builds a BlendShape deformer, one channel per target and the shape
/// geometry each channel drives -- siblings, not one node.
fn morph_nodes(morph: &MorphData) -> Vec<FbxNode> {
    let mut nodes = vec![FbxNode {
        name: "Deformer".to_string(),
        properties: vec![
            FbxProperty::I64(morph.blend_shape_id),
            FbxProperty::String(name_class("BlendShape", "Deformer")),
            FbxProperty::String("BlendShape".to_string()),
        ],
        children: Vec::new(),
    }];

    for target in &morph.targets {
        let source = &target.source;
        let name = source.name.as_deref().unwrap_or("MorphTarget");
        nodes.push(FbxNode {
            name: "Deformer".to_string(),
            properties: vec![
                FbxProperty::I64(target.channel_id),
                FbxProperty::String(name_class(name, "SubDeformer")),
                FbxProperty::String("BlendShapeChannel".to_string()),
            ],
            children: vec![
                value_node(
                    "DeformPercent",
                    FbxProperty::F64(source.default_weight as f64),
                ),
                value_node(
                    "FullWeights",
                    FbxProperty::F64Array(vec![source.full_weight as f64]),
                ),
            ],
        });
        nodes.push(FbxNode {
            name: "Geometry".to_string(),
            properties: vec![
                FbxProperty::I64(target.shape_geometry_id),
                FbxProperty::String(name_class(name, "Geometry")),
                FbxProperty::String("Shape".to_string()),
            ],
            children: vec![
                value_node(
                    "Indexes",
                    FbxProperty::I32Array(
                        source
                            .control_point_indices
                            .iter()
                            .map(|&index| index as i32)
                            .collect(),
                    ),
                ),
                value_node(
                    "Vertices",
                    FbxProperty::F64Array(
                        source
                            .position_deltas
                            .iter()
                            .flat_map(|delta| delta.iter().copied())
                            .map(f64::from)
                            .collect(),
                    ),
                ),
            ],
        });
    }

    nodes
}

fn geometry_node(mesh_data: &MeshData) -> FbxNode {
    let mut children = vec![value_node("GeometryVersion", FbxProperty::I32(124))];

    let vertices = mesh_data
        .control_points
        .as_deref()
        .unwrap_or(&mesh_data.vertices);
    if !vertices.is_empty() {
        children.push(value_node(
            "Vertices",
            FbxProperty::F64Array(vertices.to_vec()),
        ));
    }

    let polygon_indices = mesh_data
        .polygon_vertex_indices
        .as_deref()
        .unwrap_or(&mesh_data.indices);
    // Emit the node even when empty, as long as the geometry has vertices.
    // A vertices-only Geometry is legal -- Blender writes one for a curve
    // with no faces -- and the reader treats a missing PolygonVertexIndex
    // as "not a mesh", so skipping it dropped the object entirely.
    if !polygon_indices.is_empty() || !vertices.is_empty() {
        children.push(value_node(
            "PolygonVertexIndex",
            FbxProperty::I32Array(polygon_indices.to_vec()),
        ));
    }

    // Normals (preserve original layer mappings when available).
    if mesh_data.normal_sets.is_empty() {
        if let Some(normals) = &mesh_data.normals {
            children.push(layer_element_normal_node(normals));
        }
    } else {
        for normal_set in &mesh_data.normal_sets {
            children.push(layer_element_normal_set_node(normal_set));
        }
    }

    // `Edges` addresses polygon corners and is what `ByEdge` layer
    // elements index, so it is written back verbatim when present.
    if !mesh_data.edges.is_empty() {
        children.push(value_node(
            "Edges",
            FbxProperty::I32Array(mesh_data.edges.clone()),
        ));
    }

    // Vertex colours, when the source carried any.
    for color_set in &mesh_data.color_sets {
        children.push(layer_element_color_set_node(color_set));
    }

    // Tangents and their handedness, then binormals; FBX always writes the
    // pair together and no corpus file carries one without the other.
    for set in &mesh_data.tangent_sets {
        children.push(layer_element_tangent_set_node("LayerElementTangent", set));
    }
    for set in &mesh_data.binormal_sets {
        children.push(layer_element_tangent_set_node("LayerElementBinormal", set));
    }

    // Hard edges and creases, written back on whichever domain they were
    // authored on.
    for layer in &mesh_data.smoothing_layers {
        children.push(layer_element_smoothing_node(layer));
    }
    for layer in &mesh_data.crease_layers {
        children.push(layer_element_crease_node(layer));
    }

    // UVs (LayerElementUV, ByVertice/Direct).
    if mesh_data.uv_sets.is_empty() {
        if let Some(uvs) = &mesh_data.uvs {
            children.push(layer_element_uv_node(uvs));
        }
    } else {
        for uv_set in &mesh_data.uv_sets {
            children.push(layer_element_uv_set_node(uv_set));
        }
    }

    if !mesh_data.material_indices.is_empty() {
        // `LayerElementMaterial` is ByPolygon, but `material_indices` is
        // per triangle. When the original n-gon stream is being written
        // those counts differ, so collapse back to one entry per polygon.
        let per_polygon = collapse_material_indices_to_polygons(
            &mesh_data.material_indices,
            mesh_data.polygon_vertex_indices.as_deref(),
        );
        children.push(layer_element_material_node(&per_polygon));
    }

    if let Some(layer) = layer_node(mesh_data) {
        children.push(layer);
    }

    FbxNode {
        name: "Geometry".to_string(),
        properties: vec![
            FbxProperty::I64(mesh_data.geometry_id),
            FbxProperty::String(name_class(&mesh_data.name, "Geometry")),
            FbxProperty::String("Mesh".to_string()),
        ],
        children,
    }
}

/// The `Layer` aggregation node, which lists every element the geometry wrote.
///
/// FBX requires an entry per used element, or an importer will not find it --
/// a colours-only geometry used to emit an orphaned `LayerElementColor`
/// because this condition did not mention them. `None` when the geometry has
/// no layer elements at all.
fn layer_node(mesh_data: &MeshData) -> Option<FbxNode> {
    let uses_layers = mesh_data.normals.is_some()
        || !mesh_data.normal_sets.is_empty()
        || mesh_data.uvs.is_some()
        || !mesh_data.uv_sets.is_empty()
        || !mesh_data.color_sets.is_empty()
        || !mesh_data.tangent_sets.is_empty()
        || !mesh_data.binormal_sets.is_empty()
        || !mesh_data.smoothing_layers.is_empty()
        || !mesh_data.crease_layers.is_empty()
        || !mesh_data.material_indices.is_empty();
    if !uses_layers {
        return None;
    }

    let mut children = vec![value_node("Version", FbxProperty::I32(100))];
    if mesh_data.normal_sets.is_empty() && mesh_data.normals.is_some() {
        children.push(layer_element_node("LayerElementNormal", 0));
    } else {
        for index in 0..mesh_data.normal_sets.len() {
            children.push(layer_element_node("LayerElementNormal", index as i32));
        }
    }
    if mesh_data.uv_sets.is_empty() && mesh_data.uvs.is_some() {
        children.push(layer_element_node("LayerElementUV", 0));
    } else {
        for index in 0..mesh_data.uv_sets.len() {
            children.push(layer_element_node("LayerElementUV", index as i32));
        }
    }
    for index in 0..mesh_data.color_sets.len() {
        children.push(layer_element_node("LayerElementColor", index as i32));
    }
    for index in 0..mesh_data.tangent_sets.len() {
        children.push(layer_element_node("LayerElementTangent", index as i32));
    }
    for index in 0..mesh_data.binormal_sets.len() {
        children.push(layer_element_node("LayerElementBinormal", index as i32));
    }
    for index in 0..mesh_data.smoothing_layers.len() {
        children.push(layer_element_node("LayerElementSmoothing", index as i32));
    }
    for (index, layer) in mesh_data.crease_layers.iter().enumerate() {
        let element = match layer.kind {
            crate::fbx_scene::FbxCreaseKind::Edge => "LayerElementEdgeCrease",
            crate::fbx_scene::FbxCreaseKind::Vertex => "LayerElementVertexCrease",
        };
        children.push(layer_element_node(element, index as i32));
    }
    if !mesh_data.material_indices.is_empty() {
        children.push(layer_element_node("LayerElementMaterial", 0));
    }

    Some(FbxNode {
        name: "Layer".to_string(),
        properties: Vec::new(),
        children,
    })
}

/// A node holding one value and no children.
fn value_node(name: &str, value: FbxProperty) -> FbxNode {
    FbxNode {
        name: name.to_string(),
        properties: vec![value],
        children: Vec::new(),
    }
}

/// A `LayerElement` entry in a `Layer`, naming an element type and which of
/// its instances this layer uses.
fn layer_element_node(type_name: &str, index: i32) -> FbxNode {
    FbxNode {
        name: "LayerElement".to_string(),
        properties: Vec::new(),
        children: vec![
            value_node("Type", FbxProperty::String(type_name.to_string())),
            value_node("TypedIndex", FbxProperty::I32(index)),
        ],
    }
}

/// The four nodes every layer element opens with.
///
/// Version 101 is what every element except smoothing carries; smoothing
/// writes 102, as Autodesk does, and so builds its own header.
fn layer_header(layer_name: &str, mapping: &str, reference: &str) -> Vec<FbxNode> {
    vec![
        value_node("Version", FbxProperty::I32(101)),
        value_node("Name", FbxProperty::String(layer_name.to_string())),
        value_node(
            "MappingInformationType",
            FbxProperty::String(mapping.to_string()),
        ),
        value_node(
            "ReferenceInformationType",
            FbxProperty::String(reference.to_string()),
        ),
    ]
}

/// Flattens `[f32; N]` values into the single `f64` array FBX stores.
fn flatten_f64<const N: usize>(values: &[[f32; N]], components: usize) -> Vec<f64> {
    values
        .iter()
        .flat_map(|value| value[..components].iter().map(|c| f64::from(*c)))
        .collect()
}

/// The `IndexToDirect` companion array, present only when the source declared
/// that reference mode and actually carried indices.
fn index_array_node(name: &str, reference: Option<&str>, indices: &[i32]) -> Option<FbxNode> {
    (reference == Some("IndexToDirect") && !indices.is_empty())
        .then(|| value_node(name, FbxProperty::I32Array(indices.to_vec())))
}

fn layer_element_normal_node(normals: &[f64]) -> FbxNode {
    let mut children = layer_header("", "ByVertice", "Direct");
    children.push(value_node(
        "Normals",
        FbxProperty::F64Array(normals.to_vec()),
    ));
    FbxNode {
        name: "LayerElementNormal".to_string(),
        properties: Vec::new(),
        children,
    }
}

fn layer_element_normal_set_node(set: &crate::fbx_scene::FbxNormalSet) -> FbxNode {
    let mut children = layer_header(
        set.name.as_deref().unwrap_or("NormalSet0"),
        set.mapping.as_deref().unwrap_or("ByPolygonVertex"),
        set.reference.as_deref().unwrap_or("Direct"),
    );
    children.push(value_node(
        "Normals",
        FbxProperty::F64Array(flatten_f64(&set.values, 3)),
    ));
    children.extend(index_array_node(
        "NormalIndex",
        set.reference.as_deref(),
        &set.indices,
    ));
    FbxNode {
        name: "LayerElementNormal".to_string(),
        properties: Vec::new(),
        children,
    }
}

fn layer_element_uv_node(uvs: &[f64]) -> FbxNode {
    let mut children = layer_header("", "ByVertice", "Direct");
    children.push(value_node("UV", FbxProperty::F64Array(uvs.to_vec())));
    FbxNode {
        name: "LayerElementUV".to_string(),
        properties: Vec::new(),
        children,
    }
}

fn layer_element_uv_set_node(set: &crate::fbx_scene::FbxUvSet) -> FbxNode {
    let mut children = layer_header(
        set.name.as_deref().unwrap_or("UVSet0"),
        set.mapping.as_deref().unwrap_or("ByPolygonVertex"),
        set.reference.as_deref().unwrap_or("IndexToDirect"),
    );
    children.push(value_node(
        "UV",
        FbxProperty::F64Array(flatten_f64(&set.values, 2)),
    ));
    children.extend(index_array_node(
        "UVIndex",
        set.reference.as_deref(),
        &set.indices,
    ));
    FbxNode {
        name: "LayerElementUV".to_string(),
        properties: Vec::new(),
        children,
    }
}

fn layer_element_color_set_node(set: &crate::fbx_scene::FbxColorSet) -> FbxNode {
    let mut children = layer_header(
        set.name.as_deref().unwrap_or("Col"),
        set.mapping.as_deref().unwrap_or("ByPolygonVertex"),
        set.reference.as_deref().unwrap_or("Direct"),
    );
    children.push(value_node(
        "Colors",
        FbxProperty::F64Array(flatten_f64(&set.values, 4)),
    ));
    children.extend(index_array_node(
        "ColorIndex",
        set.reference.as_deref(),
        &set.indices,
    ));
    FbxNode {
        name: "LayerElementColor".to_string(),
        properties: Vec::new(),
        children,
    }
}

/// Builds a `LayerElementTangent` or `LayerElementBinormal`.
///
/// The four-component value is split back into the two sibling arrays FBX
/// uses. The handedness array is emitted only when the source had one, so a
/// pre-7500 document does not acquire a field it never carried.
fn layer_element_tangent_set_node(element: &str, set: &crate::fbx_scene::FbxTangentSet) -> FbxNode {
    let (values_node, handedness_node, index_node) = if element == "LayerElementBinormal" {
        ("Binormals", "BinormalsW", "BinormalIndex")
    } else {
        ("Tangents", "TangentsW", "TangentIndex")
    };
    let mut children = layer_header(
        set.layer.name.as_deref().unwrap_or(""),
        set.layer.mapping.as_deref().unwrap_or("ByPolygonVertex"),
        set.layer.reference.as_deref().unwrap_or("Direct"),
    );
    children.push(value_node(
        values_node,
        FbxProperty::F64Array(flatten_f64(&set.layer.values, 3)),
    ));
    if set.has_handedness {
        let signs: Vec<f64> = set
            .layer
            .values
            .iter()
            .map(|value| f64::from(value[3]))
            .collect();
        children.push(value_node(handedness_node, FbxProperty::F64Array(signs)));
    }
    children.extend(index_array_node(
        index_node,
        set.layer.reference.as_deref(),
        &set.layer.indices,
    ));
    FbxNode {
        name: element.to_string(),
        properties: Vec::new(),
        children,
    }
}

/// Builds a `LayerElementSmoothing`, whose payload is integer flags.
///
/// Smoothing carries layer version 102 rather than the 101 every other element
/// uses, which is what Autodesk writes.
fn layer_element_smoothing_node(layer: &crate::fbx_scene::FbxSmoothingLayer) -> FbxNode {
    FbxNode {
        name: "LayerElementSmoothing".to_string(),
        properties: Vec::new(),
        children: vec![
            value_node("Version", FbxProperty::I32(102)),
            value_node("Name", FbxProperty::String(String::new())),
            value_node(
                "MappingInformationType",
                FbxProperty::String(
                    layer
                        .mapping
                        .clone()
                        .unwrap_or_else(|| "ByEdge".to_string()),
                ),
            ),
            value_node(
                "ReferenceInformationType",
                FbxProperty::String("Direct".to_string()),
            ),
            value_node("Smoothing", FbxProperty::I32Array(layer.values.clone())),
        ],
    }
}

/// Builds a `LayerElementEdgeCrease` or `LayerElementVertexCrease`, whose
/// payload is floating-point weights.
fn layer_element_crease_node(layer: &crate::fbx_scene::FbxCreaseLayer) -> FbxNode {
    let (element, data_node, default_mapping) = match layer.kind {
        crate::fbx_scene::FbxCreaseKind::Edge => ("LayerElementEdgeCrease", "EdgeCrease", "ByEdge"),
        crate::fbx_scene::FbxCreaseKind::Vertex => {
            ("LayerElementVertexCrease", "VertexCrease", "ByVertice")
        }
    };
    let mut children = layer_header(
        "",
        layer.mapping.as_deref().unwrap_or(default_mapping),
        "Direct",
    );
    children.push(value_node(
        data_node,
        FbxProperty::F64Array(layer.values.clone()),
    ));
    FbxNode {
        name: element.to_string(),
        properties: Vec::new(),
        children,
    }
}

/// Collapses per-triangle material indices to one entry per source polygon.
///
/// `FbxMeshInstance::material_indices` holds one entry per fan-triangulated
/// triangle, while `LayerElementMaterial` is addressed ByPolygon. Writing the
/// triangle list verbatim beside an n-gon `PolygonVertexIndex` stream made the
/// reader take the first N entries as the polygon assignments, which silently
/// dropped every material past the first few polygons.
///
/// With no polygon stream the writer emits one triangle per polygon, so the
/// list already matches.
fn collapse_material_indices_to_polygons(
    material_indices: &[i32],
    polygon_vertex_indices: Option<&[i32]>,
) -> Vec<i32> {
    let Some(polygons) = polygon_vertex_indices else {
        return material_indices.to_vec();
    };

    let mut per_polygon = Vec::new();
    let mut triangle = 0usize;
    let mut corners = 0usize;
    for &encoded in polygons {
        corners += 1;
        if encoded >= 0 {
            continue;
        }
        // A polygon of `corners` vertices fan-triangulates into `corners - 2`
        // triangles, which all carry the same material.
        let triangles = corners.saturating_sub(2);
        per_polygon.push(material_indices.get(triangle).copied().unwrap_or(0));
        triangle += triangles;
        corners = 0;
    }
    per_polygon
}

/// Builds the `LayerElementMaterial`, whose indices are ByPolygon.
fn layer_element_material_node(material_indices: &[i32]) -> FbxNode {
    let mut children = layer_header("", "ByPolygon", "IndexToDirect");
    children.push(value_node(
        "Materials",
        FbxProperty::I32Array(material_indices.to_vec()),
    ));
    FbxNode {
        name: "LayerElementMaterial".to_string(),
        properties: Vec::new(),
        children,
    }
}

fn material_node(material_data: &MaterialData) -> FbxNode {
    // The object-level node below needs a value, but the property is only
    // written when the source actually had one: inventing "Phong" made a
    // read/write cycle add a shading model the file never declared.
    let declared_shading = material_data.source.shading_model.as_deref();
    let shading = declared_shading.unwrap_or("Phong");

    let mut properties = Vec::new();
    if let Some(shading) = declared_shading {
        properties.push(string_property_node("ShadingModel", shading));
    }
    // Only emit the fields that are present so round-trips stay clean. The
    // order is the one Autodesk writes -- each colour followed by its factor.
    let source = &material_data.source;
    for property in [
        source
            .diffuse
            .map(|v| color_property_node("DiffuseColor", v)),
        source
            .diffuse_factor
            .map(|v| scalar_property_node("DiffuseFactor", v as f64)),
        source
            .specular
            .map(|v| color_property_node("SpecularColor", v)),
        source
            .specular_factor
            .map(|v| scalar_property_node("SpecularFactor", v as f64)),
        source
            .shininess
            .map(|v| scalar_property_node("Shininess", v as f64)),
        source
            .emissive
            .map(|v| color_property_node("EmissiveColor", v)),
        source
            .emissive_factor
            .map(|v| scalar_property_node("EmissiveFactor", v as f64)),
        source
            .ambient
            .map(|v| color_property_node("AmbientColor", v)),
        source
            .reflection_factor
            .map(|v| scalar_property_node("ReflectionFactor", v as f64)),
        source
            .transparency_factor
            .map(|v| scalar_property_node("TransparencyFactor", v as f64)),
        source
            .opacity
            .map(|v| scalar_property_node("Opacity", v as f64)),
        source
            .bump_factor
            .map(|v| scalar_property_node("BumpFactor", v as f64)),
    ] {
        properties.extend(property);
    }

    let name = source
        .name
        .clone()
        .unwrap_or_else(|| "Material".to_string());
    FbxNode {
        name: "Material".to_string(),
        properties: vec![
            FbxProperty::I64(material_data.material_id),
            FbxProperty::String(name_class(&name, "Material")),
            FbxProperty::String(String::new()),
        ],
        children: vec![
            value_node("Version", FbxProperty::I32(102)),
            properties70_node(properties),
            // ShadingModel at the object level mirrors Blender's output.
            value_node("ShadingModel", FbxProperty::String(shading.to_string())),
        ],
    }
}

fn texture_node(texture_data: &TextureData) -> FbxNode {
    // An unnamed texture stays unnamed. Substituting the class name here gave
    // it one, so a document that had no texture names acquired them by being
    // rewritten -- the same fabrication as naming an unnamed Geometry after
    // its Model.
    let name = texture_data.source.name.clone().unwrap_or_default();
    let filename = texture_data.source.filename.clone().unwrap_or_default();
    FbxNode {
        name: "Texture".to_string(),
        properties: vec![
            FbxProperty::I64(texture_data.texture_id),
            FbxProperty::String(name_class(&name, "Texture")),
            FbxProperty::String(String::new()),
        ],
        children: vec![
            value_node("Media", FbxProperty::String(name)),
            value_node("FileName", FbxProperty::String(filename.clone())),
            value_node("RelativeFilename", FbxProperty::String(filename)),
        ],
    }
}

fn video_node(texture_data: &TextureData) -> FbxNode {
    // Unnamed stays unnamed, as for the `Texture` above.
    let name = texture_data.source.name.clone().unwrap_or_default();
    let filename = texture_data.source.filename.clone().unwrap_or_default();
    let mut children = vec![
        value_node("Filename", FbxProperty::String(filename.clone())),
        value_node("RelativeFilename", FbxProperty::String(filename)),
    ];
    if let Some(content) = &texture_data.source.content {
        children.push(value_node("Content", FbxProperty::Raw(content.clone())));
    }
    FbxNode {
        name: "Video".to_string(),
        properties: vec![
            FbxProperty::I64(texture_data.video_id),
            FbxProperty::String(name_class(&name, "Video")),
            FbxProperty::String("Clip".to_string()),
        ],
        children,
    }
}

/// Builds an `AnimationStack`, its single `AnimationLayer`, and the curve
/// nodes and curves of every channel -- siblings, not one node.
fn animation_stack_nodes(stack: &AnimStackData) -> Vec<FbxNode> {
    // KTime ticks-per-second used on the writer side. FBX < 8000 default is V7.
    const KTIME: i64 = 46_186_158_000;
    let stop = (stack.duration.max(0.0) as f64 * KTIME as f64) as i64;
    let name = stack
        .name
        .clone()
        .unwrap_or_else(|| "AnimStack".to_string());

    let mut nodes = vec![
        FbxNode {
            name: "AnimationStack".to_string(),
            properties: vec![
                FbxProperty::I64(stack.stack_id),
                FbxProperty::String(name_class(&name, "AnimStack")),
                FbxProperty::String(String::new()),
            ],
            children: vec![properties70_node(vec![timestamp_property_node(
                "LocalStop",
                stop,
            )])],
        },
        FbxNode {
            name: "AnimationLayer".to_string(),
            properties: vec![
                FbxProperty::I64(stack.layer_id),
                FbxProperty::String(name_class(&name, "AnimLayer")),
                FbxProperty::String(String::new()),
            ],
            children: Vec::new(),
        },
    ];
    for channel in &stack.channels {
        nodes.extend(animation_curve_nodes(stack.stack_id, channel));
    }
    nodes
}

/// Builds one channel's `AnimationCurveNode` and its component curves.
fn animation_curve_nodes(
    stack_id: i64,
    channel: &crate::fbx_scene::FbxAnimChannel,
) -> Vec<FbxNode> {
    // Allocate a stable id by hashing node + path. The Connections section
    // reuses this id when wiring the curve node to its model and curves.
    let id = anim_object_id(
        stack_id,
        channel.node_id,
        channel.path,
        channel.morph_target_index,
        "node",
    );

    // Emit default values for each component so consumers can resolve the
    // curve node even when a component curve is missing.
    let defaults: Vec<FbxNode> = ["d|X", "d|Y", "d|Z"]
        .into_iter()
        .take(channel.path.component_count())
        .map(|suffix| scalar_property_node(suffix, 0.0))
        .collect();

    let mut nodes = vec![FbxNode {
        name: "AnimationCurveNode".to_string(),
        properties: vec![
            FbxProperty::I64(id),
            FbxProperty::String(name_class("", "AnimCurveNode")),
            FbxProperty::String(String::new()),
        ],
        children: vec![properties70_node(defaults)],
    }];

    // Emit one AnimationCurve per component, connected OP via d|X/d|Y/d|Z.
    for component in 0u32..channel.path.component_count() as u32 {
        nodes.push(animation_curve_node(id, component, channel));
    }
    nodes
}

fn animation_curve_node(
    curve_node_id: i64,
    component: u32,
    channel: &crate::fbx_scene::FbxAnimChannel,
) -> FbxNode {
    const KTIME: f64 = 46_186_158_000.0;
    let components = channel.path.component_count();
    let keys = channel.sampler.input.len();

    let mut key_times = Vec::with_capacity(keys);
    let mut key_values = Vec::with_capacity(keys);
    for key in 0..keys {
        key_times.push((channel.sampler.input[key] as f64 * KTIME) as i64);
        let value_index = key * components + component as usize;
        key_values.push(
            channel
                .sampler
                .output
                .get(value_index)
                .copied()
                .unwrap_or(0.0),
        );
    }

    // FBX stores Euler rotations in degrees; the reader converted them to
    // radians, so convert back here. The same factor applies to key slopes.
    let scale = if channel.path == crate::fbx_scene::FbxAnimChannelPath::Rotation {
        180.0 / std::f32::consts::PI
    } else {
        1.0
    };

    // KeyAttr* is run-length encoded. Cubic keys carry their explicit right
    // slope and the next key's left slope in four-float records, matching
    // Blender 5 / ufbx's native FBX contract.
    let flags_value = channel.sampler.interpolation.to_key_attr_flags();
    let cubic = channel.sampler.interpolation == crate::fbx_scene::FbxAnimInterpolation::Cubic;
    let flags = if cubic {
        vec![flags_value; keys]
    } else {
        vec![flags_value]
    };

    let in_tangents = channel.sampler.in_tangents.as_deref();
    let out_tangents = channel.sampler.out_tangents.as_deref();
    let mut datafloat = Vec::with_capacity(if cubic { keys * 4 } else { 4 });
    for key in 0..if cubic { keys } else { 1 } {
        let component_index = key * components + component as usize;
        let right = out_tangents
            .and_then(|values| values.get(component_index))
            .copied()
            .unwrap_or(0.0)
            * scale;
        let next_left = in_tangents
            .and_then(|values| values.get(component_index + components))
            .copied()
            .unwrap_or(0.0)
            * scale;
        datafloat.extend([right, next_left, 0.0, 0.0]);
    }

    let refcount = if cubic {
        vec![1; keys]
    } else {
        vec![keys as i32]
    };

    FbxNode {
        name: "AnimationCurve".to_string(),
        properties: vec![
            FbxProperty::I64(anim_curve_id(curve_node_id, component)),
            FbxProperty::String(name_class("", "AnimCurve")),
            FbxProperty::String(String::new()),
        ],
        children: vec![
            value_node("Default", FbxProperty::F64(0.0)),
            value_node("KeyVer", FbxProperty::I32(4009)),
            value_node("KeyTime", FbxProperty::I64Array(key_times)),
            value_node(
                "KeyValueFloat",
                FbxProperty::F32Array(key_values.iter().map(|value| value * scale).collect()),
            ),
            value_node("KeyAttrFlags", FbxProperty::I32Array(flags)),
            value_node("KeyAttrDataFloat", FbxProperty::F32Array(datafloat)),
            value_node("KeyAttrRefCount", FbxProperty::I32Array(refcount)),
        ],
    }
}

/// Stable non-colliding id for an animation curve node.
fn anim_object_id(
    stack_id: i64,
    node_id: crate::fbx_scene::FbxNodeId,
    path: crate::fbx_scene::FbxAnimChannelPath,
    morph_target_index: Option<u32>,
    kind: &str,
) -> i64 {
    // Reserve a large positive range (2000000..) so ids never collide with
    // models/geometries/materials/textures/stacks.
    let mut hash: u64 = 2_000_000;
    hash = hash.wrapping_mul(131).wrapping_add(stack_id as u64);
    hash = hash.wrapping_mul(131).wrapping_add(node_id.0 as u64);
    hash = hash
        .wrapping_mul(131)
        .wrapping_add(path.property_name().len() as u64);
    hash = hash
        .wrapping_mul(131)
        .wrapping_add(u64::from(morph_target_index.unwrap_or(0)));
    hash = hash.wrapping_mul(131).wrapping_add(kind.len() as u64);
    hash as i64
}

fn anim_curve_id(curve_node_id: i64, component: u32) -> i64 {
    curve_node_id
        .wrapping_mul(131)
        .wrapping_add((component as i64) + 1_000_000)
}

// Wires every object type together, so it needs every object table.
#[allow(clippy::too_many_arguments)]
fn connections_node(
    models: &[ModelData],
    meshes: &[MeshData],
    textures: &[TextureData],
    anim: &[AnimStackData],
    pending: &[PendingConnection],
    skins: &[SkinData],
    morphs: &[MorphData],
) -> FbxNode {
    let mut edges = Vec::new();
    {
        // Model -> parent (OO).
        for model_data in models {
            edges.push(connection_node(
                "OO",
                model_data.model_id,
                model_data.parent_id.unwrap_or(0),
                None,
            ));
            if model_data.class == "LimbNode" {
                edges.push(connection_node(
                    "OO",
                    limb_node_attribute_id(model_data.model_id),
                    model_data.model_id,
                    None,
                ));
            }
        }
        // Geometry -> Model (OO).
        for mesh_data in meshes {
            edges.push(connection_node(
                "OO",
                mesh_data.geometry_id,
                mesh_data.model_id,
                None,
            ));
        }
        // Material -> Model (OO). Materials are connected once per model so
        // the reader maps them back via the model's material list.
        for model_data in models {
            for &material_id in &model_data.material_ids {
                edges.push(connection_node(
                    "OO",
                    material_id,
                    model_data.model_id,
                    None,
                ));
            }
        }
        for morph in morphs {
            edges.push(connection_node(
                "OO",
                morph.blend_shape_id,
                morph.geometry_id,
                None,
            ));
            for target in &morph.targets {
                edges.push(connection_node(
                    "OO",
                    target.channel_id,
                    morph.blend_shape_id,
                    None,
                ));
                edges.push(connection_node(
                    "OO",
                    target.shape_geometry_id,
                    target.channel_id,
                    None,
                ));
            }
        }
        // Video -> Texture (OO), Texture -> Material (OP, by slot).
        for texture_data in textures {
            edges.push(connection_node(
                "OO",
                texture_data.video_id,
                texture_data.texture_id,
                None,
            ));
        }
        for conn in pending {
            edges.push(connection_node(
                conn.kind,
                conn.child,
                conn.parent,
                conn.property.as_deref(),
            ));
        }
        // Geometry -> Skin and Cluster -> Skin; each Cluster also points to
        // its joint Model. This is the connection topology used by Blender's
        // native FBX importer for an armature modifier and vertex groups.
        for skin in skins {
            edges.push(connection_node("OO", skin.skin_id, skin.geometry_id, None));
            for cluster in &skin.clusters {
                edges.push(connection_node(
                    "OO",
                    cluster.cluster_id,
                    skin.skin_id,
                    None,
                ));
                if let Some(joint) = models
                    .iter()
                    .find(|model| model.scene_node_id == Some(cluster.source.joint_node_id))
                {
                    edges.push(connection_node(
                        "OO",
                        joint.model_id,
                        cluster.cluster_id,
                        None,
                    ));
                }
            }
        }
        // Animation wiring: Stack -> Document root (OO), Layer -> Stack (OO),
        // CurveNode -> Layer (OO), CurveNode -> Model or BlendShapeChannel (OP),
        // Curve -> CurveNode (OP, by component).
        for stack in anim {
            edges.push(connection_node("OO", stack.stack_id, 0, None));
            edges.push(connection_node("OO", stack.layer_id, stack.stack_id, None));
            for channel in &stack.channels {
                let acnode_id = anim_object_id(
                    stack.stack_id,
                    channel.node_id,
                    channel.path,
                    channel.morph_target_index,
                    "node",
                );
                edges.push(connection_node("OO", acnode_id, stack.layer_id, None));
                // Resolve target by stable node id, never by non-unique name.
                let scene_model_id = models
                    .iter()
                    .find(|m| m.scene_node_id == Some(channel.node_id))
                    .map(|model| model.model_id);
                let target = if channel.path == crate::fbx_scene::FbxAnimChannelPath::MorphWeight {
                    channel.morph_target_index.and_then(|target_index| {
                        morphs.iter().find_map(|morph| {
                            if Some(morph.model_id) != scene_model_id {
                                return None;
                            }
                            morph
                                .targets
                                .get(target_index as usize)
                                .map(|target| (target.channel_id, true))
                        })
                    })
                } else {
                    scene_model_id.map(|model_id| (model_id, false))
                };
                if let Some((target_id, _is_morph)) = target {
                    edges.push(connection_node(
                        "OP",
                        acnode_id,
                        target_id,
                        Some(channel.path.property_name()),
                    ));
                }
                for component in 0u32..channel.path.component_count() as u32 {
                    let curve_id = anim_curve_id(acnode_id, component);
                    let suffix = match component {
                        0 => "d|X",
                        1 => "d|Y",
                        _ => "d|Z",
                    };
                    edges.push(connection_node("OP", curve_id, acnode_id, Some(suffix)));
                }
            }
        }
    }

    FbxNode {
        name: "Connections".to_string(),
        properties: Vec::new(),
        children: edges,
    }
}

/// One `C` record: a typed edge from `child` to `parent`.
fn connection_node(kind: &str, child: i64, parent: i64, property: Option<&str>) -> FbxNode {
    let mut properties = vec![
        FbxProperty::String(kind.to_string()),
        FbxProperty::I64(child),
        FbxProperty::I64(parent),
    ];
    if let Some(property) = property {
        properties.push(FbxProperty::String(property.to_string()));
    }
    FbxNode {
        name: "C".to_string(),
        properties,
        children: Vec::new(),
    }
}

fn color_property_node(name: &str, values: [f32; 3]) -> FbxNode {
    property_node(
        name,
        "Color",
        "",
        "A",
        values
            .into_iter()
            .map(|value| FbxProperty::F64(value as f64))
            .collect(),
    )
}

fn scalar_property_node(name: &str, value: f64) -> FbxNode {
    property_node(name, "Number", "", "A", vec![FbxProperty::F64(value)])
}

fn string_property_node(name: &str, value: &str) -> FbxNode {
    property_node(
        name,
        "KString",
        "",
        "A",
        vec![FbxProperty::String(value.to_string())],
    )
}

fn timestamp_property_node(name: &str, value: i64) -> FbxNode {
    property_node(name, "KTime", "Time", "", vec![FbxProperty::I64(value)])
}

// ============================================================================
// Mesh Data Extraction
// ============================================================================

fn validate_supported_fbx_attributes(mesh: &Mesh) -> io::Result<()> {
    for i in 0..mesh.num_attributes() {
        let attribute_type = mesh.attribute(i).attribute_type();
        match attribute_type {
            GeometryAttributeType::Position
            | GeometryAttributeType::Normal
            | GeometryAttributeType::TexCoord
            | GeometryAttributeType::Color => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "FBX writer currently supports only Position, Normal, TexCoord and Color attributes; {:?} is not written",
                        attribute_type
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn extract_vertices(mesh: &Mesh) -> Vec<f64> {
    let pos_att_id = mesh.named_attribute_id(GeometryAttributeType::Position);
    if pos_att_id < 0 {
        return Vec::new();
    }

    let att = mesh.attribute(pos_att_id);
    let byte_stride = att.byte_stride() as usize;
    let buffer = att.buffer();
    let mut vertices = Vec::with_capacity(mesh.num_points() * 3);

    for i in 0..mesh.num_points() {
        let mut bytes = [0u8; 12];
        buffer.read(i * byte_stride, &mut bytes);
        let x = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64;
        let y = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as f64;
        let z = f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as f64;
        vertices.push(x);
        vertices.push(y);
        vertices.push(z);
    }
    vertices
}

fn extract_polygon_indices(mesh: &Mesh) -> Vec<i32> {
    let mut indices = Vec::with_capacity(mesh.num_faces() * 3);
    for i in 0..mesh.num_faces() as u32 {
        let face = mesh.face(FaceIndex(i));
        indices.push(face[0].0 as i32);
        indices.push(face[1].0 as i32);
        // Last index is bitwise NOT to mark end of polygon
        indices.push(!(face[2].0 as i32));
    }
    indices
}

/// Extract a 3-component attribute (e.g. normals) into flat f64 values.
fn extract_vec3_attribute(mesh: &Mesh, attribute_type: GeometryAttributeType) -> Option<Vec<f64>> {
    let id = mesh.named_attribute_id(attribute_type);
    if id < 0 {
        return None;
    }
    let att = mesh.attribute(id);
    let stride = att.byte_stride() as usize;
    let buffer = att.buffer();
    let mut values = Vec::with_capacity(mesh.num_points() * 3);
    for i in 0..mesh.num_points() {
        let mut bytes = [0u8; 12];
        buffer.read(i * stride, &mut bytes);
        for c in 0..3 {
            values.push(f32::from_le_bytes([
                bytes[c * 4],
                bytes[c * 4 + 1],
                bytes[c * 4 + 2],
                bytes[c * 4 + 3],
            ]) as f64);
        }
    }
    Some(values)
}

fn extract_normals(mesh: &Mesh) -> Option<Vec<f64>> {
    extract_vec3_attribute(mesh, GeometryAttributeType::Normal)
}

fn extract_uvs(mesh: &Mesh) -> Option<Vec<f64>> {
    let id = mesh.named_attribute_id(GeometryAttributeType::TexCoord);
    if id < 0 {
        return None;
    }
    let att = mesh.attribute(id);
    let stride = att.byte_stride() as usize;
    let buffer = att.buffer();
    let mut values = Vec::with_capacity(mesh.num_points() * 2);
    for i in 0..mesh.num_points() {
        let mut bytes = [0u8; 8];
        buffer.read(i * stride, &mut bytes);
        let u = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64;
        let v = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as f64;
        values.push(u);
        values.push(v);
    }
    Some(values)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    // Only the round-trip tests below build scenes, and those need the reader.
    #[cfg(feature = "fbx-reader")]
    use crate::fbx_scene::{
        FbxAnimation, FbxMeshInstance, FbxMeshLayers, FbxScene, FbxSceneNode, FbxTransform,
        FbxTransformStack,
    };
    use draco_core::draco_types::DataType;
    use draco_core::geometry_attribute::PointAttribute;
    use draco_core::geometry_indices::PointIndex;
    use std::io::Cursor;
    use tempfile::NamedTempFile;

    fn create_triangle_mesh() -> Mesh {
        let mut mesh = Mesh::new();
        let mut pos_att = PointAttribute::new();

        pos_att.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            3,
        );
        let buffer = pos_att.buffer_mut();
        let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        for (i, pos) in positions.iter().enumerate() {
            let bytes: Vec<u8> = pos.iter().flat_map(|v| v.to_le_bytes()).collect();
            buffer.write(i * 12, &bytes);
        }
        mesh.add_attribute(pos_att);

        mesh.set_num_faces(1);
        mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);

        mesh
    }

    /// Finds the first child with this name.
    fn child<'a>(node: &'a FbxNode, name: &str) -> Option<&'a FbxNode> {
        node.children.iter().find(|child| child.name == name)
    }

    /// The document says what it contains, and that can be asserted directly.
    ///
    /// Every other writer test goes through the reader, which means it can
    /// only see what the reader happens to look at: it never inspects
    /// `Definitions`, the declared type strings on a `P` record, or a class
    /// suffix, so none of those are covered by a round trip. Asserting on the
    /// tree needs no reader at all.
    #[test]
    fn the_document_declares_the_objects_it_writes() {
        let mut writer = FbxWriter::new();
        writer
            .add_mesh(&create_triangle_mesh(), Some("Tri"))
            .unwrap();
        let document = writer.build_document().unwrap();

        let names: Vec<&str> = document.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "FBXHeaderExtension",
                "GlobalSettings",
                "Documents",
                "Definitions",
                "Objects",
                "Connections",
            ]
        );

        // Definitions must agree with what Objects actually holds, which is
        // the invariant the hand-maintained Count used to depend on.
        let definitions = &document[3];
        let objects = &document[4];
        let declared: Vec<(String, i32)> = definitions
            .children
            .iter()
            .filter(|node| node.name == "ObjectType")
            .map(|node| {
                let FbxProperty::String(type_name) = &node.properties[0] else {
                    panic!("ObjectType names itself with a string");
                };
                let count = child(node, "Count").expect("every ObjectType declares a Count");
                let FbxProperty::I32(count) = count.properties[0] else {
                    panic!("Count is an i32");
                };
                (type_name.clone(), count)
            })
            .collect();
        assert!(declared.contains(&("Geometry".to_string(), 1)));
        assert!(declared.contains(&("Model".to_string(), 1)));
        assert_eq!(
            child(definitions, "Count").map(|node| format!("{:?}", node.properties)),
            Some(format!(
                "{:?}",
                vec![FbxProperty::I32(declared.len() as i32)]
            )),
            "the Count node must be the number of ObjectType blocks"
        );

        let geometry = child(objects, "Geometry").expect("the mesh writes a Geometry");
        assert!(
            matches!(&geometry.properties[2], FbxProperty::String(class) if class == "Mesh"),
            "a Geometry's class suffix is Mesh: {:?}",
            geometry.properties
        );
        assert!(child(geometry, "Vertices").is_some());
        assert!(child(geometry, "PolygonVertexIndex").is_some());
    }

    #[test]
    fn test_fbx_writer_new() {
        let writer = FbxWriter::new();
        assert_eq!(writer.mesh_count(), 0);
        assert!(!writer.is_compression_enabled());
    }

    #[test]
    fn test_fbx_writer_with_options() {
        let writer = FbxWriter::new()
            .with_compression(true)
            .with_compression_threshold(64);
        assert!(writer.is_compression_enabled());
    }

    #[test]
    fn test_fbx_writer_add_mesh() {
        let mesh = create_triangle_mesh();
        let mut writer = FbxWriter::new();
        Writer::add_mesh(&mut writer, &mesh, Some("TestMesh")).unwrap();
        assert_eq!(writer.mesh_count(), 1);
    }

    #[test]
    fn test_fbx_writer_write() {
        let mesh = create_triangle_mesh();
        let mut writer = FbxWriter::new();
        Writer::add_mesh(&mut writer, &mesh, Some("Triangle")).unwrap();

        let mut buffer = Cursor::new(Vec::new());
        writer.write_to(&mut buffer).unwrap();

        let data = buffer.into_inner();

        // Check magic
        assert_eq!(&data[0..21], FBX_MAGIC);
        // Check version
        let version = u32::from_le_bytes([data[23], data[24], data[25], data[26]]);
        assert_eq!(version, FBX_VERSION);
    }

    #[test]
    fn test_write_fbx_mesh_convenience() {
        let mesh = create_triangle_mesh();
        let file = NamedTempFile::new().unwrap();
        write_fbx_mesh(file.path(), &mesh).unwrap();

        let metadata = std::fs::metadata(file.path()).unwrap();
        assert!(metadata.len() > 27);
    }

    #[test]
    fn test_multiple_meshes() {
        let mesh1 = create_triangle_mesh();
        let mesh2 = create_triangle_mesh();

        let mut writer = FbxWriter::new();
        Writer::add_mesh(&mut writer, &mesh1, Some("Mesh1")).unwrap();
        Writer::add_mesh(&mut writer, &mesh2, Some("Mesh2")).unwrap();

        assert_eq!(writer.mesh_count(), 2);

        let mut buffer = Cursor::new(Vec::new());
        writer.write_to(&mut buffer).unwrap();

        let data = buffer.into_inner();
        assert!(!data.is_empty());
    }

    // Round-trip tests read back what they wrote, so they need the reader.
    #[test]
    #[cfg(feature = "fbx-reader")]
    fn scene_roundtrip_preserves_hierarchy_and_local_transforms() {
        use crate::{FbxMeshInstance, FbxScene, FbxSceneNode, FbxTransform};

        // Non-symmetric transform: 90-degree rotation about Y (swaps X and Z
        // axes), non-uniform scaling, and translation. Both the rotation and
        // the translation must survive the column-major FBX encoding — a
        // transposition bug collapses the translation into the bottom row and
        // mirrors the rotation, which a diagonal-only transform cannot detect.
        let child_transform = FbxTransform {
            matrix: [
                [0.0, 0.0, 8.0, 0.0],
                [0.0, 3.0, 0.0, 0.0],
                [-2.0, 0.0, 0.0, 0.0],
                [1.0, 2.0, 3.0, 1.0],
            ],
        };
        let scene = FbxScene {
            global_settings: None,
            root_nodes: vec![FbxSceneNode {
                id: crate::fbx_scene::FbxNodeId(1),
                name: Some("Root".to_string()),
                transform: None,
                transform_stack: None,
                has_complex_transform_stack: false,
                mesh_instances: Vec::new(),
                attribute: None,
                children: vec![FbxSceneNode {
                    id: crate::fbx_scene::FbxNodeId(2),
                    name: Some("Child".to_string()),
                    transform: Some(child_transform),
                    transform_stack: None,
                    has_complex_transform_stack: false,
                    mesh_instances: vec![FbxMeshInstance {
                        name: Some("Triangle".to_string()),
                        mesh: create_triangle_mesh(),
                        ..Default::default()
                    }],
                    attribute: None,
                    children: Vec::new(),
                }],
            }],
            materials: Vec::new(),
            textures: Vec::new(),
            animations: Vec::new(),
            warnings: Vec::new(),
        };

        let bytes = scene.to_bytes().unwrap();
        let roundtrip = FbxScene::from_bytes(&bytes).unwrap();

        assert_eq!(roundtrip.root_nodes.len(), 1);
        let root = &roundtrip.root_nodes[0];
        assert_eq!(root.name.as_deref(), Some("Root"));
        assert_eq!(root.children.len(), 1);
        let child = &root.children[0];
        assert_eq!(child.name.as_deref(), Some("Child"));
        assert_eq!(child.mesh_instances[0].name.as_deref(), Some("Triangle"));
        assert_eq!(child.mesh_instances[0].mesh.num_faces(), 1);
        let transform = child.transform.expect("child transform should round-trip");
        for row in 0..4 {
            for column in 0..4 {
                assert!(
                    (transform.matrix[row][column] - child_transform.matrix[row][column]).abs()
                        < 1e-5
                );
            }
        }
    }

    #[test]
    #[cfg(feature = "fbx-reader")]
    fn scene_roundtrip_preserves_model_transform_stack_properties() {
        // These are authored Model properties rather than a portable local
        // matrix. A source-provenance FBX export must retain them verbatim so
        // an FBX consumer can evaluate the original transform stack.
        let transform_stack = FbxTransformStack {
            translation: Some([1.25, -2.5, 3.75]),
            rotation: Some([12.0, -34.0, 56.0]),
            scaling: Some([1.0, 0.75, 1.25]),
            rotation_order: Some(1),
            rotation_active: Some(true),
            pre_rotation: Some([10.0, 20.0, -30.0]),
            post_rotation: Some([-5.0, 15.0, 25.0]),
            rotation_offset: Some([0.5, 1.0, -1.5]),
            rotation_pivot: Some([2.0, -3.0, 4.0]),
            scaling_offset: Some([-0.25, 0.5, 0.75]),
            scaling_pivot: Some([1.5, -2.5, 3.5]),
            inherit_type: Some(2),
        };
        let scene = FbxScene {
            global_settings: None,
            root_nodes: vec![FbxSceneNode {
                id: crate::fbx_scene::FbxNodeId(1),
                name: Some("StackedNode".to_string()),
                transform: Some(FbxTransform {
                    matrix: [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                        [1.25, -2.5, 3.75, 1.0],
                    ],
                }),
                transform_stack: Some(transform_stack.clone()),
                has_complex_transform_stack: true,
                mesh_instances: Vec::new(),
                attribute: None,
                children: Vec::new(),
            }],
            materials: Vec::new(),
            textures: Vec::new(),
            animations: Vec::new(),
            warnings: Vec::new(),
        };

        let output = FbxScene::from_bytes(&scene.to_bytes().unwrap()).unwrap();
        assert_eq!(
            output.root_nodes[0].transform_stack.as_ref(),
            Some(&transform_stack)
        );
    }

    #[test]
    fn fbx_matrix_arrays_use_column_major_layout() {
        // Real authoring tools (Maya/Blender/MotionBuilder) read FBX matrices
        // as column-major — Blender transposes on read via `array_to_matrix4`.
        // A 90-degree Y rotation with translation is asymmetric, so any
        // row-major encoding would be reconstructed transposed (the translation
        // collapses into the bottom row and the rotation mirrors). This test
        // asserts the on-disk byte layout directly so a future regression cannot
        // hide behind a symmetric round-trip.
        use crate::FbxTransform;
        let matrix = [
            [0.0, 0.0, 8.0, 0.0],
            [0.0, 3.0, 0.0, 0.0],
            [-2.0, 0.0, 0.0, 0.0],
            [1.0, 2.0, 3.0, 1.0],
        ];
        let column_major = super::flatten_fbx_transform(FbxTransform { matrix });
        assert_eq!(
            column_major,
            matrix
                .into_iter()
                .flatten()
                .map(f64::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[cfg(feature = "fbx-reader")]
    fn scene_roundtrip_preserves_skin_clusters_and_bind_pose() {
        let identity = crate::fbx_scene::FbxTransform {
            matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };
        let scene = FbxScene {
            global_settings: None,
            root_nodes: vec![FbxSceneNode {
                id: crate::fbx_scene::FbxNodeId(1),
                name: Some("Armature".to_string()),
                transform: None,
                transform_stack: None,
                has_complex_transform_stack: false,
                mesh_instances: Vec::new(),
                attribute: None,
                children: vec![
                    FbxSceneNode {
                        id: crate::fbx_scene::FbxNodeId(2),
                        name: Some("Bone".to_string()),
                        transform: Some(identity),
                        transform_stack: None,
                        has_complex_transform_stack: false,
                        mesh_instances: Vec::new(),
                        attribute: None,
                        children: Vec::new(),
                    },
                    FbxSceneNode {
                        id: crate::fbx_scene::FbxNodeId(3),
                        name: Some("Mesh".to_string()),
                        transform: Some(identity),
                        transform_stack: None,
                        has_complex_transform_stack: false,
                        mesh_instances: vec![FbxMeshInstance {
                            name: Some("Triangle".to_string()),
                            mesh: create_triangle_mesh(),
                            skin: Some(crate::fbx_scene::FbxSkin {
                                clusters: vec![crate::fbx_scene::FbxSkinCluster {
                                    joint_node_id: crate::fbx_scene::FbxNodeId(2),
                                    control_point_indices: vec![0, 1, 2],
                                    weights: vec![1.0, 0.5, 1.0],
                                    mesh_bind_transform: identity,
                                    joint_bind_transform: identity,
                                    armature_bind_transform: None,
                                }],
                                bind_pose: vec![
                                    (crate::fbx_scene::FbxNodeId(2), identity),
                                    (crate::fbx_scene::FbxNodeId(3), identity),
                                ],
                            }),
                            morph_targets: vec![crate::fbx_scene::FbxMorphTarget {
                                name: Some("Smile".to_string()),
                                control_point_indices: vec![1],
                                position_deltas: vec![[0.0, 0.25, 0.0]],
                                normal_deltas: None,
                                default_weight: 0.0,
                                full_weight: 100.0,
                            }],
                            ..Default::default()
                        }],
                        attribute: None,
                        children: Vec::new(),
                    },
                ],
            }],
            materials: Vec::new(),
            textures: Vec::new(),
            animations: Vec::new(),
            warnings: Vec::new(),
        };
        let output = FbxScene::from_bytes(&scene.to_bytes().unwrap()).unwrap();
        let mesh = &output.root_nodes[0].children[1].mesh_instances[0];
        let skin = mesh.skin.as_ref().expect("skin must round-trip");
        assert_eq!(skin.clusters.len(), 1);
        assert_eq!(
            skin.clusters[0].joint_node_id,
            crate::fbx_scene::FbxNodeId(2)
        );
        assert_eq!(skin.clusters[0].control_point_indices, vec![0, 1, 2]);
        assert_eq!(skin.clusters[0].weights, vec![1.0, 0.5, 1.0]);
        assert_eq!(skin.bind_pose.len(), 2);
        assert_eq!(mesh.morph_targets.len(), 1);
        assert_eq!(mesh.morph_targets[0].name.as_deref(), Some("Smile"));
        assert_eq!(
            mesh.morph_targets[0].position_deltas,
            vec![[0.0, 0.25, 0.0]]
        );
    }

    #[test]
    #[cfg(feature = "fbx-reader")]
    fn scene_roundtrip_preserves_cubic_tangents() {
        let scene = FbxScene {
            global_settings: None,
            root_nodes: vec![FbxSceneNode {
                id: crate::fbx_scene::FbxNodeId(1),
                name: Some("Root".to_string()),
                transform: None,
                transform_stack: None,
                has_complex_transform_stack: false,
                mesh_instances: Vec::new(),
                attribute: None,
                children: Vec::new(),
            }],
            materials: Vec::new(),
            textures: Vec::new(),
            animations: vec![FbxAnimation {
                name: Some("Cubic".to_string()),
                duration: 1.0,
                channels: vec![crate::fbx_scene::FbxAnimChannel {
                    node_id: crate::fbx_scene::FbxNodeId(1),
                    node_name: "Root".to_string(),
                    path: crate::fbx_scene::FbxAnimChannelPath::Translation,
                    morph_target_index: None,
                    sampler: crate::fbx_scene::FbxAnimSampler {
                        input: vec![0.0, 1.0],
                        output: vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0],
                        interpolation: crate::fbx_scene::FbxAnimInterpolation::Cubic,
                        in_tangents: Some(vec![0.0, 0.0, 0.0, 0.25, 0.5, 0.75]),
                        out_tangents: Some(vec![1.0, 2.0, 3.0, 0.0, 0.0, 0.0]),
                    },
                }],
            }],
            warnings: Vec::new(),
        };
        let output = FbxScene::from_bytes(&scene.to_bytes().unwrap()).unwrap();
        let sampler = &output.animations[0].channels[0].sampler;
        assert_eq!(
            sampler.interpolation,
            crate::fbx_scene::FbxAnimInterpolation::Cubic
        );
        assert_eq!(
            sampler.in_tangents.as_deref(),
            Some(&[0.0, 0.0, 0.0, 0.25, 0.5, 0.75][..])
        );
        assert_eq!(
            sampler.out_tangents.as_deref(),
            Some(&[1.0, 2.0, 3.0, 0.0, 0.0, 0.0][..])
        );
    }

    #[test]
    #[cfg(feature = "fbx-reader")]
    fn scene_roundtrip_preserves_per_polygon_materials_on_ngons() {
        // `LayerElementMaterial` is ByPolygon while `material_indices` is per
        // triangle. Writing the triangle list verbatim beside an n-gon polygon
        // stream made the reader take its first N entries as the polygon
        // assignments, so every quad after the first lost its material.
        let mut instance = FbxMeshInstance {
            name: Some("Quads".to_string()),
            mesh: create_triangle_mesh(),
            control_points: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [3.0, 1.0, 0.0],
                [2.0, 1.0, 0.0],
            ],
            // Two quads, so two polygons but four fan triangles.
            polygon_vertex_indices: vec![0, 1, 2, !3, 4, 5, 6, !7],
            material_indices: vec![0, 0, 1, 1],
            ..Default::default()
        };
        instance.mesh = instance.to_draco_mesh();

        let named = |name: &str| crate::fbx_scene::FbxMaterial {
            name: Some(name.to_string()),
            ..crate::fbx_scene::FbxMaterial::default()
        };
        let scene = FbxScene {
            root_nodes: vec![FbxSceneNode {
                id: crate::fbx_scene::FbxNodeId(1),
                name: Some("Node".to_string()),
                transform: None,
                transform_stack: None,
                has_complex_transform_stack: false,
                mesh_instances: vec![instance],
                attribute: None,
                children: Vec::new(),
            }],
            materials: vec![named("Red"), named("Blue")],
            ..FbxScene::default()
        };

        let output = crate::FbxScene::from_bytes(&scene.to_bytes().unwrap()).unwrap();
        assert_eq!(
            output.root_nodes[0].mesh_instances[0].material_indices,
            vec![0, 0, 1, 1],
            "the second quad should keep its own material"
        );
    }

    #[test]
    fn collapsing_material_indices_takes_one_entry_per_polygon() {
        // A quad and a triangle: 2 + 1 fan triangles.
        let collapsed =
            collapse_material_indices_to_polygons(&[7, 7, 3], Some(&[0, 1, 2, !3, 4, 5, !6]));
        assert_eq!(collapsed, vec![7, 3]);

        // Without a polygon stream the writer emits one triangle per polygon.
        assert_eq!(
            collapse_material_indices_to_polygons(&[1, 2, 3], None),
            vec![1, 2, 3]
        );
    }

    #[cfg(feature = "fbx-reader")]
    fn scene_with_tangents(has_handedness: bool) -> FbxScene {
        let set = crate::fbx_scene::FbxTangentSet {
            layer: crate::fbx_scene::FbxLayerSet {
                name: None,
                mapping: Some("ByPolygonVertex".to_string()),
                reference: Some("Direct".to_string()),
                values: vec![
                    [1.0, 0.0, 0.0, -1.0],
                    [0.0, 1.0, 0.0, -1.0],
                    [0.0, 0.0, 1.0, -1.0],
                ],
                indices: Vec::new(),
            },
            has_handedness,
        };
        FbxScene {
            root_nodes: vec![FbxSceneNode {
                id: crate::fbx_scene::FbxNodeId(1),
                name: Some("Tangential".to_string()),
                transform: None,
                transform_stack: None,
                has_complex_transform_stack: false,
                mesh_instances: vec![FbxMeshInstance {
                    name: Some("Tri".to_string()),
                    mesh: create_triangle_mesh(),
                    control_points: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    polygon_vertex_indices: vec![0, 1, !2],
                    layers: FbxMeshLayers {
                        tangent_sets: vec![set.clone()],
                        binormal_sets: vec![set],
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                attribute: None,
                children: Vec::new(),
            }],
            ..FbxScene::default()
        }
    }

    /// FBX keeps the handedness sign in a sibling array that only 7500 and
    /// later write. Merging it into `w` on read and splitting it again on write
    /// is an asymmetry that is easy to implement in one direction only, so
    /// both directions are asserted -- including that a document without the
    /// array does not acquire one, which would change its meaning for a reader
    /// that trusts the sign.
    #[test]
    #[cfg(feature = "fbx-reader")]
    fn handedness_survives_a_round_trip_and_is_not_invented() {
        for has_handedness in [true, false] {
            let bytes = scene_with_tangents(has_handedness).to_bytes().unwrap();
            let output = crate::FbxScene::from_bytes(&bytes).unwrap();
            let instance = &output.root_nodes[0].mesh_instances[0];

            assert_eq!(instance.layers.tangent_sets.len(), 1);
            assert_eq!(instance.layers.binormal_sets.len(), 1);
            let tangents = &instance.layers.tangent_sets[0];
            assert_eq!(tangents.has_handedness, has_handedness);

            let expected_w = if has_handedness { -1.0 } else { 1.0 };
            assert_eq!(
                tangents.layer.values[0],
                [1.0, 0.0, 0.0, expected_w],
                "handedness {has_handedness}: xyz must survive and w must \
                 {} ",
                if has_handedness {
                    "be read back"
                } else {
                    "default to +1"
                }
            );

            // The absence must be visible in the bytes, not merely in the
            // decoded flag: a reader other than ours looks for the node.
            let has_w_node = String::from_utf8_lossy(&bytes).contains("TangentsW");
            assert_eq!(
                has_w_node, has_handedness,
                "TangentsW node presence must match the source"
            );
        }
    }

    /// Smoothing flags and crease weights have different element types, and a
    /// shared one would round the weights. Both must survive a rewrite on the
    /// domain they were authored on, including `ByPolygon` smoothing, which is
    /// a small minority of the corpus and easy to leave unimplemented.
    #[test]
    #[cfg(feature = "fbx-reader")]
    fn smoothing_and_crease_layers_survive_a_round_trip() {
        let instance = FbxMeshInstance {
            name: Some("Creased".to_string()),
            mesh: create_triangle_mesh(),
            control_points: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            polygon_vertex_indices: vec![0, 1, !2],
            edges: vec![0, 1, 2],
            layers: FbxMeshLayers {
                smoothing_layers: vec![
                    crate::fbx_scene::FbxSmoothingLayer {
                        mapping: Some("ByEdge".to_string()),
                        values: vec![1, 0, 1],
                    },
                    crate::fbx_scene::FbxSmoothingLayer {
                        mapping: Some("ByPolygon".to_string()),
                        values: vec![1],
                    },
                ],
                crease_layers: vec![
                    crate::fbx_scene::FbxCreaseLayer {
                        kind: crate::fbx_scene::FbxCreaseKind::Edge,
                        mapping: Some("ByEdge".to_string()),
                        // A weight an integer type would flatten to 0.
                        values: vec![0.25, 0.5, 1.0],
                    },
                    crate::fbx_scene::FbxCreaseLayer {
                        kind: crate::fbx_scene::FbxCreaseKind::Vertex,
                        mapping: Some("ByVertice".to_string()),
                        values: vec![0.75, 0.0, 0.125],
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let scene = FbxScene {
            root_nodes: vec![FbxSceneNode {
                id: crate::fbx_scene::FbxNodeId(1),
                name: Some("Node".to_string()),
                transform: None,
                transform_stack: None,
                has_complex_transform_stack: false,
                mesh_instances: vec![instance.clone()],
                attribute: None,
                children: Vec::new(),
            }],
            ..FbxScene::default()
        };

        let output = crate::FbxScene::from_bytes(&scene.to_bytes().unwrap()).unwrap();
        let read_back = &output.root_nodes[0].mesh_instances[0];
        assert_eq!(
            read_back.layers.smoothing_layers,
            instance.layers.smoothing_layers
        );
        assert_eq!(
            read_back.layers.crease_layers,
            instance.layers.crease_layers
        );
    }

    /// A `ByEdge` layer in a geometry with no `Edges` array addresses the edges
    /// an importer would reconstruct from the faces. This crate does not
    /// reconstruct them, so it cannot check the length -- but discarding the
    /// layer would lose authored data a rewrite could otherwise return intact.
    #[test]
    #[cfg(feature = "fbx-reader")]
    fn a_by_edge_layer_survives_without_an_explicit_edges_array() {
        let smoothing = crate::fbx_scene::FbxSmoothingLayer {
            mapping: Some("ByEdge".to_string()),
            values: vec![1, 0, 1],
        };
        let scene = FbxScene {
            root_nodes: vec![FbxSceneNode {
                id: crate::fbx_scene::FbxNodeId(1),
                name: Some("Node".to_string()),
                transform: None,
                transform_stack: None,
                has_complex_transform_stack: false,
                mesh_instances: vec![FbxMeshInstance {
                    name: Some("Implicit".to_string()),
                    mesh: create_triangle_mesh(),
                    control_points: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    polygon_vertex_indices: vec![0, 1, !2],
                    layers: FbxMeshLayers {
                        smoothing_layers: vec![smoothing.clone()],
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                attribute: None,
                children: Vec::new(),
            }],
            ..FbxScene::default()
        };

        let output = crate::FbxScene::from_bytes(&scene.to_bytes().unwrap()).unwrap();
        let read_back = &output.root_nodes[0].mesh_instances[0];
        assert!(read_back.edges.is_empty());
        assert_eq!(read_back.layers.smoothing_layers, vec![smoothing]);
    }

    /// An FBX `Texture` need not be named, and a rewrite must not give it one.
    ///
    /// The writer substituted the class name, so a document with no texture
    /// names acquired them by passing through -- the same fabrication as
    /// naming an unnamed `Geometry` after its Model. Only an ASCII corpus file
    /// reached this, so it is pinned here rather than left to the opt-in
    /// corpus run, which CI does not have the data for.
    #[test]
    #[cfg(feature = "fbx-reader")]
    fn an_unnamed_texture_is_not_given_a_name_by_a_rewrite() {
        let scene = FbxScene {
            materials: vec![crate::fbx_scene::FbxMaterial {
                name: Some("M".to_string()),
                textures: vec![crate::fbx_scene::FbxTextureBinding {
                    slot: crate::fbx_scene::FbxTextureSlot::Diffuse,
                    texture_index: 0,
                }],
                ..Default::default()
            }],
            textures: vec![crate::fbx_scene::FbxTexture {
                name: None,
                content: None,
                filename: Some("t.png".to_string()),
            }],
            ..FbxScene::default()
        };

        let output = crate::FbxScene::from_bytes(&scene.to_bytes().unwrap()).unwrap();
        assert_eq!(output.textures.len(), 1);
        assert_eq!(
            output.textures[0].name, None,
            "an unnamed texture must stay unnamed"
        );
    }

    /// A colours-only geometry must still emit a `Layer` node listing
    /// `LayerElementColor`.
    ///
    /// Our own reader finds the element without it, so a round-trip check
    /// cannot see this; a strict importer walks `Layer` and would show an
    /// uncoloured mesh. The assertion is therefore on the written node tree.
    #[test]
    #[cfg(feature = "fbx-reader")]
    fn a_colour_only_geometry_lists_its_layer_element() {
        let scene = FbxScene {
            root_nodes: vec![FbxSceneNode {
                id: crate::fbx_scene::FbxNodeId(1),
                name: Some("Colored".to_string()),
                transform: None,
                transform_stack: None,
                has_complex_transform_stack: false,
                mesh_instances: vec![FbxMeshInstance {
                    name: Some("Tri".to_string()),
                    mesh: create_triangle_mesh(),
                    control_points: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    polygon_vertex_indices: vec![0, 1, !2],
                    layers: FbxMeshLayers {
                        color_sets: vec![crate::fbx_scene::FbxColorSet {
                            name: Some("Col".to_string()),
                            mapping: Some("ByPolygonVertex".to_string()),
                            reference: Some("Direct".to_string()),
                            values: vec![[1.0, 0.0, 0.0, 1.0]; 3],
                            indices: Vec::new(),
                        }],
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                attribute: None,
                children: Vec::new(),
            }],
            ..FbxScene::default()
        };

        let bytes = scene.to_bytes().unwrap();
        let nodes = crate::FbxReader::from_bytes(bytes)
            .unwrap()
            .read_nodes()
            .unwrap();

        fn find<'a>(
            nodes: &'a [crate::fbx_reader::FbxNode],
            name: &str,
        ) -> Option<&'a crate::fbx_reader::FbxNode> {
            nodes
                .iter()
                .find(|n| n.name == name)
                .or_else(|| nodes.iter().find_map(|n| find(&n.children, name)))
        }

        let layer = find(&nodes, "Layer").expect("colours alone must still produce a Layer node");
        let listed: Vec<&str> = layer
            .children
            .iter()
            .filter(|c| c.name == "LayerElement")
            .filter_map(|c| c.children.iter().find(|g| g.name == "Type"))
            .filter_map(|t| match t.properties.first() {
                Some(crate::fbx_reader::FbxProperty::String(s)) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            listed.contains(&"LayerElementColor"),
            "Layer must reference the colour element, listed: {listed:?}"
        );
    }

    #[test]
    #[cfg(feature = "fbx-reader")]
    fn scene_roundtrip_preserves_vertex_colors() {
        let colors = crate::fbx_scene::FbxColorSet {
            name: Some("Col".to_string()),
            mapping: Some("ByPolygonVertex".to_string()),
            reference: Some("Direct".to_string()),
            values: vec![
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 0.5],
            ],
            indices: Vec::new(),
        };
        let scene = FbxScene {
            root_nodes: vec![FbxSceneNode {
                id: crate::fbx_scene::FbxNodeId(1),
                name: Some("Colored".to_string()),
                transform: None,
                transform_stack: None,
                has_complex_transform_stack: false,
                mesh_instances: vec![FbxMeshInstance {
                    name: Some("Tri".to_string()),
                    mesh: create_triangle_mesh(),
                    control_points: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    polygon_vertex_indices: vec![0, 1, !2],
                    layers: FbxMeshLayers {
                        color_sets: vec![colors.clone()],
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                attribute: None,
                children: Vec::new(),
            }],
            ..FbxScene::default()
        };

        let output = crate::FbxScene::from_bytes(&scene.to_bytes().unwrap()).unwrap();
        let instance = &output.root_nodes[0].mesh_instances[0];
        assert_eq!(
            instance.layers.color_sets.len(),
            1,
            "colour layer should survive"
        );
        let read_back = &instance.layers.color_sets[0];
        assert_eq!(read_back.values, colors.values);
        assert_eq!(read_back.mapping.as_deref(), Some("ByPolygonVertex"));

        // Alpha must survive too, and reach the Draco mesh as a 4-component
        // Color attribute.
        let render = instance.to_render_mesh();
        assert_eq!(render.colors.len(), 1);
        assert_eq!(render.colors[0].values[2], [0.0, 0.0, 1.0, 0.5]);
        let id = instance
            .mesh
            .named_attribute_id(draco_core::geometry_attribute::GeometryAttributeType::Color);
        assert!(id >= 0, "the Draco mesh should carry a Color attribute");
        assert_eq!(instance.mesh.attribute(id).num_components(), 4);
    }

    #[test]
    #[cfg(feature = "fbx-reader")]
    fn written_files_satisfy_the_readers_strict_mode() {
        // Closes the loop: our own output must be a conventional FBX file,
        // footer included. This is what caught the truncated footer the
        // writer used to emit, since no other consumer reads it.
        let mut writer = FbxWriter::new();
        writer
            .add_mesh(&create_triangle_mesh(), Some("strict"))
            .unwrap();
        let bytes = writer.write_to_vec().unwrap();

        let scene =
            crate::FbxScene::from_bytes_with_options(&bytes, crate::FbxReadOptions::strict())
                .expect("writer output should pass strict validation");
        assert_eq!(scene.root_nodes.len(), 1);
    }

    #[cfg(feature = "compression")]
    #[test]
    fn test_write_with_compression() {
        let mesh = create_triangle_mesh();
        let mut writer = FbxWriter::new()
            .with_compression(true)
            .with_compression_threshold(0);
        Writer::add_mesh(&mut writer, &mesh, None).unwrap();

        let mut buffer = Cursor::new(Vec::new());
        writer.write_to(&mut buffer).unwrap();

        let data = buffer.into_inner();
        assert!(!data.is_empty());
    }
}
