//! FBX reader and writer WASM module.
//!
//! Provides FBX binary parsing (FBX 7.x) and generation (FBX 7.5) for web
//! applications. The reader and writer are independent: build with
//! `--features read` or `--features write` (both are on by default) to control
//! which half of the API is exported.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// FBX file magic: "Kaydara FBX Binary  \0". Shared by reader and writer.
const FBX_MAGIC: &[u8; 21] = b"Kaydara FBX Binary  \0";

/// Initialize panic hook for better error messages in browser console.
#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Get the version of this WASM module.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Get supported file extensions.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["fbx".to_string()]
}

// ===========================================================================
// Reader
// ===========================================================================

#[cfg(any(feature = "read", feature = "write"))]
use draco_core::draco_types::DataType;
#[cfg(any(feature = "read", feature = "write"))]
use draco_core::geometry_attribute::GeometryAttributeType;
#[cfg(any(feature = "read", feature = "write"))]
use draco_core::geometry_indices::FaceIndex;
#[cfg(any(feature = "read", feature = "write"))]
use draco_core::mesh::Mesh;
#[cfg(any(feature = "read", feature = "write"))]
use draco_io::{FbxNodeId, FbxScene, FbxSceneNode};

/// Mesh data produced by the FBX reader, for JavaScript interop.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MeshData {
    /// Mesh name
    pub name: Option<String>,
    /// Vertex positions as flat array [x0, y0, z0, x1, y1, z1, ...]
    pub positions: Vec<f32>,
    /// Face indices as flat array (triangles)
    pub indices: Vec<u32>,
    /// Vertex normals (if present)
    pub normals: Vec<f32>,
    /// Texture coordinates (if present)
    pub uvs: Vec<f32>,
    /// Per-triangle indices into the scene material list.
    ///
    /// This retains FBX `LayerElementMaterial` assignments for a later
    /// hierarchy-preserving export. The first value is also exposed through
    /// `material` for the preview's single-material primitive path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub material_indices: Vec<i32>,
    /// Index of the first material applied to this mesh, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<usize>,
    /// Full FBX skin clusters, without a GPU influence limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<SkinOutput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub morph_targets: Vec<MorphTargetOutput>,
    /// First four influences per point for the WebGL preview. `skin` retains
    /// every FBX influence for a later export.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joints0: Vec<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weights0: Vec<f32>,
    /// Original FBX control points, retained for scene round-trip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_points: Vec<f32>,
    /// Original FBX polygon-corner index stream.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub polygon_vertex_indices: Vec<i32>,
    /// Original UV layers, including mapping/reference metadata.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uv_sets: Vec<UvSetOutput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub normal_sets: Vec<NormalSetOutput>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct UvSetOutput {
    pub name: Option<String>,
    pub mapping: Option<String>,
    pub reference: Option<String>,
    pub values: Vec<f32>,
    pub indices: Vec<i32>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct NormalSetOutput {
    pub name: Option<String>,
    pub mapping: Option<String>,
    pub reference: Option<String>,
    pub values: Vec<f32>,
    pub indices: Vec<i32>,
}

#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MorphTargetOutput {
    pub name: Option<String>,
    pub control_point_indices: Vec<u32>,
    /// Render-vertex indices after corner-domain expansion.  A control point
    /// can occur more than once when UVs or normals have seams.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub render_point_indices: Vec<u32>,
    pub position_deltas: Vec<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub render_position_deltas: Vec<f32>,
    pub normal_deltas: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_normal_deltas: Option<Vec<f32>>,
    pub default_weight: f32,
    pub full_weight: f32,
}

#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkinClusterOutput {
    pub joint_node_id: u32,
    pub control_point_indices: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub render_point_indices: Vec<u32>,
    pub weights: Vec<f32>,
    pub mesh_bind_transform: Vec<f32>,
    pub joint_bind_transform: Vec<f32>,
    pub armature_bind_transform: Option<Vec<f32>>,
}

#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkinOutput {
    pub clusters: Vec<SkinClusterOutput>,
    pub bind_pose: Vec<BindPoseOutput>,
}

#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BindPoseOutput {
    pub node_id: u32,
    pub matrix: Vec<f32>,
}

/// Texture slot targeted by a binding.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TextureSlotOutput {
    Diffuse,
    Normal,
    Emissive,
    Specular,
    Roughness,
    Metallic,
    Ambient,
}

#[cfg(feature = "read")]
impl From<draco_io::FbxTextureSlot> for TextureSlotOutput {
    fn from(slot: draco_io::FbxTextureSlot) -> Self {
        match slot {
            draco_io::FbxTextureSlot::Diffuse => TextureSlotOutput::Diffuse,
            draco_io::FbxTextureSlot::Normal => TextureSlotOutput::Normal,
            draco_io::FbxTextureSlot::Emissive => TextureSlotOutput::Emissive,
            draco_io::FbxTextureSlot::Specular => TextureSlotOutput::Specular,
            draco_io::FbxTextureSlot::Roughness => TextureSlotOutput::Roughness,
            draco_io::FbxTextureSlot::Metallic => TextureSlotOutput::Metallic,
            draco_io::FbxTextureSlot::Ambient => TextureSlotOutput::Ambient,
        }
    }
}

/// Texture binding output to JavaScript.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TextureBindingOutput {
    pub slot: TextureSlotOutput,
    pub texture_index: usize,
}

/// Texture object output to JavaScript.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TextureOutput {
    pub name: Option<String>,
    /// Embedded image bytes (PNG/JPG), when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<u8>>,
    /// Relative filename / external reference, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

/// Material object output to JavaScript.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MaterialOutput {
    pub name: Option<String>,
    pub shading_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diffuse: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specular: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emissive: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diffuse_factor: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specular_factor: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shininess: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emissive_factor: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection_factor: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparency_factor: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bump_factor: Option<f32>,
    #[serde(default)]
    pub textures: Vec<TextureBindingOutput>,
}

/// Animation channel path (which TRS component the channel drives).
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AnimChannelPathOutput {
    Translation,
    Rotation,
    Scale,
    MorphWeight,
}

#[cfg(feature = "read")]
impl From<draco_io::FbxAnimChannelPath> for AnimChannelPathOutput {
    fn from(path: draco_io::FbxAnimChannelPath) -> Self {
        match path {
            draco_io::FbxAnimChannelPath::Translation => AnimChannelPathOutput::Translation,
            draco_io::FbxAnimChannelPath::Rotation => AnimChannelPathOutput::Rotation,
            draco_io::FbxAnimChannelPath::Scale => AnimChannelPathOutput::Scale,
            draco_io::FbxAnimChannelPath::MorphWeight => AnimChannelPathOutput::MorphWeight,
        }
    }
}

/// Animation interpolation mode.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AnimInterpolationOutput {
    Step,
    Linear,
    Cubic,
}

#[cfg(feature = "read")]
impl From<draco_io::FbxAnimInterpolation> for AnimInterpolationOutput {
    fn from(value: draco_io::FbxAnimInterpolation) -> Self {
        match value {
            draco_io::FbxAnimInterpolation::Step => AnimInterpolationOutput::Step,
            draco_io::FbxAnimInterpolation::Linear => AnimInterpolationOutput::Linear,
            draco_io::FbxAnimInterpolation::Cubic => AnimInterpolationOutput::Cubic,
        }
    }
}

/// Animation sampler (flat TRS component track).
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AnimSamplerOutput {
    /// Strictly increasing keyframe times in seconds.
    pub input: Vec<f32>,
    /// Flattened keyframe values, 3 values per input entry (radians for rotation).
    pub output: Vec<f32>,
    pub interpolation: AnimInterpolationOutput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_tangents: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_tangents: Option<Vec<f32>>,
}

/// One animation channel: drives one TRS path of one named node.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AnimChannelOutput {
    pub node_id: u32,
    /// Name of the target model node.
    pub node_name: String,
    pub path: AnimChannelPathOutput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub morph_target_index: Option<u32>,
    pub sampler: AnimSamplerOutput,
}

/// One animation take.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AnimationOutput {
    pub name: Option<String>,
    pub duration: f32,
    pub channels: Vec<AnimChannelOutput>,
}

/// Parse result containing meshes and any warnings/errors.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseResult {
    pub success: bool,
    pub meshes: Vec<MeshData>,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    /// FBX version
    pub version: Option<u32>,
    /// FBX model hierarchy and local transforms, when available.
    pub scene: Option<SceneOutput>,
    /// Materials carried at the top level, mirroring `scene.materials`.
    #[serde(default)]
    pub materials: Vec<MaterialOutput>,
    /// Textures carried at the top level, mirroring `scene.textures`.
    #[serde(default)]
    pub textures: Vec<TextureOutput>,
    /// Animations carried at the top level, mirroring `scene.animations`.
    #[serde(default)]
    pub animations: Vec<AnimationOutput>,
}

/// Scene data returned to JavaScript for hierarchy-preserving previews.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneOutput {
    pub root_nodes: Vec<SceneNodeOutput>,
    #[serde(default)]
    pub materials: Vec<MaterialOutput>,
    #[serde(default)]
    pub textures: Vec<TextureOutput>,
    #[serde(default)]
    pub animations: Vec<AnimationOutput>,
}

/// One FBX model node returned to JavaScript.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneNodeOutput {
    pub id: u32,
    pub name: Option<String>,
    /// Column-major local transform used by WebGL.
    pub matrix: Option<Vec<f32>>,
    pub meshes: Vec<MeshData>,
    pub children: Vec<SceneNodeOutput>,
}

/// Parse FBX binary file content.
#[cfg(feature = "read")]
#[wasm_bindgen]
pub fn parse_fbx(data: &[u8]) -> JsValue {
    let result = parse_fbx_scene(data);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

#[cfg(feature = "read")]
fn parse_fbx_scene(data: &[u8]) -> ParseResult {
    let version = data
        .get(23..27)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes);
    match FbxScene::from_bytes(data) {
        Ok(scene) => {
            let materials: Vec<MaterialOutput> =
                scene.materials.iter().map(material_to_output).collect();
            let textures: Vec<TextureOutput> =
                scene.textures.iter().map(texture_to_output).collect();
            let animations: Vec<AnimationOutput> =
                scene.animations.iter().map(animation_to_output).collect();
            let scene_out = SceneOutput {
                root_nodes: scene.root_nodes.iter().map(scene_node_to_output).collect(),
                materials: materials.clone(),
                textures: textures.clone(),
                animations: animations.clone(),
            };
            let mut meshes = Vec::new();
            collect_scene_meshes(&scene_out.root_nodes, &mut meshes);
            ParseResult {
                success: true,
                meshes,
                error: None,
                warnings: scene.warnings.clone(),
                version,
                scene: Some(scene_out),
                materials,
                textures,
                animations,
            }
        }
        Err(error) => ParseResult {
            success: false,
            meshes: Vec::new(),
            error: Some(error.to_string()),
            warnings: Vec::new(),
            version,
            scene: None,
            materials: Vec::new(),
            textures: Vec::new(),
            animations: Vec::new(),
        },
    }
}

#[cfg(feature = "read")]
fn scene_node_to_output(node: &FbxSceneNode) -> SceneNodeOutput {
    SceneNodeOutput {
        id: node.id.0,
        name: node.name.clone(),
        matrix: node
            .transform
            .map(|transform| transform.matrix.into_iter().flatten().collect()),
        meshes: node
            .mesh_instances
            .iter()
            .map(mesh_instance_to_data)
            .collect(),
        children: node.children.iter().map(scene_node_to_output).collect(),
    }
}

#[cfg(feature = "read")]
fn mesh_instance_to_data(instance: &draco_io::FbxMeshInstance) -> MeshData {
    let mut mesh = mesh_to_js_data(&instance.mesh);
    mesh.name = instance.name.clone();
    mesh.control_points = instance
        .control_points
        .iter()
        .flat_map(|point| point.iter().copied())
        .collect();
    mesh.polygon_vertex_indices = instance.polygon_vertex_indices.clone();
    mesh.uv_sets = instance
        .uv_sets
        .iter()
        .map(|set| UvSetOutput {
            name: set.name.clone(),
            mapping: set.mapping.clone(),
            reference: set.reference.clone(),
            values: set
                .values
                .iter()
                .flat_map(|value| value.iter().copied())
                .collect(),
            indices: set.indices.clone(),
        })
        .collect();
    mesh.normal_sets = instance
        .normal_sets
        .iter()
        .map(|set| NormalSetOutput {
            name: set.name.clone(),
            mapping: set.mapping.clone(),
            reference: set.reference.clone(),
            values: set
                .values
                .iter()
                .flat_map(|value| value.iter().copied())
                .collect(),
            indices: set.indices.clone(),
        })
        .collect();
    let render_control_points = expand_render_mesh(instance, &mut mesh);
    let mut render_points_by_control = std::collections::HashMap::<u32, Vec<u32>>::new();
    for (render, control) in render_control_points.iter().copied().enumerate() {
        render_points_by_control
            .entry(control)
            .or_default()
            .push(render as u32);
    }
    mesh.material_indices = instance.material_indices.clone();
    mesh.material = instance.material_indices.first().map(|&idx| idx as usize);
    mesh.skin = instance.skin.as_ref().map(|skin| SkinOutput {
        clusters: skin
            .clusters
            .iter()
            .map(|cluster| SkinClusterOutput {
                joint_node_id: cluster.joint_node_id.0,
                control_point_indices: cluster.control_point_indices.clone(),
                render_point_indices: cluster
                    .control_point_indices
                    .iter()
                    .flat_map(|control_point| {
                        render_points_by_control
                            .get(control_point)
                            .into_iter()
                            .flatten()
                            .copied()
                    })
                    .collect(),
                weights: cluster.weights.clone(),
                mesh_bind_transform: cluster
                    .mesh_bind_transform
                    .matrix
                    .into_iter()
                    .flatten()
                    .collect(),
                joint_bind_transform: cluster
                    .joint_bind_transform
                    .matrix
                    .into_iter()
                    .flatten()
                    .collect(),
                armature_bind_transform: cluster
                    .armature_bind_transform
                    .map(|transform| transform.matrix.into_iter().flatten().collect()),
            })
            .collect(),
        bind_pose: skin
            .bind_pose
            .iter()
            .map(|(node_id, transform)| BindPoseOutput {
                node_id: node_id.0,
                matrix: transform.matrix.into_iter().flatten().collect(),
            })
            .collect(),
    });
    if let Some(skin) = &instance.skin {
        let point_count = mesh.positions.len() / 3;
        let mut influences = vec![Vec::<(u16, f32)>::new(); point_count];
        for (joint_index, cluster) in skin.clusters.iter().enumerate() {
            let render_points = cluster.control_point_indices.iter().enumerate().flat_map(
                |(influence, control_point)| {
                    let weight = cluster.weights.get(influence).copied().unwrap_or(0.0);
                    render_points_by_control
                        .get(control_point)
                        .into_iter()
                        .flatten()
                        .map(move |render| (*render, weight))
                        .collect::<Vec<_>>()
                },
            );
            for (point, weight) in render_points {
                if let Some(entries) = influences.get_mut(point as usize) {
                    entries.push((joint_index as u16, weight));
                }
            }
        }
        mesh.joints0 = vec![0; point_count * 4];
        mesh.weights0 = vec![0.0; point_count * 4];
        for (point, entries) in influences.iter_mut().enumerate() {
            entries.sort_by(|left, right| right.1.total_cmp(&left.1));
            let sum: f32 = entries.iter().take(4).map(|entry| entry.1).sum();
            for (slot, &(joint, weight)) in entries.iter().take(4).enumerate() {
                mesh.joints0[point * 4 + slot] = joint;
                mesh.weights0[point * 4 + slot] = if sum > 0.0 { weight / sum } else { 0.0 };
            }
        }
    }
    mesh.morph_targets = instance
        .morph_targets
        .iter()
        .map(|target| {
            let mut render_point_indices = Vec::new();
            let mut render_position_deltas = Vec::new();
            let mut render_normal_deltas = target.normal_deltas.as_ref().map(|_| Vec::new());
            for (entry, control_point) in target.control_point_indices.iter().enumerate() {
                let Some(render_points) = render_points_by_control.get(control_point) else {
                    continue;
                };
                let Some(position_delta) = target.position_deltas.get(entry) else {
                    continue;
                };
                for render_point in render_points {
                    render_point_indices.push(*render_point);
                    render_position_deltas.extend(position_delta.iter().copied());
                    if let (Some(render), Some(normal_deltas)) =
                        (render_normal_deltas.as_mut(), target.normal_deltas.as_ref())
                    {
                        if let Some(normal_delta) = normal_deltas.get(entry) {
                            render.extend(normal_delta.iter().copied());
                        } else {
                            render.extend([0.0; 3]);
                        }
                    }
                }
            }
            MorphTargetOutput {
                name: target.name.clone(),
                control_point_indices: target.control_point_indices.clone(),
                render_point_indices,
                position_deltas: target
                    .position_deltas
                    .iter()
                    .flat_map(|delta| delta.iter().copied())
                    .collect(),
                render_position_deltas,
                normal_deltas: target.normal_deltas.as_ref().map(|deltas| {
                    deltas
                        .iter()
                        .flat_map(|delta| delta.iter().copied())
                        .collect()
                }),
                render_normal_deltas,
                default_weight: target.default_weight,
                full_weight: target.full_weight,
            }
        })
        .collect();
    mesh
}

#[cfg(feature = "read")]
fn collect_scene_meshes(nodes: &[SceneNodeOutput], meshes: &mut Vec<MeshData>) {
    for node in nodes {
        meshes.extend(node.meshes.iter().cloned());
        collect_scene_meshes(&node.children, meshes);
    }
}

#[cfg(feature = "read")]
fn material_to_output(material: &draco_io::FbxMaterial) -> MaterialOutput {
    MaterialOutput {
        name: material.name.clone(),
        shading_model: material.shading_model.clone(),
        diffuse: material.diffuse,
        specular: material.specular,
        emissive: material.emissive,
        ambient: material.ambient,
        diffuse_factor: material.diffuse_factor,
        specular_factor: material.specular_factor,
        shininess: material.shininess,
        emissive_factor: material.emissive_factor,
        reflection_factor: material.reflection_factor,
        transparency_factor: material.transparency_factor,
        opacity: material.opacity,
        bump_factor: material.bump_factor,
        textures: material
            .textures
            .iter()
            .map(|binding| TextureBindingOutput {
                slot: binding.slot.into(),
                texture_index: binding.texture_index,
            })
            .collect(),
    }
}

#[cfg(feature = "read")]
fn texture_to_output(texture: &draco_io::FbxTexture) -> TextureOutput {
    TextureOutput {
        name: texture.name.clone(),
        content: texture.content.clone(),
        filename: texture.filename.clone(),
    }
}

#[cfg(feature = "read")]
fn animation_to_output(animation: &draco_io::FbxAnimation) -> AnimationOutput {
    AnimationOutput {
        name: animation.name.clone(),
        duration: animation.duration,
        channels: animation
            .channels
            .iter()
            .map(|channel| AnimChannelOutput {
                node_name: channel.node_name.clone(),
                node_id: channel.node_id.0,
                path: channel.path.into(),
                morph_target_index: channel.morph_target_index,
                sampler: AnimSamplerOutput {
                    input: channel.sampler.input.clone(),
                    output: channel.sampler.output.clone(),
                    interpolation: channel.sampler.interpolation.into(),
                    in_tangents: channel.sampler.in_tangents.clone(),
                    out_tangents: channel.sampler.out_tangents.clone(),
                },
            })
            .collect(),
    }
}

#[cfg(feature = "read")]
fn mesh_to_js_data(mesh: &Mesh) -> MeshData {
    let positions = read_attribute_as_f32(mesh, GeometryAttributeType::Position, 3);
    let normals = read_attribute_as_f32(mesh, GeometryAttributeType::Normal, 3);
    let uvs = read_attribute_as_f32(mesh, GeometryAttributeType::TexCoord, 2);
    let mut indices = Vec::with_capacity(mesh.num_faces() * 3);
    for index in 0..mesh.num_faces() {
        let face = mesh.face(FaceIndex(index as u32));
        indices.extend([face[0].0, face[1].0, face[2].0]);
    }
    MeshData {
        name: None,
        positions,
        indices,
        normals,
        uvs,
        material_indices: Vec::new(),
        material: None,
        skin: None,
        morph_targets: Vec::new(),
        joints0: Vec::new(),
        weights0: Vec::new(),
        control_points: Vec::new(),
        polygon_vertex_indices: Vec::new(),
        uv_sets: Vec::new(),
        normal_sets: Vec::new(),
    }
}

#[cfg(feature = "read")]
fn expand_render_mesh(instance: &draco_io::FbxMeshInstance, mesh: &mut MeshData) -> Vec<u32> {
    if instance.control_points.is_empty() || instance.polygon_vertex_indices.is_empty() {
        return (0..mesh.positions.len() / 3)
            .map(|index| index as u32)
            .collect();
    }

    let uv = instance.uv_sets.first();
    let normal = instance.normal_sets.first();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut render_to_control = Vec::new();
    let mut polygon: Vec<(u32, usize)> = Vec::new();
    let mut corner = 0usize;

    let mut emit_corner = |control_point: u32, corner_index: usize| {
        let point = instance
            .control_points
            .get(control_point as usize)
            .copied()
            .unwrap_or([0.0; 3]);
        positions.extend(point);
        render_to_control.push(control_point);
        if let Some(layer) = normal {
            let value = resolve_layer_value_3(
                &layer.mapping,
                &layer.reference,
                &layer.indices,
                &layer.values,
                control_point as usize,
                corner_index,
            );
            normals.extend(value);
        }
        if let Some(layer) = uv {
            let value = resolve_layer_value_2(
                &layer.mapping,
                &layer.reference,
                &layer.indices,
                &layer.values,
                control_point as usize,
                corner_index,
            );
            uvs.extend(value);
        }
    };

    for encoded in &instance.polygon_vertex_indices {
        let control_point = if *encoded < 0 {
            (!*encoded) as u32
        } else {
            *encoded as u32
        };
        polygon.push((control_point, corner));
        corner += 1;
        if *encoded < 0 {
            for index in 1..polygon.len().saturating_sub(1) {
                for &(point, corner_index) in
                    [polygon[0], polygon[index], polygon[index + 1]].iter()
                {
                    emit_corner(point, corner_index);
                }
            }
            polygon.clear();
        }
    }

    if positions.is_empty() {
        return (0..mesh.positions.len() / 3)
            .map(|index| index as u32)
            .collect();
    }
    mesh.positions = positions;
    mesh.indices = (0..mesh.positions.len() / 3)
        .map(|index| index as u32)
        .collect();
    mesh.normals = if normal.is_some() {
        normals
    } else {
        Vec::new()
    };
    mesh.uvs = if uv.is_some() { uvs } else { Vec::new() };
    render_to_control
}

#[cfg(feature = "read")]
fn resolve_layer_value_2(
    mapping: &Option<String>,
    reference: &Option<String>,
    indices: &[i32],
    values: &[[f32; 2]],
    control_point: usize,
    corner: usize,
) -> [f32; 2] {
    let logical = match mapping.as_deref() {
        Some("ByPolygonVertex") => corner,
        Some("AllSame") => 0,
        _ => control_point,
    };
    let value_index = if reference.as_deref() == Some("IndexToDirect") {
        indices
            .get(logical)
            .copied()
            .unwrap_or(logical as i32)
            .max(0) as usize
    } else {
        logical
    };
    values.get(value_index).copied().unwrap_or([0.0; 2])
}

#[cfg(feature = "read")]
fn resolve_layer_value_3(
    mapping: &Option<String>,
    reference: &Option<String>,
    indices: &[i32],
    values: &[[f32; 3]],
    control_point: usize,
    corner: usize,
) -> [f32; 3] {
    let logical = match mapping.as_deref() {
        Some("ByPolygonVertex") => corner,
        Some("AllSame") => 0,
        _ => control_point,
    };
    let value_index = if reference.as_deref() == Some("IndexToDirect") {
        indices
            .get(logical)
            .copied()
            .unwrap_or(logical as i32)
            .max(0) as usize
    } else {
        logical
    };
    values.get(value_index).copied().unwrap_or([0.0; 3])
}

#[cfg(feature = "read")]
fn read_attribute_as_f32(
    mesh: &Mesh,
    attribute_type: GeometryAttributeType,
    components: usize,
) -> Vec<f32> {
    let attribute_id = mesh.named_attribute_id(attribute_type);
    if attribute_id < 0 {
        return Vec::new();
    }
    let attribute = mesh.attribute(attribute_id);
    let stride = attribute.byte_stride() as usize;
    let data = attribute.buffer().data();
    let mut output = Vec::with_capacity(mesh.num_points() * components);
    for point in 0..mesh.num_points() {
        let base = point * stride;
        for component in 0..components.min(attribute.num_components() as usize) {
            let offset = base + component * attribute.data_type().byte_length();
            let value = match attribute.data_type() {
                DataType::Float32 => {
                    f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
                }
                DataType::Float64 => {
                    f64::from_le_bytes(data[offset..offset + 8].try_into().unwrap()) as f32
                }
                DataType::Int32 => {
                    i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as f32
                }
                DataType::Uint32 => {
                    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as f32
                }
                DataType::Int16 => {
                    i16::from_le_bytes(data[offset..offset + 2].try_into().unwrap()) as f32
                }
                DataType::Uint16 => {
                    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap()) as f32
                }
                DataType::Int8 => data[offset] as i8 as f32,
                DataType::Uint8 => data[offset] as f32,
                _ => 0.0,
            };
            output.push(value);
        }
    }
    output
}

// ===========================================================================
// Writer
// ===========================================================================

#[cfg(feature = "write")]
use draco_core::geometry_attribute::PointAttribute;
#[cfg(feature = "write")]
use draco_core::geometry_indices::PointIndex;
#[cfg(feature = "write")]
use draco_io::{
    FbxAnimChannel, FbxAnimInterpolation, FbxAnimSampler, FbxMaterial, FbxMeshInstance, FbxTexture,
    FbxTextureBinding, FbxTextureSlot, FbxTransform,
};
#[cfg(feature = "write")]
use draco_io::{FbxAnimChannelPath, FbxAnimation};
#[cfg(feature = "write")]
use std::io::Write;

/// Input mesh data consumed by the FBX writer, from JavaScript.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MeshInput {
    /// Mesh name
    pub name: Option<String>,
    /// Vertex positions as flat array [x0, y0, z0, x1, y1, z1, ...]
    pub positions: Vec<f32>,
    /// Face indices as flat array (triangles)
    pub indices: Vec<u32>,
    /// Vertex normals (optional)
    pub normals: Option<Vec<f32>>,
    /// Texture coordinates (optional)
    pub uvs: Option<Vec<f32>>,
    /// Optional original FBX control points and polygon-corner stream.
    #[serde(default)]
    pub control_points: Option<Vec<f32>>,
    #[serde(default)]
    pub polygon_vertex_indices: Option<Vec<i32>>,
    #[serde(default)]
    pub uv_sets: Vec<UvSetOutput>,
    #[serde(default)]
    pub normal_sets: Vec<NormalSetOutput>,
    /// Per-triangle indices into `SceneInput::materials`.
    #[serde(default)]
    pub material_indices: Vec<i32>,
    #[serde(default)]
    pub skin: Option<SkinInput>,
    #[serde(default)]
    pub morph_targets: Vec<MorphTargetInput>,
}

#[cfg(feature = "write")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MorphTargetInput {
    pub name: Option<String>,
    pub control_point_indices: Vec<u32>,
    pub position_deltas: Vec<f32>,
    #[serde(default)]
    pub normal_deltas: Option<Vec<f32>>,
    #[serde(default)]
    pub default_weight: f32,
    #[serde(default = "default_full_morph_weight")]
    pub full_weight: f32,
}

#[cfg(feature = "write")]
fn default_full_morph_weight() -> f32 {
    100.0
}

#[cfg(feature = "write")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkinClusterInput {
    pub joint_node_id: u32,
    pub control_point_indices: Vec<u32>,
    #[serde(default)]
    pub render_point_indices: Vec<u32>,
    pub weights: Vec<f32>,
    pub mesh_bind_transform: Vec<f32>,
    pub joint_bind_transform: Vec<f32>,
    #[serde(default)]
    pub armature_bind_transform: Option<Vec<f32>>,
}

#[cfg(feature = "write")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BindPoseInput {
    pub node_id: u32,
    pub matrix: Vec<f32>,
}

#[cfg(feature = "write")]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkinInput {
    pub clusters: Vec<SkinClusterInput>,
    #[serde(default)]
    pub bind_pose: Vec<BindPoseInput>,
}

/// A hierarchy-preserving FBX export scene supplied by JavaScript.
#[cfg(feature = "write")]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneInput {
    pub root_nodes: Vec<SceneNodeInput>,
    #[serde(default)]
    pub materials: Vec<MaterialInput>,
    #[serde(default)]
    pub textures: Vec<TextureInput>,
    #[serde(default)]
    pub animations: Vec<AnimationInput>,
}

/// One FBX model node supplied by JavaScript.
#[cfg(feature = "write")]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneNodeInput {
    /// Stable scene-local node id. Missing ids are assigned deterministically.
    #[serde(default)]
    pub id: u32,
    pub name: Option<String>,
    /// Row-major local affine transform, as used by `FbxTransform`.
    pub matrix: Option<Vec<f32>>,
    #[serde(default)]
    pub meshes: Vec<MeshInput>,
    /// Per-mesh material index list, mirroring `FbxMeshInstance::material_indices`.
    #[serde(default)]
    pub children: Vec<SceneNodeInput>,
}

/// Material input supplied by JavaScript for the FBX writer.
#[cfg(feature = "write")]
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MaterialInput {
    pub name: Option<String>,
    pub shading_model: Option<String>,
    pub diffuse: Option<[f32; 3]>,
    pub specular: Option<[f32; 3]>,
    pub emissive: Option<[f32; 3]>,
    pub ambient: Option<[f32; 3]>,
    pub diffuse_factor: Option<f32>,
    pub specular_factor: Option<f32>,
    pub shininess: Option<f32>,
    pub emissive_factor: Option<f32>,
    pub reflection_factor: Option<f32>,
    pub transparency_factor: Option<f32>,
    pub opacity: Option<f32>,
    pub bump_factor: Option<f32>,
    #[serde(default)]
    pub textures: Vec<TextureBindingInput>,
}

#[cfg(feature = "write")]
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum TextureSlotInput {
    Diffuse,
    Normal,
    Emissive,
    Specular,
    Roughness,
    Metallic,
    Ambient,
}

#[cfg(feature = "write")]
impl From<TextureSlotInput> for FbxTextureSlot {
    fn from(slot: TextureSlotInput) -> Self {
        match slot {
            TextureSlotInput::Diffuse => FbxTextureSlot::Diffuse,
            TextureSlotInput::Normal => FbxTextureSlot::Normal,
            TextureSlotInput::Emissive => FbxTextureSlot::Emissive,
            TextureSlotInput::Specular => FbxTextureSlot::Specular,
            TextureSlotInput::Roughness => FbxTextureSlot::Roughness,
            TextureSlotInput::Metallic => FbxTextureSlot::Metallic,
            TextureSlotInput::Ambient => FbxTextureSlot::Ambient,
        }
    }
}

#[cfg(feature = "write")]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureBindingInput {
    pub slot: TextureSlotInput,
    pub texture_index: usize,
}

#[cfg(feature = "write")]
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TextureInput {
    pub name: Option<String>,
    pub content: Option<Vec<u8>>,
    pub filename: Option<String>,
}

/// Animation input supplied by JavaScript.
#[cfg(feature = "write")]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimationInput {
    pub name: Option<String>,
    pub duration: f32,
    pub channels: Vec<AnimChannelInput>,
}

#[cfg(feature = "write")]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimChannelInput {
    #[serde(default)]
    pub node_id: u32,
    pub node_name: String,
    pub path: AnimChannelPathInput,
    #[serde(default)]
    pub morph_target_index: Option<u32>,
    pub sampler: AnimSamplerInput,
}

#[cfg(feature = "write")]
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum AnimChannelPathInput {
    Translation,
    Rotation,
    Scale,
    MorphWeight,
}

#[cfg(feature = "write")]
impl From<AnimChannelPathInput> for FbxAnimChannelPath {
    fn from(path: AnimChannelPathInput) -> Self {
        match path {
            AnimChannelPathInput::Translation => FbxAnimChannelPath::Translation,
            AnimChannelPathInput::Rotation => FbxAnimChannelPath::Rotation,
            AnimChannelPathInput::Scale => FbxAnimChannelPath::Scale,
            AnimChannelPathInput::MorphWeight => FbxAnimChannelPath::MorphWeight,
        }
    }
}

#[cfg(feature = "write")]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimSamplerInput {
    pub input: Vec<f32>,
    pub output: Vec<f32>,
    pub interpolation: AnimInterpolationInput,
    #[serde(default)]
    pub in_tangents: Option<Vec<f32>>,
    #[serde(default)]
    pub out_tangents: Option<Vec<f32>>,
}

#[cfg(feature = "write")]
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum AnimInterpolationInput {
    Step,
    Linear,
    Cubic,
}

#[cfg(feature = "write")]
impl From<AnimInterpolationInput> for FbxAnimInterpolation {
    fn from(value: AnimInterpolationInput) -> Self {
        match value {
            AnimInterpolationInput::Step => FbxAnimInterpolation::Step,
            AnimInterpolationInput::Linear => FbxAnimInterpolation::Linear,
            AnimInterpolationInput::Cubic => FbxAnimInterpolation::Cubic,
        }
    }
}

/// Export options.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize, Default)]
pub struct ExportOptions {
    /// FBX version (default: 7500 for FBX 7.5)
    pub version: Option<u32>,
}

/// Export result.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize)]
pub struct ExportResult {
    pub success: bool,
    pub binary_data: Option<Vec<u8>>,
    pub error: Option<String>,
}

/// Create FBX binary content from mesh data.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub fn create_fbx(meshes_js: JsValue, options_js: JsValue) -> JsValue {
    let meshes: Vec<MeshInput> = match serde_wasm_bindgen::from_value(meshes_js) {
        Ok(m) => m,
        Err(e) => {
            let result = ExportResult {
                success: false,
                binary_data: None,
                error: Some(format!("Invalid mesh data: {}", e)),
            };
            return serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL);
        }
    };

    let options: ExportOptions = serde_wasm_bindgen::from_value(options_js).unwrap_or_default();
    let _ = options;
    let result = create_fbx_internal(&meshes);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Create FBX binary content while preserving model hierarchy, materials,
/// textures, animation, and local transforms.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub fn create_fbx_scene(scene_js: JsValue, _options_js: JsValue) -> JsValue {
    let result = match serde_wasm_bindgen::from_value::<SceneInput>(scene_js) {
        Ok(input) => scene_input_to_fbx_scene(input)
            .and_then(|scene| scene.to_bytes().map_err(|error| error.to_string()))
            .map(|binary_data| ExportResult {
                success: true,
                binary_data: Some(binary_data),
                error: None,
            })
            .unwrap_or_else(|error| ExportResult {
                success: false,
                binary_data: None,
                error: Some(error),
            }),
        Err(error) => ExportResult {
            success: false,
            binary_data: None,
            error: Some(format!("Invalid scene data: {error}")),
        },
    };
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

#[cfg(feature = "write")]
fn scene_input_to_fbx_scene(input: SceneInput) -> Result<FbxScene, String> {
    Ok(FbxScene {
        root_nodes: input
            .root_nodes
            .into_iter()
            .map(scene_node_to_fbx)
            .collect::<Result<_, _>>()?,
        materials: input
            .materials
            .into_iter()
            .map(material_input_to_fbx)
            .collect::<Result<_, String>>()?,
        textures: input
            .textures
            .into_iter()
            .map(texture_input_to_fbx)
            .collect(),
        animations: input
            .animations
            .into_iter()
            .map(animation_input_to_fbx)
            .collect::<Result<_, String>>()?,
        warnings: Vec::new(),
    })
}

#[cfg(feature = "write")]
fn scene_node_to_fbx(input: SceneNodeInput) -> Result<FbxSceneNode, String> {
    let transform = input
        .matrix
        .map(|matrix| {
            if matrix.len() != 16 {
                return Err("scene node matrix must contain 16 values".to_string());
            }
            let mut rows = [[0.0; 4]; 4];
            for (index, value) in matrix.into_iter().enumerate() {
                rows[index / 4][index % 4] = value;
            }
            Ok(FbxTransform { matrix: rows })
        })
        .transpose()?;
    Ok(FbxSceneNode {
        id: FbxNodeId(input.id),
        name: input.name,
        transform,
        mesh_instances: input
            .meshes
            .into_iter()
            .enumerate()
            .map(|(index, mesh)| {
                let name = mesh.name.clone().or_else(|| Some(format!("mesh_{index}")));
                Ok(FbxMeshInstance {
                    name,
                    mesh: mesh_input_to_core_mesh(&mesh)?,
                    control_points: mesh
                        .control_points
                        .as_deref()
                        .unwrap_or(&[])
                        .chunks_exact(3)
                        .map(|value| [value[0], value[1], value[2]])
                        .collect(),
                    polygon_vertex_indices: mesh.polygon_vertex_indices.clone().unwrap_or_default(),
                    uv_sets: mesh
                        .uv_sets
                        .iter()
                        .map(|set| draco_io::FbxUvSet {
                            name: set.name.clone(),
                            mapping: set.mapping.clone(),
                            reference: set.reference.clone(),
                            values: set
                                .values
                                .chunks_exact(2)
                                .map(|value| [value[0], value[1]])
                                .collect(),
                            indices: set.indices.clone(),
                        })
                        .collect(),
                    normal_sets: mesh
                        .normal_sets
                        .iter()
                        .map(|set| draco_io::FbxNormalSet {
                            name: set.name.clone(),
                            mapping: set.mapping.clone(),
                            reference: set.reference.clone(),
                            values: set
                                .values
                                .chunks_exact(3)
                                .map(|value| [value[0], value[1], value[2]])
                                .collect(),
                            indices: set.indices.clone(),
                        })
                        .collect(),
                    material_indices: mesh.material_indices.clone(),
                    skin: mesh.skin.as_ref().map(skin_input_to_fbx).transpose()?,
                    morph_targets: mesh
                        .morph_targets
                        .iter()
                        .map(morph_target_input_to_fbx)
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect::<Result<_, String>>()?,
        children: input
            .children
            .into_iter()
            .map(scene_node_to_fbx)
            .collect::<Result<_, _>>()?,
    })
}

#[cfg(feature = "write")]
fn transform_input_to_fbx(values: &[f32]) -> Result<FbxTransform, String> {
    if values.len() != 16 {
        return Err("FBX skin matrix must contain 16 values".to_string());
    }
    let mut matrix = [[0.0; 4]; 4];
    for (index, value) in values.iter().copied().enumerate() {
        matrix[index / 4][index % 4] = value;
    }
    Ok(FbxTransform { matrix })
}

#[cfg(feature = "write")]
fn skin_input_to_fbx(input: &SkinInput) -> Result<draco_io::FbxSkin, String> {
    let clusters = input
        .clusters
        .iter()
        .map(|cluster| {
            if cluster.control_point_indices.len() != cluster.weights.len() {
                return Err("FBX skin indices and weights must have equal lengths".to_string());
            }
            Ok(draco_io::FbxSkinCluster {
                joint_node_id: FbxNodeId(cluster.joint_node_id),
                control_point_indices: cluster.control_point_indices.clone(),
                weights: cluster.weights.clone(),
                mesh_bind_transform: transform_input_to_fbx(&cluster.mesh_bind_transform)?,
                joint_bind_transform: transform_input_to_fbx(&cluster.joint_bind_transform)?,
                armature_bind_transform: cluster
                    .armature_bind_transform
                    .as_deref()
                    .map(transform_input_to_fbx)
                    .transpose()?,
            })
        })
        .collect::<Result<_, String>>()?;
    let bind_pose = input
        .bind_pose
        .iter()
        .map(|entry| {
            Ok((
                FbxNodeId(entry.node_id),
                transform_input_to_fbx(&entry.matrix)?,
            ))
        })
        .collect::<Result<_, String>>()?;
    Ok(draco_io::FbxSkin {
        clusters,
        bind_pose,
    })
}

#[cfg(feature = "write")]
fn morph_target_input_to_fbx(input: &MorphTargetInput) -> Result<draco_io::FbxMorphTarget, String> {
    if input.position_deltas.len() != input.control_point_indices.len() * 3 {
        return Err("FBX morph position deltas must be a vec3 per control point".to_string());
    }
    let normal_deltas = match &input.normal_deltas {
        Some(values) if values.len() == input.control_point_indices.len() * 3 => Some(
            values
                .chunks_exact(3)
                .map(|values| [values[0], values[1], values[2]])
                .collect(),
        ),
        Some(_) => {
            return Err("FBX morph normal deltas must be a vec3 per control point".to_string())
        }
        None => None,
    };
    Ok(draco_io::FbxMorphTarget {
        name: input.name.clone(),
        control_point_indices: input.control_point_indices.clone(),
        position_deltas: input
            .position_deltas
            .chunks_exact(3)
            .map(|values| [values[0], values[1], values[2]])
            .collect(),
        normal_deltas,
        default_weight: input.default_weight,
        full_weight: input.full_weight,
    })
}

#[cfg(feature = "write")]
fn material_input_to_fbx(input: MaterialInput) -> Result<FbxMaterial, String> {
    let textures = input
        .textures
        .iter()
        .map(|binding| FbxTextureBinding {
            slot: binding.slot.into(),
            texture_index: binding.texture_index,
        })
        .collect();
    Ok(FbxMaterial {
        name: input.name,
        shading_model: input.shading_model,
        diffuse: input.diffuse,
        specular: input.specular,
        emissive: input.emissive,
        ambient: input.ambient,
        diffuse_factor: input.diffuse_factor,
        specular_factor: input.specular_factor,
        shininess: input.shininess,
        emissive_factor: input.emissive_factor,
        reflection_factor: input.reflection_factor,
        transparency_factor: input.transparency_factor,
        opacity: input.opacity,
        bump_factor: input.bump_factor,
        textures,
    })
}

#[cfg(feature = "write")]
fn texture_input_to_fbx(input: TextureInput) -> FbxTexture {
    FbxTexture {
        name: input.name,
        content: input.content,
        filename: input.filename,
    }
}

#[cfg(feature = "write")]
fn animation_input_to_fbx(input: AnimationInput) -> Result<FbxAnimation, String> {
    Ok(FbxAnimation {
        name: input.name,
        duration: input.duration,
        channels: input
            .channels
            .into_iter()
            .map(|channel| FbxAnimChannel {
                node_id: FbxNodeId(channel.node_id),
                node_name: channel.node_name,
                path: channel.path.into(),
                morph_target_index: channel.morph_target_index,
                sampler: FbxAnimSampler {
                    input: channel.sampler.input,
                    output: channel.sampler.output,
                    interpolation: channel.sampler.interpolation.into(),
                    in_tangents: channel.sampler.in_tangents,
                    out_tangents: channel.sampler.out_tangents,
                },
            })
            .collect(),
    })
}

#[cfg(feature = "write")]
fn mesh_input_to_core_mesh(input: &MeshInput) -> Result<Mesh, String> {
    if !input.positions.len().is_multiple_of(3) || !input.indices.len().is_multiple_of(3) {
        return Err("FBX mesh positions and indices must be triangle-aligned".to_string());
    }
    let point_count = input.positions.len() / 3;
    let mut mesh = Mesh::new();
    mesh.set_num_points(point_count);
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        point_count,
    );
    for (index, values) in input.positions.chunks_exact(3).enumerate() {
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        position.buffer_mut().write(index * 12, &bytes);
    }
    mesh.add_attribute(position);
    if let Some(normals) = &input.normals {
        if normals.len() >= point_count * 3 {
            let mut normal = PointAttribute::new();
            normal.init(
                GeometryAttributeType::Normal,
                3,
                DataType::Float32,
                false,
                point_count,
            );
            for (index, values) in normals.chunks_exact(3).take(point_count).enumerate() {
                let bytes: Vec<u8> = values
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect();
                normal.buffer_mut().write(index * 12, &bytes);
            }
            mesh.add_attribute(normal);
        }
    }
    if let Some(uvs) = &input.uvs {
        if uvs.len() >= point_count * 2 {
            let mut tex_coord = PointAttribute::new();
            tex_coord.init(
                GeometryAttributeType::TexCoord,
                2,
                DataType::Float32,
                false,
                point_count,
            );
            for (index, values) in uvs.chunks_exact(2).take(point_count).enumerate() {
                let bytes: Vec<u8> = values
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect();
                tex_coord.buffer_mut().write(index * 8, &bytes);
            }
            mesh.add_attribute(tex_coord);
        }
    }
    mesh.set_num_faces(input.indices.len() / 3);
    for (index, face) in input.indices.chunks_exact(3).enumerate() {
        if face.iter().any(|&point| point as usize >= point_count) {
            return Err("FBX mesh index is outside its position array".to_string());
        }
        mesh.set_face(
            FaceIndex(index as u32),
            [
                PointIndex(face[0]),
                PointIndex(face[1]),
                PointIndex(face[2]),
            ],
        );
    }
    Ok(mesh)
}

/// Size of null node record for 64-bit FBX
#[cfg(feature = "write")]
const NULL_RECORD_SIZE: usize = 25;

#[cfg(feature = "write")]
fn create_fbx_internal(meshes: &[MeshInput]) -> ExportResult {
    // Fall back to a minimal hand-written FBX when there is no scene hierarchy
    // (the legacy `create_fbx` entry point). Materials, textures, and
    // animation are not expressible without a scene, so this path emits a
    // flat mesh list only.
    let version = 7500u32;
    let mut buffer: Vec<u8> = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);

    if cursor.write_all(FBX_MAGIC).is_err() {
        return ExportResult {
            success: false,
            binary_data: None,
            error: Some("Failed to write FBX magic".to_string()),
        };
    }
    let _ = cursor.write_all(&[0x1A, 0x00]);
    let _ = cursor.write_all(&version.to_le_bytes());

    let mut next_id: i64 = 1_000_000;
    let mut geometry_ids: Vec<i64> = Vec::new();
    let mut model_ids: Vec<i64> = Vec::new();
    for _ in meshes {
        geometry_ids.push(next_id);
        next_id += 1;
        model_ids.push(next_id);
        next_id += 1;
    }

    write_header_extension(&mut cursor, version);
    write_global_settings(&mut cursor);
    write_documents(&mut cursor);
    write_node(&mut cursor, "References", &[], &[]);
    write_definitions(&mut cursor, meshes.len());

    let mut objects_children: Vec<Vec<u8>> = Vec::new();
    for (i, mesh) in meshes.iter().enumerate() {
        let mut geom_buf: Vec<u8> = Vec::new();
        write_geometry(&mut Cursor::new(&mut geom_buf), mesh, geometry_ids[i]);
        objects_children.push(geom_buf);
        let mut model_buf: Vec<u8> = Vec::new();
        write_model(&mut Cursor::new(&mut model_buf), mesh, model_ids[i]);
        objects_children.push(model_buf);
    }
    write_node_with_children(&mut cursor, "Objects", &[], &objects_children);

    let mut connections_children: Vec<Vec<u8>> = Vec::new();
    for i in 0..meshes.len() {
        let mut conn_buf: Vec<u8> = Vec::new();
        write_connection(&mut Cursor::new(&mut conn_buf), model_ids[i], 0);
        connections_children.push(conn_buf);
        let mut conn_buf2: Vec<u8> = Vec::new();
        write_connection(
            &mut Cursor::new(&mut conn_buf2),
            geometry_ids[i],
            model_ids[i],
        );
        connections_children.push(conn_buf2);
    }
    write_node_with_children(&mut cursor, "Connections", &[], &connections_children);

    let null_record = vec![0u8; NULL_RECORD_SIZE];
    let _ = cursor.write_all(&null_record);
    write_footer(&mut cursor, version);

    ExportResult {
        success: true,
        binary_data: Some(buffer),
        error: None,
    }
}

use std::io::{Cursor, Seek, SeekFrom};

#[cfg(feature = "write")]
fn write_node<W: Write + Seek>(
    writer: &mut W,
    name: &str,
    properties: &[FbxProp],
    _children: &[Vec<u8>],
) {
    write_node_with_children(writer, name, properties, &[]);
}

#[cfg(feature = "write")]
fn write_node_with_children<W: Write + Seek>(
    writer: &mut W,
    name: &str,
    properties: &[FbxProp],
    children: &[Vec<u8>],
) {
    let start_pos = writer.stream_position().unwrap();
    let _ = writer.write_all(&[0u8; 24]);
    let _ = writer.write_all(&[name.len() as u8]);
    let _ = writer.write_all(name.as_bytes());
    let props_start = writer.stream_position().unwrap();
    for prop in properties {
        write_property(writer, prop);
    }
    let props_end = writer.stream_position().unwrap();
    let props_len = props_end - props_start;
    for child in children {
        let child_start = writer.stream_position().unwrap();
        let mut relocated = child.clone();
        rebase_node_offsets(&mut relocated, child_start);
        let _ = writer.write_all(&relocated);
    }
    if !children.is_empty() {
        let _ = writer.write_all(&[0u8; NULL_RECORD_SIZE]);
    }
    let end_pos = writer.stream_position().unwrap();
    let _ = writer.seek(SeekFrom::Start(start_pos));
    let _ = writer.write_all(&end_pos.to_le_bytes());
    let _ = writer.write_all(&(properties.len() as u64).to_le_bytes());
    let _ = writer.write_all(&props_len.to_le_bytes());
    let _ = writer.write_all(&[name.len() as u8]);
    let _ = writer.seek(SeekFrom::Start(end_pos));
}

/// Rebase every node end offset in a temporary node buffer before that buffer
/// is copied into its final position in the FBX stream.
#[cfg(feature = "write")]
fn rebase_node_offsets(data: &mut [u8], base_offset: u64) {
    fn visit(data: &mut [u8], node_start: usize, base_offset: u64) -> Option<usize> {
        if node_start.checked_add(25)? > data.len() {
            return None;
        }
        let local_end = u64::from_le_bytes(data[node_start..node_start + 8].try_into().ok()?);
        let local_end = usize::try_from(local_end).ok()?;
        if local_end <= node_start || local_end > data.len() {
            return None;
        }
        let property_len =
            u64::from_le_bytes(data[node_start + 16..node_start + 24].try_into().ok()?);
        let property_len = usize::try_from(property_len).ok()?;
        let name_len = data[node_start + 24] as usize;
        let mut child_start = node_start
            .checked_add(25)?
            .checked_add(name_len)?
            .checked_add(property_len)?;
        data[node_start..node_start + 8]
            .copy_from_slice(&(base_offset + local_end as u64).to_le_bytes());
        while child_start.checked_add(25)? <= local_end {
            if data[child_start..child_start + 25]
                .iter()
                .all(|byte| *byte == 0)
            {
                break;
            }
            child_start = visit(data, child_start, base_offset)?;
        }
        Some(local_end)
    }
    let _ = visit(data, 0, base_offset);
}

#[cfg(feature = "write")]
#[derive(Clone)]
#[allow(dead_code)]
enum FbxProp {
    I32(i32),
    I64(i64),
    F64(f64),
    String(String),
    F64Array(Vec<f64>),
    I32Array(Vec<i32>),
}

#[cfg(feature = "write")]
fn write_property<W: Write>(writer: &mut W, prop: &FbxProp) {
    match prop {
        FbxProp::I32(v) => {
            let _ = writer.write_all(b"I");
            let _ = writer.write_all(&v.to_le_bytes());
        }
        FbxProp::I64(v) => {
            let _ = writer.write_all(b"L");
            let _ = writer.write_all(&v.to_le_bytes());
        }
        FbxProp::F64(v) => {
            let _ = writer.write_all(b"D");
            let _ = writer.write_all(&v.to_le_bytes());
        }
        FbxProp::String(s) => {
            let _ = writer.write_all(b"S");
            let _ = writer.write_all(&(s.len() as u32).to_le_bytes());
            let _ = writer.write_all(s.as_bytes());
        }
        FbxProp::F64Array(arr) => {
            let _ = writer.write_all(b"d");
            let _ = writer.write_all(&(arr.len() as u32).to_le_bytes());
            let _ = writer.write_all(&0u32.to_le_bytes());
            let _ = writer.write_all(&((arr.len() * 8) as u32).to_le_bytes());
            for v in arr {
                let _ = writer.write_all(&v.to_le_bytes());
            }
        }
        FbxProp::I32Array(arr) => {
            let _ = writer.write_all(b"i");
            let _ = writer.write_all(&(arr.len() as u32).to_le_bytes());
            let _ = writer.write_all(&0u32.to_le_bytes());
            let _ = writer.write_all(&((arr.len() * 4) as u32).to_le_bytes());
            for v in arr {
                let _ = writer.write_all(&v.to_le_bytes());
            }
        }
    }
}

#[cfg(feature = "write")]
fn write_header_extension<W: Write + Seek>(writer: &mut W, version: u32) {
    let mut children: Vec<Vec<u8>> = Vec::new();
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "FBXHeaderVersion",
        &[FbxProp::I32(1003)],
        &[],
    );
    children.push(buf);
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "FBXVersion",
        &[FbxProp::I32(version as i32)],
        &[],
    );
    children.push(buf);
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "Creator",
        &[FbxProp::String("draco-io WASM".to_string())],
        &[],
    );
    children.push(buf);
    write_node_with_children(writer, "FBXHeaderExtension", &[], &children);
}

#[cfg(feature = "write")]
fn write_global_settings<W: Write + Seek>(writer: &mut W) {
    let mut children = Vec::new();
    let mut version = Vec::new();
    write_node(
        &mut Cursor::new(&mut version),
        "Version",
        &[FbxProp::I32(1000)],
        &[],
    );
    children.push(version);
    let mut properties = Vec::new();
    let mut property_children = Vec::new();
    for (name, value) in [
        ("UpAxis", 1),
        ("UpAxisSign", 1),
        ("FrontAxis", 2),
        ("FrontAxisSign", 1),
        ("CoordAxis", 0),
        ("CoordAxisSign", 1),
    ] {
        let mut property = Vec::new();
        write_node(
            &mut Cursor::new(&mut property),
            "P",
            &[
                FbxProp::String(name.to_string()),
                FbxProp::String("int".to_string()),
                FbxProp::String("Integer".to_string()),
                FbxProp::String(String::new()),
                FbxProp::I32(value),
            ],
            &[],
        );
        property_children.push(property);
    }
    // FBX documents its base unit as the centimeter; Blender's importer
    // applies a `UnitScaleFactor / 100` multiplier. 100.0 makes the file
    // round-trip at its true (meter) scale; the legacy value 1.0 caused
    // every exported scene to come in 100x too small.
    for name in ["UnitScaleFactor", "OriginalUnitScaleFactor"] {
        let mut property = Vec::new();
        write_node(
            &mut Cursor::new(&mut property),
            "P",
            &[
                FbxProp::String(name.to_string()),
                FbxProp::String("double".to_string()),
                FbxProp::String("Number".to_string()),
                FbxProp::String(String::new()),
                FbxProp::F64(100.0),
            ],
            &[],
        );
        property_children.push(property);
    }
    write_node_with_children(
        &mut Cursor::new(&mut properties),
        "Properties70",
        &[],
        &property_children,
    );
    children.push(properties);
    write_node_with_children(writer, "GlobalSettings", &[], &children);
}

#[cfg(feature = "write")]
fn write_documents<W: Write + Seek>(writer: &mut W) {
    let mut children: Vec<Vec<u8>> = Vec::new();
    let mut buf = Vec::new();
    write_node(&mut Cursor::new(&mut buf), "Count", &[FbxProp::I64(1)], &[]);
    children.push(buf);
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "Document",
        &[
            FbxProp::I64(1),
            FbxProp::String("Scene".to_string()),
            FbxProp::String("Scene".to_string()),
        ],
        &[],
    );
    children.push(buf);
    write_node_with_children(writer, "Documents", &[], &children);
}

#[cfg(feature = "write")]
fn write_definitions<W: Write + Seek>(writer: &mut W, mesh_count: usize) {
    let mut children: Vec<Vec<u8>> = Vec::new();
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "Version",
        &[FbxProp::I64(100)],
        &[],
    );
    children.push(buf);
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "Count",
        &[FbxProp::I64((mesh_count * 2) as i64)],
        &[],
    );
    children.push(buf);
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "ObjectType",
        &[FbxProp::String("Geometry".to_string())],
        &[],
    );
    children.push(buf);
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "ObjectType",
        &[FbxProp::String("Model".to_string())],
        &[],
    );
    children.push(buf);
    write_node_with_children(writer, "Definitions", &[], &children);
}

#[cfg(feature = "write")]
fn write_geometry<W: Write + Seek>(writer: &mut W, mesh: &MeshInput, id: i64) {
    let name = mesh.name.as_deref().unwrap_or("Mesh");
    let full_name = format!("{}\0\x01Geometry", name);
    let mut children: Vec<Vec<u8>> = Vec::new();
    let vertices: Vec<f64> = mesh.positions.iter().map(|&v| v as f64).collect();
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "Vertices",
        &[FbxProp::F64Array(vertices)],
        &[],
    );
    children.push(buf);
    let mut polygon_indices: Vec<i32> = Vec::new();
    for chunk in mesh.indices.chunks(3) {
        if chunk.len() == 3 {
            polygon_indices.push(chunk[0] as i32);
            polygon_indices.push(chunk[1] as i32);
            polygon_indices.push(!(chunk[2] as i32));
        }
    }
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "PolygonVertexIndex",
        &[FbxProp::I32Array(polygon_indices)],
        &[],
    );
    children.push(buf);
    write_node_with_children(
        writer,
        "Geometry",
        &[
            FbxProp::I64(id),
            FbxProp::String(full_name),
            FbxProp::String("Mesh".to_string()),
        ],
        &children,
    );
}

#[cfg(feature = "write")]
fn write_model<W: Write + Seek>(writer: &mut W, mesh: &MeshInput, id: i64) {
    let name = mesh.name.as_deref().unwrap_or("Model");
    let full_name = format!("{}\0\x01Model", name);
    let mut children = Vec::new();
    for (node_name, property) in [
        ("Version", FbxProp::I32(232)),
        ("Shading", FbxProp::I32(1)),
        ("Culling", FbxProp::String("CullingOff".to_string())),
    ] {
        let mut child = Vec::new();
        write_node(&mut Cursor::new(&mut child), node_name, &[property], &[]);
        children.push(child);
    }
    let mut properties = Vec::new();
    write_node(&mut Cursor::new(&mut properties), "Properties70", &[], &[]);
    children.insert(1, properties);
    write_node_with_children(
        writer,
        "Model",
        &[
            FbxProp::I64(id),
            FbxProp::String(full_name),
            FbxProp::String("Mesh".to_string()),
        ],
        &children,
    );
}

#[cfg(feature = "write")]
fn write_connection<W: Write + Seek>(writer: &mut W, child_id: i64, parent_id: i64) {
    write_node(
        writer,
        "C",
        &[
            FbxProp::String("OO".to_string()),
            FbxProp::I64(child_id),
            FbxProp::I64(parent_id),
        ],
        &[],
    );
}

#[cfg(feature = "write")]
fn write_footer<W: Write>(writer: &mut W, version: u32) {
    let footer_id = [
        0xF8, 0x5A, 0x8C, 0x6A, 0xDE, 0xF5, 0xD9, 0x7E, 0xEC, 0xE9, 0x0C, 0xE3, 0x75, 0x8F, 0x29,
        0x0B,
    ];
    let _ = writer.write_all(&[0u8; 4]);
    let _ = writer.write_all(&footer_id);
    let _ = writer.write_all(&[0u8; 4]);
    let _ = writer.write_all(&version.to_le_bytes());
    let _ = writer.write_all(&[0u8; 120]);
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(all(test, feature = "read"))]
mod reader_tests {
    use super::*;

    #[test]
    fn parse_fbx_round_trips_through_scene() {
        // Build a minimal scene (one triangle) via the writer, then parse it
        // back through the shared reader to verify the WASM glue.
        let scene = FbxScene {
            root_nodes: vec![FbxSceneNode {
                id: FbxNodeId(1),
                name: Some("Root".to_string()),
                transform: None,
                mesh_instances: vec![FbxMeshInstance {
                    name: Some("Triangle".to_string()),
                    mesh: triangle_mesh(),
                    control_points: Vec::new(),
                    polygon_vertex_indices: Vec::new(),
                    uv_sets: Vec::new(),
                    normal_sets: Vec::new(),
                    material_indices: Vec::new(),
                    skin: None,
                    morph_targets: Vec::new(),
                }],
                children: Vec::new(),
            }],
            materials: Vec::new(),
            textures: Vec::new(),
            animations: Vec::new(),
            warnings: Vec::new(),
        };
        let bytes = scene.to_bytes().expect("write scene");
        let result = parse_fbx_scene(&bytes);
        assert!(result.success, "{:?}", result.error);
        assert_eq!(result.meshes.len(), 1);
        assert_eq!(result.meshes[0].positions.len(), 9);
        assert_eq!(result.meshes[0].indices, vec![0, 1, 2]);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }

    #[test]
    fn parse_fbx_exposes_material_and_animation_outputs() {
        let scene = FbxScene {
            root_nodes: vec![FbxSceneNode {
                id: FbxNodeId(1),
                name: Some("Root".to_string()),
                transform: None,
                mesh_instances: Vec::new(),
                children: Vec::new(),
            }],
            materials: vec![draco_io::FbxMaterial {
                name: Some("Red".to_string()),
                shading_model: Some("Phong".to_string()),
                diffuse: Some([1.0, 0.0, 0.0]),
                specular: None,
                emissive: None,
                ambient: None,
                diffuse_factor: None,
                specular_factor: None,
                shininess: Some(20.0),
                emissive_factor: None,
                reflection_factor: None,
                transparency_factor: None,
                opacity: None,
                bump_factor: None,
                textures: Vec::new(),
            }],
            textures: Vec::new(),
            animations: vec![draco_io::FbxAnimation {
                name: Some("Take".to_string()),
                duration: 1.0,
                channels: vec![draco_io::FbxAnimChannel {
                    node_id: FbxNodeId(1),
                    node_name: "Root".to_string(),
                    path: draco_io::FbxAnimChannelPath::Translation,
                    morph_target_index: None,
                    sampler: draco_io::FbxAnimSampler {
                        input: vec![0.0, 1.0],
                        output: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                        interpolation: draco_io::FbxAnimInterpolation::Linear,
                        in_tangents: None,
                        out_tangents: None,
                    },
                }],
            }],
            warnings: Vec::new(),
        };
        let bytes = scene.to_bytes().expect("write scene");
        let result = parse_fbx_scene(&bytes);
        assert!(result.success, "{:?}", result.error);
        assert_eq!(result.materials.len(), 1);
        assert_eq!(result.materials[0].name.as_deref(), Some("Red"));
        assert_eq!(result.materials[0].shading_model.as_deref(), Some("Phong"));
        assert_eq!(result.animations.len(), 1);
        assert_eq!(result.animations[0].name.as_deref(), Some("Take"));
        assert_eq!(result.animations[0].channels.len(), 1);
        assert_eq!(
            result.animations[0].channels[0].sampler.input,
            vec![0.0, 1.0]
        );
    }

    fn triangle_mesh() -> Mesh {
        let positions = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let point_count = positions.len() / 3;
        let mut mesh = Mesh::new();
        mesh.set_num_points(point_count);
        let mut position = PointAttribute::new();
        position.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            point_count,
        );
        for (index, chunk) in positions.chunks_exact(3).enumerate() {
            let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
            position.buffer_mut().write(index * 12, &bytes);
        }
        mesh.add_attribute(position);
        mesh.set_num_faces(1);
        mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
        mesh
    }
}

#[cfg(all(test, feature = "write"))]
mod writer_tests {
    use super::*;

    #[test]
    fn test_create_simple_fbx() {
        let mesh = MeshInput {
            name: Some("Triangle".to_string()),
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0],
            indices: vec![0, 1, 2],
            normals: None,
            uvs: None,
            control_points: None,
            polygon_vertex_indices: None,
            uv_sets: Vec::new(),
            normal_sets: Vec::new(),
            material_indices: Vec::new(),
            skin: None,
            morph_targets: Vec::new(),
        };

        let result = create_fbx_internal(&[mesh]);
        assert!(result.success);
        assert!(result.binary_data.is_some());

        let data = result.binary_data.unwrap();
        assert!(data.len() > 27);
        assert_eq!(&data[0..21], FBX_MAGIC);
    }
}
