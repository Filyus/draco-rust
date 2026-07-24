//! FBX binary format reader for meshes, materials, and TRS animation.
//!
//! Supports reading:
//! - Binary FBX format (versions 7.x)
//! - Vertex positions, normals, and texture coordinates
//! - Polygon/face indices
//! - Model hierarchy and local transforms through [`FbxReader::read_scene`]
//! - Phong/Lambert materials, textures, and per-polygon material indices
//! - Node-TRS animation (`AnimationStack` / `AnimationLayer` /
//!   `AnimationCurveNode` / `AnimationCurve`) flattened into TRS channels
//!
//! FBX pivots and inheritance rules, cameras, lights, and arbitrary metadata
//! are not represented. Skin clusters, bind poses, blend-shape deltas, and
//! local TRS animation are retained in [`crate::FbxScene`].
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

use std::fs::{self, File};
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;

use crate::traits::ReadFromBytes;

#[derive(Debug)]
struct FbxGeometrySource {
    mesh: Mesh,
    material_indices: Vec<i32>,
    control_points: Vec<[f32; 3]>,
    polygon_vertex_indices: Vec<i32>,
    uv_sets: Vec<crate::fbx_scene::FbxUvSet>,
    normal_sets: Vec<crate::fbx_scene::FbxNormalSet>,
}

#[doc(hidden)]
pub use crate::fbx_scene::{
    FbxAnimChannel, FbxAnimChannelPath, FbxAnimInterpolation, FbxAnimSampler, FbxAnimation,
    FbxMeshInstance, FbxNodeId, FbxScene, FbxSceneNode, FbxTexture, FbxTextureBinding,
    FbxTextureSlot, FbxTransform,
};

/// FBX file magic: "Kaydara FBX Binary  \0"
const FBX_MAGIC: &[u8; 21] = b"Kaydara FBX Binary  \0";

/// FBX reader for binary FBX files.
pub struct FbxReader<R: Read + Seek = BufReader<File>> {
    reader: R,
    version: u32,
}

/// FBX reader backed by in-memory bytes.
pub type FbxMemoryReader = FbxReader<Cursor<Vec<u8>>>;

/// An FBX node with properties and children.
#[derive(Debug, Clone)]
pub struct FbxNode {
    /// Node name, such as `Objects`, `Geometry`, `Model`, or `Connections`.
    pub name: String,
    /// Properties stored directly on this node.
    pub properties: Vec<FbxProperty>,
    /// Child nodes nested under this node.
    pub children: Vec<FbxNode>,
}

/// FBX property value.
#[derive(Debug, Clone)]
pub enum FbxProperty {
    /// Boolean property.
    Bool(bool),
    /// 16-bit signed integer property.
    I16(i16),
    /// 32-bit signed integer property.
    I32(i32),
    /// 64-bit signed integer property.
    I64(i64),
    /// 32-bit floating-point property.
    F32(f32),
    /// 64-bit floating-point property.
    F64(f64),
    /// UTF-8-ish string property decoded lossily from FBX bytes.
    String(String),
    /// Raw binary property.
    Raw(Vec<u8>),
    /// Boolean array property.
    BoolArray(Vec<bool>),
    /// 32-bit signed integer array property.
    I32Array(Vec<i32>),
    /// 64-bit signed integer array property.
    I64Array(Vec<i64>),
    /// 32-bit floating-point array property.
    F32Array(Vec<f32>),
    /// 64-bit floating-point array property.
    F64Array(Vec<f64>),
}

impl FbxReader<BufReader<File>> {
    /// Open an FBX file from a path.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        Self::new(reader)
    }
}

impl FbxReader<Cursor<Vec<u8>>> {
    /// Create an FBX reader from in-memory bytes.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> io::Result<Self> {
        Self::new(Cursor::new(bytes.into()))
    }

    /// Read all meshes directly from in-memory bytes.
    pub fn read_from_bytes(bytes: &[u8]) -> io::Result<Vec<Mesh>> {
        let mut reader = Self::from_bytes(bytes.to_vec())?;
        reader.read_meshes()
    }
}

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

        // Build object id -> node maps for every object type we care about.
        use std::collections::HashMap;
        let mut model_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut geometry_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut material_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut texture_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut video_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut astack_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut alayer_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut acnode_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut acurve_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut deformer_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut pose_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut saw_deformer = false;
        let mut saw_blend_shape = false;
        let mut connections: Vec<FbxConnection> = Vec::new();

        for n in &nodes {
            if n.name == "Objects" {
                for child in &n.children {
                    let Some(FbxProperty::I64(id)) = child.properties.first() else {
                        continue;
                    };
                    match child.name.as_str() {
                        "Model" => {
                            model_map.insert(*id, child);
                        }
                        "Geometry" => {
                            geometry_map.insert(*id, child);
                        }
                        "Material" => {
                            material_map.insert(*id, child);
                        }
                        "Texture" => {
                            texture_map.insert(*id, child);
                        }
                        "Video" => {
                            video_map.insert(*id, child);
                        }
                        "AnimationStack" => {
                            astack_map.insert(*id, child);
                        }
                        "AnimationLayer" => {
                            alayer_map.insert(*id, child);
                        }
                        "AnimationCurveNode" => {
                            acnode_map.insert(*id, child);
                        }
                        "AnimationCurve" => {
                            acurve_map.insert(*id, child);
                        }
                        "Pose" => {
                            pose_map.insert(*id, child);
                        }
                        "Deformer" => {
                            deformer_map.insert(*id, child);
                            // Deformers are either skin clusters or blend shapes.
                            // Inspect the subclass on the name property.
                            if object_name_subclass(child)
                                .map(|s| s == "BlendShape" || s == "BlendShapeChannel")
                                .unwrap_or(true)
                            {
                                saw_blend_shape = true;
                            } else {
                                saw_deformer = true;
                            }
                        }
                        "BlendShape" | "BlendShapeChannel" => {
                            saw_blend_shape = true;
                        }
                        _ => {}
                    }
                }
            } else if n.name == "Connections" {
                for c in &n.children {
                    let kind = match c.properties.first() {
                        Some(FbxProperty::String(s)) if s == "OO" => ConnectionKind::Oo,
                        Some(FbxProperty::String(s)) if s == "OP" => ConnectionKind::Op,
                        _ => continue,
                    };
                    let Some(FbxProperty::I64(child)) = c.properties.get(1) else {
                        continue;
                    };
                    let Some(FbxProperty::I64(parent)) = c.properties.get(2) else {
                        continue;
                    };
                    let property = match c.properties.get(3) {
                        Some(FbxProperty::String(s)) => Some(s.clone()),
                        _ => None,
                    };
                    connections.push(FbxConnection {
                        kind,
                        child: *child,
                        parent: *parent,
                        property,
                    });
                }
            }
        }

        // Detect skins more reliably: any Cluster SubDeformer counts as skin.
        if !saw_deformer {
            for n in &nodes {
                if n.name == "Objects" {
                    for child in &n.children {
                        if child.name == "Deformer"
                            && object_name_subclass(child)
                                .map(|s| s == "Cluster")
                                .unwrap_or(false)
                        {
                            saw_deformer = true;
                            break;
                        }
                    }
                }
            }
        }

        let mut warnings = Vec::new();
        let mut warned_inherit_type = false;
        for model in model_map.values() {
            for property in model
                .children
                .iter()
                .filter(|child| child.name == "Properties70")
            {
                for entry in &property.children {
                    let Some(FbxProperty::String(name)) = entry.properties.first() else {
                        continue;
                    };
                    if name != "InheritType" || warned_inherit_type {
                        continue;
                    }
                    let inherit_type = entry.properties.iter().find_map(|value| match value {
                        FbxProperty::I32(value) => Some(*value),
                        FbxProperty::I64(value) => i32::try_from(*value).ok(),
                        _ => None,
                    });
                    let local_scale = model
                        .children
                        .iter()
                        .find(|child| child.name == "Properties70")
                        .and_then(|properties| {
                            properties.children.iter().find_map(|property| {
                                let Some(FbxProperty::String(name)) = property.properties.first()
                                else {
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
                                (values.len() == 3).then_some([values[0], values[1], values[2]])
                            })
                        });
                    let uniform_scale = local_scale
                        .map(|scale| {
                            (scale[0] - scale[1]).abs() <= 1e-5
                                && (scale[1] - scale[2]).abs() <= 1e-5
                        })
                        .unwrap_or(true);
                    let known_type = matches!(inherit_type, Some(0..=2));
                    if !(known_type && uniform_scale) {
                        warnings.push(format!(
                            "FBX model uses unsupported {name}; local TRS was imported without that FBX transform rule"
                        ));
                        warned_inherit_type = true;
                    }
                }
            }
        }
        let _ = (saw_deformer, saw_blend_shape);

        // ---- Materials ----------------------------------------------------
        let mut materials: Vec<crate::fbx_scene::FbxMaterial> = Vec::new();
        let mut material_index_by_id: HashMap<i64, usize> = HashMap::new();
        let mut material_ids: Vec<i64> = material_map.keys().copied().collect();
        material_ids.sort_unstable();
        for id in material_ids {
            let node = material_map[&id];
            let mut material = parse_material(node);
            material.textures = collect_material_texture_bindings(id, &texture_map, &connections);
            material_index_by_id.insert(id, materials.len());
            materials.push(material);
        }

        // ---- Textures -----------------------------------------------------
        let mut textures: Vec<crate::fbx_scene::FbxTexture> = Vec::new();
        let mut texture_index_by_id: HashMap<i64, usize> = HashMap::new();
        // Build texture -> video connection map first so each Texture knows
        // where to pull its embedded bytes from.
        let mut texture_video: HashMap<i64, i64> = HashMap::new();
        for conn in &connections {
            if conn.kind == ConnectionKind::Oo
                && texture_map.contains_key(&conn.child)
                && video_map.contains_key(&conn.parent)
            {
                texture_video.entry(conn.child).or_insert(conn.parent);
            }
            // Video -> Texture (media -> clip) is also common.
            if conn.kind == ConnectionKind::Oo
                && video_map.contains_key(&conn.child)
                && texture_map.contains_key(&conn.parent)
            {
                texture_video.entry(conn.parent).or_insert(conn.child);
            }
        }
        let mut texture_ids: Vec<i64> = texture_map.keys().copied().collect();
        texture_ids.sort_unstable();
        for id in texture_ids {
            let node = texture_map[&id];
            let mut texture = parse_texture(node);
            if let Some(&video_id) = texture_video.get(&id) {
                if let Some(video) = video_map.get(&video_id) {
                    let v = parse_texture(video);
                    if texture.content.is_none() {
                        texture.content = v.content;
                    }
                    if texture.filename.is_none() {
                        texture.filename = v.filename;
                    }
                    if texture.name.is_none() {
                        texture.name = v.name;
                    }
                }
            }
            texture_index_by_id.insert(id, textures.len());
            textures.push(texture);
        }

        // Remap material texture bindings from FBX texture ids to scene indices.
        for material in &mut materials {
            for binding in &mut material.textures {
                let fbx_id = binding.texture_index as i64;
                if let Some(&resolved) = texture_index_by_id.get(&fbx_id) {
                    binding.texture_index = resolved;
                }
            }
        }

        // ---- Model hierarchy + per-model materials -----------------------
        // Map each model id to the list of material indices connected to it.
        let mut model_material_ids: HashMap<i64, Vec<i32>> = HashMap::new();
        for conn in &connections {
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
        fn parse_transform(node: &FbxNode) -> Option<FbxTransform> {
            let mut translation = None;
            let mut rotation = None;
            let mut scaling = None;
            let mut pre_rotation = None;
            let mut post_rotation = None;
            let mut rotation_offset = None;
            let mut rotation_pivot = None;
            let mut scaling_offset = None;
            let mut scaling_pivot = None;

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
                    [
                        cos_z * cos_y,
                        sin_z * cos_y,
                        -sin_y,
                        0.0,
                    ],
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

            Some(FbxTransform { matrix: mat })
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

        let mut ordered_model_ids: Vec<i64> = model_map.keys().copied().collect();
        ordered_model_ids.sort_unstable();
        let model_node_ids: HashMap<i64, FbxNodeId> = ordered_model_ids
            .into_iter()
            .enumerate()
            .map(|(index, id)| (id, FbxNodeId((index + 1) as u32)))
            .collect();

        fn build_model_node(
            id: i64,
            model_map: &std::collections::HashMap<i64, &FbxNode>,
            model_children: &std::collections::HashMap<i64, Vec<i64>>,
            model_mesh_instances: &std::collections::HashMap<i64, Vec<FbxMeshInstance>>,
            model_node_ids: &std::collections::HashMap<i64, FbxNodeId>,
        ) -> FbxSceneNode {
            let node_src = model_map.get(&id).unwrap();
            let mut node = FbxSceneNode::new(object_name(node_src));
            node.id = model_node_ids[&id];
            node.transform = parse_transform(node_src);
            if let Some(mesh_instances) = model_mesh_instances.get(&id) {
                node.mesh_instances.extend(mesh_instances.clone());
            }

            if let Some(children) = model_children.get(&id) {
                for &cid in children {
                    if model_map.contains_key(&cid) {
                        node.children.push(build_model_node(
                            cid,
                            model_map,
                            model_children,
                            model_mesh_instances,
                            model_node_ids,
                        ));
                    }
                }
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
            if let Some(source) = self.geometry_to_mesh(geom_node)? {
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
                            uv_sets: source.uv_sets.clone(),
                            normal_sets: source.normal_sets.clone(),
                            material_indices: indices,
                            skin: parse_skin_for_geometry(
                                geom_id,
                                &deformer_map,
                                &pose_map,
                                &connections,
                                &model_node_ids,
                            ),
                            morph_targets: parse_morph_targets_for_geometry(
                                geom_id,
                                &geometry_map,
                                &deformer_map,
                                &connections,
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
            &connections,
            &astack_map,
            &alayer_map,
            &acnode_map,
            &acurve_map,
            &model_map,
            &model_name_map,
            &model_node_ids,
            &morph_animation_targets(&geometry_map, &deformer_map, &connections, &model_map),
        );

        // Build root nodes: any model with parent 0 (or with no parent present)
        let mut root_nodes = Vec::new();
        // find top-level model ids
        let top_level: Vec<i64> = model_map
            .keys()
            .cloned()
            .filter(|id| {
                !connections
                    .iter()
                    .any(|conn| conn.child == *id && model_map.contains_key(&conn.parent))
            })
            .collect();

        for id in top_level {
            root_nodes.push(build_model_node(
                id,
                &model_map,
                &model_children,
                &model_mesh_instances,
                &model_node_ids,
            ));
        }

        Ok(FbxScene {
            root_nodes,
            materials,
            textures,
            animations,
            warnings,
        })
    }
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
    for pose in poses.values() {
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

/// A parsed FBX connection entry.
#[derive(Debug, Clone)]
struct FbxConnection {
    kind: ConnectionKind,
    child: i64,
    parent: i64,
    property: Option<String>,
}

/// Reads the object subclass from a `\0\x01`-separated FBX name property.
fn object_name_subclass(node: &FbxNode) -> Option<String> {
    match node.properties.get(1) {
        Some(FbxProperty::String(name)) => name.split('\u{1}').nth(1).map(str::to_string),
        _ => None,
    }
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
            "RelativeFilename" | "FileName" | "Filename" => {
                if filename.is_none() {
                    if let Some(FbxProperty::String(s)) = child.properties.first() {
                        if !s.is_empty() {
                            filename = Some(s.clone());
                        }
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

impl ReadFromBytes for FbxReader<Cursor<Vec<u8>>> {
    fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        Self::from_bytes(bytes.to_vec())
    }
}

impl<R: Read + Seek> FbxReader<R> {
    /// Create a new FBX reader from a reader.
    pub fn new(mut reader: R) -> io::Result<Self> {
        // Read and verify magic
        let mut magic = [0u8; 21];
        reader.read_exact(&mut magic)?;
        if &magic != FBX_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a valid binary FBX file",
            ));
        }

        // Skip 2 unknown bytes
        reader.seek(SeekFrom::Current(2))?;

        // Read version
        let mut version_bytes = [0u8; 4];
        reader.read_exact(&mut version_bytes)?;
        let version = u32::from_le_bytes(version_bytes);

        Ok(Self { reader, version })
    }

    /// Get the FBX file version.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Check if this is FBX 7.5+ (uses 64-bit offsets).
    fn is_64bit(&self) -> bool {
        self.version >= 7500
    }

    /// Read a node record.
    fn read_node(&mut self) -> io::Result<Option<FbxNode>> {
        let (end_offset, num_properties, _property_list_len, name_len) = if self.is_64bit() {
            let mut buf = [0u8; 25];
            self.reader.read_exact(&mut buf)?;
            let end_offset = u64::from_le_bytes([
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            ]);
            let num_properties = u64::from_le_bytes([
                buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
            ]);
            let property_list_len = u64::from_le_bytes([
                buf[16], buf[17], buf[18], buf[19], buf[20], buf[21], buf[22], buf[23],
            ]);
            let name_len = buf[24];
            (
                end_offset,
                num_properties as u32,
                property_list_len,
                name_len,
            )
        } else {
            let mut buf = [0u8; 13];
            self.reader.read_exact(&mut buf)?;
            let end_offset = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as u64;
            let num_properties = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
            let _property_list_len = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as u64;
            let name_len = buf[12];
            (end_offset, num_properties, _property_list_len, name_len)
        };

        // NULL record marks end of children
        if end_offset == 0 {
            return Ok(None);
        }

        // Read name
        let mut name_bytes = vec![0u8; name_len as usize];
        self.reader.read_exact(&mut name_bytes)?;
        let name = String::from_utf8_lossy(&name_bytes).to_string();

        // Read properties
        let mut properties = Vec::with_capacity(num_properties as usize);
        for _ in 0..num_properties {
            properties.push(self.read_property()?);
        }

        // Read children
        let mut children = Vec::new();
        let current_pos = self.reader.stream_position()?;
        if current_pos < end_offset {
            while let Some(child) = self.read_node()? {
                children.push(child);
            }
        }

        // Seek to end offset to be safe
        self.reader.seek(SeekFrom::Start(end_offset))?;

        Ok(Some(FbxNode {
            name,
            properties,
            children,
        }))
    }

    /// Read a property.
    fn read_property(&mut self) -> io::Result<FbxProperty> {
        let mut type_code = [0u8; 1];
        self.reader.read_exact(&mut type_code)?;

        match type_code[0] {
            b'C' => {
                let mut v = [0u8; 1];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::Bool(v[0] != 0))
            }
            b'Y' => {
                let mut v = [0u8; 2];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::I16(i16::from_le_bytes(v)))
            }
            b'I' => {
                let mut v = [0u8; 4];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::I32(i32::from_le_bytes(v)))
            }
            b'L' => {
                let mut v = [0u8; 8];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::I64(i64::from_le_bytes(v)))
            }
            b'F' => {
                let mut v = [0u8; 4];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::F32(f32::from_le_bytes(v)))
            }
            b'D' => {
                let mut v = [0u8; 8];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::F64(f64::from_le_bytes(v)))
            }
            b'S' | b'R' => {
                let mut len_bytes = [0u8; 4];
                self.reader.read_exact(&mut len_bytes)?;
                let len = u32::from_le_bytes(len_bytes) as usize;
                let mut data = vec![0u8; len];
                self.reader.read_exact(&mut data)?;
                if type_code[0] == b'S' {
                    Ok(FbxProperty::String(
                        String::from_utf8_lossy(&data).to_string(),
                    ))
                } else {
                    Ok(FbxProperty::Raw(data))
                }
            }
            b'b' => Ok(FbxProperty::BoolArray(self.read_array_bool()?)),
            b'i' => Ok(FbxProperty::I32Array(self.read_array_i32()?)),
            b'l' => Ok(FbxProperty::I64Array(self.read_array_i64()?)),
            b'f' => Ok(FbxProperty::F32Array(self.read_array_f32()?)),
            b'd' => Ok(FbxProperty::F64Array(self.read_array_f64()?)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unknown property type: {}", type_code[0] as char),
            )),
        }
    }

    /// Read array header and return (length, encoding, compressed_length).
    fn read_array_header(&mut self) -> io::Result<(u32, u32, u32)> {
        let mut buf = [0u8; 12];
        self.reader.read_exact(&mut buf)?;
        let array_len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let encoding = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let compressed_len = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        Ok((array_len, encoding, compressed_len))
    }

    /// Read array data (handles compression).
    fn read_array_data(
        &mut self,
        encoding: u32,
        compressed_len: u32,
        uncompressed_size: usize,
    ) -> io::Result<Vec<u8>> {
        if encoding == 0 {
            let mut data = vec![0u8; uncompressed_size];
            self.reader.read_exact(&mut data)?;
            Ok(data)
        } else if encoding == 1 {
            // Deflate/zlib compressed
            let mut compressed = vec![0u8; compressed_len as usize];
            self.reader.read_exact(&mut compressed)?;

            #[cfg(feature = "compression")]
            {
                use miniz_oxide::inflate::decompress_to_vec_zlib;
                decompress_to_vec_zlib(&compressed).map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Decompression error: {:?}", e),
                    )
                })
            }

            #[cfg(not(feature = "compression"))]
            {
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "FBX array compression not supported (enable 'compression' feature)",
                ))
            }
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unknown array encoding: {}", encoding),
            ))
        }
    }

    fn read_array_bool(&mut self) -> io::Result<Vec<bool>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(encoding, compressed_len, len as usize)?;
        Ok(data.into_iter().map(|b| b != 0).collect())
    }

    fn read_array_i32(&mut self) -> io::Result<Vec<i32>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(encoding, compressed_len, len as usize * 4)?;
        Ok(data
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }

    fn read_array_i64(&mut self) -> io::Result<Vec<i64>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(encoding, compressed_len, len as usize * 8)?;
        Ok(data
            .chunks_exact(8)
            .map(|c| i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect())
    }

    fn read_array_f32(&mut self) -> io::Result<Vec<f32>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(encoding, compressed_len, len as usize * 4)?;
        Ok(data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }

    fn read_array_f64(&mut self) -> io::Result<Vec<f64>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(encoding, compressed_len, len as usize * 8)?;
        Ok(data
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect())
    }

    /// Read all top-level nodes.
    pub fn read_nodes(&mut self) -> io::Result<Vec<FbxNode>> {
        // Seek to start of nodes (after header)
        self.reader.seek(SeekFrom::Start(27))?;

        let mut nodes = Vec::new();
        while let Some(node) = self.read_node()? {
            nodes.push(node);
        }
        Ok(nodes)
    }

    /// Read meshes from the FBX file.
    pub fn read_meshes(&mut self) -> io::Result<Vec<Mesh>> {
        let nodes = self.read_nodes()?;
        let mut meshes = Vec::new();

        // Find Objects node
        for node in &nodes {
            if node.name == "Objects" {
                for child in &node.children {
                    if child.name == "Geometry" {
                        if let Some(source) = self.geometry_to_mesh(child)? {
                            meshes.push(source.mesh);
                        }
                    }
                }
            }
        }

        Ok(meshes)
    }

    /// Convert a Geometry node to a Mesh, plus per-triangle material indices.
    ///
    /// The returned `material_indices` align with the fan-triangulated face
    /// order of the Draco `Mesh` (one entry per triangle). The list is empty
    /// when the geometry does not carry a `LayerElementMaterial` layer.
    fn geometry_to_mesh(&self, geometry: &FbxNode) -> io::Result<Option<FbxGeometrySource>> {
        let mut vertices: Option<Vec<f64>> = None;
        let mut polygon_indices: Option<Vec<i32>> = None;
        let mut normals_layers: Vec<&FbxNode> = Vec::new();
        let mut uv_layers: Vec<&FbxNode> = Vec::new();
        let mut material_layer: Option<&FbxNode> = None;

        for child in &geometry.children {
            match child.name.as_str() {
                "Vertices" => {
                    if let Some(FbxProperty::F64Array(arr)) = child.properties.first() {
                        vertices = Some(arr.clone());
                    }
                }
                "PolygonVertexIndex" => {
                    if let Some(FbxProperty::I32Array(arr)) = child.properties.first() {
                        polygon_indices = Some(arr.clone());
                    }
                }
                "LayerElementNormal" => normals_layers.push(child),
                "LayerElementUV" => uv_layers.push(child),
                "LayerElementMaterial" if material_layer.is_none() => {
                    material_layer = Some(child);
                }
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

        // Build mesh
        let mut mesh = Mesh::new();

        // Add positions
        let num_vertices = vertices.len() / 3;
        let mut pos_att = PointAttribute::new();
        pos_att.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            num_vertices,
        );
        let buffer = pos_att.buffer_mut();
        for i in 0..num_vertices {
            let x = vertices[i * 3] as f32;
            let y = vertices[i * 3 + 1] as f32;
            let z = vertices[i * 3 + 2] as f32;
            let bytes: Vec<u8> = [x, y, z].iter().flat_map(|v| v.to_le_bytes()).collect();
            buffer.write(i * 12, &bytes);
        }
        mesh.add_attribute(pos_att);

        // Parse polygon indices (FBX uses negative index to mark end of polygon)
        let mut faces: Vec<[u32; 3]> = Vec::new();
        let mut current_polygon: Vec<i32> = Vec::new();
        // Track the original polygon index per generated triangle so we can
        // remap ByPolygon material indices to fan-triangulated face order.
        let mut tri_polygon_index: Vec<usize> = Vec::new();
        let mut polygon_count = 0usize;

        for &idx in &polygon_indices {
            if idx < 0 {
                // End of polygon (index is bitwise NOT of actual index)
                let actual_idx = !idx;
                current_polygon.push(actual_idx);

                // Triangulate polygon (simple fan triangulation)
                if current_polygon.len() >= 3 {
                    let v0 = current_polygon[0] as u32;
                    for i in 1..current_polygon.len() - 1 {
                        let v1 = current_polygon[i] as u32;
                        let v2 = current_polygon[i + 1] as u32;
                        faces.push([v0, v1, v2]);
                        tri_polygon_index.push(polygon_count);
                    }
                }
                current_polygon.clear();
                polygon_count += 1;
            } else {
                current_polygon.push(idx);
            }
        }

        // Set faces
        mesh.set_num_faces(faces.len());
        for (i, face) in faces.iter().enumerate() {
            mesh.set_face(
                FaceIndex(i as u32),
                [
                    PointIndex(face[0]),
                    PointIndex(face[1]),
                    PointIndex(face[2]),
                ],
            );
        }

        // Match C++ Draco behavior: deduplicate point IDs in face-traversal order.
        // This ensures binary compatibility when encoding.
        mesh.deduplicate_point_ids();

        // Attach normals (ByVertice / Direct, the same convention the FBX
        // writer emits). FBX stores one normal per control point under
        // `ByVertice`, or per polygon-vertex under `ByPolygonVertex`; we
        // expand both to per-point storage so Draco can compress them.
        if let Some(layer) = normals_layers.first().copied() {
            if let Some(values) = read_layer_float3(layer, "Normals") {
                add_per_point_attribute(
                    &mut mesh,
                    GeometryAttributeType::Normal,
                    num_vertices,
                    polygon_indices.as_ref(),
                    layer,
                    &values,
                );
            }
        }
        if let Some(layer) = uv_layers.first().copied() {
            if let Some(values) = read_layer_float2(layer, "UV") {
                add_per_point_attribute_2(
                    &mut mesh,
                    GeometryAttributeType::TexCoord,
                    num_vertices,
                    polygon_indices.as_ref(),
                    layer,
                    &values,
                );
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

        let uv_sets = uv_layers
            .into_iter()
            .filter_map(|layer| {
                let values = read_layer_float2(layer, "UV")?
                    .chunks_exact(2)
                    .map(|value| [value[0], value[1]])
                    .collect();
                Some(crate::fbx_scene::FbxUvSet {
                    name: layer_string(layer, "Name"),
                    mapping: layer_string(layer, "MappingInformationType"),
                    reference: layer_string(layer, "ReferenceInformationType"),
                    values,
                    indices: layer_int_array(layer, "UVIndex").unwrap_or_default(),
                })
            })
            .collect();
        let normal_sets = normals_layers
            .into_iter()
            .filter_map(|layer| {
                let values = read_layer_float3(layer, "Normals")?
                    .chunks_exact(3)
                    .map(|value| [value[0], value[1], value[2]])
                    .collect();
                Some(crate::fbx_scene::FbxNormalSet {
                    name: layer_string(layer, "Name"),
                    mapping: layer_string(layer, "MappingInformationType"),
                    reference: layer_string(layer, "ReferenceInformationType"),
                    values,
                    indices: layer_int_array(layer, "NormalsIndex")
                        .or_else(|| layer_int_array(layer, "NormalIndex"))
                        .unwrap_or_default(),
                })
            })
            .collect();

        Ok(Some(FbxGeometrySource {
            mesh,
            material_indices,
            control_points,
            polygon_vertex_indices: polygon_indices,
            uv_sets,
            normal_sets,
        }))
    }

    /// Flatten the FBX animation graph into one [`FbxAnimation`] per
    /// `AnimationStack` + first connected `AnimationLayer`.
    fn parse_animations(
        &self,
        nodes: &[FbxNode],
        connections: &[FbxConnection],
        astack_map: &std::collections::HashMap<i64, &FbxNode>,
        alayer_map: &std::collections::HashMap<i64, &FbxNode>,
        acnode_map: &std::collections::HashMap<i64, &FbxNode>,
        acurve_map: &std::collections::HashMap<i64, &FbxNode>,
        model_map: &std::collections::HashMap<i64, &FbxNode>,
        model_name_map: &std::collections::HashMap<i64, String>,
        model_node_ids: &std::collections::HashMap<i64, FbxNodeId>,
        morph_targets: &std::collections::HashMap<i64, (i64, u32)>,
    ) -> Vec<FbxAnimation> {
        let fbx_ktime = fbx_ktime_for(nodes, self.version);
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
        let mut stacks_layers: std::collections::HashMap<
            i64,
            std::collections::HashMap<i64, Vec<(i64, i64, FbxAnimChannelPath, Option<u32>)>>,
        > = std::collections::HashMap::new();
        for (acnode_id, (layer_id, model_id, path, morph_target_index)) in &acnode_targets {
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
        for (stack_id, layers) in stacks_layers {
            let stack_node = astack_map.get(&stack_id);
            let name = stack_node.and_then(|n| match n.properties.get(1) {
                Some(FbxProperty::String(raw)) => raw
                    .split('\0')
                    .next()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                _ => None,
            });
            // Flatten all layers into one animation. Blender does the same:
            // each layer produces an independent action, but most files have a
            // single layer per stack.
            let mut channels = Vec::new();
            let mut max_time = 0.0f32;
            for (_layer_id, entries) in layers {
                // Group curve nodes by (model, path) before flattening.
                let mut groups: std::collections::HashMap<
                    (i64, FbxAnimChannelPath, Option<u32>),
                    Vec<i64>,
                > = std::collections::HashMap::new();
                for (acnode_id, model_id, path, morph_target_index) in entries {
                    groups
                        .entry((model_id, path, morph_target_index))
                        .or_default()
                        .push(acnode_id);
                }
                for ((model_id, path, morph_target_index), acnode_ids) in groups {
                    // Combine the X/Y/Z curves across all matching curve nodes
                    // (Blender notes that each curve node has a unique set of
                    // channels, so in practice there is exactly one entry).
                    let mut by_component: std::collections::BTreeMap<u32, FbxAnimCurveData> =
                        std::collections::BTreeMap::new();
                    for acnode_id in acnode_ids {
                        if let Some(curves) = acnode_curves.get(&acnode_id) {
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
            }
            if channels.is_empty() {
                continue;
            }
            animations.push(FbxAnimation {
                name,
                duration: max_time,
                channels,
            });
        }
        animations
    }
}

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
                if let Some(FbxProperty::F32Array(arr)) = child.properties.first() {
                    key_values = Some(arr.clone());
                }
            }
            "KeyAttrFlags" => {
                if let Some(FbxProperty::I32Array(arr)) = child.properties.first() {
                    key_attr_flags = Some(arr.clone());
                }
            }
            "KeyAttrDataFloat" => {
                if let Some(FbxProperty::F32Array(arr)) = child.properties.first() {
                    key_attr_data = Some(arr.clone());
                }
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

fn read_layer_float3(layer: &FbxNode, name: &str) -> Option<Vec<f32>> {
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

fn read_layer_float2(layer: &FbxNode, name: &str) -> Option<Vec<f32>> {
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

fn add_per_point_attribute(
    mesh: &mut Mesh,
    attribute_type: GeometryAttributeType,
    num_points: usize,
    polygon_indices: &[i32],
    layer: &FbxNode,
    values: &[f32],
) {
    let mapping = layer_string(layer, "MappingInformationType").unwrap_or_default();
    let reference = layer_string(layer, "ReferenceInformationType").unwrap_or_default();
    let index = if reference == "IndexToDirect" {
        layer_int_array(layer, "NormalsIndex").or_else(|| layer_int_array(layer, "NormalIndex"))
    } else {
        None
    };
    let stride = 3;
    let resolved = resolve_layer_values(
        &mapping,
        &reference,
        index.as_deref(),
        values,
        stride,
        num_points,
        polygon_indices,
    );
    let count = resolved.len() / stride;
    let mut att = PointAttribute::new();
    att.init(
        attribute_type,
        3,
        DataType::Float32,
        false,
        count.max(num_points),
    );
    let buffer = att.buffer_mut();
    for i in 0..count {
        let bytes: Vec<u8> = (0..stride)
            .flat_map(|c| resolved[i * stride + c].to_le_bytes())
            .collect();
        buffer.write(i * 12, &bytes);
    }
    mesh.add_attribute(att);
}

fn add_per_point_attribute_2(
    mesh: &mut Mesh,
    attribute_type: GeometryAttributeType,
    num_points: usize,
    polygon_indices: &[i32],
    layer: &FbxNode,
    values: &[f32],
) {
    let mapping = layer_string(layer, "MappingInformationType").unwrap_or_default();
    let reference = layer_string(layer, "ReferenceInformationType").unwrap_or_default();
    let index = if reference == "IndexToDirect" {
        layer_int_array(layer, "UVIndex")
    } else {
        None
    };
    let stride = 2;
    let resolved = resolve_layer_values(
        &mapping,
        &reference,
        index.as_deref(),
        values,
        stride,
        num_points,
        polygon_indices,
    );
    let count = resolved.len() / stride;
    let mut att = PointAttribute::new();
    att.init(
        attribute_type,
        2,
        DataType::Float32,
        false,
        count.max(num_points),
    );
    let buffer = att.buffer_mut();
    for i in 0..count {
        let bytes: Vec<u8> = (0..stride)
            .flat_map(|c| resolved[i * stride + c].to_le_bytes())
            .collect();
        buffer.write(i * 8, &bytes);
    }
    mesh.add_attribute(att);
}

/// Resolve a `LayerElement*` data array to per-point values.
///
/// FBX supports `ByVertice`/`ByPolygonVertex`/`AllSame` mapping and
/// `Direct`/`IndexToDirect` reference modes. We always emit one value per
/// control point so the writer can re-serialize under `ByVertice`/`Direct`,
/// matching the existing FBX writer convention.
fn resolve_layer_values(
    mapping: &str,
    reference: &str,
    index: Option<&[i32]>,
    values: &[f32],
    stride: usize,
    num_points: usize,
    polygon_indices: &[i32],
) -> Vec<f32> {
    // Helper: pick the value tuple at a logical position.
    let tuple_at = |pos: usize| -> Vec<f32> {
        let resolved_pos = if reference == "IndexToDirect" {
            index
                .and_then(|idx| idx.get(pos).map(|v| *v as usize))
                .unwrap_or(pos)
        } else {
            pos
        };
        let start = resolved_pos * stride;
        if start + stride <= values.len() {
            values[start..start + stride].to_vec()
        } else {
            vec![0.0; stride]
        }
    };

    match mapping {
        "ByVertice" | "ByVertex" | "ByVerticeOrPolygon" => {
            (0..num_points).flat_map(tuple_at).collect()
        }
        "AllSame" => {
            let first = tuple_at(0);
            (0..num_points)
                .flat_map(|_| first.iter().copied())
                .collect()
        }
        "ByPolygonVertex" => {
            // Spread per-polygon-vertex values back to control points. We
            // pick the last value seen per control point (common DCC behavior
            // for hard normals).
            let mut per_point = vec![0f32; num_points * stride];
            let mut vertex_pos = 0usize;
            for &idx in polygon_indices {
                let actual = if idx < 0 { !idx } else { idx };
                if let Some(point) =
                    per_point.get_mut(actual as usize * stride..actual as usize * stride + stride)
                {
                    let tuple = tuple_at(vertex_pos);
                    for c in 0..stride.min(point.len()) {
                        point[c] = tuple[c];
                    }
                }
                vertex_pos += 1;
            }
            per_point
        }
        _ => (0..num_points).flat_map(tuple_at).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_fbx_magic() {
        let mut data = Vec::new();
        data.extend_from_slice(FBX_MAGIC);
        data.extend_from_slice(&[0x1A, 0x00]); // Unknown bytes
        data.extend_from_slice(&7300u32.to_le_bytes()); // Version 7.3
                                                        // Add null record to end nodes
        data.extend_from_slice(&[0u8; 13]);

        let cursor = Cursor::new(data);
        let reader = FbxReader::new(cursor).unwrap();
        assert_eq!(reader.version(), 7300);
    }

    #[test]
    fn test_invalid_magic() {
        let data = b"Not an FBX file at all";
        let cursor = Cursor::new(data.to_vec());
        assert!(FbxReader::new(cursor).is_err());
    }

    #[test]
    fn memory_reader_reads_an_empty_scene() {
        let mut data = Vec::new();
        data.extend_from_slice(FBX_MAGIC);
        data.extend_from_slice(&[0x1A, 0x00]);
        data.extend_from_slice(&7300u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 13]);

        let mut reader = FbxReader::new(Cursor::new(data)).unwrap();
        let scene = reader.read_scene().unwrap();
        assert!(scene.root_nodes.is_empty());
    }
}
