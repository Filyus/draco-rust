//! Shared, lossy FBX scene values.

use std::fmt;
use std::io;

use draco_core::mesh::Mesh;

/// What kind of deviation or loss a [`FbxWarning`] describes.
///
/// Codes are stable identifiers; the human-readable text lives on the warning
/// itself and may be reworded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum FbxWarningCode {
    /// A terminator record carried non-zero property fields.
    MalformedNullRecord,
    /// A named node declared no end offset, so it was read without children.
    MissingNodeEndOffset,
    /// A node's property list ended somewhere other than its header declared.
    PropertyListLengthMismatch,
    /// A node record claimed to end past the end of the file.
    NodeEndPastEndOfFile,
    /// A Model used an FBX inheritance rule the imported transform cannot
    /// express, so its local TRS is missing that rule.
    UnsupportedTransformInherit,
    /// A layer element used a mapping or reference mode that could not be
    /// resolved, so default values were substituted.
    UnsupportedLayerMapping,
    /// A geometry carried a `LayerElement*` this crate does not import, so
    /// that layer's data is absent from the decoded scene.
    DroppedLayerElement,
    /// The document uses the pre-7000 object model, which identifies objects
    /// and connections by name instead of by id, so nothing was imported from
    /// its `Objects` block.
    NameKeyedObjectModel,
    /// A node carried a `NodeAttribute` class this crate does not represent,
    /// so the attribute's own properties are absent from the scene.
    DroppedNodeAttribute,
    /// A Model reached neither the document root nor a parent Model by
    /// object connection, so it is not part of the decoded scene graph --
    /// the same objects the source file kept out of its scene.
    UnconnectedModelDropped,
}

impl FbxWarningCode {
    /// Whether data present in the file is absent from the decoded scene.
    ///
    /// Container-layout notices describe a tolerated deviation but no loss;
    /// the semantic codes describe something the caller will not find in the
    /// result. A converter can use this to decide what to surface downstream.
    pub fn is_data_loss(self) -> bool {
        match self {
            FbxWarningCode::MalformedNullRecord
            | FbxWarningCode::PropertyListLengthMismatch
            | FbxWarningCode::NodeEndPastEndOfFile => false,
            FbxWarningCode::MissingNodeEndOffset
            | FbxWarningCode::UnsupportedTransformInherit
            | FbxWarningCode::UnsupportedLayerMapping
            | FbxWarningCode::DroppedLayerElement
            | FbxWarningCode::NameKeyedObjectModel
            | FbxWarningCode::DroppedNodeAttribute
            | FbxWarningCode::UnconnectedModelDropped => true,
        }
    }

    /// Stable machine-readable slug, for logs and downstream reports.
    pub fn as_str(self) -> &'static str {
        match self {
            FbxWarningCode::MalformedNullRecord => "malformed-null-record",
            FbxWarningCode::MissingNodeEndOffset => "missing-node-end-offset",
            FbxWarningCode::PropertyListLengthMismatch => "property-list-length-mismatch",
            FbxWarningCode::NodeEndPastEndOfFile => "node-end-past-end-of-file",
            FbxWarningCode::UnsupportedTransformInherit => "unsupported-transform-inherit",
            FbxWarningCode::UnsupportedLayerMapping => "unsupported-layer-mapping",
            FbxWarningCode::DroppedLayerElement => "dropped-layer-element",
            FbxWarningCode::NameKeyedObjectModel => "name-keyed-object-model",
            FbxWarningCode::DroppedNodeAttribute => "dropped-node-attribute",
            FbxWarningCode::UnconnectedModelDropped => "unconnected-model-dropped",
        }
    }
}

/// One non-fatal notice raised while reading an FBX document.
///
/// Occurrences are collapsed by `(code, subject)`: a malformed pattern
/// repeated across thousands of nodes yields one warning with a count, not
/// thousands of identical strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FbxWarning {
    /// Stable classification of the notice.
    pub code: FbxWarningCode,
    /// Human-readable description.
    pub message: String,
    /// Owning FBX object name, when one is known.
    pub subject: Option<String>,
    /// How many times this `(code, subject)` pair fired. Never zero.
    pub count: u32,
}

impl FbxWarning {
    /// Creates a warning that has fired once.
    pub fn new(code: FbxWarningCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            subject: None,
            count: 1,
        }
    }

    /// Attaches the FBX object this notice is about.
    #[must_use]
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }
}

/// Appends a warning, collapsing repeats of the same `(code, subject)` pair
/// into a single entry with a count.
///
/// Without this, a malformed pattern repeated across every node in a large
/// file produces thousands of identical strings and buries anything else.
///
/// Lives beside [`FbxWarning`] rather than in either reader half because both
/// the container decoder and the scene layer raise notices. Both of those are
/// read-side, so a writer-only build has no caller for it.
#[cfg(feature = "fbx-reader")]
pub(crate) fn push_warning(
    warnings: &mut Vec<FbxWarning>,
    code: FbxWarningCode,
    message: String,
    subject: Option<&str>,
) {
    if let Some(existing) = warnings
        .iter_mut()
        .find(|warning| warning.code == code && warning.subject.as_deref() == subject)
    {
        existing.count = existing.count.saturating_add(1);
        return;
    }
    let mut warning = FbxWarning::new(code, message);
    warning.subject = subject.map(str::to_owned);
    warnings.push(warning);
}

impl fmt::Display for FbxWarning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)?;
        if self.count > 1 {
            write!(formatter, " (x{})", self.count)?;
        }
        Ok(())
    }
}

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

/// Source Model transform-stack properties required to reproduce FBX local
/// animation semantics. Values remain in authored FBX units and degrees.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FbxTransformStack {
    /// `Lcl Translation` in source units.
    pub translation: Option<[f32; 3]>,
    /// `Lcl Rotation` in degrees.
    pub rotation: Option<[f32; 3]>,
    /// `Lcl Scaling`.
    pub scaling: Option<[f32; 3]>,
    /// FBX `RotationOrder` enum value.
    pub rotation_order: Option<i32>,
    /// FBX `RotationActive` flag. Its absence is distinct from `false` in a
    /// source-provenance export because the Model template supplies defaults.
    pub rotation_active: Option<bool>,
    /// `PreRotation` in degrees.
    pub pre_rotation: Option<[f32; 3]>,
    /// `PostRotation` in degrees.
    pub post_rotation: Option<[f32; 3]>,
    /// `RotationOffset` in source units.
    pub rotation_offset: Option<[f32; 3]>,
    /// `RotationPivot` in source units.
    pub rotation_pivot: Option<[f32; 3]>,
    /// `ScalingOffset` in source units.
    pub scaling_offset: Option<[f32; 3]>,
    /// `ScalingPivot` in source units.
    pub scaling_pivot: Option<[f32; 3]>,
    /// FBX `InheritType` enum value.
    pub inherit_type: Option<i32>,
}

/// Source FBX global coordinate, unit, and display-time settings retained for
/// FBX-to-FBX provenance exports. This is intentionally not part of the
/// portable SceneDocument contract.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FbxGlobalSettings {
    /// FBX `UpAxis` enum value.
    pub up_axis: Option<i32>,
    /// FBX `UpAxisSign` value.
    pub up_axis_sign: Option<i32>,
    /// FBX `FrontAxis` enum value.
    pub front_axis: Option<i32>,
    /// FBX `FrontAxisSign` value.
    pub front_axis_sign: Option<i32>,
    /// FBX `CoordAxis` enum value.
    pub coord_axis: Option<i32>,
    /// FBX `CoordAxisSign` value.
    pub coord_axis_sign: Option<i32>,
    /// FBX `UnitScaleFactor` value.
    pub unit_scale_factor: Option<f64>,
    /// FBX `OriginalUnitScaleFactor` value.
    pub original_unit_scale_factor: Option<f64>,
    /// FBX `TimeMode` enum value.
    pub time_mode: Option<i32>,
}

/// The layer elements preserved from one FBX geometry node.
///
/// Grouped rather than spread across [`FbxMeshInstance`] because they are one
/// kind of thing that grows together: every format capability added so far
/// arrived as another layer family, and each one that lands as a bare field
/// has to be threaded through every construction site and every signature
/// that carries geometry. [`crate::FbxGeometryLayers`] is the borrowed view of
/// this plus the positions and indices it indexes into.
#[derive(Debug, Clone, Default)]
pub struct FbxMeshLayers {
    /// Original UV layer elements, including mapping/reference information.
    pub uv_sets: Vec<FbxUvSet>,
    /// Original normal layer elements, including mapping/reference information.
    pub normal_sets: Vec<FbxNormalSet>,
    /// Original colour layer elements, including mapping/reference information.
    pub color_sets: Vec<FbxColorSet>,
    /// Original tangent layer elements, with handedness merged into `w`.
    ///
    /// Draco has no tangent attribute, so these never reach
    /// [`FbxMeshInstance::mesh`]; they travel on the instance and through
    /// [`crate::FbxRenderMesh`] instead, the same way extra UV sets do.
    pub tangent_sets: Vec<FbxTangentSet>,
    /// Original binormal layer elements.
    ///
    /// Derivable from the normal and tangent, and absent from glTF, so these
    /// exist only so an FBX document survives a rewrite unchanged.
    pub binormal_sets: Vec<FbxBinormalSet>,
    /// Original `LayerElementSmoothing` layers.
    ///
    /// Hard and soft edges. glTF has no equivalent, so these survive an
    /// FBX-to-FBX rewrite but do not travel further. A layer whose length does
    /// not match the domain its mapping names is dropped with a warning rather
    /// than kept as misaligned data.
    pub smoothing_layers: Vec<FbxSmoothingLayer>,
    /// Original `LayerElementEdgeCrease` and `LayerElementVertexCrease` layers.
    pub crease_layers: Vec<FbxCreaseLayer>,
}

/// Geometry attached to one [`FbxSceneNode`].
///
/// This is materialized Draco geometry, not a lossless FBX geometry object.
#[derive(Debug, Clone, Default)]
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
    /// Layer elements preserved from the source geometry node.
    pub layers: FbxMeshLayers,
    /// Original FBX `Edges` array, verbatim.
    ///
    /// Each entry indexes [`Self::polygon_vertex_indices`], naming the polygon
    /// corner an edge starts at. FBX does not require this to list every
    /// topological edge -- importers reconstruct the missing ones from faces --
    /// so it is kept raw rather than normalized. It is also the domain
    /// `ByEdge` layer elements address.
    pub edges: Vec<i32>,
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

/// A preserved FBX layer element carrying `N` float components per value.
///
/// Every float-valued FBX layer element has this shape -- a name, a mapping
/// and reference mode, a value array, and an optional index array -- and
/// differs only in its component count and the node names it is read from.
/// The per-family aliases below name the ones this crate understands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FbxLayerSet<const N: usize> {
    /// FBX layer set name.
    pub name: Option<String>,
    /// FBX mapping information type, e.g. `ByPolygonVertex` or `ByVertice`.
    pub mapping: Option<String>,
    /// FBX reference information type, e.g. `Direct` or `IndexToDirect`.
    pub reference: Option<String>,
    /// Direct values.
    pub values: Vec<[f32; N]>,
    /// Optional direct-value indices, used when `reference` is `IndexToDirect`.
    pub indices: Vec<i32>,
}

/// A preserved FBX `LayerElementUV`.
pub type FbxUvSet = FbxLayerSet<2>;

/// A preserved FBX `LayerElementNormal`.
pub type FbxNormalSet = FbxLayerSet<3>;

/// A preserved FBX `LayerElementColor`.
///
/// FBX normally stores four components; a three-component source is padded
/// with an opaque alpha when read.
pub type FbxColorSet = FbxLayerSet<4>;

/// A preserved FBX `LayerElementTangent` or `LayerElementBinormal`.
///
/// FBX splits these across two sibling arrays: `Tangents` holds three
/// components and the handedness sign lives in a separate `TangentsW`, which
/// only FBX 7500 and later write. They are merged into one four-component
/// value here because that is the form glTF's `TANGENT` needs, and split again
/// on write.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FbxTangentSet {
    /// Tangent vectors, with the handedness sign in `w`.
    pub layer: FbxLayerSet<4>,
    /// Whether the source carried an explicit handedness array.
    ///
    /// When it did not, `w` was defaulted to `+1.0`. The writer emits the
    /// sibling array only when this is set, so a document that had no
    /// handedness does not gain one by being rewritten.
    pub has_handedness: bool,
}

/// A preserved FBX `LayerElementBinormal`.
///
/// Structurally identical to a tangent set, and always written alongside one:
/// no corpus file carries either alone.
pub type FbxBinormalSet = FbxTangentSet;

/// What a `NodeAttribute` attached to a scene node describes.
///
/// Only the two classes this crate represents appear here. Others are reported
/// through [`FbxWarningCode::DroppedNodeAttribute`] rather than given a variant
/// that would carry nothing.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum FbxNodeAttribute {
    /// A `Camera` attribute.
    Camera(FbxCamera),
    /// A `Light` attribute.
    Light(FbxLight),
}

/// An FBX `Camera` node attribute.
///
/// Every field is optional because FBX omits any property left at its class
/// default. Fields are limited to those that actually occur across the `ufbx`
/// corpus; angles are in degrees and distances in the document's own units.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct FbxCamera {
    /// Eye position, in world space rather than relative to the node.
    pub position: Option<[f32; 3]>,
    /// Point the camera looks at, in the same space as [`Self::position`].
    pub interest_position: Option<[f32; 3]>,
    /// Up vector.
    pub up_vector: Option<[f32; 3]>,
    /// `CameraProjectionType`: 0 perspective, 1 orthographic.
    pub projection_type: Option<i32>,
    /// Diagonal field of view, in degrees.
    pub field_of_view: Option<f32>,
    /// Horizontal field of view, in degrees.
    pub field_of_view_x: Option<f32>,
    /// Vertical field of view, in degrees.
    pub field_of_view_y: Option<f32>,
    /// Focal length in millimetres.
    pub focal_length: Option<f32>,
    /// Near clip distance.
    pub near_plane: Option<f32>,
    /// Far clip distance.
    pub far_plane: Option<f32>,
    /// Render aperture width in pixels.
    pub aspect_width: Option<f32>,
    /// Render aperture height in pixels.
    pub aspect_height: Option<f32>,
    /// Film-back width in **inches**, not millimetres.
    ///
    /// This is the sensor size, and a consumer needs it with
    /// [`Self::focal_length`] to reach a field of view: Blender computes
    /// `sensor_width = film_width * 25.4` and falls back to its own 32 mm
    /// default when the property is absent, which silently changes the framing
    /// of every camera in the document.
    pub film_width: Option<f32>,
    /// Film-back height in inches.
    pub film_height: Option<f32>,
    /// Film-back aspect ratio, `film_width / film_height`.
    pub film_aspect_ratio: Option<f32>,
    /// `ApertureMode`: which of the aperture and field-of-view properties the
    /// authoring tool treats as authoritative when they disagree.
    pub aperture_mode: Option<i32>,
    /// Orthographic zoom, meaningful when [`Self::projection_type`] is 1.
    pub ortho_zoom: Option<f32>,
}

/// An FBX `Light` node attribute.
///
/// As with [`FbxCamera`], every field is optional and the set is limited to
/// what the corpus contains. Notably no file carries `InnerAngle` or
/// `OuterAngle`, so spot cone angles are not represented.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct FbxLight {
    /// `LightType`: 0 point, 1 directional, 2 spot, 3 area, 4 volume.
    pub light_type: Option<i32>,
    /// Linear RGB colour.
    pub color: Option<[f32; 3]>,
    /// Intensity, where 100 is FBX's unit brightness.
    pub intensity: Option<f32>,
    /// Whether the light contributes at all.
    pub cast_light: Option<bool>,
    /// Whether the light casts shadows.
    pub cast_shadows: Option<bool>,
    /// `DecayType`: 0 none, 1 linear, 2 quadratic, 3 cubic.
    pub decay_type: Option<i32>,
    /// Distance at which decay begins.
    pub decay_start: Option<f32>,
}

/// A preserved FBX `LayerElementSmoothing`.
///
/// Smoothing is an integer flag per edge or per polygon -- whether the edge is
/// soft, or the polygon smooth-shaded -- and is kept separate from
/// [`FbxCreaseLayer`] because that one is a floating-point weight. Rounding one
/// through the other's type would quietly change authored crease values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FbxSmoothingLayer {
    /// FBX mapping information type: `ByEdge` or `ByPolygon`.
    pub mapping: Option<String>,
    /// One flag per edge or per polygon, matching `mapping`.
    pub values: Vec<i32>,
}

/// Which domain a [`FbxCreaseLayer`] sharpens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbxCreaseKind {
    /// `LayerElementEdgeCrease`, one weight per entry in
    /// [`FbxMeshInstance::edges`].
    Edge,
    /// `LayerElementVertexCrease`, one weight per control point.
    Vertex,
}

/// A preserved FBX `LayerElementEdgeCrease` or `LayerElementVertexCrease`.
#[derive(Debug, Clone, PartialEq)]
pub struct FbxCreaseLayer {
    /// Whether this sharpens edges or control points.
    pub kind: FbxCreaseKind,
    /// FBX mapping information type: `ByEdge` or `ByVertice`.
    pub mapping: Option<String>,
    /// Crease weights, normally in `0..=1`.
    pub values: Vec<f64>,
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

/// What an FBX `Model` record declared its node to be, beyond what the
/// geometry and attributes attached to it say.
///
/// Every exporter writes a joint's class on the Model itself, so that is the
/// signal this carries; the `Skeleton` `NodeAttribute` a joint usually also
/// carries adds nothing the scene keeps. It matters exactly where nothing
/// else says what the node is: a joint no skin cluster names — a bone's
/// `*_end` tail helper, which holds no weights — and a `Null` grouping node
/// such as an armature's root object. Without it both rewrite as plain mesh
/// Models, which is how a round trip turned a rig into joints Blender cannot
/// form a chain from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbxNodeKind {
    /// A `LimbNode` or `Limb` Model: one joint of a skeleton.
    Joint,
    /// A `Null` or `Root` Model: a transform-only grouping node, the class an
    /// armature's root object carries.
    Null,
}

/// A node in a hierarchy extracted from or written to FBX Model connections.
///
/// The supported raw Model transform stack is retained for source-provenance
/// FBX exports. Skin and blend-shape data lives on mesh instances.
#[derive(Debug, Clone)]
pub struct FbxSceneNode {
    /// Stable id used by skin clusters and animation channels.
    pub id: FbxNodeId,
    /// Name supplied by the FBX model node, when available.
    pub name: Option<String>,
    /// Supported local transform properties synthesized into a matrix.
    pub transform: Option<FbxTransform>,
    /// Optional authored FBX stack behind `transform`.
    pub transform_stack: Option<FbxTransformStack>,
    /// Whether the node's static local transform uses FBX rotation/pivot
    /// terms beyond plain local TRS. Consumers that only receive the lossy
    /// matrix can use the skin bind pose as the baked local basis for these
    /// nodes while preserving raw Model TRS for ordinary nodes.
    pub has_complex_transform_stack: bool,
    /// The Model's own class, when it declares the node to be something other
    /// than a mesh; see [`FbxNodeKind`].
    pub kind: Option<FbxNodeKind>,
    /// Geometry attached directly to this model node.
    pub mesh_instances: Vec<FbxMeshInstance>,
    /// Camera or light attached to this model node, when it carries one.
    pub attribute: Option<FbxNodeAttribute>,
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
            transform_stack: None,
            has_complex_transform_stack: false,
            kind: None,
            mesh_instances: Vec::new(),
            attribute: None,
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
    /// Source-only global settings used by compatible FBX re-export.
    pub global_settings: Option<FbxGlobalSettings>,
    /// Top-level FBX model nodes.
    pub root_nodes: Vec<FbxSceneNode>,
    /// Material objects, referenced by index from `mesh_instances` via
    /// `FbxMeshInstance::material_indices`.
    pub materials: Vec<FbxMaterial>,
    /// Texture objects, referenced by index from `FbxMaterial::textures`.
    pub textures: Vec<FbxTexture>,
    /// Animation takes (one per `AnimationStack` + first `AnimationLayer`).
    pub animations: Vec<FbxAnimation>,
    /// Non-fatal notices collected while reading: tolerated container-layout
    /// deviations, and FBX semantics the decoded scene cannot express.
    ///
    /// Filter on [`FbxWarningCode::is_data_loss`] to separate "this file is
    /// unusual" from "something in this file is missing from the result".
    pub warnings: Vec<FbxWarning>,
}

impl FbxScene {
    /// Reads a supported FBX scene from binary bytes.
    #[cfg(feature = "fbx-reader")]
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let mut reader = crate::fbx_reader::FbxMemoryReader::from_bytes(bytes)?;
        reader.read_scene()
    }

    /// Reads a supported FBX scene from binary bytes with explicit options.
    ///
    /// Use this to tighten [`crate::FbxDecodeLimits`] for untrusted input, or
    /// to enable strict container validation.
    #[cfg(feature = "fbx-reader")]
    pub fn from_bytes_with_options(
        bytes: &[u8],
        options: crate::fbx_options::FbxReadOptions,
    ) -> io::Result<Self> {
        let mut reader =
            crate::fbx_reader::FbxMemoryReader::from_bytes_with_options(bytes, options)?;
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

    /// Writes this scene as ASCII FBX text.
    ///
    /// The same document as [`Self::to_bytes`], spelled as text rather than as
    /// records: diffable, and readable by any FBX importer, at the cost of the
    /// precision [`crate::fbx_writer::FbxFormat`] describes.
    #[cfg(feature = "fbx-writer")]
    pub fn to_ascii_bytes(&self) -> io::Result<Vec<u8>> {
        let mut writer =
            crate::fbx_writer::FbxWriter::new().with_format(crate::fbx_writer::FbxFormat::Ascii);
        writer.add_scene(self)?;
        writer.write_to_vec()
    }
}
