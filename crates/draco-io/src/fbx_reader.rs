//! FBX scene reader: a tree of [`FbxNode`](crate::fbx_container::FbxNode) in,
//! an [`FbxScene`] out.
//!
//! Supports reading:
//! - Vertex positions, normals, texture coordinates, colours, and tangents
//! - Polygon/face indices, edges, smoothing flags, and crease weights
//! - Model hierarchy and local transforms through [`FbxReader::read_scene`]
//! - Phong/Lambert materials, textures, and per-polygon material indices
//! - Skin clusters, bind poses, and blend-shape targets
//! - Node-TRS animation (`AnimationStack` / `AnimationLayer` /
//!   `AnimationCurveNode` / `AnimationCurve`) flattened into TRS channels
//! - Cameras and lights, read into [`crate::FbxScene`] but never written back
//!
//! FBX pivots and inheritance rules and arbitrary metadata are not
//! represented.
//!
//! This module reads only the node tree; the containers that produce it are
//! [`crate::fbx_container`] for binary and [`crate::fbx_ascii`] for text.
//! Nothing here knows which one it was given, which is what lets one scene
//! layer serve both.
//!
//! # Example
//!
//! ```no_run
//! use draco_io::fbx_reader::FbxReader;
//! use draco_io::Reader;
//!
//! let mut reader = FbxReader::open("model.fbx")?;
//! let meshes = reader.read_meshes()?;
//! for mesh in meshes {
//!     println!("Mesh has {} vertices", mesh.num_points());
//! }
//! # Ok::<(), std::io::Error>(())
//! ```

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, Cursor, Read, Seek};
use std::path::Path;

use draco_core::mesh::Mesh;

use crate::fbx_scene::push_warning;
use crate::fbx_templates::{ObjectProperties, PropertyTemplates};
use crate::fbx_transform::{
    collect_transform_warnings, identity_transform, parse_transform, transform_array,
};

/// The container types, re-exported so `draco_io::fbx_reader::FbxNode` keeps
/// resolving for callers written before the decoder moved out.
pub use crate::fbx_container::{FbxMemoryReader, FbxNode, FbxProperty, FbxReader};

#[derive(Debug)]
struct FbxGeometrySource {
    mesh: Mesh,
    material_indices: Vec<i32>,
    control_points: Vec<[f32; 3]>,
    polygon_vertex_indices: Vec<i32>,
    layers: FbxMeshLayers,
    edges: Vec<i32>,
}

#[doc(hidden)]
pub use crate::fbx_scene::{
    FbxAnimChannel, FbxAnimChannelPath, FbxAnimInterpolation, FbxAnimSampler, FbxAnimation,
    FbxBinormalSet, FbxColorSet, FbxCreaseKind, FbxCreaseLayer, FbxLayerSet, FbxMeshInstance,
    FbxMeshLayers, FbxNodeAttribute, FbxNodeId, FbxNormalSet, FbxScene, FbxSceneNode,
    FbxSmoothingLayer, FbxTangentSet, FbxTexture, FbxTextureBinding, FbxTextureSlot, FbxTransform,
    FbxUvSet, FbxWarning, FbxWarningCode,
};
// Implement the Reader trait for the concrete BufReader<File> specialization.
impl crate::traits::Reader for FbxReader<BufReader<File>> {
    fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        FbxReader::open(path)
    }

    fn read_meshes(&mut self) -> io::Result<Vec<draco_core::mesh::Mesh>> {
        // Call the inherent method which already reads all meshes.
        // Use fully qualified syntax to avoid recursion.
        FbxReader::read_meshes(self)
    }
}

impl crate::traits::Reader for FbxReader<Cursor<Vec<u8>>> {
    fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::from_bytes(fs::read(path)?)
    }

    fn read_meshes(&mut self) -> io::Result<Vec<draco_core::mesh::Mesh>> {
        FbxReader::read_meshes(self)
    }
}
impl<R: Read + Seek> FbxReader<R> {
    /// Reads the supported hierarchy, materials, textures, and animation from FBX.
    ///
    /// The result retains model names, local transforms, materialized mesh
    /// geometry (positions, normals, UVs), per-polygon material indices,
    /// Phong/Lambert materials, textures, and node-TRS animation. It
    /// intentionally omits FBX pivots, pre/post rotations, inheritance rules,
    /// cameras, and arbitrary metadata. It retains skin clusters, bind poses,
    /// blend-shape deltas, and local TRS animation.
    ///
    /// ```no_run
    /// use draco_io::FbxMemoryReader;
    ///
    /// let bytes = std::fs::read("model.fbx")?;
    /// let mut reader = FbxMemoryReader::from_bytes(bytes)?;
    /// let scene = reader.read_scene()?;
    /// assert!(!scene.root_nodes.is_empty());
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn read_scene(&mut self) -> io::Result<FbxScene> {
        let nodes = self.read_nodes()?;
        let global_settings = parse_global_settings(&nodes);

        let index = FbxObjectIndex::build(&nodes);
        // Borrow the fields the rest of this function reads. `index` stays
        // whole so it can be handed to the animation pass as one argument.
        let FbxObjectIndex {
            model_map,
            model_order,
            geometry_map,
            attribute_map,
            material_map,
            texture_map,
            video_map,
            deformer_map,
            pose_map,
            connections,
            material_slots,
            names,
            templates,
            ..
        } = &index;

        // Container-layout notices raised by `read_nodes` above ride along
        // with the semantic ones, so a caller sees every tolerated deviation.
        let mut warnings = self.warnings().to_vec();
        collect_transform_warnings(model_map, model_order, templates, &mut warnings);
        let node_attributes = parse_node_attributes(
            attribute_map,
            model_map,
            connections,
            templates,
            &mut warnings,
        );

        // ---- Materials and textures ---------------------------------------
        let (materials, material_index_by_id, textures) = parse_materials_and_textures(
            material_map,
            texture_map,
            video_map,
            connections,
            templates,
        );

        // ---- Model hierarchy + per-model materials -----------------------
        // Each Model's slots, as scene material indices. The slot list is the
        // one place a repeated connection means a repeated thing, so it is
        // read from the index's own record of it rather than from the
        // deduplicated connection list.
        let model_material_ids: HashMap<i64, Vec<i32>> = material_slots
            .iter()
            .map(|(&model_id, slots)| {
                (
                    model_id,
                    slots
                        .iter()
                        .map(|id| material_index_by_id[id] as i32)
                        .collect(),
                )
            })
            .collect();

        // Build parent map for models (same as before but over FbxConnection).
        let mut model_children: HashMap<i64, Vec<i64>> = HashMap::new();
        for conn in connections.iter() {
            if model_map.contains_key(&conn.child) || model_map.contains_key(&conn.parent) {
                model_children
                    .entry(conn.parent)
                    .or_default()
                    .push(conn.child);
            }
        }

        let ordered_model_ids = model_order;
        let model_node_ids: HashMap<i64, FbxNodeId> = ordered_model_ids
            .iter()
            .copied()
            .enumerate()
            .map(|(index, id)| (id, FbxNodeId((index + 1) as u32)))
            .collect();

        // Map geometries to models and create mesh instances.
        let mut model_mesh_instances: std::collections::HashMap<i64, Vec<FbxMeshInstance>> =
            std::collections::HashMap::new();
        // Resolve a geometry's own material slots against the materials
        // connected to the Model that carries it.
        let resolve_material_indices =
            |source: &FbxGeometrySource, model_mats: Option<&Vec<i32>>| -> Vec<i32> {
                let indices = source.material_indices.clone();
                let Some(model_mats) = model_mats.filter(|mats| !mats.is_empty()) else {
                    // The Model has no material slots, so the layer's values
                    // name nothing. In a document with no `Material` objects
                    // at all (Revit exports) they are kept as written: there is
                    // nothing they could be confused with, and they still
                    // encode which faces belong together. With materials in
                    // the document they are dropped instead -- read through,
                    // they look like scene material indices, and the writer
                    // would resolve them against the material table and send
                    // the faces to materials the file never connected.
                    return if material_index_by_id.is_empty() {
                        indices
                    } else {
                        Vec::new()
                    };
                };
                if indices.is_empty() {
                    // One entry per triangulated face.
                    return vec![model_mats[0]; source.mesh.num_faces()];
                }
                // LayerElementMaterial values address the material slots
                // attached to this Model. Map them back to the document-wide
                // material indices exposed by FbxScene; a slot past the end
                // resolves to the first, as the reference reader does.
                indices
                    .into_iter()
                    .map(|slot| {
                        usize::try_from(slot)
                            .ok()
                            .and_then(|slot| model_mats.get(slot).copied())
                            .unwrap_or(model_mats[0])
                    })
                    .collect()
            };
        let mut geometry_ids: Vec<i64> = geometry_map.keys().copied().collect();
        geometry_ids.sort_unstable();
        let mut morph_channel_targets: std::collections::HashMap<i64, u32> =
            std::collections::HashMap::new();
        for geom_id in geometry_ids {
            let geom_node = geometry_map[&geom_id];
            if let Some(source) = geometry_to_mesh(geom_node, &mut warnings)? {
                // find connection mapping geometry -> model
                for conn in connections.iter() {
                    if conn.child == geom_id && model_map.contains_key(&conn.parent) {
                        let (mesh_instance, channel_targets) = build_mesh_instance(
                            geom_id,
                            geom_node,
                            &source,
                            resolve_material_indices(&source, model_material_ids.get(&conn.parent)),
                            names,
                            deformer_map,
                            pose_map,
                            connections,
                            &model_node_ids,
                            geometry_map,
                        );
                        morph_channel_targets.extend(channel_targets);
                        model_mesh_instances
                            .entry(conn.parent)
                            .or_default()
                            .push(mesh_instance);
                    }
                }
            }
        }

        // A pre-7000 document has no separate Geometry object: the mesh lives
        // on the Model itself, keyed by nothing but its node.
        for &model_id in ordered_model_ids.iter() {
            let model_node = model_map[&model_id];
            if !model_node
                .children
                .iter()
                .any(|child| child.name == "Vertices")
            {
                continue;
            }
            if let Some(source) = geometry_to_mesh(model_node, &mut warnings)? {
                let (mesh_instance, channel_targets) = build_mesh_instance(
                    model_id,
                    model_node,
                    &source,
                    resolve_material_indices(&source, model_material_ids.get(&model_id)),
                    names,
                    deformer_map,
                    pose_map,
                    connections,
                    &model_node_ids,
                    geometry_map,
                );
                morph_channel_targets.extend(channel_targets);
                model_mesh_instances
                    .entry(model_id)
                    .or_default()
                    .push(mesh_instance);
            }
        }

        // ---- Animation ----------------------------------------------------
        let model_name_map: HashMap<i64, String> = model_map
            .iter()
            .filter_map(|(id, node)| object_name(node).map(|name| (*id, name)))
            .collect();

        let mut animations = self.parse_animations(
            &nodes,
            &index,
            &model_name_map,
            &model_node_ids,
            &morph_animation_targets(
                geometry_map,
                deformer_map,
                connections,
                model_map,
                &morph_channel_targets,
            ),
        );
        if animations.is_empty() {
            // A pre-7000 document states no AnimationStack objects; its clips
            // live in the Takes section instead.
            animations = parse_takes_animations(
                &nodes,
                self.version(),
                &index,
                &model_name_map,
                &model_node_ids,
            );
        }

        // Build root nodes. A Model enters the scene through an object
        // connection to the document root, or through a parent Model that
        // did. Rooting by absence -- any Model with no Model-to-Model
        // connection of any kind -- resurrected objects the source kept out
        // of the scene graph: MotionBuilder binds its seven Producer
        // cameras and its Camera Switcher only by OP CurrentCamera records,
        // and a rewrite emitted them as eight real scene objects per file,
        // where Blender's importer skips them. Dropping such Models was
        // checked against the corpus first: thirteen of its 1456 documents
        // hold any, the only two that carry geometry at all hold NurbsCurve
        // objects this reader represents no scene data for, and none has a
        // Model parented under it.
        let connects_to_root = |id: i64| {
            connections
                .iter()
                .any(|conn| conn.kind == ConnectionKind::Oo && conn.child == id && conn.parent == 0)
        };
        let mut in_scene: std::collections::HashSet<i64> = ordered_model_ids
            .iter()
            .copied()
            .filter(|id| connects_to_root(*id))
            .collect();
        let mut frontier: Vec<i64> = in_scene.iter().copied().collect();
        while let Some(id) = frontier.pop() {
            if let Some(children) = model_children.get(&id) {
                for &child in children {
                    if model_map.contains_key(&child) && in_scene.insert(child) {
                        frontier.push(child);
                    }
                }
            }
        }
        let dropped = ordered_model_ids.len() - in_scene.len();
        if dropped > 0 {
            push_warning(
                &mut warnings,
                FbxWarningCode::UnconnectedModelDropped,
                format!(
                    "{dropped} FBX Models reach neither the document root nor a parent Model \
                     by object connection, so they are not part of the scene graph"
                ),
                None,
            );
        }
        let mut root_nodes = Vec::new();
        // Roots are the models connected straight to the document root that
        // no other Model parents -- a model with both kinds of connection
        // belongs under its parent, as before.
        let top_level: Vec<i64> = ordered_model_ids
            .iter()
            .copied()
            .filter(|id| {
                connects_to_root(*id)
                    && !connections
                        .iter()
                        .any(|conn| conn.child == *id && model_map.contains_key(&conn.parent))
            })
            .collect();

        let graph = ModelGraph {
            models: model_map,
            children: &model_children,
            mesh_instances: &model_mesh_instances,
            node_ids: &model_node_ids,
            attributes: &node_attributes,
            templates,
        };
        for id in top_level {
            root_nodes.push(build_model_node(id, &graph, &mut Vec::new(), &mut warnings));
        }

        Ok(FbxScene {
            global_settings,
            root_nodes,
            materials,
            textures,
            animations,
            warnings,
        })
    }
}

// Build nodes recursively
/// The `Name\0\x01Class` property of an object record, whichever position the
/// document's object model puts it at.
///
/// FBX 7000 writes `[id, Name\0\x01Class, class]`; FBX 6100 writes
/// `[Name\0\x01Class, class]` and states no id. Both containers normalize an
/// ASCII key into the binary spelling, so one form covers both.
fn object_key(node: &FbxNode) -> Option<&str> {
    node.properties.iter().find_map(|property| match property {
        FbxProperty::String(raw) if raw.contains('\0') => Some(raw.as_str()),
        _ => None,
    })
}

fn object_name(node: &FbxNode) -> Option<String> {
    object_key(node)?
        .split('\0')
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

/// The object record's class token, which follows the name key.
///
/// A 7.x record with an empty name has no key property to find the class by,
/// so its fixed `[id, "", class]` layout is read positionally.
pub(crate) fn object_class(node: &FbxNode) -> Option<&str> {
    if let Some(key) = object_key(node) {
        let after = node
            .properties
            .iter()
            .position(|property| matches!(property, FbxProperty::String(raw) if raw == key))?
            + 1;
        return match node.properties.get(after) {
            Some(FbxProperty::String(class)) => Some(class.as_str()),
            _ => None,
        };
    }
    match node.properties.get(2) {
        Some(FbxProperty::String(class)) => Some(class.as_str()),
        _ => None,
    }
}

/// The Model record's own class, when it declares the node to be something
/// other than a mesh.
///
/// A skin cluster names the joints it deforms with, but a rig holds joints no
/// cluster ever references — a bone's `*_end` tail helper carries no weights —
/// and an armature's root object is a `Null`. Both are joints or grouping
/// nodes by their Model class alone, and losing that left the writer nothing
/// to key on but cluster membership, which is how a round trip rewrote a
/// rig's tails as plain mesh Models and broke the chain importers form.
fn model_kind(node_src: &FbxNode) -> Option<crate::fbx_scene::FbxNodeKind> {
    match object_class(node_src) {
        Some("LimbNode" | "Limb") => Some(crate::fbx_scene::FbxNodeKind::Joint),
        Some("Null" | "Root") => Some(crate::fbx_scene::FbxNodeKind::Null),
        _ => None,
    }
}

/// Everything `build_model_node` needs about the document's model graph.
///
/// These six are only ever passed together, and passing them one by one put
/// the function at the argument limit before it had a resolver to carry.
struct ModelGraph<'a, 'n> {
    models: &'a std::collections::HashMap<i64, &'n FbxNode>,
    children: &'a std::collections::HashMap<i64, Vec<i64>>,
    mesh_instances: &'a std::collections::HashMap<i64, Vec<FbxMeshInstance>>,
    node_ids: &'a std::collections::HashMap<i64, FbxNodeId>,
    attributes: &'a std::collections::HashMap<i64, FbxNodeAttribute>,
    templates: &'a PropertyTemplates<'n>,
}

fn build_model_node(
    id: i64,
    graph: &ModelGraph<'_, '_>,
    ancestors: &mut Vec<i64>,
    warnings: &mut Vec<FbxWarning>,
) -> FbxSceneNode {
    let node_src = graph.models.get(&id).unwrap();
    let mut node = FbxSceneNode::new(object_name(node_src));
    node.id = graph.node_ids[&id];
    if let Some((transform, transform_stack, has_complex_transform_stack)) =
        parse_transform(ObjectProperties::new(node_src, graph.templates))
    {
        node.transform = Some(transform);
        node.transform_stack = Some(transform_stack);
        node.has_complex_transform_stack = has_complex_transform_stack;
    }
    node.attribute = graph.attributes.get(&id).cloned();
    node.kind = model_kind(node_src);
    if let Some(mesh_instances) = graph.mesh_instances.get(&id) {
        // `Geometric*` is authored on the Model, not on the Geometry, so it is
        // resolved here rather than in `build_mesh_instance`: one geometry
        // shared by several Models carries a different offset per Model.
        let geometric = crate::fbx_transform::parse_geometric_transform(ObjectProperties::new(
            node_src,
            graph.templates,
        ));
        node.mesh_instances
            .extend(mesh_instances.iter().cloned().map(|mut instance| {
                instance.geometric_transform = geometric.clone();
                instance
            }));
    }

    // The ancestor check below stops a plain cycle. This bounds the
    // rest: a document can chain models far deeper than any scene
    // graph needs, and the depth is the file's to choose.
    const MAX_MODEL_DEPTH: usize = 256;
    if ancestors.len() >= MAX_MODEL_DEPTH {
        // Said, and said once. Cutting the tree silently leaves a hole that
        // every skin cluster bound below it then reports on its own account: a
        // 340-bone chain produced 1116 warnings about missing joints and not
        // one word about where the joints went.
        push_warning(
            warnings,
            FbxWarningCode::ModelDepthLimitReached,
            format!(
                "FBX Model hierarchy runs deeper than the {MAX_MODEL_DEPTH} this reader \
                 descends, so what hangs below is not in the scene"
            ),
            node.name.as_deref(),
        );
        return node;
    }
    if let Some(children) = graph.children.get(&id) {
        ancestors.push(id);
        for &cid in children {
            // A document may connect a Model to one of its own
            // ancestors -- `synthetic_id_collision_7500` in the ufbx
            // corpus does -- and following that cycle recurses until
            // the stack is gone. The scene simply stops there.
            if graph.models.contains_key(&cid) && !ancestors.contains(&cid) {
                node.children
                    .push(build_model_node(cid, graph, ancestors, warnings));
            }
        }
        ancestors.pop();
    }
    node
}

fn parse_global_settings(nodes: &[FbxNode]) -> Option<crate::fbx_scene::FbxGlobalSettings> {
    let properties = nodes
        .iter()
        .find(|node| node.name == "GlobalSettings")?
        .children
        .iter()
        .find(|node| node.name == "Properties70" || node.name == "Properties60")?;
    let integer = |property: &FbxNode| {
        property.properties.iter().find_map(|value| match value {
            FbxProperty::I16(value) => Some(*value as i32),
            FbxProperty::I32(value) => Some(*value),
            FbxProperty::I64(value) => i32::try_from(*value).ok(),
            _ => None,
        })
    };
    let number = |property: &FbxNode| {
        property.properties.iter().find_map(|value| match value {
            FbxProperty::F32(value) => Some(f64::from(*value)),
            FbxProperty::F64(value) => Some(*value),
            _ => None,
        })
    };
    let mut result = crate::fbx_scene::FbxGlobalSettings::default();
    for property in &properties.children {
        let Some(FbxProperty::String(name)) = property.properties.first() else {
            continue;
        };
        match name.as_str() {
            "UpAxis" => result.up_axis = integer(property),
            "UpAxisSign" => result.up_axis_sign = integer(property),
            "FrontAxis" => result.front_axis = integer(property),
            "FrontAxisSign" => result.front_axis_sign = integer(property),
            "CoordAxis" => result.coord_axis = integer(property),
            "CoordAxisSign" => result.coord_axis_sign = integer(property),
            "UnitScaleFactor" => result.unit_scale_factor = number(property),
            "OriginalUnitScaleFactor" => result.original_unit_scale_factor = number(property),
            "TimeMode" => result.time_mode = integer(property),
            _ => {}
        }
    }
    (result != crate::fbx_scene::FbxGlobalSettings::default()).then_some(result)
}

fn child_i32_array(node: &FbxNode, child_name: &str) -> Vec<i32> {
    node.children
        .iter()
        .find(|child| child.name == child_name)
        .and_then(int_values)
        .unwrap_or_default()
}

fn child_f64_array(node: &FbxNode, child_name: &str) -> Vec<f64> {
    node.children
        .iter()
        .find(|child| child.name == child_name)
        .and_then(float_values)
        .unwrap_or_default()
}

/// Builds one mesh instance, whether its geometry came from a separate
/// `Geometry` object or -- pre-7000 -- from the `Model` itself. `id` is
/// whichever of the two the skin, morph and material connections are stated
/// against.
#[allow(clippy::too_many_arguments)]
fn build_mesh_instance(
    id: i64,
    node: &FbxNode,
    source: &FbxGeometrySource,
    material_indices: Vec<i32>,
    names: &NameInterner,
    deformer_map: &std::collections::HashMap<i64, &FbxNode>,
    pose_map: &std::collections::HashMap<i64, &FbxNode>,
    connections: &[FbxConnection],
    model_node_ids: &std::collections::HashMap<i64, FbxNodeId>,
    geometry_map: &std::collections::HashMap<i64, &FbxNode>,
) -> (FbxMeshInstance, std::collections::HashMap<i64, u32>) {
    let (morph_targets, morph_channel_targets) =
        parse_morph_targets_for_geometry(id, geometry_map, deformer_map, connections);
    (
        FbxMeshInstance {
            name: object_name(node),
            material_indices,
            mesh: source.mesh.clone(),
            control_points: source.control_points.clone(),
            polygon_vertex_indices: source.polygon_vertex_indices.clone(),
            layers: source.layers.clone(),
            edges: source.edges.clone(),
            skin: parse_skin_for_geometry(
                id,
                names,
                deformer_map,
                pose_map,
                connections,
                model_node_ids,
            ),
            morph_targets,
            // Filled in by `build_model_node`, which is where the attaching Model
            // and therefore the offset is known.
            geometric_transform: None,
        },
        morph_channel_targets,
    )
}

fn parse_skin_for_geometry(
    geometry_id: i64,
    names: &NameInterner,
    deformers: &std::collections::HashMap<i64, &FbxNode>,
    poses: &std::collections::HashMap<i64, &FbxNode>,
    connections: &[FbxConnection],
    model_node_ids: &std::collections::HashMap<i64, FbxNodeId>,
) -> Option<crate::fbx_scene::FbxSkin> {
    let skin_ids: Vec<i64> = connections
        .iter()
        .filter(|connection| {
            connection.kind == ConnectionKind::Oo && connection.parent == geometry_id
        })
        .map(|connection| connection.child)
        .filter(|id| {
            deformers
                .get(id)
                .and_then(|node| deformer_type(node).map(str::to_string))
                .as_deref()
                == Some("Skin")
        })
        .collect();
    if skin_ids.is_empty() {
        return None;
    }

    let mut clusters = Vec::new();
    for skin_id in skin_ids {
        for cluster_id in connections
            .iter()
            .filter(|connection| {
                connection.kind == ConnectionKind::Oo && connection.parent == skin_id
            })
            .map(|connection| connection.child)
        {
            let Some(cluster) = deformers.get(&cluster_id) else {
                continue;
            };
            if deformer_type(cluster) != Some("Cluster") {
                continue;
            }
            let Some(joint_model_id) = connections
                .iter()
                .find(|connection| {
                    connection.kind == ConnectionKind::Oo && connection.parent == cluster_id
                })
                .map(|connection| connection.child)
            else {
                continue;
            };
            let Some(&joint_node_id) = model_node_ids.get(&joint_model_id) else {
                continue;
            };
            // `Indexes` and `Weights` pair by position. Dropping an entry on
            // one side only shifts every later influence onto another control
            // point, so the arrays are filtered together: a negative index
            // loses its weight with it, arrays of unequal length reject the
            // cluster, and an `Indexes` array that cannot be read as `i32`
            // -- or is absent -- leaves nothing to pair and rejects it too,
            // rather than accepting a cluster that moves nothing.
            let raw_indices = child_i32_array(cluster, "Indexes");
            let weights = child_f64_array(cluster, "Weights")
                .into_iter()
                .map(|weight| weight as f32)
                .collect::<Vec<_>>();
            if raw_indices.len() != weights.len() {
                continue;
            }
            let (indices, weights): (Vec<u32>, Vec<f32>) = raw_indices
                .into_iter()
                .zip(weights)
                .filter_map(|(index, weight)| {
                    u32::try_from(index).ok().map(|index| (index, weight))
                })
                .unzip();
            if indices.is_empty() {
                continue;
            }
            clusters.push(crate::fbx_scene::FbxSkinCluster {
                joint_node_id,
                control_point_indices: indices,
                weights,
                mesh_bind_transform: transform_array(cluster, "Transform")
                    .unwrap_or_else(identity_transform),
                joint_bind_transform: transform_array(cluster, "TransformLink")
                    .unwrap_or_else(identity_transform),
                armature_bind_transform: transform_array(cluster, "TransformAssociateModel"),
            });
        }
    }

    let mut bind_pose = Vec::new();
    // Walk poses in id order. The dedup below is first-wins, so hash order
    // would decide which `Pose` supplies a node's matrix when a file has more
    // than one, and two reads of the same bytes could disagree.
    let mut pose_ids: Vec<i64> = poses.keys().copied().collect();
    pose_ids.sort_unstable();
    for pose in pose_ids.iter().map(|id| poses[id]) {
        let is_bind_pose = pose
            .children
            .iter()
            .find(|child| child.name == "Type")
            .and_then(|child| child.properties.first())
            .and_then(|value| match value {
                FbxProperty::String(value) => Some(value == "BindPose"),
                _ => None,
            })
            .unwrap_or(false);
        if !is_bind_pose {
            continue;
        }
        for pose_node in &pose.children {
            if pose_node.name != "PoseNode" {
                continue;
            }
            let model_id = pose_node
                .children
                .iter()
                .find(|child| child.name == "Node")
                .and_then(|child| child.properties.first())
                // Through a lookup that accepts either width, because ASCII
                // does not record an integer's: an id that fits in 32 bits
                // comes back as an `I32` there and the whole bind pose was
                // dropped. Authored exports use ids far above that range, so
                // only a document with small ids -- this crate's own output --
                // showed it. A pre-7000 document names the node by key.
                .and_then(|value| match value {
                    FbxProperty::I64(value) => Some(*value),
                    FbxProperty::I32(value) => Some(i64::from(*value)),
                    FbxProperty::String(key) => names.lookup(key),
                    _ => None,
                });
            let matrix = transform_array(pose_node, "Matrix");
            if let (Some(_model_id), Some(matrix), Some(&node_id)) = (
                model_id,
                matrix,
                model_id.and_then(|id| model_node_ids.get(&id)),
            ) {
                if !bind_pose.iter().any(|(existing, _)| *existing == node_id) {
                    bind_pose.push((node_id, matrix));
                }
            }
        }
    }
    Some(crate::fbx_scene::FbxSkin {
        clusters,
        bind_pose,
    })
}

fn child_f64(node: &FbxNode, name: &str) -> Option<f64> {
    node.children
        .iter()
        .find(|child| child.name == name)
        .and_then(|child| child.properties.first())
        .and_then(|value| match value {
            FbxProperty::F64(value) => Some(*value),
            FbxProperty::F32(value) => Some(*value as f64),
            // ASCII writes a whole-valued double without a decimal point, so a
            // `DeformPercent: 100` arrives as an integer and would otherwise
            // read as a missing weight.
            FbxProperty::I32(value) => Some(f64::from(*value)),
            FbxProperty::I64(value) => Some(*value as f64),
            _ => None,
        })
}

/// Read the blend-shape targets of one geometry and record where each
/// BlendShapeChannel landed in the flat target list.
///
/// The animation reader must name a target by this list's index, so the two
/// have to come out of one traversal: an independent count over channels
/// would disagree whenever a geometry carries more than one `BlendShape`
/// deformer, a channel carries more than one shape, or a shape fails its
/// validation and is dropped -- and two channels numbered alike collapse into
/// one animation group, losing a curve in silence.
///
/// The map holds the first target a channel produced: a channel's animation
/// drives its weight, and the scene exposes one target index per channel.
fn parse_morph_targets_for_geometry(
    geometry_id: i64,
    geometries: &std::collections::HashMap<i64, &FbxNode>,
    deformers: &std::collections::HashMap<i64, &FbxNode>,
    connections: &[FbxConnection],
) -> (
    Vec<crate::fbx_scene::FbxMorphTarget>,
    std::collections::HashMap<i64, u32>,
) {
    let mut targets = Vec::new();
    let mut channel_targets = std::collections::HashMap::new();
    for blend_shape_id in connections
        .iter()
        .filter(|connection| {
            connection.kind == ConnectionKind::Oo && connection.parent == geometry_id
        })
        .map(|connection| connection.child)
    {
        let Some(blend_shape) = deformers.get(&blend_shape_id) else {
            continue;
        };
        if deformer_type(blend_shape) != Some("BlendShape") {
            continue;
        }
        for channel_id in connections
            .iter()
            .filter(|connection| {
                connection.kind == ConnectionKind::Oo && connection.parent == blend_shape_id
            })
            .map(|connection| connection.child)
        {
            let Some(channel) = deformers.get(&channel_id) else {
                continue;
            };
            if deformer_type(channel) != Some("BlendShapeChannel") {
                continue;
            }
            for shape_id in connections
                .iter()
                .filter(|connection| {
                    connection.kind == ConnectionKind::Oo && connection.parent == channel_id
                })
                .map(|connection| connection.child)
            {
                let Some(shape) = geometries.get(&shape_id) else {
                    continue;
                };
                let indices = child_i32_array(shape, "Indexes")
                    .into_iter()
                    .filter_map(|index| u32::try_from(index).ok())
                    .collect::<Vec<_>>();
                let vertices = child_f64_array(shape, "Vertices");
                if vertices.len() != indices.len() * 3 {
                    continue;
                }
                let position_deltas = vertices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|values| [values[0] as f32, values[1] as f32, values[2] as f32])
                    .collect();
                let full_weight = child_f64_array(channel, "FullWeights")
                    .first()
                    .copied()
                    .unwrap_or(100.0) as f32;
                channel_targets
                    .entry(channel_id)
                    .or_insert_with(|| targets.len() as u32);
                targets.push(crate::fbx_scene::FbxMorphTarget {
                    name: match shape.properties.get(1) {
                        Some(FbxProperty::String(name)) => {
                            name.split('\0').next().map(str::to_string)
                        }
                        _ => None,
                    },
                    control_point_indices: indices,
                    position_deltas,
                    normal_deltas: None,
                    default_weight: child_f64(channel, "DeformPercent").unwrap_or(0.0) as f32,
                    full_weight,
                });
            }
        }
    }
    (targets, channel_targets)
}

/// Resolve BlendShapeChannel object ids to their owning Model and target slot.
/// FBX animation curves target the channel deformer rather than the mesh
/// model, so this bridge is required to expose them through the scene API.
///
/// `channel_targets` comes from [`parse_morph_targets_for_geometry`], the same
/// traversal that built the target list the slot indexes into.
fn morph_animation_targets(
    geometries: &std::collections::HashMap<i64, &FbxNode>,
    deformers: &std::collections::HashMap<i64, &FbxNode>,
    connections: &[FbxConnection],
    models: &std::collections::HashMap<i64, &FbxNode>,
    channel_targets: &std::collections::HashMap<i64, u32>,
) -> std::collections::HashMap<i64, (i64, u32)> {
    let mut result = std::collections::HashMap::new();
    for geometry_id in geometries.keys().copied() {
        let Some(model_id) = connections.iter().find_map(|connection| {
            (connection.kind == ConnectionKind::Oo
                && connection.child == geometry_id
                && models.contains_key(&connection.parent))
            .then_some(connection.parent)
        }) else {
            continue;
        };
        for channel_id in connections
            .iter()
            .filter(|connection| {
                connection.kind == ConnectionKind::Oo
                    && connection.parent == geometry_id
                    && deformers
                        .get(&connection.child)
                        .and_then(|node| deformer_type(node))
                        == Some("BlendShape")
            })
            .map(|connection| connection.child)
            .flat_map(|blend_shape_id| {
                connections
                    .iter()
                    .filter(move |connection| {
                        connection.kind == ConnectionKind::Oo
                            && connection.parent == blend_shape_id
                            && deformers
                                .get(&connection.child)
                                .and_then(|node| deformer_type(node))
                                == Some("BlendShapeChannel")
                    })
                    .map(|connection| connection.child)
            })
        {
            if let Some(&target_index) = channel_targets.get(&channel_id) {
                result.insert(channel_id, (model_id, target_index));
            }
        }
    }
    result
}

/// FBX object-to-object connection type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ConnectionKind {
    /// `OO` object-to-object connection.
    Oo,
    /// `OP` object-to-property connection (carries a property name).
    Op,
}

/// Every `Objects` entry indexed by id, plus the `Connections` graph.
///
/// Built once per document. The maps are for lookup only: iterating a
/// `HashMap` would make node ids, root order and channel order depend on the
/// process rather than the file, so anything order-sensitive walks
/// [`Self::model_order`] or a sorted key list instead.
struct FbxObjectIndex<'a> {
    model_map: HashMap<i64, &'a FbxNode>,
    /// Authored `Model` order, which `model_map` cannot preserve.
    model_order: Vec<i64>,
    geometry_map: HashMap<i64, &'a FbxNode>,
    material_map: HashMap<i64, &'a FbxNode>,
    texture_map: HashMap<i64, &'a FbxNode>,
    video_map: HashMap<i64, &'a FbxNode>,
    astack_map: HashMap<i64, &'a FbxNode>,
    alayer_map: HashMap<i64, &'a FbxNode>,
    acnode_map: HashMap<i64, &'a FbxNode>,
    acurve_map: HashMap<i64, &'a FbxNode>,
    deformer_map: HashMap<i64, &'a FbxNode>,
    pose_map: HashMap<i64, &'a FbxNode>,
    attribute_map: HashMap<i64, &'a FbxNode>,
    connections: Vec<FbxConnection>,
    /// Each Model's material slots, in the order the document connects them
    /// and with the repeats it spells.
    ///
    /// `LayerElementMaterial` addresses this list by position, so a Model that
    /// connects one Material into two of its slots means two slots and the
    /// second is not the same as the first being stated twice. This is the one
    /// reading of a connection that counts copies, which is why it is taken
    /// before `connections` drops them.
    material_slots: HashMap<i64, Vec<i64>>,
    /// The name keys of a pre-7000 document, for resolving the references
    /// that reach the scene passes after this index was built.
    names: NameInterner,
    /// Class defaults the document states once, in `Definitions`.
    templates: PropertyTemplates<'a>,
}

impl<'a> FbxObjectIndex<'a> {
    fn build(nodes: &'a [FbxNode]) -> Self {
        let mut index = Self {
            model_map: HashMap::new(),
            model_order: Vec::new(),
            geometry_map: HashMap::new(),
            material_map: HashMap::new(),
            texture_map: HashMap::new(),
            video_map: HashMap::new(),
            astack_map: HashMap::new(),
            alayer_map: HashMap::new(),
            acnode_map: HashMap::new(),
            acurve_map: HashMap::new(),
            deformer_map: HashMap::new(),
            pose_map: HashMap::new(),
            attribute_map: HashMap::new(),
            connections: Vec::new(),
            material_slots: HashMap::new(),
            names: NameInterner::default(),
            templates: PropertyTemplates::build(nodes),
        };
        // Keyed by whichever spelling identifies objects in this document:
        // an `i64` id in 7000 and later, an interned name key before that.
        let mut names = NameInterner::default();

        for node in nodes {
            if node.name == "Objects" {
                for child in &node.children {
                    let id = child
                        .properties
                        .first()
                        .and_then(|property| object_ref(property, &mut names));
                    if let Some(id) = id {
                        match child.name.as_str() {
                            "Model" => {
                                // Keep the authored order only for ids seen first.
                                let first_occurrence = index.model_map.insert(id, child).is_none();
                                if first_occurrence {
                                    index.model_order.push(id);
                                }
                            }
                            "Geometry" => drop(index.geometry_map.insert(id, child)),
                            "Material" => drop(index.material_map.insert(id, child)),
                            "Texture" => drop(index.texture_map.insert(id, child)),
                            "Video" => drop(index.video_map.insert(id, child)),
                            "AnimationStack" => drop(index.astack_map.insert(id, child)),
                            "AnimationLayer" => drop(index.alayer_map.insert(id, child)),
                            "AnimationCurveNode" => drop(index.acnode_map.insert(id, child)),
                            "AnimationCurve" => drop(index.acurve_map.insert(id, child)),
                            "Pose" => drop(index.pose_map.insert(id, child)),
                            "NodeAttribute" => drop(index.attribute_map.insert(id, child)),
                            "Deformer" => drop(index.deformer_map.insert(id, child)),
                            _ => {}
                        }
                    }
                }
            } else if node.name == "Connections" {
                index.connections.extend(
                    node.children
                        .iter()
                        .filter_map(|child| FbxConnection::from_node(child, &mut names)),
                );
            }
        }
        index.names = names;
        // Objects and Connections are separate records and a document may
        // spell them in either order, so the slots are read once the loop
        // above has seen both.
        for connection in &index.connections {
            if connection.kind == ConnectionKind::Oo
                && index.material_map.contains_key(&connection.child)
                && index.model_map.contains_key(&connection.parent)
            {
                index
                    .material_slots
                    .entry(connection.parent)
                    .or_default()
                    .push(connection.child);
            }
        }
        // Everything else reads a connection as a relation, where stating it
        // twice states it once. Those readings walk the list per object and
        // nest -- a geometry's BlendShapes, their channels, their shapes --
        // so a repeated edge does not cost one extra target but multiplies
        // the whole subtree under it, and a document may repeat an edge as
        // often as it has room to spell it. ZBrush connects a BlendShape to
        // its geometry twice in an ordinary export, which decoded to each of
        // its shapes twice over.
        let mut seen: std::collections::HashSet<(ConnectionKind, i64, i64, Option<String>)> =
            std::collections::HashSet::new();
        index.connections.retain(|connection| {
            seen.insert((
                connection.kind,
                connection.child,
                connection.parent,
                connection.property.clone(),
            ))
        });
        index
    }
}

/// Assigns dense ids to the name keys of a pre-7000 document.
///
/// The scene machinery addresses objects by `i64`, so a 6100 document's
/// `"Name\0\x01Class"` keys are interned into that space instead of being
/// carried alongside it. The root object `Scene\0\x01Model` interns to 0,
/// which is the id 7.x uses for the document root, so connection handling
/// does not need to know which layout it is reading. A 6100 document states
/// no numeric ids of its own, so the two spaces never mix.
#[derive(Default)]
struct NameInterner {
    ids: HashMap<String, i64>,
    next: i64,
}

impl NameInterner {
    /// The id a name key interns to, without assigning one to an unseen key.
    fn lookup(&self, key: &str) -> Option<i64> {
        self.ids.get(key).copied()
    }

    fn intern(&mut self, key: &str) -> i64 {
        if let Some(id) = self.ids.get(key) {
            return *id;
        }
        let id = if key == "Scene\0\x01Model" {
            0
        } else {
            self.next += 1;
            self.next
        };
        self.ids.insert(key.to_string(), id);
        id
    }
}

/// Reads an FBX object reference, whatever identifies it.
///
/// The binary container always writes 7.x ids as `i64`; ASCII writes a bare
/// number, so an id small enough to fit in `i32` arrives as one -- matching
/// only `I64` skipped every object in such a document, and the scene came back
/// empty with nothing to explain it. A pre-7000 document keys its objects by
/// the `"Name\0\x01Class"` string instead, and references them by the same
/// string, which resolves through the document's name interner.
fn object_ref(property: &FbxProperty, names: &mut NameInterner) -> Option<i64> {
    match property {
        FbxProperty::I64(value) => Some(*value),
        FbxProperty::I32(value) => Some(i64::from(*value)),
        FbxProperty::String(key) => Some(names.intern(key)),
        _ => None,
    }
}

/// Reads an `i64` array off a node, in either spelling the object model uses.
///
/// Animation key times exceed `i32` -- one second of FBX KTime is 46186158000
/// ticks -- so they get their own reader rather than sharing [`int_values`].
fn i64_values(node: &FbxNode) -> Option<Vec<i64>> {
    if let Some(FbxProperty::I64Array(values)) = node.properties.first() {
        return Some(values.clone());
    }
    let scalars = node
        .properties
        .iter()
        .map(|value| match value {
            FbxProperty::I32(value) => Some(i64::from(*value)),
            FbxProperty::I64(value) => Some(*value),
            _ => None,
        })
        .collect::<Option<Vec<i64>>>()?;
    (!scalars.is_empty()).then_some(scalars)
}

/// Reads an `f64` array off a node, in either spelling the object model uses.
///
/// Version 7000 writes one typed array property, as does ASCII of any version.
/// Binary 6100 stores the payload as repeated scalar properties on the node,
/// so the values are gathered from those when no array property leads.
fn float_values(node: &FbxNode) -> Option<Vec<f64>> {
    match node.properties.first() {
        Some(FbxProperty::F64Array(values)) => return Some(values.clone()),
        Some(FbxProperty::F32Array(values)) => {
            return Some(values.iter().copied().map(f64::from).collect())
        }
        _ => {}
    }
    let scalars = node
        .properties
        .iter()
        .map(|value| match value {
            FbxProperty::F64(value) => Some(*value),
            FbxProperty::F32(value) => Some(f64::from(*value)),
            // An exporter can write whole-number coordinates as integers; the
            // ASCII container coerces the same shape to floats by schema.
            FbxProperty::I32(value) => Some(f64::from(*value)),
            FbxProperty::I64(value) => Some(*value as f64),
            _ => None,
        })
        .collect::<Option<Vec<f64>>>()?;
    (!scalars.is_empty()).then_some(scalars)
}

/// Reads an `i32` array off a node, in either spelling the object model uses.
///
/// The typed-array case first, then the repeated scalar properties of a binary
/// 6100 document -- see [`float_values`].
fn int_values(node: &FbxNode) -> Option<Vec<i32>> {
    match node.properties.first() {
        Some(FbxProperty::I32Array(values)) => return Some(values.clone()),
        // An out-of-range value here means the file lies about being an i32
        // array; narrowing it with `as` would silently turn it into a
        // different, in-range index instead of refusing to decode it.
        Some(FbxProperty::I64Array(values)) => {
            return values
                .iter()
                .map(|value| i32::try_from(*value).ok())
                .collect();
        }
        _ => {}
    }
    let scalars = node
        .properties
        .iter()
        .map(|value| match value {
            FbxProperty::I32(value) => Some(*value),
            FbxProperty::I64(value) => i32::try_from(*value).ok(),
            _ => None,
        })
        .collect::<Option<Vec<i32>>>()?;
    (!scalars.is_empty()).then_some(scalars)
}

/// A parsed FBX connection entry.
#[derive(Debug, Clone)]
struct FbxConnection {
    kind: ConnectionKind,
    child: i64,
    parent: i64,
    property: Option<String>,
}

impl FbxConnection {
    /// Parses one `C` entry (or its 6100 spelling, `Connect`), skipping
    /// relation codes this reader ignores.
    fn from_node(node: &FbxNode, names: &mut NameInterner) -> Option<Self> {
        let kind = match node.properties.first() {
            Some(FbxProperty::String(code)) if code == "OO" => ConnectionKind::Oo,
            Some(FbxProperty::String(code)) if code == "OP" => ConnectionKind::Op,
            _ => return None,
        };
        let child = node
            .properties
            .get(1)
            .and_then(|property| object_ref(property, names))?;
        let parent = node
            .properties
            .get(2)
            .and_then(|property| object_ref(property, names))?;
        let property = match node.properties.get(3) {
            Some(FbxProperty::String(name)) => Some(name.clone()),
            _ => None,
        };
        Some(Self {
            kind,
            child,
            parent,
            property,
        })
    }
}

/// Decodes every `Material` and `Texture` object, resolving each material's
/// texture bindings to indices into the returned texture list.
///
/// Both lists are ordered by FBX object id rather than hash order, so a
/// document always decodes to the same material and texture indices.
fn parse_materials_and_textures<'a>(
    material_map: &HashMap<i64, &'a FbxNode>,
    texture_map: &HashMap<i64, &FbxNode>,
    video_map: &HashMap<i64, &FbxNode>,
    connections: &[FbxConnection],
    templates: &PropertyTemplates<'a>,
) -> (
    Vec<crate::fbx_scene::FbxMaterial>,
    HashMap<i64, usize>,
    Vec<crate::fbx_scene::FbxTexture>,
) {
    let mut materials: Vec<crate::fbx_scene::FbxMaterial> = Vec::new();
    let mut material_index_by_id: HashMap<i64, usize> = HashMap::new();
    let mut material_ids: Vec<i64> = material_map.keys().copied().collect();
    material_ids.sort_unstable();
    for id in material_ids {
        let mut material = parse_material(ObjectProperties::new(material_map[&id], templates));
        material.textures = collect_material_texture_bindings(id, texture_map, connections);
        material_index_by_id.insert(id, materials.len());
        materials.push(material);
    }

    // Map each Texture to the Video that carries its bytes. FBX writes the
    // connection in either direction, so accept both.
    let mut texture_video: HashMap<i64, i64> = HashMap::new();
    for conn in connections {
        if conn.kind != ConnectionKind::Oo {
            continue;
        }
        if texture_map.contains_key(&conn.child) && video_map.contains_key(&conn.parent) {
            texture_video.entry(conn.child).or_insert(conn.parent);
        }
        if video_map.contains_key(&conn.child) && texture_map.contains_key(&conn.parent) {
            texture_video.entry(conn.parent).or_insert(conn.child);
        }
    }

    let mut textures: Vec<crate::fbx_scene::FbxTexture> = Vec::new();
    let mut texture_index_by_id: HashMap<i64, usize> = HashMap::new();
    let mut texture_ids: Vec<i64> = texture_map.keys().copied().collect();
    texture_ids.sort_unstable();
    for id in texture_ids {
        let mut texture = parse_texture(texture_map[&id]);
        if let Some(video) = texture_video.get(&id).and_then(|id| video_map.get(id)) {
            let from_video = parse_texture(video);
            texture.content = texture.content.or(from_video.content);
            texture.filename = texture.filename.or(from_video.filename);
            texture.name = texture.name.or(from_video.name);
        }
        texture_index_by_id.insert(id, textures.len());
        textures.push(texture);
    }

    // Bindings carry FBX texture ids until now; rewrite them as scene indices.
    for material in &mut materials {
        for binding in &mut material.textures {
            let fbx_id = binding.texture_index as i64;
            if let Some(&resolved) = texture_index_by_id.get(&fbx_id) {
                binding.texture_index = resolved;
            }
        }
    }

    (materials, material_index_by_id, textures)
}

/// FBX Deformer objects carry their effective kind after the name key; the
/// second name component is merely `Deformer`/`SubDeformer`.
fn deformer_type(node: &FbxNode) -> Option<&str> {
    object_class(node).filter(|value| !value.is_empty())
}

/// Collects material property texture bindings as placeholders; the FBX
/// texture id is stored in `texture_index` and resolved to a scene index by
/// the caller after the texture list is finalized.
fn collect_material_texture_bindings(
    material_id: i64,
    texture_map: &std::collections::HashMap<i64, &FbxNode>,
    connections: &[FbxConnection],
) -> Vec<crate::fbx_scene::FbxTextureBinding> {
    let mut bindings = Vec::new();
    for conn in connections {
        if conn.kind != ConnectionKind::Op || conn.parent != material_id {
            continue;
        }
        let Some(slot_name) = conn.property.as_deref() else {
            continue;
        };
        let Some(slot) = crate::fbx_scene::FbxTextureSlot::from_property_name(slot_name) else {
            continue;
        };
        if !texture_map.contains_key(&conn.child) {
            continue;
        }
        bindings.push(crate::fbx_scene::FbxTextureBinding {
            slot,
            texture_index: conn.child as usize,
        });
    }
    bindings
}

/// Resolves each Model's `NodeAttribute`, keeping the classes this crate
/// represents and reporting the rest.
///
/// Attributes are attached to their Model by an ordinary object connection, so
/// this walks the connection list once rather than searching per node. Ids are
/// visited in sorted order so a document always produces the same warnings in
/// the same order.
fn parse_node_attributes<'a>(
    attribute_map: &HashMap<i64, &'a FbxNode>,
    model_map: &HashMap<i64, &FbxNode>,
    connections: &[FbxConnection],
    templates: &PropertyTemplates<'a>,
    warnings: &mut Vec<FbxWarning>,
) -> HashMap<i64, FbxNodeAttribute> {
    let mut by_model: Vec<(i64, i64)> = connections
        .iter()
        .filter(|conn| {
            attribute_map.contains_key(&conn.child) && model_map.contains_key(&conn.parent)
        })
        .map(|conn| (conn.parent, conn.child))
        .collect();
    by_model.sort_unstable();

    let mut resolved = HashMap::new();
    for (model_id, attribute_id) in by_model {
        let node = attribute_map[&attribute_id];
        let Some(class) = object_class(node) else {
            continue;
        };
        match class {
            "Camera" => {
                let properties = ObjectProperties::new(node, templates);
                resolved.insert(model_id, FbxNodeAttribute::Camera(parse_camera(properties)));
            }
            "Light" => {
                let properties = ObjectProperties::new(node, templates);
                resolved.insert(model_id, FbxNodeAttribute::Light(parse_light(properties)));
            }
            // A skeleton attribute is consumed by the skin path and a `Null`
            // carries nothing but a transform, so neither is a loss worth
            // reporting. The rest describe something the scene will not have.
            "LimbNode" | "Limb" | "Null" | "Root" => {}
            other => push_warning(
                warnings,
                FbxWarningCode::DroppedNodeAttribute,
                format!(
                    "FBX NodeAttribute of class {other} is not represented, so its properties \
                     are absent from the scene"
                ),
                Some(other),
            ),
        }
    }
    resolved
}

fn parse_camera(properties: ObjectProperties<'_>) -> crate::fbx_scene::FbxCamera {
    let scalar = |name: &str| properties.get(name).and_then(property_scalar);
    let vector = |name: &str| properties.get(name).and_then(property_vec3);
    crate::fbx_scene::FbxCamera {
        position: vector("Position"),
        interest_position: vector("InterestPosition"),
        up_vector: vector("UpVector"),
        projection_type: scalar("CameraProjectionType").map(|v| v as i32),
        field_of_view: scalar("FieldOfView"),
        field_of_view_x: scalar("FieldOfViewX"),
        field_of_view_y: scalar("FieldOfViewY"),
        focal_length: scalar("FocalLength"),
        near_plane: scalar("NearPlane"),
        far_plane: scalar("FarPlane"),
        aspect_width: scalar("AspectWidth"),
        aspect_height: scalar("AspectHeight"),
        film_width: scalar("FilmWidth"),
        film_height: scalar("FilmHeight"),
        film_aspect_ratio: scalar("FilmAspectRatio"),
        aperture_mode: scalar("ApertureMode").map(|v| v as i32),
        ortho_zoom: scalar("OrthoZoom"),
    }
}

fn parse_light(properties: ObjectProperties<'_>) -> crate::fbx_scene::FbxLight {
    let scalar = |name: &str| properties.get(name).and_then(property_scalar);
    crate::fbx_scene::FbxLight {
        light_type: scalar("LightType").map(|v| v as i32),
        color: properties.get("Color").and_then(property_vec3),
        intensity: scalar("Intensity"),
        cast_light: scalar("CastLight").map(|v| v != 0.0),
        cast_shadows: scalar("CastShadows").map(|v| v != 0.0),
        decay_type: scalar("DecayType").map(|v| v as i32),
        decay_start: scalar("DecayStart"),
    }
}

/// Where a property record's values start.
///
/// A `Properties70` `P` record leads with four strings (`name, type, subtype,
/// flags`); a `Properties60` `Property` record leads with three. The values
/// follow in both.
fn value_offset(prop: &FbxNode) -> usize {
    if prop.name == "Property" {
        3
    } else {
        4
    }
}

fn property_scalar(prop: &FbxNode) -> Option<f32> {
    for value in prop.properties.iter().skip(value_offset(prop)) {
        match value {
            FbxProperty::F64(v) => return Some(*v as f32),
            FbxProperty::F32(v) => return Some(*v),
            FbxProperty::I32(v) => return Some(*v as f32),
            FbxProperty::I64(v) => return Some(*v as f32),
            _ => {}
        }
    }
    None
}

fn property_vec3(prop: &FbxNode) -> Option<[f32; 3]> {
    let values: Vec<f32> = prop
        .properties
        .iter()
        .skip(value_offset(prop))
        .filter_map(|value| match value {
            FbxProperty::F64(v) => Some(*v as f32),
            FbxProperty::F32(v) => Some(*v),
            _ => None,
        })
        .take(3)
        .collect();
    (values.len() == 3).then(|| [values[0], values[1], values[2]])
}

/// Static per-axis values an `AnimationCurveNode` states for itself.
///
/// The `d|X`/`d|Y`/`d|Z` entries of its `Properties70` are the components the
/// node connects no curve to -- an exporter writes the axis's constant there
/// instead of a curve. `None` means the node states nothing for that axis.
fn curve_node_static_values(node: &FbxNode) -> [Option<f32>; 3] {
    let mut values = [None; 3];
    let Some(properties) = node
        .children
        .iter()
        .find(|child| child.name == "Properties70" || child.name == "Properties60")
    else {
        return values;
    };
    for property in &properties.children {
        let Some(FbxProperty::String(name)) = property.properties.first() else {
            continue;
        };
        let component = match name.as_str() {
            "d|X" => 0,
            "d|Y" => 1,
            "d|Z" => 2,
            _ => continue,
        };
        if values[component].is_none() {
            values[component] = property_scalar(property);
        }
    }
    values
}

/// Static per-axis values for one pre-7000 take channel group.
///
/// The `Takes` section carries no per-axis statics: an axis without a
/// `Channel` subtree is constant at the Model's own transform property. A
/// missing scaling property means identity -- the format's default -- not
/// zero, which would collapse the object.
fn model_static_values(model: &FbxNode, path: FbxAnimChannelPath) -> [Option<f32>; 3] {
    let property_name = match path {
        FbxAnimChannelPath::Translation => "Lcl Translation",
        FbxAnimChannelPath::Rotation => "Lcl Rotation",
        FbxAnimChannelPath::Scale => "Lcl Scaling",
        FbxAnimChannelPath::MorphWeight => return [None; 3],
    };
    let property = model
        .children
        .iter()
        .find(|child| child.name == "Properties70" || child.name == "Properties60")
        .and_then(|properties| {
            properties
                .children
                .iter()
                .find(|child| {
                    matches!(child.properties.first(), Some(FbxProperty::String(name)) if *name == property_name)
                })
        });
    match property.and_then(property_vec3) {
        Some(values) => values.map(Some),
        None => {
            [if path == FbxAnimChannelPath::Scale {
                Some(1.0)
            } else {
                None
            }; 3]
        }
    }
}

fn parse_material(properties: ObjectProperties<'_>) -> crate::fbx_scene::FbxMaterial {
    let name = object_name(properties.node());
    let shading_model = read_shading_model(properties);

    let get_color = |name: &str| properties.get(name).and_then(property_vec3);
    let get_scalar = |name: &str| properties.get(name).and_then(property_scalar);

    crate::fbx_scene::FbxMaterial {
        name,
        shading_model,
        diffuse: get_color("DiffuseColor"),
        specular: get_color("SpecularColor"),
        emissive: get_color("EmissiveColor"),
        ambient: get_color("AmbientColor"),
        diffuse_factor: get_scalar("DiffuseFactor"),
        specular_factor: get_scalar("SpecularFactor"),
        shininess: get_scalar("Shininess"),
        emissive_factor: get_scalar("EmissiveFactor"),
        reflection_factor: get_scalar("ReflectionFactor"),
        transparency_factor: get_scalar("TransparencyFactor"),
        opacity: get_scalar("Opacity"),
        bump_factor: get_scalar("BumpFactor"),
        textures: Vec::new(),
    }
}

/// Reads `ShadingModel`, which a document may state in any of four places.
///
/// In order, because they disagree: a `Properties70` entry on the material, a
/// `ShadingModel` node beside `Properties70`, the class template, and finally
/// the object record's own class string.
///
/// The middle two are what make the order matter. Maya writes the material's
/// real model -- `lambert`, `phong`, `unknown`, differing per material -- as
/// the sibling node, and 174 of the 188 materials in this crate's corpus get
/// theirs only from the template. Consulting the template before the sibling
/// would relabel every one of those Maya materials with the template's single
/// class default, which is how a rewrite turned a `phong` material into a
/// `Lambert` one.
fn read_shading_model(properties: ObjectProperties<'_>) -> Option<String> {
    let object = properties.node();
    let from_own_properties = properties
        .node()
        .children
        .iter()
        .filter(|child| child.name == "Properties70" || child.name == "Properties60")
        .find_map(|block| crate::fbx_templates::find_property(block, "ShadingModel"))
        .and_then(string_value);
    let from_sibling_node = object
        .children
        .iter()
        .find(|child| child.name == "ShadingModel")
        .and_then(|child| match child.properties.first() {
            Some(FbxProperty::String(model)) if !model.is_empty() => Some(model.clone()),
            _ => None,
        });
    let from_template = properties
        .template()
        .and_then(|block| crate::fbx_templates::find_property(block, "ShadingModel"))
        .and_then(string_value);
    let from_class = object_class(object)
        .filter(|raw| !raw.is_empty())
        .map(str::to_string);

    from_own_properties
        .or(from_sibling_node)
        .or(from_template)
        .or(from_class)
}

/// The first string value of a `P` record, past its four name and type fields.
fn string_value(property: &FbxNode) -> Option<String> {
    property
        .properties
        .iter()
        .skip(value_offset(property))
        .find_map(|value| match value {
            FbxProperty::String(text) => Some(text.clone()),
            _ => None,
        })
}

fn parse_texture(node: &FbxNode) -> crate::fbx_scene::FbxTexture {
    let name = object_name(node);
    let mut filename = None;
    let mut content = None;
    for child in &node.children {
        match child.name.as_str() {
            "RelativeFilename" | "FileName" | "Filename" if filename.is_none() => {
                if let Some(FbxProperty::String(s)) = child.properties.first() {
                    if !s.is_empty() {
                        filename = Some(s.clone());
                    }
                }
            }
            "Content" => {
                if let Some(FbxProperty::Raw(bytes)) = child.properties.first() {
                    if !bytes.is_empty() {
                        content = Some(bytes.clone());
                    }
                }
            }
            _ => {}
        }
    }
    crate::fbx_scene::FbxTexture {
        name,
        content,
        filename,
    }
}
impl<R: Read + Seek> FbxReader<R> {
    /// Read meshes from the FBX file.
    pub fn read_meshes(&mut self) -> io::Result<Vec<Mesh>> {
        let nodes = self.read_nodes()?;
        let mut meshes = Vec::new();
        // Collected separately because `geometry_to_mesh` borrows `self`
        // immutably; merged back afterwards so this path reports the same
        // geometry notices `read_scene` does.
        let mut warnings = Vec::new();

        // Find Objects node
        for node in &nodes {
            if node.name == "Objects" {
                for child in &node.children {
                    // A pre-7000 document carries its geometry on the Model.
                    if child.name == "Geometry"
                        || (child.name == "Model"
                            && child.children.iter().any(|c| c.name == "Vertices"))
                    {
                        if let Some(source) = geometry_to_mesh(child, &mut warnings)? {
                            meshes.push(source.mesh);
                        }
                    }
                }
            }
        }

        self.extend_warnings(warnings);
        Ok(meshes)
    }
}

/// The `LayerElement*` children of one geometry node, bucketed by family.
///
/// Collecting them before parsing keeps the dispatch over node names -- which
/// has to be exhaustive so an unknown family raises a warning rather than
/// vanishing -- separate from the per-family decoding.
#[derive(Default)]
struct RawLayerNodes<'a> {
    normals: Vec<&'a FbxNode>,
    uvs: Vec<&'a FbxNode>,
    colors: Vec<&'a FbxNode>,
    tangents: Vec<&'a FbxNode>,
    binormals: Vec<&'a FbxNode>,
    smoothing: Vec<&'a FbxNode>,
    creases: Vec<(FbxCreaseKind, &'a FbxNode)>,
    material: Option<&'a FbxNode>,
}

/// The element counts a non-corner layer's length has to agree with.
#[derive(Clone, Copy)]
struct LayerDomains {
    edges: Option<usize>,
    polygons: usize,
    control_points: usize,
}

impl LayerDomains {
    /// Resolves what a mapping name claims about a layer's length.
    ///
    /// `ByEdge` with no `Edges` array is deliberately unverifiable rather than
    /// wrong: FBX does not require the array, and the layer then addresses the
    /// edges an importer would reconstruct from the faces. This crate does not
    /// reconstruct them, so it cannot check the length -- but it must not
    /// destroy the data either, since preserving it verbatim is what makes a
    /// rewrite lossless.
    fn check(self, mapping: Option<&str>) -> DomainCheck {
        match mapping {
            Some("ByEdge") => match self.edges {
                Some(count) => DomainCheck::Expect(count),
                None => DomainCheck::Unverifiable,
            },
            Some("ByPolygon") => DomainCheck::Expect(self.polygons),
            Some("ByVertice") | Some("ByVertex") | Some("ByControlPoint") => {
                DomainCheck::Expect(self.control_points)
            }
            _ => DomainCheck::Unknown,
        }
    }
}

/// Convert a Geometry node to a Mesh, plus per-triangle material indices.
///
/// The returned `material_indices` align with the fan-triangulated face
/// order of the Draco `Mesh` (one entry per triangle). The list is empty
/// when the geometry does not carry a `LayerElementMaterial` layer.
fn geometry_to_mesh(
    geometry: &FbxNode,
    warnings: &mut Vec<FbxWarning>,
) -> io::Result<Option<FbxGeometrySource>> {
    let mut vertices: Option<Vec<f64>> = None;
    let mut polygon_indices: Option<Vec<i32>> = None;
    let mut edges: Vec<i32> = Vec::new();
    let mut raw = RawLayerNodes::default();

    for child in &geometry.children {
        match child.name.as_str() {
            "Vertices" => {
                if let Some(values) = float_values(child) {
                    vertices = Some(values);
                }
            }
            "Edges" => {
                if let Some(values) = int_values(child) {
                    edges = values;
                }
            }
            "PolygonVertexIndex" => {
                if let Some(values) = int_values(child) {
                    polygon_indices = Some(values);
                }
            }
            "LayerElementNormal" => raw.normals.push(child),
            "LayerElementColor" => raw.colors.push(child),
            "LayerElementUV" => raw.uvs.push(child),
            "LayerElementTangent" => raw.tangents.push(child),
            "LayerElementBinormal" => raw.binormals.push(child),
            "LayerElementSmoothing" => raw.smoothing.push(child),
            "LayerElementEdgeCrease" => raw.creases.push((FbxCreaseKind::Edge, child)),
            "LayerElementVertexCrease" => raw.creases.push((FbxCreaseKind::Vertex, child)),
            "LayerElementMaterial" if raw.material.is_none() => {
                raw.material = Some(child);
            }
            // Any layer family this crate does not import lands here. They
            // used to vanish without a trace; naming them makes the gap
            // visible to a caller instead of only to the source code.
            other if other.starts_with("LayerElement") => push_warning(
                warnings,
                FbxWarningCode::DroppedLayerElement,
                format!("FBX {other} is not imported, so its data is absent from the scene"),
                Some(other),
            ),
            _ => {}
        }
    }

    let vertices = match vertices {
        Some(v) => v,
        None => return Ok(None),
    };
    let polygon_indices = match polygon_indices {
        Some(p) => p,
        None => return Ok(None),
    };

    let control_points = vertices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|value| [value[0] as f32, value[1] as f32, value[2] as f32])
        .collect::<Vec<_>>();

    // Track the polygon each fan triangle came from, so `ByPolygon`
    // material indices can be remapped onto triangle order.
    let mut tri_polygon_index: Vec<usize> = Vec::new();
    let mut polygon_count = 0usize;
    let mut corners_in_polygon = 0usize;
    for &idx in &polygon_indices {
        corners_in_polygon += 1;
        if idx < 0 {
            for _ in 0..corners_in_polygon.saturating_sub(2) {
                tri_polygon_index.push(polygon_count);
            }
            corners_in_polygon = 0;
            polygon_count += 1;
        }
    }

    // Per-triangle material indices.
    let material_indices = raw
        .material
        .and_then(|layer| {
            let mapping = layer_string(layer, "MappingInformationType");
            let reference = layer_string(layer, "ReferenceInformationType");
            let data = layer_int_array(layer, "Materials");
            expand_material_indices(
                mapping.as_deref(),
                reference.as_deref(),
                data.as_deref(),
                polygon_count,
                &tri_polygon_index,
            )
        })
        .unwrap_or_default();

    let domains = LayerDomains {
        edges: (!edges.is_empty()).then_some(edges.len()),
        polygons: polygon_count,
        control_points: control_points.len(),
    };
    let layers = parse_geometry_layers(raw, domains, warnings);

    // Build the Draco mesh on the polygon-corner domain. Resolving layer
    // elements onto control points cannot represent a UV or hard-normal
    // seam, and silently averaged them away.
    let render = crate::fbx_render_mesh::expand_to_render_mesh(
        crate::fbx_render_mesh::FbxGeometryLayers::new(&control_points, &polygon_indices, &layers),
    );
    let mesh = crate::fbx_render_mesh::build_draco_mesh(&render);

    Ok(Some(FbxGeometrySource {
        mesh,
        material_indices,
        control_points,
        polygon_vertex_indices: polygon_indices,
        layers,
        edges,
    }))
}

/// Decodes each layer-element family into the form the scene retains.
fn parse_geometry_layers(
    raw: RawLayerNodes<'_>,
    domains: LayerDomains,
    warnings: &mut Vec<FbxWarning>,
) -> FbxMeshLayers {
    let uv_sets: Vec<FbxUvSet> = raw
        .uvs
        .into_iter()
        .filter_map(|layer| {
            let values = chunk_layer_values(&read_layer_floats(layer, "UV")?);
            Some(layer_set(layer, values, &["UVIndex"]))
        })
        .collect();
    let normal_sets: Vec<FbxNormalSet> = raw
        .normals
        .into_iter()
        .filter_map(|layer| {
            let values = chunk_layer_values(&read_layer_floats(layer, "Normals")?);
            // Exporters disagree on the index node's name.
            Some(layer_set(layer, values, &["NormalsIndex", "NormalIndex"]))
        })
        .collect();
    for set in &uv_sets {
        warn_unsupported_layer_mapping("LayerElementUV", set, warnings);
    }
    for set in &normal_sets {
        warn_unsupported_layer_mapping("LayerElementNormal", set, warnings);
    }
    let color_sets: Vec<FbxColorSet> = raw
        .colors
        .into_iter()
        .filter_map(|layer| {
            let floats = read_layer_floats(layer, "Colors")?;
            // FBX writes RGBA here, but a three-component source is legal
            // in the wild; pad it opaque rather than dropping the layer.
            let values = if floats.len() % 4 == 0 {
                chunk_layer_values(&floats)
            } else {
                floats
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|value| [value[0], value[1], value[2], 1.0])
                    .collect()
            };
            Some(layer_set(layer, values, &["ColorIndex"]))
        })
        .collect();
    for set in &color_sets {
        warn_unsupported_layer_mapping("LayerElementColor", set, warnings);
    }
    let tangent_sets: Vec<FbxTangentSet> = raw
        .tangents
        .into_iter()
        .filter_map(|layer| parse_tangent_like(layer, "Tangents", "TangentsW", "TangentIndex"))
        .collect();
    let binormal_sets: Vec<FbxBinormalSet> = raw
        .binormals
        .into_iter()
        .filter_map(|layer| parse_tangent_like(layer, "Binormals", "BinormalsW", "BinormalIndex"))
        .collect();
    for set in &tangent_sets {
        warn_unsupported_layer_mapping("LayerElementTangent", &set.layer, warnings);
    }
    for set in &binormal_sets {
        warn_unsupported_layer_mapping("LayerElementBinormal", &set.layer, warnings);
    }

    // Smoothing and crease layers address edges, polygons or control points --
    // never polygon corners -- so they are kept raw beside `edges` rather than
    // resolved onto the render mesh. A layer whose length disagrees with the
    // domain its mapping names is misaligned data, and keeping it would
    // silently sharpen the wrong edges.
    let mut smoothing_layers = Vec::new();
    for layer in raw.smoothing {
        let mapping = layer_string(layer, "MappingInformationType");
        let Some(values) = layer_int_array(layer, "Smoothing") else {
            continue;
        };
        if domains.check(mapping.as_deref()).accepts(values.len()) {
            smoothing_layers.push(FbxSmoothingLayer { mapping, values });
        } else {
            warn_misaligned_layer(
                "LayerElementSmoothing",
                mapping.as_deref(),
                values.len(),
                warnings,
            );
        }
    }
    let mut crease_layers = Vec::new();
    for (kind, layer) in raw.creases {
        let element = match kind {
            FbxCreaseKind::Edge => "LayerElementEdgeCrease",
            FbxCreaseKind::Vertex => "LayerElementVertexCrease",
        };
        let mapping = layer_string(layer, "MappingInformationType");
        let Some(values) = layer_f64_array(layer, element.trim_start_matches("LayerElement"))
        else {
            continue;
        };
        match domains.check(mapping.as_deref()) {
            domain if domain.accepts(values.len()) => {
                crease_layers.push(FbxCreaseLayer {
                    kind,
                    mapping,
                    values,
                });
            }
            _ => warn_misaligned_layer(element, mapping.as_deref(), values.len(), warnings),
        }
    }

    FbxMeshLayers {
        uv_sets,
        normal_sets,
        color_sets,
        tangent_sets,
        binormal_sets,
        smoothing_layers,
        crease_layers,
    }
}

impl<R: Read + Seek> FbxReader<R> {
    /// Flatten the FBX animation graph into one [`FbxAnimation`] per
    /// `AnimationStack` + first connected `AnimationLayer`.
    fn parse_animations(
        &self,
        nodes: &[FbxNode],
        index: &FbxObjectIndex<'_>,
        model_name_map: &HashMap<i64, String>,
        model_node_ids: &HashMap<i64, FbxNodeId>,
        morph_targets: &HashMap<i64, (i64, u32)>,
    ) -> Vec<FbxAnimation> {
        let FbxObjectIndex {
            connections,
            astack_map,
            alayer_map,
            acnode_map,
            acurve_map,
            model_map,
            ..
        } = index;
        let fbx_ktime = fbx_ktime_for(nodes, self.version());
        // Held as `f64`, and divided as `f64`, even though the sampler stores
        // seconds as `f32`. A tick count is around 2e10 for a one-second key,
        // where one `f32` step is 2048 ticks: narrowing either the count or
        // the divisor before the division quantizes the result to about
        // 4e-8 s, far coarser than the `f32` seconds can hold. Narrowing after
        // it costs nothing.
        let ktime_f = match fbx_ktime {
            0 => 1.0,
            v => v as f64,
        };

        // acnode_id -> (layer_id, model_id, path). The FBX convention (and
        // Blender's io_scene_fbx) wires the AnimationCurveNode as the *child*
        // of an OP connection whose parent is the animated Model, with the
        // animated property name ("Lcl Translation" etc.) as the 4th field.
        let mut acnode_targets: std::collections::HashMap<
            i64,
            (i64, i64, FbxAnimChannelPath, Option<u32>),
        > = std::collections::HashMap::new();
        for conn in connections {
            if conn.kind != ConnectionKind::Op {
                continue;
            }
            if !acnode_map.contains_key(&conn.child) {
                continue;
            }
            let Some(property) = conn.property.as_deref() else {
                continue;
            };
            let Some(path) = FbxAnimChannelPath::from_property_name(property) else {
                continue;
            };
            let (model_id, morph_target_index) = if model_map.contains_key(&conn.parent) {
                (conn.parent, None)
            } else if path == FbxAnimChannelPath::MorphWeight {
                let Some(&(model_id, target_index)) = morph_targets.get(&conn.parent) else {
                    continue;
                };
                (model_id, Some(target_index))
            } else {
                continue;
            };
            // Find the layer that owns this curve node (OO curvenode -> layer).
            let mut layer_id = None;
            for c2 in connections {
                if c2.kind == ConnectionKind::Oo
                    && c2.child == conn.child
                    && alayer_map.contains_key(&c2.parent)
                {
                    layer_id = Some(c2.parent);
                    break;
                }
            }
            if let Some(layer_id) = layer_id {
                acnode_targets.insert(conn.child, (layer_id, model_id, path, morph_target_index));
            }
        }

        // acnode_id -> component -> curve.
        let mut acnode_curves: std::collections::HashMap<i64, ComponentCurves> =
            std::collections::HashMap::new();
        // acnode_id -> static value per component, from the node's own
        // `d|X`/`d|Y`/`d|Z`: an axis the node connects no curve to is not
        // animated but still holds a value, and zeroing it would flatten a
        // scale or snap a translation to the origin.
        let mut acnode_statics: std::collections::HashMap<i64, [Option<f32>; 3]> =
            std::collections::HashMap::new();
        for conn in connections {
            if conn.kind != ConnectionKind::Op {
                continue;
            }
            if !acurve_map.contains_key(&conn.child) {
                continue;
            }
            if !acnode_targets.contains_key(&conn.parent) {
                continue;
            }
            let component = match conn.property.as_deref() {
                Some("d|X") => 0,
                Some("d|Y") => 1,
                Some("d|Z") => 2,
                _ => continue,
            };
            acnode_statics
                .entry(conn.parent)
                .or_insert_with(|| curve_node_static_values(acnode_map[&conn.parent]));
            if let Some(curve) = parse_curve(acurve_map[&conn.child]) {
                // A later curve for the same component overwrites, which is
                // what the map's `insert` did.
                acnode_curves.entry(conn.parent).or_default()[component] = Some(curve);
            }
        }

        // Group curve nodes by (stack, layer, model, path).
        //
        // Every iteration below walks ids in sorted order rather than hash
        // order. FBX object ids are stable within a document, so this makes
        // the channel list a property of the file instead of the process --
        // otherwise two reads of the same bytes produce differently ordered
        // channels and any positional comparison comes out garbage.
        let mut stacks_layers: StacksLayers = std::collections::HashMap::new();
        let mut acnode_ids_sorted: Vec<i64> = acnode_targets.keys().copied().collect();
        acnode_ids_sorted.sort_unstable();
        for acnode_id in &acnode_ids_sorted {
            let (layer_id, model_id, path, morph_target_index) = &acnode_targets[acnode_id];
            // Find stacks owning this layer.
            let mut stack_ids = Vec::new();
            for c2 in connections {
                if c2.kind == ConnectionKind::Oo
                    && c2.child == *layer_id
                    && astack_map.contains_key(&c2.parent)
                {
                    stack_ids.push(c2.parent);
                }
            }
            for stack_id in stack_ids {
                stacks_layers
                    .entry(stack_id)
                    .or_default()
                    .entry(*layer_id)
                    .or_default()
                    .push((*acnode_id, *model_id, *path, *morph_target_index));
            }
        }

        let mut animations = Vec::new();
        let mut stack_ids_sorted: Vec<i64> = stacks_layers.keys().copied().collect();
        stack_ids_sorted.sort_unstable();
        for stack_id in stack_ids_sorted {
            let layers = &stacks_layers[&stack_id];
            let stack_node = astack_map.get(&stack_id);
            let name = stack_node.and_then(|node| object_name(node));
            // One clip per layer, which is what Blender's importer does: it
            // "does not mix layers, each layer results in an independent set
            // of actions". Merging them instead produced several channels
            // driving the same node and path, and any consumer applying them
            // in order silently kept only the last.
            let mut layer_ids_sorted: Vec<i64> = layers.keys().copied().collect();
            layer_ids_sorted.sort_unstable();
            let multiple_layers = layer_ids_sorted.len() > 1;
            for (layer_index, layer_id) in layer_ids_sorted.iter().copied().enumerate() {
                let mut channels = Vec::new();
                let mut max_time = 0.0f32;
                let entries = &layers[&layer_id];
                // Group curve nodes by (model, path) before flattening.
                let mut groups: std::collections::HashMap<
                    (i64, FbxAnimChannelPath, Option<u32>),
                    Vec<i64>,
                > = std::collections::HashMap::new();
                for &(acnode_id, model_id, path, morph_target_index) in entries {
                    groups
                        .entry((model_id, path, morph_target_index))
                        .or_default()
                        .push(acnode_id);
                }
                let mut group_keys: Vec<(i64, FbxAnimChannelPath, Option<u32>)> =
                    groups.keys().copied().collect();
                group_keys.sort_unstable_by_key(|(model_id, path, morph_target_index)| {
                    (*model_id, *path as u8, *morph_target_index)
                });
                for (model_id, path, morph_target_index) in group_keys {
                    let acnode_ids = &groups[&(model_id, path, morph_target_index)];
                    // Combine the X/Y/Z curves across all matching curve nodes
                    // (Blender notes that each curve node has a unique set of
                    // channels, so in practice there is exactly one entry).
                    // First curve wins per component, which the map's
                    // `or_insert_with` guaranteed.
                    let mut by_component: ComponentCurves = Default::default();
                    let mut static_values = [None; 3];
                    for acnode_id in acnode_ids {
                        if let Some(curves) = acnode_curves.get(acnode_id) {
                            for (component, curve) in curves.iter().enumerate() {
                                if by_component[component].is_none() {
                                    by_component[component] = curve.clone();
                                }
                            }
                        }
                        if let Some(statics) = acnode_statics.get(acnode_id) {
                            for (slot, value) in static_values.iter_mut().zip(statics) {
                                if slot.is_none() {
                                    *slot = *value;
                                }
                            }
                        }
                    }
                    let Some(channel) = flatten_curve(&by_component, &static_values, path, ktime_f)
                    else {
                        continue;
                    };
                    if let (Some(node_name), Some(&node_id)) =
                        (model_name_map.get(&model_id), model_node_ids.get(&model_id))
                    {
                        max_time =
                            max_time.max(channel.sampler.input.last().copied().unwrap_or(0.0));
                        channels.push(FbxAnimChannel {
                            node_id,
                            node_name: node_name.clone(),
                            path,
                            morph_target_index,
                            sampler: channel.sampler,
                        });
                    }
                }
                if channels.is_empty() {
                    continue;
                }
                // Name extra layers so they stay distinguishable; a
                // single-layer stack keeps the stack name unchanged.
                let clip_name = if multiple_layers {
                    let layer_name = alayer_map
                        .get(&layer_id)
                        .and_then(|node| object_name(node))
                        .unwrap_or_else(|| format!("Layer{layer_index}"));
                    Some(match &name {
                        Some(stack) => format!("{stack}|{layer_name}"),
                        None => layer_name,
                    })
                } else {
                    name.clone()
                };
                animations.push(FbxAnimation {
                    name: clip_name,
                    duration: max_time,
                    channels,
                });
            }
        }
        animations
    }
}

/// Curve nodes grouped by `AnimationStack` id, then by `AnimationLayer` id.
///
/// Each entry is `(curve_node_id, model_id, path, morph_target_index)`.
type StacksLayers = std::collections::HashMap<
    i64,
    std::collections::HashMap<i64, Vec<(i64, i64, FbxAnimChannelPath, Option<u32>)>>,
>;

#[derive(Debug, Clone)]
struct FbxAnimCurveData {
    key_times: Vec<i64>,
    key_values: Vec<f32>,
    key_attr_flags: Vec<i32>,
    in_tangents: Vec<f32>,
    out_tangents: Vec<f32>,
}

/// Per-axis TRS curves, indexed by component (`X` 0, `Y` 1, `Z` 2).
///
/// A curve node carries at most these three, so a fixed three-slot array
/// replaces the ordered map the profiler showed paying for tree walks on
/// every insertion; slot order is component order, which is what the map's
/// iteration guaranteed.
type ComponentCurves = [Option<FbxAnimCurveData>; 3];

fn parse_curve(node: &FbxNode) -> Option<FbxAnimCurveData> {
    let mut key_times = None;
    let mut key_values: Option<Vec<f32>> = None;
    let mut key_attr_flags = None;
    let mut key_attr_data: Option<Vec<f32>> = None;
    let mut key_attr_ref_count = None;
    for child in &node.children {
        match child.name.as_str() {
            "KeyTime" => key_times = i64_values(child),
            "KeyValueFloat" => {
                key_values = float_values(child)
                    .map(|values| values.into_iter().map(|value| value as f32).collect());
            }
            "KeyAttrFlags" => {
                key_attr_flags = int_values(child);
            }
            "KeyAttrDataFloat" => {
                key_attr_data = float_values(child)
                    .map(|values| values.into_iter().map(|value| value as f32).collect());
            }
            "KeyAttrRefCount" => {
                key_attr_ref_count = int_values(child);
            }
            _ => {}
        }
    }
    let key_times = key_times?;
    let key_values = key_values?;
    if key_times.is_empty() || key_values.len() != key_times.len() {
        return None;
    }
    let mut expanded_flags = Vec::with_capacity(key_times.len());
    let mut expanded_attrs = Vec::with_capacity(key_times.len());
    if let (Some(flags), Some(data), Some(refs)) =
        (key_attr_flags, key_attr_data, key_attr_ref_count)
    {
        if flags.len() == refs.len() && data.len() == refs.len() * 4 {
            for ((flag, count), attrs) in flags.into_iter().zip(refs).zip(data.as_chunks::<4>().0) {
                for _ in 0..count.max(0) {
                    expanded_flags.push(flag);
                    expanded_attrs.push([attrs[0], attrs[1]]);
                }
            }
        }
    }
    if expanded_flags.len() != key_times.len() {
        expanded_flags = vec![0x4; key_times.len()];
        expanded_attrs = vec![[0.0, 0.0]; key_times.len()];
    }
    let mut in_tangents = vec![0.0; key_times.len()];
    let mut out_tangents = vec![0.0; key_times.len()];
    for (index, attrs) in expanded_attrs.iter().enumerate() {
        out_tangents[index] = attrs[0];
        if index + 1 < in_tangents.len() {
            in_tangents[index + 1] = attrs[1];
        }
    }
    Some(FbxAnimCurveData {
        key_times,
        key_values,
        key_attr_flags: expanded_flags,
        in_tangents,
        out_tangents,
    })
}

/// Reads the `Takes` animation of a pre-7000 document, which has no
/// `AnimationStack` objects: each `Take` names a Model by key and nests
/// `Channel` trees whose `Key` payloads hold the curve.
fn parse_takes_animations(
    nodes: &[FbxNode],
    version: u32,
    index: &FbxObjectIndex<'_>,
    model_name_map: &HashMap<i64, String>,
    model_node_ids: &HashMap<i64, FbxNodeId>,
) -> Vec<FbxAnimation> {
    let ktime_f = match fbx_ktime_for(nodes, version) {
        0 => 1.0,
        v => v as f64,
    };
    let mut animations = Vec::new();
    for takes in nodes.iter().filter(|node| node.name == "Takes") {
        for take in takes.children.iter().filter(|node| node.name == "Take") {
            let name = match take.properties.first() {
                Some(FbxProperty::String(name)) => Some(name.clone()),
                _ => None,
            };
            let mut channels = Vec::new();
            let mut max_time = 0.0f32;
            for model in take.children.iter().filter(|node| node.name == "Model") {
                let Some(FbxProperty::String(model_key)) = model.properties.first() else {
                    continue;
                };
                let Some(model_id) = index.names.lookup(model_key) else {
                    continue;
                };
                let Some(model_node) = index.model_map.get(&model_id) else {
                    continue;
                };
                let (Some(node_name), Some(&node_id)) =
                    (model_name_map.get(&model_id), model_node_ids.get(&model_id))
                else {
                    continue;
                };
                for transform in model
                    .children
                    .iter()
                    .filter(|child| child.name == "Channel")
                {
                    if !matches!(
                        transform.properties.first(),
                        Some(FbxProperty::String(name)) if name == "Transform"
                    ) {
                        continue;
                    }
                    for group in transform
                        .children
                        .iter()
                        .filter(|child| child.name == "Channel")
                    {
                        let path = match group.properties.first() {
                            Some(FbxProperty::String(name)) => match name.as_str() {
                                "T" => FbxAnimChannelPath::Translation,
                                "R" => FbxAnimChannelPath::Rotation,
                                "S" => FbxAnimChannelPath::Scale,
                                _ => continue,
                            },
                            _ => continue,
                        };
                        let mut by_component: ComponentCurves = Default::default();
                        for component in group
                            .children
                            .iter()
                            .filter(|child| child.name == "Channel")
                        {
                            let axis = match component.properties.first() {
                                Some(FbxProperty::String(name)) => match name.as_str() {
                                    "X" => 0,
                                    "Y" => 1,
                                    "Z" => 2,
                                    _ => continue,
                                },
                                _ => continue,
                            };
                            if let Some(curve) = parse_legacy_curve(component, ktime_f) {
                                by_component[axis] = Some(curve);
                            }
                        }
                        let static_values = model_static_values(model_node, path);
                        let Some(channel) =
                            flatten_curve(&by_component, &static_values, path, ktime_f)
                        else {
                            continue;
                        };
                        max_time =
                            max_time.max(channel.sampler.input.last().copied().unwrap_or(0.0));
                        channels.push(FbxAnimChannel {
                            node_id,
                            node_name: node_name.clone(),
                            path,
                            morph_target_index: None,
                            sampler: channel.sampler,
                        });
                    }
                }
            }
            if !channels.is_empty() {
                animations.push(FbxAnimation {
                    name,
                    duration: max_time,
                    channels,
                });
            }
        }
    }
    animations
}

/// One field of a pre-7000 `Key` payload: a number, or a mode letter.
enum LegacyKeyField {
    Num(f64),
    Char(u8),
}

/// Reads a `Key` node's heterogeneous payload.
///
/// The binary container writes the mode letters as `C`-typed bytes (read as
/// `U8`); the ASCII container leaves them as bare words, which parse as
/// single-character strings. An exporter that packs the whole run into one
/// `d` array -- the shape `ufbx` describes -- is accepted too, with the
/// letters then indistinguishable and the curve refused by the state machine
/// rather than misread.
fn legacy_key_fields(node: &FbxNode) -> Option<Vec<LegacyKeyField>> {
    if let Some(FbxProperty::F64Array(values)) = node.properties.first() {
        return Some(values.iter().map(|v| LegacyKeyField::Num(*v)).collect());
    }
    node.properties
        .iter()
        .map(|value| match value {
            FbxProperty::I64(v) => Some(LegacyKeyField::Num(*v as f64)),
            FbxProperty::I32(v) => Some(LegacyKeyField::Num(f64::from(*v))),
            FbxProperty::F64(v) => Some(LegacyKeyField::Num(*v)),
            FbxProperty::F32(v) => Some(LegacyKeyField::Num(f64::from(*v))),
            FbxProperty::U8(b) => Some(LegacyKeyField::Char(*b)),
            FbxProperty::String(word) if word.len() == 1 => {
                Some(LegacyKeyField::Char(word.as_bytes()[0]))
            }
            _ => None,
        })
        .collect()
}

/// Reads one pre-7000 animation channel (`Channel: "X"` and its siblings).
///
/// Follows the field layout `ufbx` reverse-engineered: per key, a time, a
/// value and an interpolation letter, then the letters and numbers that spell
/// that key's tangents. Where `ufbx` solves automatic slopes from the
/// neighbouring keys, this keeps the slopes an auto mode implies (zero) --
/// the corpus's 6100 documents are baked or linear, and a wrong-but-labelled
/// curve would be worse than a refused one only where the state machine
/// cannot tell, which it always can.
fn parse_legacy_curve(channel: &FbxNode, ktime_f: f64) -> Option<FbxAnimCurveData> {
    let key_node = channel.children.iter().find(|child| child.name == "Key")?;
    let key_count = channel
        .children
        .iter()
        .find(|child| child.name == "KeyCount")
        .and_then(|child| match child.properties.first() {
            Some(FbxProperty::I32(v)) => usize::try_from(*v).ok(),
            Some(FbxProperty::I64(v)) => usize::try_from(*v).ok(),
            _ => None,
        })?;
    let key_ver = channel
        .children
        .iter()
        .find(|child| child.name == "KeyVer")
        .and_then(|child| match child.properties.first() {
            Some(FbxProperty::I32(v)) => Some(*v),
            _ => None,
        })
        .unwrap_or(4005);
    let data = legacy_key_fields(key_node)?;
    if key_count == 0 {
        return None;
    }

    let mut times = Vec::with_capacity(key_count);
    let mut values = Vec::with_capacity(key_count);
    let mut flags = Vec::with_capacity(key_count);
    let mut in_tangents = Vec::with_capacity(key_count);
    let mut out_tangents = Vec::with_capacity(key_count);

    let mut cursor = 0usize;
    let number = |cursor: &mut usize| -> Option<f64> {
        let field = data.get(*cursor)?;
        *cursor += 1;
        match field {
            LegacyKeyField::Num(v) => Some(*v),
            LegacyKeyField::Char(_) => None,
        }
    };
    let letter = |cursor: &mut usize| -> Option<u8> {
        let field = data.get(*cursor)?;
        *cursor += 1;
        match field {
            LegacyKeyField::Char(b) => Some(*b),
            LegacyKeyField::Num(_) => None,
        }
    };

    let mut next_time = number(&mut cursor)?;
    let mut next_value = number(&mut cursor)?;
    // The incoming slope this key's predecessor left behind.
    let mut slope_left = 0.0f64;
    for index in 0..key_count {
        let time = next_time;
        let value = next_value;
        let mode = letter(&mut cursor)?;
        // Per-channel interpolation is read from the first key, which is what
        // the 7.x flattening below expects.
        let flag = match mode {
            b'U' => 0x8,
            b'L' => 0x0,
            b'C' => 0x2,
            _ => return None,
        };
        let mut slope_right = 0.0f64;
        let mut next_slope_left = 0.0f64;
        if mode == b'U' {
            let slope_mode = letter(&mut cursor)?;
            let mut weights = match slope_mode {
                b's' | b'b' => {
                    slope_right = number(&mut cursor)?;
                    next_slope_left = number(&mut cursor)?;
                    (key_ver != 4003) as usize
                }
                b'a' => {
                    if key_ver <= 4004 {
                        0
                    } else {
                        1
                    }
                }
                b'p' | b'q' => {
                    number(&mut cursor)?;
                    number(&mut cursor)?;
                    if key_ver <= 4004 {
                        1
                    } else {
                        2
                    }
                }
                b't' => {
                    number(&mut cursor)?;
                    number(&mut cursor)?;
                    number(&mut cursor)?;
                    0
                }
                b'd' => {
                    number(&mut cursor)?;
                    1
                }
                _ => return None,
            };
            while weights > 0 {
                weights -= 1;
                match letter(&mut cursor)? {
                    b'n' | b'c' => {}
                    b'a' => {
                        number(&mut cursor)?;
                        number(&mut cursor)?;
                    }
                    b'l' | b'r' => {
                        number(&mut cursor)?;
                    }
                    _ => return None,
                }
            }
        } else if mode == b'C' && key_ver >= 4004 {
            letter(&mut cursor)?; // 'n' (hold) or the previous value
        }
        if index + 1 < key_count {
            next_time = number(&mut cursor)?;
            next_value = number(&mut cursor)?;
        }
        // A linear key's slope is the segment to the next key, in output
        // units per second -- the unit the 7.x tangents carry. The times are
        // still KTime ticks here, so the divisor converts once, through
        // `f64` for the same reason the 7.x sampler input does it.
        if mode == b'L' && next_time > time {
            slope_right = (next_value - value) / ((next_time - time) / ktime_f);
            next_slope_left = slope_right;
        }

        times.push(time as i64);
        values.push(value as f32);
        flags.push(flag);
        in_tangents.push(slope_left as f32);
        out_tangents.push(slope_right as f32);
        slope_left = next_slope_left;
    }
    Some(FbxAnimCurveData {
        key_times: times,
        key_values: values,
        key_attr_flags: flags,
        in_tangents,
        out_tangents,
    })
}

/// Value of one component curve at an arbitrary key time.
///
/// Linear between the bracketing keys, constant outside the curve's own range,
/// which is FBX's default extrapolation.
fn sample_curve_at(curve: &FbxAnimCurveData, time: i64) -> f32 {
    let value_at = |index: usize| curve.key_values.get(index).copied().unwrap_or(0.0);
    match curve.key_times.binary_search(&time) {
        Ok(index) => value_at(index),
        Err(0) => value_at(0),
        Err(index) if index >= curve.key_times.len() => value_at(curve.key_times.len() - 1),
        Err(index) => {
            let (before, after) = (curve.key_times[index - 1], curve.key_times[index]);
            let span = after.saturating_sub(before) as f64;
            let position = if span > 0.0 {
                (time.saturating_sub(before) as f64) / span
            } else {
                0.0
            };
            let start = f64::from(value_at(index - 1));
            let end = f64::from(value_at(index));
            (start + (end - start) * position) as f32
        }
    }
}

/// Combine per-component curves into a single TRS channel sampler.
///
/// Each component is a separate `AnimationCurve` object with its own key grid;
/// nothing in the format ties the three together, and exporters routinely
/// write a different key count per axis. So the times are the union of every
/// component's keys, and each component is sampled at them.
///
/// Where the components do agree the keys are used as authored, tangents and
/// all. That is the case a file this decoder wrote always falls into.
///
/// `static_values` carries what an axis with no curve holds instead -- the
/// `AnimationCurveNode`'s own `d|X`/`d|Y`/`d|Z` for 7.x, the model's
/// transform property for a pre-7000 take. A component slot that has neither
/// a curve nor a static value keeps zero, which for a scale channel would
/// collapse the object.
fn flatten_curve(
    by_component: &ComponentCurves,
    static_values: &[Option<f32>; 3],
    path: FbxAnimChannelPath,
    ktime_f: f64,
) -> Option<FbxAnimChannel> {
    let time_axis = by_component[0]
        .as_ref()
        .or_else(|| by_component[1].as_ref())
        .or_else(|| by_component[2].as_ref())?;
    let component_count = path.component_count();
    // Reading component `i` at component 0's `i`-th key time is wrong twice
    // over: it takes the value from whatever moment that axis happens to have
    // a key at, and past the shorter curve's last key it takes no value at
    // all. One character's take had 254 component curves end early, the root
    // bone's vertical translation among them: it fell to zero six seconds in
    // and put the skeleton through the floor.
    let times_agree = by_component
        .iter()
        .take(component_count)
        .flatten()
        .all(|curve| curve.key_times == time_axis.key_times);
    let merged_times = (!times_agree).then(|| {
        let mut times: Vec<i64> = by_component
            .iter()
            .take(component_count)
            .flatten()
            .flat_map(|curve| curve.key_times.iter().copied())
            .collect();
        times.sort_unstable();
        times.dedup();
        times
    });
    let key_times: &[i64] = merged_times.as_deref().unwrap_or(&time_axis.key_times);
    let n = key_times.len();
    let mut input = Vec::with_capacity(n);
    let mut output = Vec::with_capacity(n * component_count);
    let mut in_tangents = Vec::with_capacity(n * component_count);
    let mut out_tangents = Vec::with_capacity(n * component_count);
    let flags = time_axis.key_attr_flags.first().copied().unwrap_or(0);
    // A merged grid puts keys between the authored ones, where the authored
    // tangents describe a segment that no longer exists. The sampled values
    // are on the curve and the segments between them are straight.
    let interpolation = if times_agree {
        FbxAnimInterpolation::from_key_attr_flags(flags)
    } else {
        FbxAnimInterpolation::Linear
    };
    // What an axis holds when the document states nothing for it at all: no
    // curve, and no value beside the curve node either. That is the property's
    // own default, and for a scale it is one -- zero would collapse the object
    // on the axis the file happened to leave out. The Takes path reasons the
    // same way about a Model that states no `Lcl Scaling`.
    let unstated = match path {
        FbxAnimChannelPath::Scale => 1.0,
        _ => 0.0,
    };
    for (i, &time) in key_times.iter().enumerate() {
        input.push((time as f64 / ktime_f) as f32);
        // `component_count` is 1 or 3 and `by_component` holds three slots, so
        // the take never shortens a path that has more components than curves.
        for (component_index, slot) in by_component.iter().take(component_count).enumerate() {
            let Some(curve) = slot.as_ref() else {
                output.push(static_values[component_index].unwrap_or(unstated));
                in_tangents.push(0.0);
                out_tangents.push(0.0);
                continue;
            };
            if times_agree {
                output.push(curve.key_values.get(i).copied().unwrap_or(0.0));
                in_tangents.push(curve.in_tangents.get(i).copied().unwrap_or(0.0));
                out_tangents.push(curve.out_tangents.get(i).copied().unwrap_or(0.0));
            } else {
                output.push(sample_curve_at(curve, time));
                in_tangents.push(0.0);
                out_tangents.push(0.0);
            }
        }
    }
    // FBX stores Euler rotations in degrees; convert to radians so the JS
    // viewer's Euler→quaternion helper matches expectations. Translation and
    // scale are passed through unchanged.
    //
    // Through `f64`, and narrowing once at the end. `f32::to_radians` rounds
    // its own factor and then rounds the product, so composing it with the
    // writer's inverse moved an angle by a bit on every rewrite.
    let radians = |value: f32| f64::from(value).to_radians() as f32;
    if path == FbxAnimChannelPath::Rotation {
        for chunk in output.chunks_mut(3) {
            for value in chunk.iter_mut() {
                *value = radians(*value);
            }
        }
        for chunk in in_tangents.chunks_mut(3) {
            for value in chunk.iter_mut() {
                *value = radians(*value);
            }
        }
        for chunk in out_tangents.chunks_mut(3) {
            for value in chunk.iter_mut() {
                *value = radians(*value);
            }
        }
    }
    Some(FbxAnimChannel {
        node_id: FbxNodeId(0),
        node_name: String::new(),
        path,
        morph_target_index: None,
        sampler: FbxAnimSampler {
            input,
            output,
            interpolation,
            in_tangents: (interpolation == FbxAnimInterpolation::Cubic).then_some(in_tangents),
            out_tangents: (interpolation == FbxAnimInterpolation::Cubic).then_some(out_tangents),
        },
    })
}

/// Determine the FBX KTime ticks-per-second value.
///
/// Pre-7.7 files use `46186158000`. FBX 2019.5+ (version 7700+) introduced an
/// opt-in `141120000` ticks/second default; the legacy value is selected by
/// `FBXHeaderExtension/OtherFlags/TCDefinition == 127`. See Blender's
/// `io_scene_fbx` `FBX_KTIME` constants for the canonical encoding.
fn fbx_ktime_for(nodes: &[FbxNode], version: u32) -> u64 {
    const KTIME_V7: u64 = 46_186_158_000;
    const KTIME_V8: u64 = 141_120_000;
    if version >= 8000 {
        return KTIME_V8;
    }
    if version >= 7700 {
        // Inspect OtherFlags/TCDefinition. 127 selects the legacy V7 value;
        // anything else (or missing) opts into V8.
        for n in nodes {
            if n.name != "FBXHeaderExtension" {
                continue;
            }
            let mut header_version = 0;
            let mut other_flags: Option<&FbxNode> = None;
            for child in &n.children {
                if child.name == "FBXHeaderVersion" {
                    if let Some(FbxProperty::I32(v)) = child.properties.first() {
                        header_version = *v;
                    }
                } else if child.name == "OtherFlags" && other_flags.is_none() {
                    other_flags = Some(child);
                }
            }
            if header_version >= 1004 {
                if let Some(flags) = other_flags {
                    for flag in &flags.children {
                        if flag.name == "TCDefinition" {
                            if let Some(FbxProperty::I32(v)) = flag.properties.first() {
                                return if *v == 127 { KTIME_V7 } else { KTIME_V8 };
                            }
                        }
                    }
                }
            }
        }
        // Pre-8000 default for 7.7+ files without an explicit TCDefinition is V7.
        return KTIME_V7;
    }
    KTIME_V7
}

fn layer_string(layer: &FbxNode, name: &str) -> Option<String> {
    for child in &layer.children {
        if child.name == name {
            if let Some(FbxProperty::String(s)) = child.properties.first() {
                return Some(s.clone());
            }
        }
    }
    None
}

fn layer_int_array(layer: &FbxNode, name: &str) -> Option<Vec<i32>> {
    for child in &layer.children {
        if child.name == name {
            if let Some(values) = int_values(child) {
                return Some(values);
            }
        }
    }
    None
}

/// Reports a layer element whose mapping or reference mode this crate does not
/// recognize.
///
/// The value is still resolved on the control-point domain, which is the most
/// likely intent and what the reader has always done. The warning exists so a
/// caller learns the substitution happened rather than inferring it from
/// unexpected geometry.
fn warn_unsupported_layer_mapping<const N: usize>(
    element: &str,
    set: &FbxLayerSet<N>,
    warnings: &mut Vec<FbxWarning>,
) {
    const KNOWN_MAPPINGS: [&str; 7] = [
        "ByPolygonVertex",
        "ByPolygon",
        "ByVertice",
        "ByVertex",
        "ByControlPoint",
        "AllSame",
        "AllSameOrPolygon",
    ];
    if let Some(mapping) = set.mapping.as_deref() {
        if !KNOWN_MAPPINGS.contains(&mapping) {
            let subject = format!("{element}/{mapping}");
            push_warning(
                warnings,
                FbxWarningCode::UnsupportedLayerMapping,
                format!(
                    "FBX {element} uses mapping {mapping}, which was resolved on the \
                     control-point domain"
                ),
                Some(&subject),
            );
        }
    }
    if let Some(reference) = set.reference.as_deref() {
        if reference != "Direct" && reference != "IndexToDirect" {
            let subject = format!("{element}/{reference}");
            push_warning(
                warnings,
                FbxWarningCode::UnsupportedLayerMapping,
                format!("FBX {element} uses reference mode {reference}, which was read as Direct"),
                Some(&subject),
            );
        }
    }
}

/// What can be said about the length a non-corner layer element should have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DomainCheck {
    /// The domain has a known size, and the layer must match it exactly.
    Expect(usize),
    /// The domain exists but its size is not known here, so the layer is kept
    /// as authored rather than judged.
    Unverifiable,
    /// The mapping names no domain this crate recognizes.
    Unknown,
}

impl DomainCheck {
    fn accepts(self, len: usize) -> bool {
        match self {
            DomainCheck::Expect(expected) => expected == len,
            DomainCheck::Unverifiable => true,
            DomainCheck::Unknown => false,
        }
    }
}

/// Reports a smoothing or crease layer whose length disagrees with the domain
/// its mapping names, and which was therefore dropped.
fn warn_misaligned_layer(
    element: &str,
    mapping: Option<&str>,
    len: usize,
    warnings: &mut Vec<FbxWarning>,
) {
    let mapping = mapping.unwrap_or("no mapping");
    let subject = format!("{element}/{mapping}");
    push_warning(
        warnings,
        FbxWarningCode::UnsupportedLayerMapping,
        format!(
            "FBX {element} has {len} values, which does not match the domain \
             {mapping} addresses, so the layer was dropped"
        ),
        Some(&subject),
    );
}

/// Reads a layer element's `f64` payload, for the crease weights that are not
/// vectors and so do not go through [`chunk_layer_values`].
fn layer_f64_array(layer: &FbxNode, name: &str) -> Option<Vec<f64>> {
    for child in &layer.children {
        if child.name == name {
            if let Some(values) = float_values(child) {
                return Some(values);
            }
        }
    }
    None
}

/// Reads a `LayerElementTangent` or `LayerElementBinormal`.
///
/// The handedness sign is a separate sibling array present only from FBX 7500
/// on; when it is missing, or disagrees with the vector count, `w` defaults to
/// `+1.0` and the set records that it was synthesized.
fn parse_tangent_like(
    layer: &FbxNode,
    values_node: &str,
    handedness_node: &str,
    index_node: &str,
) -> Option<FbxTangentSet> {
    let vectors: Vec<[f32; 3]> = chunk_layer_values(&read_layer_floats(layer, values_node)?);
    let handedness = read_layer_floats(layer, handedness_node)
        .filter(|signs| signs.len() == vectors.len())
        .unwrap_or_default();
    let has_handedness = !handedness.is_empty();
    let values = vectors
        .iter()
        .enumerate()
        .map(|(index, v)| {
            let sign = handedness.get(index).copied().unwrap_or(1.0);
            [v[0], v[1], v[2], sign]
        })
        .collect();
    Some(FbxTangentSet {
        layer: layer_set(layer, values, &[index_node]),
        has_handedness,
    })
}

/// Reads the parts every float layer element shares.
///
/// `index_nodes` lists the names the index array may appear under, tried in
/// order: exporters disagree on some of them.
fn layer_set<const N: usize>(
    layer: &FbxNode,
    values: Vec<[f32; N]>,
    index_nodes: &[&str],
) -> FbxLayerSet<N> {
    FbxLayerSet {
        name: layer_string(layer, "Name"),
        mapping: layer_string(layer, "MappingInformationType"),
        reference: layer_string(layer, "ReferenceInformationType"),
        values,
        indices: index_nodes
            .iter()
            .find_map(|name| layer_int_array(layer, name))
            .unwrap_or_default(),
    }
}

/// Groups a flat float payload into `N`-component values, dropping a trailing
/// partial value.
fn chunk_layer_values<const N: usize>(raw: &[f32]) -> Vec<[f32; N]> {
    raw.as_chunks::<N>()
        .0
        .iter()
        .map(|value| std::array::from_fn(|i| value[i]))
        .collect()
}

/// Reads a layer element's flat float payload, whatever its component count.
///
/// FBX writes these as `f64` arrays; some exporters use `f32`.
fn read_layer_floats(layer: &FbxNode, name: &str) -> Option<Vec<f32>> {
    for child in &layer.children {
        if child.name == name {
            if let Some(values) = float_values(child) {
                return Some(values.into_iter().map(|v| v as f32).collect());
            }
        }
    }
    None
}

/// Expand a `LayerElementMaterial` data array to per-triangle material indices.
fn expand_material_indices(
    mapping: Option<&str>,
    reference: Option<&str>,
    data: Option<&[i32]>,
    polygon_count: usize,
    tri_polygon_index: &[usize],
) -> Option<Vec<i32>> {
    let mapping = mapping.unwrap_or("AllSame");
    let data = data?;
    // `IndexToDirect` semantics: each entry of `Materials` is itself the
    // absolute material index (FBX rarely uses a separate index array for
    // materials, but we honour `IndexToDirect` by treating `data` as the
    // direct list when no separate index exists).
    let _ = reference;
    let per_polygon: Vec<i32> = match mapping {
        "AllSame" => {
            let value = data.first().copied().unwrap_or(0);
            vec![value; polygon_count.max(1)]
        }
        "ByPolygon" | "ByPolygonSide" => data.to_vec(),
        "ByPolygonVertex" => {
            // We do not retain per-vertex polygon order here; pick the first
            // vertex entry of each polygon. The caller passes
            // `tri_polygon_index` keyed by polygon index.
            // Without polygon-vertex correspondence we fall back to AllSame.
            let value = data.first().copied().unwrap_or(0);
            vec![value; polygon_count.max(1)]
        }
        _ => return None,
    };
    if per_polygon.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::with_capacity(tri_polygon_index.len());
    for &polygon_index in tri_polygon_index {
        let value = per_polygon
            .get(polygon_index)
            .copied()
            .unwrap_or(per_polygon[0]);
        out.push(value);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KTIME: f64 = 46_186_158_000.0;

    fn curve(times: &[i64], values: &[f32]) -> FbxAnimCurveData {
        FbxAnimCurveData {
            key_times: times.to_vec(),
            key_values: values.to_vec(),
            key_attr_flags: vec![0x4; times.len()],
            in_tangents: vec![0.0; times.len()],
            out_tangents: vec![0.0; times.len()],
        }
    }

    /// Seconds to FBX ticks, for building key times a test can read back.
    fn ticks(seconds: f64) -> i64 {
        (seconds * KTIME) as i64
    }

    #[test]
    fn components_sharing_a_key_grid_keep_their_authored_keys() {
        let by_component = [
            Some(curve(&[0, ticks(1.0)], &[1.0, 2.0])),
            Some(curve(&[0, ticks(1.0)], &[3.0, 4.0])),
            Some(curve(&[0, ticks(1.0)], &[5.0, 6.0])),
        ];
        let channel = flatten_curve(
            &by_component,
            &[None; 3],
            FbxAnimChannelPath::Translation,
            KTIME,
        )
        .expect("channel");
        assert_eq!(channel.sampler.input, vec![0.0, 1.0]);
        assert_eq!(channel.sampler.output, vec![1.0, 3.0, 5.0, 2.0, 4.0, 6.0]);
    }

    #[test]
    fn a_short_component_holds_its_last_value_instead_of_dropping_to_zero() {
        // The shape that put a character through the floor: X is animated
        // across the take while Y stops early. Y must hold, not vanish.
        let by_component = [
            Some(curve(&[0, ticks(1.0), ticks(2.0)], &[0.0, 10.0, 20.0])),
            Some(curve(&[0], &[7.0])),
            None,
        ];
        let channel = flatten_curve(
            &by_component,
            &[None; 3],
            FbxAnimChannelPath::Translation,
            KTIME,
        )
        .expect("channel");
        assert_eq!(channel.sampler.input, vec![0.0, 1.0, 2.0]);
        let y: Vec<f32> = channel.sampler.output.chunks(3).map(|c| c[1]).collect();
        assert_eq!(y, vec![7.0, 7.0, 7.0]);
    }

    #[test]
    fn differing_grids_merge_and_each_component_is_read_at_the_right_time() {
        // X keys at 0 s and 2 s, Y at 0 s and 1 s. Pairing by index would read
        // Y's 1 s value at X's 2 s key; the merged grid has all three times and
        // samples each curve where it actually is.
        let by_component = [
            Some(curve(&[0, ticks(2.0)], &[0.0, 20.0])),
            Some(curve(&[0, ticks(1.0)], &[0.0, 5.0])),
            None,
        ];
        let channel = flatten_curve(
            &by_component,
            &[None; 3],
            FbxAnimChannelPath::Translation,
            KTIME,
        )
        .expect("channel");
        assert_eq!(channel.sampler.input, vec![0.0, 1.0, 2.0]);
        let x: Vec<f32> = channel.sampler.output.chunks(3).map(|c| c[0]).collect();
        let y: Vec<f32> = channel.sampler.output.chunks(3).map(|c| c[1]).collect();
        // X is interpolated onto Y's key, Y holds past its own last key.
        assert_eq!(x, vec![0.0, 10.0, 20.0]);
        assert_eq!(y, vec![0.0, 5.0, 5.0]);
        assert_eq!(channel.sampler.interpolation, FbxAnimInterpolation::Linear);
    }

    #[test]
    fn a_merged_grid_drops_tangents_that_no_longer_describe_a_segment() {
        let mut x = curve(&[0, ticks(2.0)], &[0.0, 20.0]);
        x.key_attr_flags = vec![0x8; 2];
        x.in_tangents = vec![1.0, 2.0];
        x.out_tangents = vec![3.0, 4.0];
        let by_component = [Some(x), Some(curve(&[0, ticks(1.0)], &[0.0, 5.0])), None];
        let channel = flatten_curve(
            &by_component,
            &[None; 3],
            FbxAnimChannelPath::Translation,
            KTIME,
        )
        .expect("channel");
        assert_eq!(channel.sampler.interpolation, FbxAnimInterpolation::Linear);
        assert!(channel.sampler.in_tangents.is_none());
        assert!(channel.sampler.out_tangents.is_none());
    }
}
