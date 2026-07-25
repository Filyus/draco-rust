//! FBX scene reader: a tree of [`FbxNode`] in, an [`FbxScene`] out.
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
            ..
        } = &index;

        // Container-layout notices raised by `read_nodes` above ride along
        // with the semantic ones, so a caller sees every tolerated deviation.
        let mut warnings = self.warnings().to_vec();
        // A pre-7000 document keys its objects and connections by name rather
        // than by id, and puts geometry on the `Model` itself. None of that is
        // read, so the scene comes back structurally valid but empty. Saying so
        // is the difference between "this file has no meshes" and "this reader
        // did not look for them".
        if index.name_keyed_objects > 0 {
            let count = index.name_keyed_objects;
            push_warning(
                &mut warnings,
                FbxWarningCode::NameKeyedObjectModel,
                format!(
                    "FBX document identifies its {count} objects by name rather than by id, \
                     which is the pre-7000 layout; no geometry, materials or animation were \
                     imported from it"
                ),
                None,
            );
        }
        collect_transform_warnings(model_map, model_order, &mut warnings);
        let node_attributes =
            parse_node_attributes(attribute_map, model_map, connections, &mut warnings);

        // ---- Materials and textures ---------------------------------------
        let (materials, material_index_by_id, textures) =
            parse_materials_and_textures(material_map, texture_map, video_map, connections);

        // ---- Model hierarchy + per-model materials -----------------------
        // Map each model id to the list of material indices connected to it.
        let mut model_material_ids: HashMap<i64, Vec<i32>> = HashMap::new();
        for conn in connections {
            if conn.kind == ConnectionKind::Oo
                && material_map.contains_key(&conn.child)
                && model_map.contains_key(&conn.parent)
            {
                let mat_index = material_index_by_id[&conn.child] as i32;
                model_material_ids
                    .entry(conn.parent)
                    .or_default()
                    .push(mat_index);
            }
        }

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

        // Helper to parse transform from Model node's Properties70
        fn parse_transform(
            node: &FbxNode,
        ) -> Option<(FbxTransform, crate::fbx_scene::FbxTransformStack, bool)> {
            let mut translation = None;
            let mut rotation = None;
            let mut scaling = None;
            let mut pre_rotation = None;
            let mut post_rotation = None;
            let mut rotation_offset = None;
            let mut rotation_pivot = None;
            let mut scaling_offset = None;
            let mut scaling_pivot = None;
            let mut rotation_order = None;
            let mut rotation_active = None;
            let mut inherit_type = None;

            fn property_vec3(property: &FbxNode) -> Option<[f32; 3]> {
                for value in &property.properties {
                    if let crate::fbx_reader::FbxProperty::F64Array(values) = value {
                        if values.len() >= 3 {
                            return Some([values[0] as f32, values[1] as f32, values[2] as f32]);
                        }
                    }
                }

                let values: Vec<f32> = property
                    .properties
                    .iter()
                    .filter_map(|value| match value {
                        crate::fbx_reader::FbxProperty::F64(value) => Some(*value as f32),
                        crate::fbx_reader::FbxProperty::F32(value) => Some(*value),
                        _ => None,
                    })
                    .take(3)
                    .collect();
                (values.len() == 3).then(|| [values[0], values[1], values[2]])
            }

            fn property_i32(property: &FbxNode) -> Option<i32> {
                property.properties.iter().find_map(|value| match value {
                    crate::fbx_reader::FbxProperty::I32(value) => Some(*value),
                    crate::fbx_reader::FbxProperty::I16(value) => Some(*value as i32),
                    crate::fbx_reader::FbxProperty::I64(value) => i32::try_from(*value).ok(),
                    _ => None,
                })
            }

            fn property_bool(property: &FbxNode) -> Option<bool> {
                property.properties.iter().find_map(|value| match value {
                    crate::fbx_reader::FbxProperty::Bool(value) => Some(*value),
                    crate::fbx_reader::FbxProperty::I32(value) => Some(*value != 0),
                    crate::fbx_reader::FbxProperty::I16(value) => Some(*value != 0),
                    crate::fbx_reader::FbxProperty::I64(value) => Some(*value != 0),
                    _ => None,
                })
            }

            for child in &node.children {
                if child.name == "Properties70" {
                    for prop in &child.children {
                        // property nodes often have first property as name string
                        if let Some(crate::fbx_reader::FbxProperty::String(name)) =
                            prop.properties.first()
                        {
                            if name.contains("Lcl Translation") {
                                translation = property_vec3(prop);
                            }
                            if name.contains("Lcl Rotation") {
                                rotation = property_vec3(prop);
                            }
                            if name.contains("Lcl Scaling") {
                                scaling = property_vec3(prop);
                            }
                            match name.as_str() {
                                "PreRotation" => pre_rotation = property_vec3(prop),
                                "PostRotation" => post_rotation = property_vec3(prop),
                                "RotationOffset" => rotation_offset = property_vec3(prop),
                                "RotationPivot" => rotation_pivot = property_vec3(prop),
                                "ScalingOffset" => scaling_offset = property_vec3(prop),
                                "ScalingPivot" => scaling_pivot = property_vec3(prop),
                                "RotationOrder" => rotation_order = property_i32(prop),
                                "RotationActive" => rotation_active = property_bool(prop),
                                "InheritType" => inherit_type = property_i32(prop),
                                _ => {}
                            }
                        }
                    }
                }
            }

            if translation.is_none() && rotation.is_none() && scaling.is_none() {
                return None;
            }

            // FBX local transform stack (without the parent-dependent
            // InheritType rule):
            // T * Roff * Rp * PreR * R * PostR^-1 * Rp^-1 * Soff * Sp * S * Sp^-1.
            // The packed scene layout is also the WebGL column-major layout.
            let t = translation.unwrap_or([0.0, 0.0, 0.0]);
            let r_deg = rotation.unwrap_or([0.0, 0.0, 0.0]);
            let s = scaling.unwrap_or([1.0, 1.0, 1.0]);

            fn identity() -> [[f32; 4]; 4] {
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ]
            }
            // FbxTransform is packed column-major: its outer index is the
            // column and its inner index is the row. Evaluate the local stack
            // in that layout so the result can go straight to WebGL.
            fn multiply(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
                let mut result = [[0.0; 4]; 4];
                for column in 0..4 {
                    for row in 0..4 {
                        result[column][row] =
                            (0..4).map(|index| a[index][row] * b[column][index]).sum();
                    }
                }
                result
            }
            fn translation_matrix(values: [f32; 3]) -> [[f32; 4]; 4] {
                let mut matrix = identity();
                matrix[3][0] = values[0];
                matrix[3][1] = values[1];
                matrix[3][2] = values[2];
                matrix
            }
            fn scale(values: [f32; 3]) -> [[f32; 4]; 4] {
                [
                    [values[0], 0.0, 0.0, 0.0],
                    [0.0, values[1], 0.0, 0.0],
                    [0.0, 0.0, values[2], 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ]
            }
            fn rotation_matrix(values: [f32; 3]) -> [[f32; 4]; 4] {
                let (sin_x, cos_x) = values[0].to_radians().sin_cos();
                let (sin_y, cos_y) = values[1].to_radians().sin_cos();
                let (sin_z, cos_z) = values[2].to_radians().sin_cos();
                // Rz * Ry * Rx, packed by column for FBX/WebGL.
                [
                    [cos_z * cos_y, sin_z * cos_y, -sin_y, 0.0],
                    [
                        cos_z * sin_y * sin_x - sin_z * cos_x,
                        sin_z * sin_y * sin_x + cos_z * cos_x,
                        cos_y * sin_x,
                        0.0,
                    ],
                    [
                        cos_z * sin_y * cos_x + sin_z * sin_x,
                        sin_z * sin_y * cos_x - cos_z * sin_x,
                        cos_y * cos_x,
                        0.0,
                    ],
                    [0.0, 0.0, 0.0, 1.0],
                ]
            }
            let inverse_translation =
                |values: [f32; 3]| translation_matrix([-values[0], -values[1], -values[2]]);
            let inverse_rotation = |values: [f32; 3]| {
                let rotation = rotation_matrix(values);
                let mut inverse = [[0.0; 4]; 4];
                for column in 0..4 {
                    for row in 0..4 {
                        inverse[column][row] = rotation[row][column];
                    }
                }
                inverse
            };
            // Blender's FBX bind matrices encode the pre-rotation with the
            // opposite handedness from the local Euler property.  Applying
            // its inverse here keeps Mixamo-style armatures aligned while
            // leaving ordinary TRS-only files unchanged.
            let inverse_pre_rotation = pre_rotation
                .map(|values| [-values[0], -values[1], -values[2]])
                .unwrap_or([0.0; 3]);
            // The scene matrix uses the same packed layout as the writer:
            // translation occupies the final packed column.  Do not multiply
            // the local translation through the rotation stack here; doing so
            // rotates a node's origin (and breaks ordinary TRS-only FBX
            // written by us).  The pivot terms below only shape the linear
            // part and their translation compensation.
            let mut mat = identity();
            for term in [
                translation_matrix(rotation_offset.unwrap_or([0.0; 3])),
                translation_matrix(rotation_pivot.unwrap_or([0.0; 3])),
                rotation_matrix(inverse_pre_rotation),
                rotation_matrix(r_deg),
                inverse_rotation(post_rotation.unwrap_or([0.0; 3])),
                inverse_translation(rotation_pivot.unwrap_or([0.0; 3])),
                translation_matrix(scaling_offset.unwrap_or([0.0; 3])),
                translation_matrix(scaling_pivot.unwrap_or([0.0; 3])),
                scale(s),
                inverse_translation(scaling_pivot.unwrap_or([0.0; 3])),
            ] {
                mat = multiply(mat, term);
            }
            mat[3][0] += t[0];
            mat[3][1] += t[1];
            mat[3][2] += t[2];

            // A bind pose contains the exporter-evaluated local orientation
            // for nodes that use FBX's pre/post rotation or pivot terms. The
            // semantic preview keeps that baked basis for animation, while
            // ordinary Model TRS nodes keep their authored local values.
            let non_zero = |values: Option<[f32; 3]>| {
                values.is_some_and(|values| values.iter().any(|value| value.abs() > f32::EPSILON))
            };
            let transform_stack = crate::fbx_scene::FbxTransformStack {
                translation,
                rotation,
                scaling,
                rotation_order,
                rotation_active,
                pre_rotation,
                post_rotation,
                rotation_offset,
                rotation_pivot,
                scaling_offset,
                scaling_pivot,
                inherit_type,
            };
            // RotationOrder and InheritType are source-provenance metadata;
            // their ordinary/default values do not mean that the static
            // Model TRS has been baked into the skin BindPose. Keep the
            // runtime flag limited to actual pivot/offset/pre/post terms so
            // plain TRS clips (including Samba Dancing) retain authored
            // animation composition while the metadata is still re-emitted.
            // Non-default rotation-order/inheritance evaluation remains an
            // explicit compatibility caveat at the animation boundary.
            let has_complex_transform_stack = non_zero(pre_rotation)
                || non_zero(post_rotation)
                || non_zero(rotation_offset)
                || non_zero(rotation_pivot)
                || non_zero(scaling_offset)
                || non_zero(scaling_pivot);

            Some((
                FbxTransform { matrix: mat },
                transform_stack,
                has_complex_transform_stack,
            ))
        }

        // Build nodes recursively
        fn object_name(node: &FbxNode) -> Option<String> {
            match node.properties.get(1) {
                Some(FbxProperty::String(name)) => name
                    .split('\0')
                    .next()
                    .filter(|name| !name.is_empty())
                    .map(str::to_string),
                _ => None,
            }
        }

        let ordered_model_ids = model_order;
        let model_node_ids: HashMap<i64, FbxNodeId> = ordered_model_ids
            .iter()
            .copied()
            .enumerate()
            .map(|(index, id)| (id, FbxNodeId((index + 1) as u32)))
            .collect();

        fn build_model_node(
            id: i64,
            model_map: &std::collections::HashMap<i64, &FbxNode>,
            model_children: &std::collections::HashMap<i64, Vec<i64>>,
            model_mesh_instances: &std::collections::HashMap<i64, Vec<FbxMeshInstance>>,
            model_node_ids: &std::collections::HashMap<i64, FbxNodeId>,
            node_attributes: &std::collections::HashMap<i64, FbxNodeAttribute>,
            ancestors: &mut Vec<i64>,
        ) -> FbxSceneNode {
            let node_src = model_map.get(&id).unwrap();
            let mut node = FbxSceneNode::new(object_name(node_src));
            node.id = model_node_ids[&id];
            if let Some((transform, transform_stack, has_complex_transform_stack)) =
                parse_transform(node_src)
            {
                node.transform = Some(transform);
                node.transform_stack = Some(transform_stack);
                node.has_complex_transform_stack = has_complex_transform_stack;
            }
            node.attribute = node_attributes.get(&id).cloned();
            if let Some(mesh_instances) = model_mesh_instances.get(&id) {
                node.mesh_instances.extend(mesh_instances.clone());
            }

            // The ancestor check below stops a plain cycle. This bounds the
            // rest: a document can chain models far deeper than any scene
            // graph needs, and the depth is the file's to choose.
            const MAX_MODEL_DEPTH: usize = 256;
            if ancestors.len() >= MAX_MODEL_DEPTH {
                return node;
            }
            if let Some(children) = model_children.get(&id) {
                ancestors.push(id);
                for &cid in children {
                    // A document may connect a Model to one of its own
                    // ancestors -- `synthetic_id_collision_7500` in the ufbx
                    // corpus does -- and following that cycle recurses until
                    // the stack is gone. The scene simply stops there.
                    if model_map.contains_key(&cid) && !ancestors.contains(&cid) {
                        node.children.push(build_model_node(
                            cid,
                            model_map,
                            model_children,
                            model_mesh_instances,
                            model_node_ids,
                            node_attributes,
                            ancestors,
                        ));
                    }
                }
                ancestors.pop();
            }
            node
        }

        // Map geometries to models and create mesh instances.
        let mut model_mesh_instances: std::collections::HashMap<i64, Vec<FbxMeshInstance>> =
            std::collections::HashMap::new();
        let mut geometry_ids: Vec<i64> = geometry_map.keys().copied().collect();
        geometry_ids.sort_unstable();
        for geom_id in geometry_ids {
            let geom_node = geometry_map[&geom_id];
            if let Some(source) = self.geometry_to_mesh(geom_node, &mut warnings)? {
                let mesh = &source.mesh;
                let material_indices = source.material_indices.clone();
                // find connection mapping geometry -> model
                for conn in connections.iter() {
                    if conn.child == geom_id && model_map.contains_key(&conn.parent) {
                        // If the geometry does not carry its own material layer,
                        // fall back to materials connected directly to the model.
                        let mut indices = material_indices.clone();
                        if let Some(model_mats) = model_material_ids.get(&conn.parent) {
                            if indices.is_empty() {
                                if !model_mats.is_empty() {
                                    let first = model_mats[0];
                                    // One entry per triangulated face.
                                    indices = vec![first; mesh.num_faces()];
                                }
                            } else {
                                // LayerElementMaterial values address the
                                // material slots attached to this Model. Map
                                // them back to the document-wide material
                                // indices exposed by FbxScene.
                                indices = indices
                                    .into_iter()
                                    .map(|slot| {
                                        usize::try_from(slot)
                                            .ok()
                                            .and_then(|slot| model_mats.get(slot).copied())
                                            .unwrap_or(model_mats[0])
                                    })
                                    .collect();
                            }
                        }
                        let mesh_instance = FbxMeshInstance {
                            name: object_name(geom_node),
                            mesh: source.mesh.clone(),
                            control_points: source.control_points.clone(),
                            polygon_vertex_indices: source.polygon_vertex_indices.clone(),
                            layers: source.layers.clone(),
                            edges: source.edges.clone(),
                            material_indices: indices,
                            skin: parse_skin_for_geometry(
                                geom_id,
                                deformer_map,
                                pose_map,
                                connections,
                                &model_node_ids,
                            ),
                            morph_targets: parse_morph_targets_for_geometry(
                                geom_id,
                                geometry_map,
                                deformer_map,
                                connections,
                            ),
                        };
                        model_mesh_instances
                            .entry(conn.parent)
                            .or_default()
                            .push(mesh_instance);
                    }
                }
            }
        }

        // ---- Animation ----------------------------------------------------
        let model_name_map: HashMap<i64, String> = model_map
            .iter()
            .filter_map(|(id, node)| object_name(node).map(|name| (*id, name)))
            .collect();

        let animations = self.parse_animations(
            &nodes,
            &index,
            &model_name_map,
            &model_node_ids,
            &morph_animation_targets(geometry_map, deformer_map, connections, model_map),
        );

        // Build root nodes: any model with parent 0 (or with no parent present)
        let mut root_nodes = Vec::new();
        // find top-level model ids
        let top_level: Vec<i64> = ordered_model_ids
            .iter()
            .copied()
            .filter(|id| {
                !connections
                    .iter()
                    .any(|conn| conn.child == *id && model_map.contains_key(&conn.parent))
            })
            .collect();

        for id in top_level {
            root_nodes.push(build_model_node(
                id,
                model_map,
                &model_children,
                &model_mesh_instances,
                &model_node_ids,
                &node_attributes,
                &mut Vec::new(),
            ));
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

fn parse_global_settings(nodes: &[FbxNode]) -> Option<crate::fbx_scene::FbxGlobalSettings> {
    let properties = nodes
        .iter()
        .find(|node| node.name == "GlobalSettings")?
        .children
        .iter()
        .find(|node| node.name == "Properties70")?;
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

fn identity_transform() -> FbxTransform {
    FbxTransform {
        matrix: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

fn transform_array(node: &FbxNode, child_name: &str) -> Option<FbxTransform> {
    let values = node
        .children
        .iter()
        .find(|child| child.name == child_name)?
        .properties
        .first()?;
    let values: Vec<f32> = match values {
        FbxProperty::F64Array(values) => values.iter().copied().map(|value| value as f32).collect(),
        FbxProperty::F32Array(values) => values.clone(),
        _ => return None,
    };
    if values.len() != 16 {
        return None;
    }
    // Preserve the 16-value FBX matrix layout. In particular, cluster
    // Transform/TransformLink translations are already at elements 12..14;
    // transposing here moves them into the projective row and explodes a
    // skinned mesh.
    let mut matrix = [[0.0; 4]; 4];
    for (index, value) in values.into_iter().enumerate() {
        matrix[index / 4][index % 4] = value;
    }
    Some(FbxTransform { matrix })
}

fn child_i32_array(node: &FbxNode, child_name: &str) -> Vec<i32> {
    node.children
        .iter()
        .find(|child| child.name == child_name)
        .and_then(|child| child.properties.first())
        .and_then(|value| match value {
            FbxProperty::I32Array(values) => Some(values.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn child_f64_array(node: &FbxNode, child_name: &str) -> Vec<f64> {
    node.children
        .iter()
        .find(|child| child.name == child_name)
        .and_then(|child| child.properties.first())
        .and_then(|value| match value {
            FbxProperty::F64Array(values) => Some(values.clone()),
            FbxProperty::F32Array(values) => Some(values.iter().copied().map(f64::from).collect()),
            _ => None,
        })
        .unwrap_or_default()
}

fn parse_skin_for_geometry(
    geometry_id: i64,
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
            let indices = child_i32_array(cluster, "Indexes")
                .into_iter()
                .filter_map(|index| u32::try_from(index).ok())
                .collect::<Vec<_>>();
            let mut weights = child_f64_array(cluster, "Weights")
                .into_iter()
                .map(|weight| weight as f32)
                .collect::<Vec<_>>();
            weights.truncate(indices.len());
            if weights.len() != indices.len() {
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
                .and_then(|value| match value {
                    FbxProperty::I64(value) => Some(*value),
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

fn parse_morph_targets_for_geometry(
    geometry_id: i64,
    geometries: &std::collections::HashMap<i64, &FbxNode>,
    deformers: &std::collections::HashMap<i64, &FbxNode>,
    connections: &[FbxConnection],
) -> Vec<crate::fbx_scene::FbxMorphTarget> {
    let mut targets = Vec::new();
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
                    .chunks_exact(3)
                    .map(|values| [values[0] as f32, values[1] as f32, values[2] as f32])
                    .collect();
                let full_weight = child_f64_array(channel, "FullWeights")
                    .first()
                    .copied()
                    .unwrap_or(100.0) as f32;
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
    targets
}

/// Resolve BlendShapeChannel object ids to their owning Model and target slot.
/// FBX animation curves target the channel deformer rather than the mesh
/// model, so this bridge is required to expose them through the scene API.
fn morph_animation_targets(
    geometries: &std::collections::HashMap<i64, &FbxNode>,
    deformers: &std::collections::HashMap<i64, &FbxNode>,
    connections: &[FbxConnection],
    models: &std::collections::HashMap<i64, &FbxNode>,
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
        for blend_shape_id in connections
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
        {
            for (index, channel_id) in connections
                .iter()
                .filter(|connection| {
                    connection.kind == ConnectionKind::Oo
                        && connection.parent == blend_shape_id
                        && deformers
                            .get(&connection.child)
                            .and_then(|node| deformer_type(node))
                            == Some("BlendShapeChannel")
                })
                .map(|connection| connection.child)
                .enumerate()
            {
                result.insert(channel_id, (model_id, index as u32));
            }
        }
    }
    result
}

/// FBX object-to-object connection type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// `Objects` children skipped because they are keyed by name, not by id.
    ///
    /// FBX 6100 and earlier identify objects by a name string ending in a
    /// class marker, and connect them by that string rather than by the `i64`
    /// id 7.x uses. Nothing in this index can hold them, so counting them is
    /// how the reader notices it decoded a document it does not understand.
    name_keyed_objects: usize,
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
            name_keyed_objects: 0,
        };

        for node in nodes {
            if node.name == "Objects" {
                for child in &node.children {
                    let Some(id) = child.properties.first().and_then(object_id) else {
                        if matches!(child.properties.first(), Some(FbxProperty::String(_))) {
                            index.name_keyed_objects += 1;
                        }
                        continue;
                    };
                    let id = &id;
                    match child.name.as_str() {
                        "Model" => {
                            // Keep the authored order only for ids seen first.
                            let first_occurrence = index.model_map.insert(*id, child).is_none();
                            if first_occurrence {
                                index.model_order.push(*id);
                            }
                        }
                        "Geometry" => drop(index.geometry_map.insert(*id, child)),
                        "Material" => drop(index.material_map.insert(*id, child)),
                        "Texture" => drop(index.texture_map.insert(*id, child)),
                        "Video" => drop(index.video_map.insert(*id, child)),
                        "AnimationStack" => drop(index.astack_map.insert(*id, child)),
                        "AnimationLayer" => drop(index.alayer_map.insert(*id, child)),
                        "AnimationCurveNode" => drop(index.acnode_map.insert(*id, child)),
                        "AnimationCurve" => drop(index.acurve_map.insert(*id, child)),
                        "Pose" => drop(index.pose_map.insert(*id, child)),
                        "NodeAttribute" => drop(index.attribute_map.insert(*id, child)),
                        "Deformer" => drop(index.deformer_map.insert(*id, child)),
                        _ => {}
                    }
                }
            } else if node.name == "Connections" {
                index
                    .connections
                    .extend(node.children.iter().filter_map(FbxConnection::from_node));
            }
        }
        index
    }
}

/// Reads a float array, whatever precision it was stored at.
///
/// The binary container tags single and double precision separately, but ASCII
/// writes a bare number and cannot. A consumer that matched only one width
/// found nothing in an ASCII document, which is how animation curves came back
/// empty from files whose objects and connections had parsed perfectly.
fn float_array(property: &FbxProperty) -> Option<Vec<f32>> {
    match property {
        FbxProperty::F32Array(values) => Some(values.clone()),
        FbxProperty::F64Array(values) => Some(values.iter().map(|v| *v as f32).collect()),
        _ => None,
    }
}

/// Reads an FBX object id, whatever width it was stored at.
///
/// The binary container always writes these as `i64`. ASCII writes a bare
/// number, so an id small enough to fit in `i32` arrives as one -- matching
/// only `I64` skipped every object in such a document, and the scene came back
/// empty with nothing to explain it.
fn object_id(property: &FbxProperty) -> Option<i64> {
    match property {
        FbxProperty::I64(value) => Some(*value),
        FbxProperty::I32(value) => Some(i64::from(*value)),
        _ => None,
    }
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
    /// Parses one `C` entry, skipping relation codes this reader ignores.
    fn from_node(node: &FbxNode) -> Option<Self> {
        let kind = match node.properties.first() {
            Some(FbxProperty::String(code)) if code == "OO" => ConnectionKind::Oo,
            Some(FbxProperty::String(code)) if code == "OP" => ConnectionKind::Op,
            _ => return None,
        };
        let child = node.properties.get(1).and_then(object_id)?;
        let parent = node.properties.get(2).and_then(object_id)?;
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

/// Reports Models whose FBX inheritance rule the imported transform cannot
/// express.
///
/// Only the first such Model is reported: the notice describes a property of
/// the file, and repeating it per Model would bury everything else. Models are
/// visited in authored order so the scan does not depend on hash iteration.
fn collect_transform_warnings(
    model_map: &HashMap<i64, &FbxNode>,
    model_order: &[i64],
    warnings: &mut Vec<FbxWarning>,
) {
    for model in model_order.iter().filter_map(|id| model_map.get(id)) {
        for properties in model
            .children
            .iter()
            .filter(|child| child.name == "Properties70")
        {
            for entry in &properties.children {
                let Some(FbxProperty::String(name)) = entry.properties.first() else {
                    continue;
                };
                if name != "InheritType" {
                    continue;
                }
                let inherit_type = entry.properties.iter().find_map(|value| match value {
                    FbxProperty::I32(value) => Some(*value),
                    FbxProperty::I64(value) => i32::try_from(*value).ok(),
                    _ => None,
                });
                let local_scale = properties.children.iter().find_map(|property| {
                    let Some(FbxProperty::String(name)) = property.properties.first() else {
                        return None;
                    };
                    if !name.contains("Lcl Scaling") {
                        return None;
                    }
                    let values: Vec<f32> = property
                        .properties
                        .iter()
                        .filter_map(|value| match value {
                            FbxProperty::F64(value) => Some(*value as f32),
                            FbxProperty::F32(value) => Some(*value),
                            _ => None,
                        })
                        .take(3)
                        .collect();
                    // `then_some` evaluates its argument eagerly, so indexing
                    // here panicked whenever the property carried fewer than
                    // three numbers.
                    (values.len() == 3).then(|| [values[0], values[1], values[2]])
                });
                // A non-uniform scale is what makes the unsupported inherit
                // modes observable; uniform scale behaves the same either way.
                let uniform_scale = local_scale
                    .map(|scale| {
                        (scale[0] - scale[1]).abs() <= 1e-5 && (scale[1] - scale[2]).abs() <= 1e-5
                    })
                    .unwrap_or(true);
                if matches!(inherit_type, Some(0..=2)) && uniform_scale {
                    continue;
                }
                push_warning(
                    warnings,
                    FbxWarningCode::UnsupportedTransformInherit,
                    format!(
                        "FBX model uses unsupported {name}; local TRS was imported \
                         without that FBX transform rule"
                    ),
                    None,
                );
                return;
            }
        }
    }
}

/// Decodes every `Material` and `Texture` object, resolving each material's
/// texture bindings to indices into the returned texture list.
///
/// Both lists are ordered by FBX object id rather than hash order, so a
/// document always decodes to the same material and texture indices.
fn parse_materials_and_textures(
    material_map: &HashMap<i64, &FbxNode>,
    texture_map: &HashMap<i64, &FbxNode>,
    video_map: &HashMap<i64, &FbxNode>,
    connections: &[FbxConnection],
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
        let mut material = parse_material(material_map[&id]);
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

/// FBX Deformer objects carry their effective kind in the third object
/// property; the second name component is merely `Deformer`/`SubDeformer`.
fn deformer_type(node: &FbxNode) -> Option<&str> {
    match node.properties.get(2) {
        Some(FbxProperty::String(value)) if !value.is_empty() => Some(value.as_str()),
        _ => None,
    }
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
fn parse_node_attributes(
    attribute_map: &HashMap<i64, &FbxNode>,
    model_map: &HashMap<i64, &FbxNode>,
    connections: &[FbxConnection],
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
        let class = match node.properties.get(2) {
            Some(FbxProperty::String(class)) => class.as_str(),
            _ => continue,
        };
        match class {
            "Camera" => {
                resolved.insert(model_id, FbxNodeAttribute::Camera(parse_camera(node)));
            }
            "Light" => {
                resolved.insert(model_id, FbxNodeAttribute::Light(parse_light(node)));
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

fn parse_camera(node: &FbxNode) -> crate::fbx_scene::FbxCamera {
    let scalar = |name: &str| properties70_lookup(node, name).and_then(property_scalar);
    let vector = |name: &str| properties70_lookup(node, name).and_then(property_vec3);
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
        ortho_zoom: scalar("OrthoZoom"),
    }
}

fn parse_light(node: &FbxNode) -> crate::fbx_scene::FbxLight {
    let scalar = |name: &str| properties70_lookup(node, name).and_then(property_scalar);
    crate::fbx_scene::FbxLight {
        light_type: scalar("LightType").map(|v| v as i32),
        color: properties70_lookup(node, "Color").and_then(property_vec3),
        intensity: scalar("Intensity"),
        cast_light: scalar("CastLight").map(|v| v != 0.0),
        cast_shadows: scalar("CastShadows").map(|v| v != 0.0),
        decay_type: scalar("DecayType").map(|v| v as i32),
        decay_start: scalar("DecayStart"),
    }
}

/// Extracts a named scalar/`Color`/`Vector`/`Vector3D` property from a
/// `Properties70` block of the given FBX object.
fn properties70_lookup<'a>(node: &'a FbxNode, name: &str) -> Option<&'a FbxNode> {
    for child in &node.children {
        if child.name != "Properties70" {
            continue;
        }
        for prop in &child.children {
            if prop.name != "P" {
                continue;
            }
            if let Some(FbxProperty::String(prop_name)) = prop.properties.first() {
                if prop_name == name {
                    return Some(prop);
                }
            }
        }
    }
    None
}

fn property_scalar(prop: &FbxNode) -> Option<f32> {
    // Properties70 P node layout: [name, type, subtype, flags, value(s)...]
    // Scalar properties start at index 4.
    for value in prop.properties.iter().skip(4) {
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
        .skip(4)
        .filter_map(|value| match value {
            FbxProperty::F64(v) => Some(*v as f32),
            FbxProperty::F32(v) => Some(*v),
            _ => None,
        })
        .take(3)
        .collect();
    (values.len() == 3).then(|| [values[0], values[1], values[2]])
}

fn parse_material(node: &FbxNode) -> crate::fbx_scene::FbxMaterial {
    let name = match node.properties.get(1) {
        Some(FbxProperty::String(raw)) => raw
            .split('\0')
            .next()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        _ => None,
    };
    let shading_model = read_shading_model(node);

    let get_color = |name: &str| properties70_lookup(node, name).and_then(property_vec3);
    let get_scalar = |name: &str| properties70_lookup(node, name).and_then(property_scalar);

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

/// Reads `ShadingModel`, which some files store as a Properties70 string and
/// others as the third object-level string property.
fn read_shading_model(node: &FbxNode) -> Option<String> {
    for child in &node.children {
        if child.name != "Properties70" {
            continue;
        }
        for prop in &child.children {
            if prop.name != "P" {
                continue;
            }
            if let Some(FbxProperty::String(name)) = prop.properties.first() {
                if name == "ShadingModel" {
                    for value in prop.properties.iter().skip(4) {
                        if let FbxProperty::String(s) = value {
                            return Some(s.clone());
                        }
                    }
                }
            }
        }
    }
    // Fall back to the object-level third property.
    if let Some(FbxProperty::String(raw)) = node.properties.get(2) {
        if !raw.is_empty() {
            return Some(raw.clone());
        }
    }
    None
}

fn parse_texture(node: &FbxNode) -> crate::fbx_scene::FbxTexture {
    let name = match node.properties.get(1) {
        Some(FbxProperty::String(raw)) => raw
            .split('\0')
            .next()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        _ => None,
    };
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
                    if child.name == "Geometry" {
                        if let Some(source) = self.geometry_to_mesh(child, &mut warnings)? {
                            meshes.push(source.mesh);
                        }
                    }
                }
            }
        }

        self.extend_warnings(warnings);
        Ok(meshes)
    }

    /// Convert a Geometry node to a Mesh, plus per-triangle material indices.
    ///
    /// The returned `material_indices` align with the fan-triangulated face
    /// order of the Draco `Mesh` (one entry per triangle). The list is empty
    /// when the geometry does not carry a `LayerElementMaterial` layer.
    fn geometry_to_mesh(
        &self,
        geometry: &FbxNode,
        warnings: &mut Vec<FbxWarning>,
    ) -> io::Result<Option<FbxGeometrySource>> {
        let mut vertices: Option<Vec<f64>> = None;
        let mut polygon_indices: Option<Vec<i32>> = None;
        let mut edges: Vec<i32> = Vec::new();
        let mut normals_layers: Vec<&FbxNode> = Vec::new();
        let mut uv_layers: Vec<&FbxNode> = Vec::new();
        let mut color_layers: Vec<&FbxNode> = Vec::new();
        let mut tangent_layers: Vec<&FbxNode> = Vec::new();
        let mut binormal_layers: Vec<&FbxNode> = Vec::new();
        let mut material_layer: Option<&FbxNode> = None;
        let mut smoothing_layers_raw: Vec<&FbxNode> = Vec::new();
        let mut crease_layers_raw: Vec<(FbxCreaseKind, &FbxNode)> = Vec::new();

        for child in &geometry.children {
            match child.name.as_str() {
                "Vertices" => {
                    if let Some(FbxProperty::F64Array(arr)) = child.properties.first() {
                        vertices = Some(arr.clone());
                    }
                }
                "Edges" => {
                    if let Some(FbxProperty::I32Array(arr)) = child.properties.first() {
                        edges = arr.clone();
                    }
                }
                "PolygonVertexIndex" => {
                    if let Some(FbxProperty::I32Array(arr)) = child.properties.first() {
                        polygon_indices = Some(arr.clone());
                    }
                }
                "LayerElementNormal" => normals_layers.push(child),
                "LayerElementColor" => color_layers.push(child),
                "LayerElementUV" => uv_layers.push(child),
                "LayerElementTangent" => tangent_layers.push(child),
                "LayerElementBinormal" => binormal_layers.push(child),
                "LayerElementSmoothing" => smoothing_layers_raw.push(child),
                "LayerElementEdgeCrease" => crease_layers_raw.push((FbxCreaseKind::Edge, child)),
                "LayerElementVertexCrease" => {
                    crease_layers_raw.push((FbxCreaseKind::Vertex, child))
                }
                "LayerElementMaterial" if material_layer.is_none() => {
                    material_layer = Some(child);
                }
                // Tangents, binormals, smoothing and creases land here. They
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
            .chunks_exact(3)
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
        let material_indices = material_layer
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

        let uv_sets: Vec<FbxUvSet> = uv_layers
            .into_iter()
            .filter_map(|layer| {
                let values = chunk_layer_values(&read_layer_floats(layer, "UV")?);
                Some(layer_set(layer, values, &["UVIndex"]))
            })
            .collect();
        let normal_sets: Vec<FbxNormalSet> = normals_layers
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
        let color_sets: Vec<FbxColorSet> = color_layers
            .into_iter()
            .filter_map(|layer| {
                let raw = read_layer_floats(layer, "Colors")?;
                // FBX writes RGBA here, but a three-component source is legal
                // in the wild; pad it opaque rather than dropping the layer.
                let values = if raw.len() % 4 == 0 {
                    chunk_layer_values(&raw)
                } else {
                    raw.chunks_exact(3)
                        .map(|value| [value[0], value[1], value[2], 1.0])
                        .collect()
                };
                Some(layer_set(layer, values, &["ColorIndex"]))
            })
            .collect();
        for set in &color_sets {
            warn_unsupported_layer_mapping("LayerElementColor", set, warnings);
        }
        let tangent_sets: Vec<FbxTangentSet> = tangent_layers
            .into_iter()
            .filter_map(|layer| parse_tangent_like(layer, "Tangents", "TangentsW", "TangentIndex"))
            .collect();
        let binormal_sets: Vec<FbxBinormalSet> = binormal_layers
            .into_iter()
            .filter_map(|layer| {
                parse_tangent_like(layer, "Binormals", "BinormalsW", "BinormalIndex")
            })
            .collect();
        for set in &tangent_sets {
            warn_unsupported_layer_mapping("LayerElementTangent", &set.layer, warnings);
        }
        for set in &binormal_sets {
            warn_unsupported_layer_mapping("LayerElementBinormal", &set.layer, warnings);
        }

        // Smoothing and crease layers address edges, polygons or control
        // points -- never polygon corners -- so they are kept raw beside
        // `edges` rather than resolved onto the render mesh. A layer whose
        // length disagrees with the domain its mapping names is misaligned
        // data, and keeping it would silently sharpen the wrong edges.
        //
        // `ByEdge` with no `Edges` array is a third case: FBX does not require
        // the array, and the layer then addresses the edges an importer would
        // reconstruct from the faces. This crate does not reconstruct them, so
        // it cannot check the length -- but it also must not destroy the data,
        // since preserving it verbatim is what makes a rewrite lossless.
        let check = |mapping: Option<&str>| -> DomainCheck {
            match mapping {
                Some("ByEdge") if edges.is_empty() => DomainCheck::Unverifiable,
                Some("ByEdge") => DomainCheck::Expect(edges.len()),
                Some("ByPolygon") => DomainCheck::Expect(polygon_count),
                Some("ByVertice") | Some("ByVertex") | Some("ByControlPoint") => {
                    DomainCheck::Expect(control_points.len())
                }
                _ => DomainCheck::Unknown,
            }
        };
        let mut smoothing_layers = Vec::new();
        for layer in smoothing_layers_raw {
            let mapping = layer_string(layer, "MappingInformationType");
            let Some(values) = layer_int_array(layer, "Smoothing") else {
                continue;
            };
            if check(mapping.as_deref()).accepts(values.len()) {
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
        for (kind, layer) in crease_layers_raw {
            let element = match kind {
                FbxCreaseKind::Edge => "LayerElementEdgeCrease",
                FbxCreaseKind::Vertex => "LayerElementVertexCrease",
            };
            let mapping = layer_string(layer, "MappingInformationType");
            let Some(values) = layer_f64_array(layer, element.trim_start_matches("LayerElement"))
            else {
                continue;
            };
            match check(mapping.as_deref()) {
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

        // Build the Draco mesh on the polygon-corner domain. Resolving layer
        // elements onto control points cannot represent a UV or hard-normal
        // seam, and silently averaged them away.
        let layers = FbxMeshLayers {
            uv_sets,
            normal_sets,
            color_sets,
            tangent_sets,
            binormal_sets,
            smoothing_layers,
            crease_layers,
        };
        let render = crate::fbx_render_mesh::expand_to_render_mesh(
            crate::fbx_render_mesh::FbxGeometryLayers::new(
                &control_points,
                &polygon_indices,
                &layers,
            ),
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
        let ktime_f = match fbx_ktime {
            0 => 1.0,
            v => v as f32,
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

        // acnode_id -> { component -> (times, values, flags) }
        let mut acnode_curves: std::collections::HashMap<
            i64,
            std::collections::BTreeMap<u32, FbxAnimCurveData>,
        > = std::collections::HashMap::new();
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
            if let Some(curve) = parse_curve(acurve_map[&conn.child]) {
                acnode_curves
                    .entry(conn.parent)
                    .or_default()
                    .insert(component, curve);
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
            let name = stack_node.and_then(|n| match n.properties.get(1) {
                Some(FbxProperty::String(raw)) => raw
                    .split('\0')
                    .next()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                _ => None,
            });
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
                    let mut by_component: std::collections::BTreeMap<u32, FbxAnimCurveData> =
                        std::collections::BTreeMap::new();
                    for acnode_id in acnode_ids {
                        if let Some(curves) = acnode_curves.get(acnode_id) {
                            for (component, curve) in curves {
                                by_component
                                    .entry(*component)
                                    .or_insert_with(|| curve.clone());
                            }
                        }
                    }
                    let Some(channel) = flatten_curve(&by_component, path, ktime_f) else {
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
                        .and_then(|node| match node.properties.get(1) {
                            Some(FbxProperty::String(raw)) => raw
                                .split('\0')
                                .next()
                                .filter(|part| !part.is_empty())
                                .map(str::to_string),
                            _ => None,
                        })
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

fn parse_curve(node: &FbxNode) -> Option<FbxAnimCurveData> {
    let mut key_times = None;
    let mut key_values = None;
    let mut key_attr_flags = None;
    let mut key_attr_data = None;
    let mut key_attr_ref_count = None;
    for child in &node.children {
        match child.name.as_str() {
            "KeyTime" => {
                if let Some(FbxProperty::I64Array(arr)) = child.properties.first() {
                    key_times = Some(arr.clone());
                }
            }
            "KeyValueFloat" => {
                key_values = child.properties.first().and_then(float_array);
            }
            "KeyAttrFlags" => {
                if let Some(FbxProperty::I32Array(arr)) = child.properties.first() {
                    key_attr_flags = Some(arr.clone());
                }
            }
            "KeyAttrDataFloat" => {
                key_attr_data = child.properties.first().and_then(float_array);
            }
            "KeyAttrRefCount" => {
                if let Some(FbxProperty::I32Array(arr)) = child.properties.first() {
                    key_attr_ref_count = Some(arr.clone());
                }
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
            for ((flag, count), attrs) in flags.into_iter().zip(refs).zip(data.chunks_exact(4)) {
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

/// Combine per-component curves into a single TRS channel sampler.
///
/// Times are taken from the X (component 0) curve when available, then Y, then
/// Z. Missing components default to 0. Interpolation is read from the first
/// `KeyAttrFlags` entry of the chosen time axis.
fn flatten_curve(
    by_component: &std::collections::BTreeMap<u32, FbxAnimCurveData>,
    path: FbxAnimChannelPath,
    ktime_f: f32,
) -> Option<FbxAnimChannel> {
    let time_axis = by_component
        .get(&0)
        .or_else(|| by_component.get(&1))
        .or_else(|| by_component.get(&2))?;
    let n = time_axis.key_times.len();
    let mut input = Vec::with_capacity(n);
    let component_count = path.component_count();
    let mut output = Vec::with_capacity(n * component_count);
    let mut in_tangents = Vec::with_capacity(n * component_count);
    let mut out_tangents = Vec::with_capacity(n * component_count);
    let flags = time_axis.key_attr_flags.first().copied().unwrap_or(0);
    let interpolation = FbxAnimInterpolation::from_key_attr_flags(flags);
    for i in 0..n {
        input.push(time_axis.key_times[i] as f32 / ktime_f);
        for component in 0..component_count as u32 {
            let value = by_component.get(&component).and_then(|curve| {
                if i < curve.key_values.len() {
                    Some(curve.key_values[i])
                } else {
                    None
                }
            });
            output.push(value.unwrap_or(0.0));
            in_tangents.push(
                by_component
                    .get(&component)
                    .and_then(|curve| curve.in_tangents.get(i))
                    .copied()
                    .unwrap_or(0.0),
            );
            out_tangents.push(
                by_component
                    .get(&component)
                    .and_then(|curve| curve.out_tangents.get(i))
                    .copied()
                    .unwrap_or(0.0),
            );
        }
    }
    // FBX stores Euler rotations in degrees; convert to radians so the JS
    // viewer's Euler→quaternion helper matches expectations. Translation and
    // scale are passed through unchanged.
    if path == FbxAnimChannelPath::Rotation {
        for chunk in output.chunks_mut(3) {
            for value in chunk.iter_mut() {
                *value = value.to_radians();
            }
        }
        for chunk in in_tangents.chunks_mut(3) {
            for value in chunk.iter_mut() {
                *value = value.to_radians();
            }
        }
        for chunk in out_tangents.chunks_mut(3) {
            for value in chunk.iter_mut() {
                *value = value.to_radians();
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
            if let Some(FbxProperty::I32Array(arr)) = child.properties.first() {
                return Some(arr.clone());
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
            return match child.properties.first() {
                Some(FbxProperty::F64Array(arr)) => Some(arr.clone()),
                Some(FbxProperty::F32Array(arr)) => {
                    Some(arr.iter().map(|v| f64::from(*v)).collect())
                }
                _ => None,
            };
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
    raw.chunks_exact(N)
        .map(|value| std::array::from_fn(|i| value[i]))
        .collect()
}

/// Reads a layer element's flat float payload, whatever its component count.
///
/// FBX writes these as `f64` arrays; some exporters use `f32`.
fn read_layer_floats(layer: &FbxNode, name: &str) -> Option<Vec<f32>> {
    for child in &layer.children {
        if child.name == name {
            if let Some(FbxProperty::F64Array(arr)) = child.properties.first() {
                return Some(arr.iter().map(|v| *v as f32).collect());
            }
            if let Some(FbxProperty::F32Array(arr)) = child.properties.first() {
                return Some(arr.clone());
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
