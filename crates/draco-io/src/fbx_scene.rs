//! Shared, lossy FBX scene values.

use std::io;

use draco_core::mesh::Mesh;

/// Stable, document-local identifier for an FBX model node.
///
/// FBX names are not unique. Scene relationships and animation targets use
/// this 32-bit identifier, which is also safe to transfer through JavaScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FbxNodeId(pub u32);

/// Local transform extracted from or written to an FBX model node.
///
/// This matrix is synthesized from local translation, rotation, and scaling.
/// It does not represent FBX pivot or inheritance rules.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FbxTransform {
    /// Packed transform matrix compatible with the FBX 16-value layout.
    ///
    /// The outer arrays are emitted in order and can be passed directly to a
    /// column-major WebGL matrix. Translation therefore occupies
    /// `matrix[3][0..3]`.
    pub matrix: [[f32; 4]; 4],
}

/// Geometry attached to one [`FbxSceneNode`].
///
/// This is materialized Draco geometry, not a lossless FBX geometry object.
#[derive(Debug, Clone)]
pub struct FbxMeshInstance {
    /// Name supplied by the FBX geometry node, when available.
    pub name: Option<String>,
    /// Decoded mesh geometry.
    pub mesh: Mesh,
    /// Original FBX control-point positions, retained independently from the
    /// resolved render mesh used by Draco/WebGL.
    pub control_points: Vec<[f32; 3]>,
    /// Original FBX polygon-corner indices. Negative values terminate a face.
    pub polygon_vertex_indices: Vec<i32>,
    /// Original UV layer elements, including mapping/reference information.
    pub uv_sets: Vec<FbxUvSet>,
    /// Original normal layer elements, including mapping/reference information.
    pub normal_sets: Vec<FbxNormalSet>,
    /// Per-polygon material index from `LayerElementMaterial`, when present.
    ///
    /// Each entry corresponds to one triangle in fan-triangulation order
    /// produced by [`crate::FbxReader`]. Entries are absolute indices into
    /// [`FbxScene::materials`]. The list is empty when the geometry does not
    /// carry a material layer (callers fall back to the first material).
    pub material_indices: Vec<i32>,
    /// Skin binding for this geometry, when it is armature-deformed.
    pub skin: Option<FbxSkin>,
    /// Blend-shape targets defined for this geometry.
    pub morph_targets: Vec<FbxMorphTarget>,
}

/// A preserved FBX `LayerElementUV`.
#[derive(Debug, Clone, Default)]
pub struct FbxUvSet {
    /// FBX UV set name.
    pub name: Option<String>,
    /// FBX mapping information type.
    pub mapping: Option<String>,
    /// FBX reference information type.
    pub reference: Option<String>,
    /// Direct UV values.
    pub values: Vec<[f32; 2]>,
    /// Optional direct-value indices.
    pub indices: Vec<i32>,
}

/// A preserved FBX `LayerElementNormal`.
#[derive(Debug, Clone, Default)]
pub struct FbxNormalSet {
    /// FBX normal set name.
    pub name: Option<String>,
    /// FBX mapping information type.
    pub mapping: Option<String>,
    /// FBX reference information type.
    pub reference: Option<String>,
    /// Direct normal values.
    pub values: Vec<[f32; 3]>,
    /// Optional direct-value indices.
    pub indices: Vec<i32>,
}

/// All influences from one joint onto a mesh's control points.
#[derive(Debug, Clone)]
pub struct FbxSkinCluster {
    /// Joint Model that owns this cluster.
    pub joint_node_id: FbxNodeId,
    /// Affected mesh control-point indices.
    pub control_point_indices: Vec<u32>,
    /// Weight for each entry in [`Self::control_point_indices`].
    pub weights: Vec<f32>,
    /// Mesh global transform captured in the bind pose (`Transform`).
    pub mesh_bind_transform: FbxTransform,
    /// Joint global transform captured in the bind pose (`TransformLink`).
    pub joint_bind_transform: FbxTransform,
    /// Armature global transform captured in the bind pose
    /// (`TransformAssociateModel`), when supplied by the source.
    pub armature_bind_transform: Option<FbxTransform>,
}

/// Skinning data attached to a mesh instance.
#[derive(Debug, Clone)]
pub struct FbxSkin {
    /// All clusters, one for every influencing joint.
    pub clusters: Vec<FbxSkinCluster>,
    /// Explicit FBX BindPose matrices keyed by model id, when present.
    pub bind_pose: Vec<(FbxNodeId, FbxTransform)>,
}

/// One sparse FBX blend-shape target.
#[derive(Debug, Clone)]
pub struct FbxMorphTarget {
    /// Display name of the shape geometry.
    pub name: Option<String>,
    /// Control-point indices affected by this target.
    pub control_point_indices: Vec<u32>,
    /// Position deltas for those indices.
    pub position_deltas: Vec<[f32; 3]>,
    /// Normal deltas for those indices, when present.
    pub normal_deltas: Option<Vec<[f32; 3]>>,
    /// Default weight in percent.
    pub default_weight: f32,
    /// Full-deformation weight in percent.
    pub full_weight: f32,
}

/// A node in a hierarchy extracted from or written to FBX Model connections.
///
/// Pivot transforms and unsupported layer data are intentionally not
/// represented here. Skin and blend-shape data lives on mesh instances.
#[derive(Debug, Clone)]
pub struct FbxSceneNode {
    /// Stable id used by skin clusters and animation channels.
    pub id: FbxNodeId,
    /// Name supplied by the FBX model node, when available.
    pub name: Option<String>,
    /// Supported local transform properties synthesized into a matrix.
    pub transform: Option<FbxTransform>,
    /// Geometry attached directly to this model node.
    pub mesh_instances: Vec<FbxMeshInstance>,
    /// Child model nodes.
    pub children: Vec<FbxSceneNode>,
}

#[cfg(feature = "fbx-reader")]
impl FbxSceneNode {
    pub(crate) fn new(name: Option<String>) -> Self {
        Self {
            id: FbxNodeId(0),
            name,
            transform: None,
            mesh_instances: Vec::new(),
            children: Vec::new(),
        }
    }
}

/// Texture slot targeted by an [`FbxTextureBinding`].
///
/// These are the FBX property names commonly used to link a texture to a
/// material property. The mapping follows the conventions used by the FBX SDK
/// and Blender's `io_scene_fbx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbxTextureSlot {
    /// `DiffuseColor` / `DiffuseColor` texture link.
    Diffuse,
    /// `NormalMap` texture link.
    Normal,
    /// `EmissiveColor` texture link.
    Emissive,
    /// `SpecularColor` texture link.
    Specular,
    /// `Shininess` / roughness texture link.
    Roughness,
    /// `ReflectionFactor` / metallic texture link.
    Metallic,
    /// `AmbientColor` texture link.
    Ambient,
}

impl FbxTextureSlot {
    /// Returns the FBX property name used to wire a texture to a material.
    pub fn property_name(self) -> &'static str {
        match self {
            FbxTextureSlot::Diffuse => "DiffuseColor",
            FbxTextureSlot::Normal => "NormalMap",
            FbxTextureSlot::Emissive => "EmissiveColor",
            FbxTextureSlot::Specular => "SpecularColor",
            FbxTextureSlot::Roughness => "ShininessExponent",
            FbxTextureSlot::Metallic => "ReflectionFactor",
            FbxTextureSlot::Ambient => "AmbientColor",
        }
    }

    /// Parses the FBX connection property name into a slot, if recognized.
    pub fn from_property_name(name: &str) -> Option<Self> {
        match name {
            "DiffuseColor" => Some(FbxTextureSlot::Diffuse),
            "NormalMap" | "Bump" => Some(FbxTextureSlot::Normal),
            "EmissiveColor" => Some(FbxTextureSlot::Emissive),
            "SpecularColor" => Some(FbxTextureSlot::Specular),
            "ShininessExponent" | "Shininess" => Some(FbxTextureSlot::Roughness),
            "ReflectionFactor" => Some(FbxTextureSlot::Metallic),
            "AmbientColor" => Some(FbxTextureSlot::Ambient),
            _ => None,
        }
    }
}

/// Bind a [`FbxTexture`] to a slot of a material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbxTextureBinding {
    /// Material slot the texture feeds.
    pub slot: FbxTextureSlot,
    /// Index into [`FbxScene::textures`].
    pub texture_index: usize,
}

/// Texture object extracted from an FBX `Texture` / `Video` pair.
#[derive(Debug, Clone, Default)]
pub struct FbxTexture {
    /// Name supplied by the FBX texture node, when available.
    pub name: Option<String>,
    /// Embedded image bytes from `Video.Content` (PNG/JPG), when available.
    pub content: Option<Vec<u8>>,
    /// `RelativeFilename` / `FileName` from the FBX video/texture node.
    pub filename: Option<String>,
}

/// Material object extracted from an FBX `Material` node.
///
/// Covers the canonical `KFbxSurfacePhong` / `KFbxSurfaceLambert` property
/// set. Colors are linear RGB triples; scalar factors are unit-less.
#[derive(Debug, Clone, Default)]
pub struct FbxMaterial {
    /// Name supplied by the FBX material node, when available.
    pub name: Option<String>,
    /// `ShadingModel` (`"Phong"`, `"Lambert"`, or empty for PBR/unknown).
    pub shading_model: Option<String>,
    /// `DiffuseColor`.
    pub diffuse: Option<[f32; 3]>,
    /// `SpecularColor`.
    pub specular: Option<[f32; 3]>,
    /// `EmissiveColor`.
    pub emissive: Option<[f32; 3]>,
    /// `AmbientColor`.
    pub ambient: Option<[f32; 3]>,
    /// `DiffuseFactor`.
    pub diffuse_factor: Option<f32>,
    /// `SpecularFactor`.
    pub specular_factor: Option<f32>,
    /// `Shininess` (Phong exponent).
    pub shininess: Option<f32>,
    /// `EmissiveFactor`.
    pub emissive_factor: Option<f32>,
    /// `ReflectionFactor` (≈ metallic).
    pub reflection_factor: Option<f32>,
    /// `TransparencyFactor` (1 = fully transparent).
    pub transparency_factor: Option<f32>,
    /// `Opacity` (1 = fully opaque; alternate form of `TransparencyFactor`).
    pub opacity: Option<f32>,
    /// `BumpFactor`.
    pub bump_factor: Option<f32>,
    /// Texture links, indexed into [`FbxScene::textures`].
    pub textures: Vec<FbxTextureBinding>,
}

/// Animated TRS property of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FbxAnimChannelPath {
    /// `Lcl Translation`.
    Translation,
    /// `Lcl Rotation`.
    Rotation,
    /// `Lcl Scaling`.
    Scale,
    /// `DeformPercent` on a blend-shape channel.
    MorphWeight,
}

impl FbxAnimChannelPath {
    /// Returns the FBX property name this channel drives.
    pub fn property_name(self) -> &'static str {
        match self {
            FbxAnimChannelPath::Translation => "Lcl Translation",
            FbxAnimChannelPath::Rotation => "Lcl Rotation",
            FbxAnimChannelPath::Scale => "Lcl Scaling",
            FbxAnimChannelPath::MorphWeight => "DeformPercent",
        }
    }

    /// Parses an FBX connection property name into a channel path.
    pub fn from_property_name(name: &str) -> Option<Self> {
        match name {
            "Lcl Translation" => Some(FbxAnimChannelPath::Translation),
            "Lcl Rotation" => Some(FbxAnimChannelPath::Rotation),
            "Lcl Scaling" => Some(FbxAnimChannelPath::Scale),
            "DeformPercent" => Some(FbxAnimChannelPath::MorphWeight),
            _ => None,
        }
    }

    /// Number of output components for scalar and TRS paths.
    pub fn component_count(self) -> usize {
        match self {
            FbxAnimChannelPath::MorphWeight => 1,
            _ => 3,
        }
    }
}

/// Coarse interpolation kind decoded from `KeyAttrFlags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbxAnimInterpolation {
    /// Hold previous value.
    Step,
    /// Linear blend between keys.
    Linear,
    /// Cubic Hermite blend with explicit per-key tangents.
    Cubic,
}

impl FbxAnimInterpolation {
    /// Maps to the FBX `KeyAttrFlags` interpolation bits.
    pub fn to_key_attr_flags(self) -> i32 {
        match self {
            // ufbx / Blender FBX flag constants.
            FbxAnimInterpolation::Step => 0x2,
            FbxAnimInterpolation::Linear => 0x4,
            // Explicit user tangents avoid Blender recomputing an auto curve.
            FbxAnimInterpolation::Cubic => 0x8 | 0x400 | 0x800,
        }
    }

    /// Decodes the interpolation mode from a `KeyAttrFlags` entry.
    pub fn from_key_attr_flags(flags: i32) -> Self {
        if flags & 0x2 != 0 {
            FbxAnimInterpolation::Step
        } else if flags & 0x8 != 0 {
            FbxAnimInterpolation::Cubic
        } else {
            FbxAnimInterpolation::Linear
        }
    }
}

/// One animation sampler (a flat TRS or morph-weight track).
#[derive(Debug, Clone)]
pub struct FbxAnimSampler {
    /// Strictly increasing keyframe times in seconds.
    pub input: Vec<f32>,
    /// Flattened keyframe values, `component_count()` values per input entry.
    pub output: Vec<f32>,
    /// Coarse interpolation mode.
    pub interpolation: FbxAnimInterpolation,
    /// Flattened incoming cubic tangents in output units per second.
    pub in_tangents: Option<Vec<f32>>,
    /// Flattened outgoing cubic tangents in output units per second.
    pub out_tangents: Option<Vec<f32>>,
}

/// One animation channel: drives one TRS path or blend-shape weight.
#[derive(Debug, Clone)]
pub struct FbxAnimChannel {
    /// Stable target Model id; never resolve a channel by display name.
    pub node_id: FbxNodeId,
    /// Name of the target model node (matches [`FbxSceneNode::name`]).
    pub node_name: String,
    /// Which node or blend-shape property the channel drives.
    pub path: FbxAnimChannelPath,
    /// Blend-shape target slot for [`FbxAnimChannelPath::MorphWeight`].
    /// `None` for ordinary node TRS channels.
    pub morph_target_index: Option<u32>,
    /// Sampler data.
    pub sampler: FbxAnimSampler,
}

/// One animation take (derived from an `AnimationStack` + its first layer).
#[derive(Debug, Clone)]
pub struct FbxAnimation {
    /// Name of the `AnimationStack`, when available.
    pub name: Option<String>,
    /// Clip duration in seconds (max last sampler input).
    pub duration: f32,
    /// Flat list of TRS channels.
    pub channels: Vec<FbxAnimChannel>,
}

/// Hierarchy, geometry, materials, and animation extracted from or written to FBX.
///
/// Unlike `draco_gltf::Document`, this is a deliberately lossy format-specific
/// view. Use `FbxReader::read_nodes` when callers need the parsed FBX nodes.
#[derive(Debug, Clone, Default)]
pub struct FbxScene {
    /// Top-level FBX model nodes.
    pub root_nodes: Vec<FbxSceneNode>,
    /// Material objects, referenced by index from `mesh_instances` via
    /// `FbxMeshInstance::material_indices`.
    pub materials: Vec<FbxMaterial>,
    /// Texture objects, referenced by index from `FbxMaterial::textures`.
    pub textures: Vec<FbxTexture>,
    /// Animation takes (one per `AnimationStack` + first `AnimationLayer`).
    pub animations: Vec<FbxAnimation>,
    /// Non-fatal notices collected while reading, such as unsupported FBX
    /// pivot or inheritance-mode properties.
    pub warnings: Vec<String>,
}

impl FbxScene {
    /// Reads a supported FBX scene from binary bytes.
    #[cfg(feature = "fbx-reader")]
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let mut reader = crate::fbx_reader::FbxMemoryReader::from_bytes(bytes)?;
        reader.read_scene()
    }

    /// Writes this scene as binary FBX bytes.
    ///
    /// This method is available with the `fbx-writer` feature. It preserves
    /// mesh geometry, model names, hierarchy, local affine TRS transforms,
    /// materials, textures, and node-TRS animation.
    #[cfg(feature = "fbx-writer")]
    pub fn to_bytes(&self) -> io::Result<Vec<u8>> {
        let mut writer = crate::fbx_writer::FbxWriter::new();
        writer.add_scene(self)?;
        writer.write_to_vec()
    }
}
