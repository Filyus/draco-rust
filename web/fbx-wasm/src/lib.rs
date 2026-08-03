//! FBX reader and writer WASM module.
//!
//! Provides FBX binary parsing (FBX 7.x) and generation (FBX 7.5) for web
//! applications. The reader and writer are independent: build with
//! `--features read` or `--features write` (both are on by default) to control
//! which half of the API is exported.

use wasm_bindgen::prelude::*;

#[cfg(feature = "read")]
use js_sys::Uint16Array;
use js_sys::{
    Array, Float32Array, Float64Array, Int32Array, Object, Reflect, Uint32Array, Uint8Array,
};
#[cfg(feature = "write")]
use wasm_bindgen::JsCast;

// ===========================================================================
// JavaScript bridge
//
// Values cross the wasm boundary as plain JavaScript objects built and read by
// hand instead of serde structures: geometry goes over as typed arrays, which
// cross in bulk and give JavaScript an array it owns outright. The cost of
// dropping serde is that nothing here validates untrusted input for us, so
// every field read from JavaScript is guarded individually. Small fixed-size
// values (matrices, vec3s, per-triangle material assignments) stay plain
// arrays because the shell distinguishes them with `Array.isArray`.
// ===========================================================================

/// Set a property on a JavaScript object.
fn set_js(obj: &Object, key: &str, value: &JsValue) {
    let _ = Reflect::set(obj, &JsValue::from_str(key), value);
}

fn set_bool(obj: &Object, key: &str, value: bool) {
    set_js(obj, key, &JsValue::from_bool(value));
}

fn set_u32(obj: &Object, key: &str, value: u32) {
    set_js(obj, key, &JsValue::from_f64(value as f64));
}

#[cfg(feature = "read")]
fn set_i32(obj: &Object, key: &str, value: i32) {
    set_js(obj, key, &JsValue::from_f64(value as f64));
}

#[cfg(feature = "read")]
fn set_f64(obj: &Object, key: &str, value: f64) {
    set_js(obj, key, &JsValue::from_f64(value));
}

#[cfg(feature = "read")]
fn set_string_array(obj: &Object, key: &str, values: &[String]) {
    let array = Array::new();
    for value in values {
        array.push(&JsValue::from_str(value));
    }
    set_js(obj, key, &array.into());
}

#[cfg(feature = "read")]
fn f32_array_to_js(values: &[f32]) -> JsValue {
    Float32Array::from(values).into()
}

#[cfg(feature = "read")]
fn u32_array_to_js(values: &[u32]) -> JsValue {
    Uint32Array::from(values).into()
}

#[cfg(feature = "read")]
fn i32_array_to_js(values: &[i32]) -> JsValue {
    Int32Array::from(values).into()
}

#[cfg(feature = "read")]
fn u16_array_to_js(values: &[u16]) -> JsValue {
    Uint16Array::from(values).into()
}

#[cfg(feature = "read")]
fn f64_array_to_js(values: &[f64]) -> JsValue {
    Float64Array::from(values).into()
}

fn u8_array_to_js(values: &[u8]) -> JsValue {
    Uint8Array::from(values).into()
}

/// A small fixed-size value (matrix row, vec3 colour, ...) that the shell
/// checks with `Array.isArray`, so it crosses as a plain array, not a typed
/// one. The reader's own matrix fields keep this exact serde shape.
#[cfg(feature = "read")]
fn plain_f32_array_to_js(values: &[f32]) -> JsValue {
    let array = Array::new();
    for value in values {
        array.push(&JsValue::from_f64(*value as f64));
    }
    array.into()
}

/// Read a field that must be present on an object. An absent key reads back as
/// `undefined` from JavaScript, which is an error to be reported, not a value.
#[cfg(feature = "write")]
fn get_field(obj: &JsValue, key: &str) -> Option<JsValue> {
    if !obj.is_object() {
        return None;
    }
    match Reflect::get(obj, &JsValue::from_str(key)) {
        Ok(value) if value.is_undefined() || value.is_null() => None,
        Ok(value) => Some(value),
        Err(_) => None,
    }
}

/// Read a string field, tolerating an absent one.
#[cfg(feature = "write")]
fn opt_string_from_js(obj: &JsValue, key: &str) -> Option<String> {
    if !obj.is_object() {
        return None;
    }
    match Reflect::get(obj, &JsValue::from_str(key)) {
        Ok(value) if value.is_string() => value.as_string(),
        _ => None,
    }
}

#[cfg(feature = "write")]
fn opt_bool_from_js(obj: &JsValue, key: &str) -> Option<bool> {
    if !obj.is_object() {
        return None;
    }
    match Reflect::get(obj, &JsValue::from_str(key)) {
        Ok(value) => value.as_bool(),
        _ => None,
    }
}

#[cfg(feature = "write")]
fn opt_u32_from_js(obj: &JsValue, key: &str) -> Option<u32> {
    if !obj.is_object() {
        return None;
    }
    match Reflect::get(obj, &JsValue::from_str(key)) {
        Ok(value) => value.as_f64().map(|number| number as u32),
        _ => None,
    }
}

#[cfg(feature = "write")]
fn opt_i32_from_js(obj: &JsValue, key: &str) -> Option<i32> {
    if !obj.is_object() {
        return None;
    }
    match Reflect::get(obj, &JsValue::from_str(key)) {
        Ok(value) => value.as_f64().map(|number| number as i32),
        _ => None,
    }
}

#[cfg(feature = "write")]
fn opt_f64_from_js(obj: &JsValue, key: &str) -> Option<f64> {
    if !obj.is_object() {
        return None;
    }
    match Reflect::get(obj, &JsValue::from_str(key)) {
        Ok(value) => value.as_f64(),
        _ => None,
    }
}

#[cfg(feature = "write")]
fn f32_array_from_js(value: &JsValue, field: &str) -> Result<Vec<f32>, String> {
    if let Some(typed) = value.dyn_ref::<Float32Array>() {
        return Ok(typed.to_vec());
    }
    if let Some(typed) = value.dyn_ref::<Float64Array>() {
        return Ok(typed
            .to_vec()
            .into_iter()
            .map(|value| value as f32)
            .collect());
    }
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| format!("{field} must be a Float32Array or a plain array"))?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        match array.get(index).as_f64() {
            Some(value) => out.push(value as f32),
            None => return Err(format!("{field} must contain only numbers")),
        }
    }
    Ok(out)
}

#[cfg(feature = "write")]
fn u32_array_from_js(value: &JsValue, field: &str) -> Result<Vec<u32>, String> {
    if let Some(typed) = value.dyn_ref::<Uint32Array>() {
        return Ok(typed.to_vec());
    }
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| format!("{field} must be a Uint32Array or a plain array"))?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        match array.get(index).as_f64() {
            Some(value) => out.push(value as u32),
            None => return Err(format!("{field} must contain only numbers")),
        }
    }
    Ok(out)
}

#[cfg(feature = "write")]
fn i32_array_from_js(value: &JsValue, field: &str) -> Result<Vec<i32>, String> {
    if let Some(typed) = value.dyn_ref::<Int32Array>() {
        return Ok(typed.to_vec());
    }
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| format!("{field} must be an Int32Array or a plain array"))?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        match array.get(index).as_f64() {
            Some(value) => out.push(value as i32),
            None => return Err(format!("{field} must contain only numbers")),
        }
    }
    Ok(out)
}

#[cfg(feature = "write")]
fn f64_array_from_js(value: &JsValue, field: &str) -> Result<Vec<f64>, String> {
    if let Some(typed) = value.dyn_ref::<Float64Array>() {
        return Ok(typed.to_vec());
    }
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| format!("{field} must be a Float64Array or a plain array"))?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        match array.get(index).as_f64() {
            Some(value) => out.push(value),
            None => return Err(format!("{field} must contain only numbers")),
        }
    }
    Ok(out)
}

#[cfg(feature = "write")]
fn u8_array_from_js(value: &JsValue, field: &str) -> Result<Vec<u8>, String> {
    if let Some(typed) = value.dyn_ref::<Uint8Array>() {
        return Ok(typed.to_vec());
    }
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| format!("{field} must be a Uint8Array or a plain array"))?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        match array.get(index).as_f64() {
            Some(value) => out.push(value as u8),
            None => return Err(format!("{field} must contain only numbers")),
        }
    }
    Ok(out)
}

/// A required float array field on an object.
#[cfg(feature = "write")]
fn required_f32_array(value: &JsValue, field: &str) -> Result<Vec<f32>, String> {
    let value =
        get_field(value, field).ok_or_else(|| format!("mesh must be an object with {field}"))?;
    f32_array_from_js(&value, field)
}

#[cfg(feature = "write")]
fn optional_f32_array(value: &JsValue, field: &str) -> Result<Option<Vec<f32>>, String> {
    match get_field(value, field) {
        Some(value) => Ok(Some(f32_array_from_js(&value, field)?)),
        None => Ok(None),
    }
}

/// A required numeric scalar field.
#[cfg(feature = "write")]
fn required_u32(value: &JsValue, field: &str) -> Result<u32, String> {
    let value = get_field(value, field).ok_or_else(|| format!("{field} is required"))?;
    value
        .as_f64()
        .map(|number| number as u32)
        .ok_or_else(|| format!("{field} must be a number"))
}

#[cfg(feature = "write")]
fn required_f64(value: &JsValue, field: &str) -> Result<f64, String> {
    let value = get_field(value, field).ok_or_else(|| format!("{field} is required"))?;
    value
        .as_f64()
        .ok_or_else(|| format!("{field} must be a number"))
}

#[cfg(feature = "write")]
fn required_string(value: &JsValue, field: &str) -> Result<String, String> {
    let value = get_field(value, field).ok_or_else(|| format!("{field} is required"))?;
    value
        .as_string()
        .ok_or_else(|| format!("{field} must be a string"))
}

/// Read a fixed-size vec3 from an optional field. Absent becomes `None`;
/// present but not three numbers is an error, matching the old serde shape.
#[cfg(feature = "write")]
fn optional_vec3(value: &JsValue, field: &str) -> Result<Option<[f32; 3]>, String> {
    match get_field(value, field) {
        Some(field_value) => {
            let values = f32_array_from_js(&field_value, field)?;
            if values.len() != 3 {
                return Err(format!("{field} must contain exactly 3 values"));
            }
            Ok(Some([values[0], values[1], values[2]]))
        }
        None => Ok(None),
    }
}

#[cfg(feature = "write")]
fn optional_u32_array(value: &JsValue, field: &str) -> Result<Option<Vec<u32>>, String> {
    match get_field(value, field) {
        Some(field_value) => Ok(Some(u32_array_from_js(&field_value, field)?)),
        None => Ok(None),
    }
}

#[cfg(feature = "write")]
fn optional_i32_array(value: &JsValue, field: &str) -> Result<Option<Vec<i32>>, String> {
    match get_field(value, field) {
        Some(field_value) => Ok(Some(i32_array_from_js(&field_value, field)?)),
        None => Ok(None),
    }
}

/// Read a required array field off an object.
#[cfg(feature = "write")]
fn required_u32_array(value: &JsValue, field: &str) -> Result<Vec<u32>, String> {
    let value = get_field(value, field).ok_or_else(|| format!("{field} is required"))?;
    u32_array_from_js(&value, field)
}

#[cfg(feature = "write")]
fn required_i32_array(value: &JsValue, field: &str) -> Result<Vec<i32>, String> {
    let value = get_field(value, field).ok_or_else(|| format!("{field} is required"))?;
    i32_array_from_js(&value, field)
}

#[cfg(feature = "write")]
fn required_f64_array(value: &JsValue, field: &str) -> Result<Vec<f64>, String> {
    let value = get_field(value, field).ok_or_else(|| format!("{field} is required"))?;
    f64_array_from_js(&value, field)
}

/// Read a required object field.
#[cfg(feature = "write")]
fn required_object(value: &JsValue, field: &str) -> Result<JsValue, String> {
    get_field(value, field).ok_or_else(|| format!("{field} is required"))
}

/// Read a `Vec<u8>`-backed field (embedded texture bytes), accepting both a
/// `Uint8Array` and a plain array.
#[cfg(feature = "write")]
fn optional_u8_array(value: &JsValue, field: &str) -> Result<Option<Vec<u8>>, String> {
    match get_field(value, field) {
        Some(value) => Ok(Some(u8_array_from_js(&value, field)?)),
        None => Ok(None),
    }
}

/// FBX file magic: "Kaydara FBX Binary  \0".
///
/// Serialization lives in `draco-io`; this copy only lets the tests below
/// assert that what we hand back to JavaScript really is an FBX file.
#[cfg(test)]
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
use draco_io::{FbxGlobalSettings, FbxScene, FbxSceneNode, FbxTransformStack};
#[cfg(feature = "write")]
use draco_io::{FbxNodeId, FbxWriteStats, FbxWriter};

/// Mesh data produced by the FBX reader, for JavaScript interop.
#[cfg(feature = "read")]
#[derive(Clone)]
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
    /// Per-render-vertex linear RGBA, from the first colour layer.
    pub colors: Vec<f32>,
    /// Per-render-vertex tangents from the first tangent layer, `xyzw` with
    /// the handedness sign in `w` -- exactly glTF's `TANGENT` layout.
    pub tangents: Vec<f32>,
    /// Every UV layer resolved onto render vertices, in source order.
    ///
    /// `uvs` is the first of these; the rest become `TEXCOORD_1`..
    pub uv_layers: Vec<Vec<f32>>,
    /// Per-triangle indices into the scene material list.
    ///
    /// This retains FBX `LayerElementMaterial` assignments for a later
    /// hierarchy-preserving export. The first value is also exposed through
    /// `material` for the preview's single-material primitive path.
    pub material_indices: Vec<i32>,
    /// Index of the first material applied to this mesh, when present.
    pub material: Option<usize>,
    /// Full FBX skin clusters, without a GPU influence limit.
    pub skin: Option<SkinOutput>,
    pub morph_targets: Vec<MorphTargetOutput>,
    /// First four influences per point for the WebGL preview. `skin` retains
    /// every FBX influence for a later export.
    pub joints0: Vec<u16>,
    pub weights0: Vec<f32>,
    /// Optional second four-influence set for portable eight-influence data.
    /// The viewer may consume both sets; exporters can preserve them even
    /// when a source format exposes more influences than the GPU path needs.
    pub joints1: Vec<u16>,
    pub weights1: Vec<f32>,
    /// Original FBX control points, retained for scene round-trip.
    pub control_points: Vec<f32>,
    /// Original FBX polygon-corner index stream.
    pub polygon_vertex_indices: Vec<i32>,
    /// Original UV layers, including mapping/reference metadata.
    pub uv_sets: Vec<UvSetOutput>,
    pub normal_sets: Vec<NormalSetOutput>,
    /// Original `LayerElementColor` layers, including mapping metadata.
    pub color_sets: Vec<ColorSetOutput>,
    /// Original `LayerElementTangent` layers.
    pub tangent_sets: Vec<TangentSetOutput>,
    /// Original `LayerElementBinormal` layers.
    pub binormal_sets: Vec<TangentSetOutput>,
    /// Original `LayerElementSmoothing` layers, carried opaquely: glTF has no
    /// hard-edge concept, so these exist to survive an FBX-to-FBX rewrite.
    pub smoothing_layers: Vec<SmoothingLayerOutput>,
    /// Original edge and vertex crease layers, carried opaquely.
    pub crease_layers: Vec<CreaseLayerOutput>,
}

/// A `LayerElementSmoothing` crossing the WASM boundary, in both directions.
#[derive(Clone, Default)]
pub struct SmoothingLayerOutput {
    pub mapping: Option<String>,
    /// One integer flag per edge or per polygon.
    pub values: Vec<i32>,
}

/// A `LayerElementEdgeCrease` or `LayerElementVertexCrease` crossing the WASM
/// boundary, in both directions.
#[derive(Clone, Default)]
pub struct CreaseLayerOutput {
    /// `"edge"` or `"vertex"`; anything else is read as `"edge"`.
    pub kind: String,
    pub mapping: Option<String>,
    /// Crease weights, normally in `0..=1`.
    pub values: Vec<f64>,
}

/// A `LayerElementTangent` or `LayerElementBinormal` crossing the WASM
/// boundary, in both directions.
#[derive(Clone, Default)]
pub struct TangentSetOutput {
    pub name: Option<String>,
    pub mapping: Option<String>,
    pub reference: Option<String>,
    /// Flat `xyzw`; `w` is the handedness sign.
    pub values: Vec<f32>,
    pub indices: Vec<i32>,
    /// Whether `w` came from the file rather than being defaulted to `+1`.
    pub has_handedness: bool,
}

#[derive(Clone, Default)]
pub struct ColorSetOutput {
    pub name: Option<String>,
    pub mapping: Option<String>,
    pub reference: Option<String>,
    /// Flat linear RGBA.
    pub values: Vec<f32>,
    pub indices: Vec<i32>,
}

#[derive(Clone, Default)]
pub struct UvSetOutput {
    pub name: Option<String>,
    pub mapping: Option<String>,
    pub reference: Option<String>,
    pub values: Vec<f32>,
    pub indices: Vec<i32>,
}

#[derive(Clone, Default)]
pub struct NormalSetOutput {
    pub name: Option<String>,
    pub mapping: Option<String>,
    pub reference: Option<String>,
    pub values: Vec<f32>,
    pub indices: Vec<i32>,
}

#[cfg(feature = "read")]
#[derive(Clone)]
pub struct MorphTargetOutput {
    pub name: Option<String>,
    pub control_point_indices: Vec<u32>,
    /// Render-vertex indices after corner-domain expansion.  A control point
    /// can occur more than once when UVs or normals have seams.
    pub render_point_indices: Vec<u32>,
    pub position_deltas: Vec<f32>,
    pub render_position_deltas: Vec<f32>,
    pub normal_deltas: Option<Vec<f32>>,
    pub render_normal_deltas: Option<Vec<f32>>,
    pub default_weight: f32,
    pub full_weight: f32,
}

#[cfg(feature = "read")]
#[derive(Clone)]
pub struct SkinClusterOutput {
    pub joint_node_id: u32,
    pub control_point_indices: Vec<u32>,
    pub render_point_indices: Vec<u32>,
    pub weights: Vec<f32>,
    pub mesh_bind_transform: Vec<f32>,
    pub joint_bind_transform: Vec<f32>,
    pub armature_bind_transform: Option<Vec<f32>>,
}

#[cfg(feature = "read")]
#[derive(Clone)]
pub struct SkinOutput {
    pub clusters: Vec<SkinClusterOutput>,
    pub bind_pose: Vec<BindPoseOutput>,
}

#[cfg(feature = "read")]
#[derive(Clone)]
pub struct BindPoseOutput {
    pub node_id: u32,
    pub matrix: Vec<f32>,
}

/// Texture slot targeted by a binding.
#[cfg(feature = "read")]
#[derive(Clone, Copy, PartialEq, Eq)]
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
#[derive(Clone)]
pub struct TextureBindingOutput {
    pub slot: TextureSlotOutput,
    pub texture_index: usize,
}

/// Texture object output to JavaScript.
#[cfg(feature = "read")]
#[derive(Clone, Default)]
pub struct TextureOutput {
    pub name: Option<String>,
    /// Embedded image bytes (PNG/JPG), when present.
    pub content: Option<Vec<u8>>,
    /// Relative filename / external reference, when present.
    pub filename: Option<String>,
}

/// Material object output to JavaScript.
#[cfg(feature = "read")]
#[derive(Clone, Default)]
pub struct MaterialOutput {
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
    pub textures: Vec<TextureBindingOutput>,
}

/// Animation channel path (which TRS component the channel drives).
#[cfg(feature = "read")]
#[derive(Clone, Copy, PartialEq, Eq)]
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
#[derive(Clone, Copy, PartialEq, Eq)]
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
#[derive(Clone)]
pub struct AnimSamplerOutput {
    /// Strictly increasing keyframe times in seconds.
    pub input: Vec<f32>,
    /// Flattened keyframe values, 3 values per input entry (radians for rotation).
    pub output: Vec<f32>,
    pub interpolation: AnimInterpolationOutput,
    pub in_tangents: Option<Vec<f32>>,
    pub out_tangents: Option<Vec<f32>>,
}

/// One animation channel: drives one TRS path of one named node.
#[cfg(feature = "read")]
#[derive(Clone)]
pub struct AnimChannelOutput {
    pub node_id: u32,
    /// Name of the target model node.
    pub node_name: String,
    pub path: AnimChannelPathOutput,
    pub morph_target_index: Option<u32>,
    pub sampler: AnimSamplerOutput,
}

/// One animation take.
#[cfg(feature = "read")]
#[derive(Clone)]
pub struct AnimationOutput {
    pub name: Option<String>,
    pub duration: f32,
    pub channels: Vec<AnimChannelOutput>,
}

/// Parse result containing meshes and any warnings/errors.
#[cfg(feature = "read")]
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
    pub materials: Vec<MaterialOutput>,
    /// Textures carried at the top level, mirroring `scene.textures`.
    pub textures: Vec<TextureOutput>,
    /// Animations carried at the top level, mirroring `scene.animations`.
    pub animations: Vec<AnimationOutput>,
}

/// Scene data returned to JavaScript for hierarchy-preserving previews.
#[cfg(feature = "read")]
pub struct SceneOutput {
    pub global_settings: Option<GlobalSettingsOutput>,
    pub root_nodes: Vec<SceneNodeOutput>,
    pub materials: Vec<MaterialOutput>,
    pub textures: Vec<TextureOutput>,
    pub animations: Vec<AnimationOutput>,
}

/// Source-only FBX coordinate/unit/time metadata for provenance exports.
#[cfg(feature = "read")]
#[derive(Clone, Default)]
pub struct GlobalSettingsOutput {
    pub up_axis: Option<i32>,
    pub up_axis_sign: Option<i32>,
    pub front_axis: Option<i32>,
    pub front_axis_sign: Option<i32>,
    pub coord_axis: Option<i32>,
    pub coord_axis_sign: Option<i32>,
    pub unit_scale_factor: Option<f64>,
    pub original_unit_scale_factor: Option<f64>,
    pub time_mode: Option<i32>,
}

/// One FBX model node returned to JavaScript.
#[cfg(feature = "read")]
pub struct SceneNodeOutput {
    pub id: u32,
    pub name: Option<String>,
    /// Column-major local transform used by WebGL.
    pub matrix: Option<Vec<f32>>,
    pub transform_stack: Option<TransformStackOutput>,
    /// True when the source Model used pre/post rotation or pivot terms.
    /// The JS FBX adapter uses the skin bind pose as the baked local basis
    /// for these nodes; plain Model TRS remains authored animation data.
    pub has_complex_transform_stack: bool,
    pub meshes: Vec<MeshData>,
    pub children: Vec<SceneNodeOutput>,
}

/// Raw FBX Model transform-stack values preserved for source-provenance export.
#[cfg(feature = "read")]
#[derive(Clone, Default)]
pub struct TransformStackOutput {
    pub translation: Option<[f32; 3]>,
    pub rotation: Option<[f32; 3]>,
    pub scaling: Option<[f32; 3]>,
    pub rotation_order: Option<i32>,
    pub rotation_active: Option<bool>,
    pub pre_rotation: Option<[f32; 3]>,
    pub post_rotation: Option<[f32; 3]>,
    pub rotation_offset: Option<[f32; 3]>,
    pub rotation_pivot: Option<[f32; 3]>,
    pub scaling_offset: Option<[f32; 3]>,
    pub scaling_pivot: Option<[f32; 3]>,
    pub inherit_type: Option<i32>,
}

// ---------------------------------------------------------------------------
// Reader: Rust output structures -> JavaScript objects
//
// Geometry arrays cross as typed arrays; small fixed-size values (matrices,
// vec3s) and the per-triangle material index list stay plain because the shell
// inspects them with `Array.isArray`. Optional fields match the old serde
// shape: `null` when always-present, omitted when `skip_serializing_if` used
// to drop them.
// ---------------------------------------------------------------------------

fn set_opt_string_null(obj: &Object, key: &str, value: &Option<String>) {
    match value {
        Some(text) => set_js(obj, key, &JsValue::from_str(text)),
        None => set_js(obj, key, &JsValue::NULL),
    }
}

/// A `Vec<f32>` field the old serde contract always emitted.
#[cfg(feature = "read")]
fn set_f32_array(obj: &Object, key: &str, values: &[f32]) {
    set_js(obj, key, &f32_array_to_js(values));
}

#[cfg(feature = "read")]
fn set_u32_array(obj: &Object, key: &str, values: &[u32]) {
    set_js(obj, key, &u32_array_to_js(values));
}

#[cfg(feature = "read")]
fn set_i32_array(obj: &Object, key: &str, values: &[i32]) {
    set_js(obj, key, &i32_array_to_js(values));
}

#[cfg(feature = "read")]
fn set_f64_array(obj: &Object, key: &str, values: &[f64]) {
    set_js(obj, key, &f64_array_to_js(values));
}

#[cfg(feature = "read")]
fn set_plain_f32_array(obj: &Object, key: &str, values: &[f32]) {
    set_js(obj, key, &plain_f32_array_to_js(values));
}

/// Set a fixed-size vec3 ([x, y, z]) as a plain array, matching serde.
#[cfg(feature = "read")]
fn set_opt_vec3(obj: &Object, key: &str, value: &Option<[f32; 3]>) {
    match value {
        Some(v) => {
            let array = Array::new();
            for component in v {
                array.push(&JsValue::from_f64(*component as f64));
            }
            set_js(obj, key, &array.into());
        }
        None => set_js(obj, key, &JsValue::NULL),
    }
}

/// Omitted when empty, like the serde `skip_serializing_if = "Vec::is_empty"`.
#[cfg(feature = "read")]
fn set_opt_vec3_skipped(obj: &Object, key: &str, value: &Option<[f32; 3]>) {
    if value.is_some() {
        set_opt_vec3(obj, key, value);
    }
}

/// Omitted when empty, like the serde `skip_serializing_if = "Vec::is_empty"`.
#[cfg(feature = "read")]
fn set_skipped_f32_array(obj: &Object, key: &str, values: &[f32]) {
    if !values.is_empty() {
        set_f32_array(obj, key, values);
    }
}

#[cfg(feature = "read")]
fn set_skipped_u32_array(obj: &Object, key: &str, values: &[u32]) {
    if !values.is_empty() {
        set_u32_array(obj, key, values);
    }
}

#[cfg(feature = "read")]
fn set_skipped_i32_array(obj: &Object, key: &str, values: &[i32]) {
    if !values.is_empty() {
        set_i32_array(obj, key, values);
    }
}

/// Per-triangle material assignments stay a plain array: the shell checks them
/// with `Array.isArray` before feeding them back into the writer.
#[cfg(feature = "read")]
fn set_skipped_plain_i32_array(obj: &Object, key: &str, values: &[i32]) {
    if values.is_empty() {
        return;
    }
    let array = Array::new();
    for value in values {
        array.push(&JsValue::from_f64(*value as f64));
    }
    set_js(obj, key, &array.into());
}

#[cfg(feature = "read")]
fn set_skipped_string_array(obj: &Object, key: &str, values: &[String]) {
    if !values.is_empty() {
        set_string_array(obj, key, values);
    }
}

#[cfg(feature = "read")]
fn set_skipped_opt_string(obj: &Object, key: &str, value: &Option<String>) {
    if value.is_some() {
        set_opt_string_null(obj, key, value);
    }
}

#[cfg(feature = "read")]
fn set_skipped_u8_array(obj: &Object, key: &str, values: &Option<Vec<u8>>) {
    if let Some(bytes) = values {
        set_js(obj, key, &u8_array_to_js(bytes));
    }
}

#[cfg(feature = "read")]
fn texture_slot_to_js(slot: &TextureSlotOutput) -> JsValue {
    let text = match slot {
        TextureSlotOutput::Diffuse => "diffuse",
        TextureSlotOutput::Normal => "normal",
        TextureSlotOutput::Emissive => "emissive",
        TextureSlotOutput::Specular => "specular",
        TextureSlotOutput::Roughness => "roughness",
        TextureSlotOutput::Metallic => "metallic",
        TextureSlotOutput::Ambient => "ambient",
    };
    JsValue::from_str(text)
}

#[cfg(feature = "read")]
fn anim_channel_path_to_js(path: &AnimChannelPathOutput) -> JsValue {
    let text = match path {
        AnimChannelPathOutput::Translation => "translation",
        AnimChannelPathOutput::Rotation => "rotation",
        AnimChannelPathOutput::Scale => "scale",
        AnimChannelPathOutput::MorphWeight => "morphweight",
    };
    JsValue::from_str(text)
}

#[cfg(feature = "read")]
fn anim_interpolation_to_js(interpolation: &AnimInterpolationOutput) -> JsValue {
    let text = match interpolation {
        AnimInterpolationOutput::Step => "step",
        AnimInterpolationOutput::Linear => "linear",
        AnimInterpolationOutput::Cubic => "cubic",
    };
    JsValue::from_str(text)
}

#[cfg(feature = "read")]
fn smoothing_layer_to_js(layer: &SmoothingLayerOutput) -> Object {
    let obj = Object::new();
    set_opt_string_null(&obj, "mapping", &layer.mapping);
    set_i32_array(&obj, "values", &layer.values);
    obj
}

#[cfg(feature = "read")]
fn crease_layer_to_js(layer: &CreaseLayerOutput) -> Object {
    let obj = Object::new();
    set_js(&obj, "kind", &JsValue::from_str(&layer.kind));
    set_opt_string_null(&obj, "mapping", &layer.mapping);
    set_f64_array(&obj, "values", &layer.values);
    obj
}

#[cfg(feature = "read")]
fn tangent_set_to_js(set: &TangentSetOutput) -> Object {
    let obj = Object::new();
    set_opt_string_null(&obj, "name", &set.name);
    set_opt_string_null(&obj, "mapping", &set.mapping);
    set_opt_string_null(&obj, "reference", &set.reference);
    set_f32_array(&obj, "values", &set.values);
    set_i32_array(&obj, "indices", &set.indices);
    set_bool(&obj, "hasHandedness", set.has_handedness);
    obj
}

#[cfg(feature = "read")]
fn color_set_to_js(set: &ColorSetOutput) -> Object {
    let obj = Object::new();
    set_opt_string_null(&obj, "name", &set.name);
    set_opt_string_null(&obj, "mapping", &set.mapping);
    set_opt_string_null(&obj, "reference", &set.reference);
    set_f32_array(&obj, "values", &set.values);
    set_i32_array(&obj, "indices", &set.indices);
    obj
}

#[cfg(feature = "read")]
fn uv_set_to_js(set: &UvSetOutput) -> Object {
    let obj = Object::new();
    set_opt_string_null(&obj, "name", &set.name);
    set_opt_string_null(&obj, "mapping", &set.mapping);
    set_opt_string_null(&obj, "reference", &set.reference);
    set_f32_array(&obj, "values", &set.values);
    set_i32_array(&obj, "indices", &set.indices);
    obj
}

#[cfg(feature = "read")]
fn normal_set_to_js(set: &NormalSetOutput) -> Object {
    let obj = Object::new();
    set_opt_string_null(&obj, "name", &set.name);
    set_opt_string_null(&obj, "mapping", &set.mapping);
    set_opt_string_null(&obj, "reference", &set.reference);
    set_f32_array(&obj, "values", &set.values);
    set_i32_array(&obj, "indices", &set.indices);
    obj
}

#[cfg(feature = "read")]
fn morph_target_to_js(target: &MorphTargetOutput) -> Object {
    let obj = Object::new();
    set_opt_string_null(&obj, "name", &target.name);
    set_u32_array(&obj, "controlPointIndices", &target.control_point_indices);
    set_skipped_u32_array(&obj, "renderPointIndices", &target.render_point_indices);
    set_f32_array(&obj, "positionDeltas", &target.position_deltas);
    set_skipped_f32_array(&obj, "renderPositionDeltas", &target.render_position_deltas);
    match &target.normal_deltas {
        Some(deltas) => set_f32_array(&obj, "normalDeltas", deltas),
        None => set_js(&obj, "normalDeltas", &JsValue::NULL),
    }
    set_skipped_opt_string_owned(&obj, "renderNormalDeltas", &target.render_normal_deltas);
    set_f64(&obj, "defaultWeight", target.default_weight as f64);
    set_f64(&obj, "fullWeight", target.full_weight as f64);
    obj
}

/// A `Vec<f32>` that was `Option<Vec<f32>>` with a skip on None.
#[cfg(feature = "read")]
fn set_skipped_opt_string_owned(obj: &Object, key: &str, value: &Option<Vec<f32>>) {
    if let Some(values) = value {
        set_f32_array(obj, key, values);
    }
}

#[cfg(feature = "read")]
fn skin_cluster_to_js(cluster: &SkinClusterOutput) -> Object {
    let obj = Object::new();
    set_u32(&obj, "jointNodeId", cluster.joint_node_id);
    set_u32_array(&obj, "controlPointIndices", &cluster.control_point_indices);
    set_skipped_u32_array(&obj, "renderPointIndices", &cluster.render_point_indices);
    set_f32_array(&obj, "weights", &cluster.weights);
    set_plain_f32_array(&obj, "meshBindTransform", &cluster.mesh_bind_transform);
    set_plain_f32_array(&obj, "jointBindTransform", &cluster.joint_bind_transform);
    match &cluster.armature_bind_transform {
        Some(matrix) => set_plain_f32_array(&obj, "armatureBindTransform", matrix),
        None => set_js(&obj, "armatureBindTransform", &JsValue::NULL),
    }
    obj
}

#[cfg(feature = "read")]
fn bind_pose_to_js(pose: &BindPoseOutput) -> Object {
    let obj = Object::new();
    set_u32(&obj, "nodeId", pose.node_id);
    set_plain_f32_array(&obj, "matrix", &pose.matrix);
    obj
}

#[cfg(feature = "read")]
fn skin_to_js(skin: &SkinOutput) -> Object {
    let obj = Object::new();
    let clusters = Array::new();
    for cluster in &skin.clusters {
        clusters.push(&skin_cluster_to_js(cluster).into());
    }
    set_js(&obj, "clusters", &clusters.into());
    let bind_pose = Array::new();
    for pose in &skin.bind_pose {
        bind_pose.push(&bind_pose_to_js(pose).into());
    }
    set_js(&obj, "bindPose", &bind_pose.into());
    obj
}

#[cfg(feature = "read")]
fn texture_binding_to_js(binding: &TextureBindingOutput) -> Object {
    let obj = Object::new();
    set_js(&obj, "slot", &texture_slot_to_js(&binding.slot));
    set_u32(&obj, "textureIndex", binding.texture_index as u32);
    obj
}

#[cfg(feature = "read")]
fn texture_to_js(texture: &TextureOutput) -> Object {
    let obj = Object::new();
    set_opt_string_null(&obj, "name", &texture.name);
    set_skipped_u8_array(&obj, "content", &texture.content);
    set_skipped_opt_string(&obj, "filename", &texture.filename);
    obj
}

#[cfg(feature = "read")]
fn material_to_js(material: &MaterialOutput) -> Object {
    let obj = Object::new();
    set_opt_string_null(&obj, "name", &material.name);
    set_opt_string_null(&obj, "shadingModel", &material.shading_model);
    set_opt_vec3_skipped(&obj, "diffuse", &material.diffuse);
    set_opt_vec3_skipped(&obj, "specular", &material.specular);
    set_opt_vec3_skipped(&obj, "emissive", &material.emissive);
    set_opt_vec3_skipped(&obj, "ambient", &material.ambient);
    set_skipped_opt_f64(&obj, "diffuseFactor", &material.diffuse_factor);
    set_skipped_opt_f64(&obj, "specularFactor", &material.specular_factor);
    set_skipped_opt_f64(&obj, "shininess", &material.shininess);
    set_skipped_opt_f64(&obj, "emissiveFactor", &material.emissive_factor);
    set_skipped_opt_f64(&obj, "reflectionFactor", &material.reflection_factor);
    set_skipped_opt_f64(&obj, "transparencyFactor", &material.transparency_factor);
    set_skipped_opt_f64(&obj, "opacity", &material.opacity);
    set_skipped_opt_f64(&obj, "bumpFactor", &material.bump_factor);
    let textures = Array::new();
    for binding in &material.textures {
        textures.push(&texture_binding_to_js(binding).into());
    }
    set_js(&obj, "textures", &textures.into());
    obj
}

#[cfg(feature = "read")]
fn set_skipped_opt_f64(obj: &Object, key: &str, value: &Option<f32>) {
    if let Some(value) = value {
        set_f64(obj, key, *value as f64);
    }
}

#[cfg(feature = "read")]
fn anim_sampler_to_js(sampler: &AnimSamplerOutput) -> Object {
    let obj = Object::new();
    set_f32_array(&obj, "input", &sampler.input);
    set_f32_array(&obj, "output", &sampler.output);
    set_js(
        &obj,
        "interpolation",
        &anim_interpolation_to_js(&sampler.interpolation),
    );
    match &sampler.in_tangents {
        Some(values) => set_f32_array(&obj, "inTangents", values),
        None => (),
    }
    match &sampler.out_tangents {
        Some(values) => set_f32_array(&obj, "outTangents", values),
        None => (),
    }
    obj
}

#[cfg(feature = "read")]
fn anim_channel_to_js(channel: &AnimChannelOutput) -> Object {
    let obj = Object::new();
    set_u32(&obj, "nodeId", channel.node_id);
    set_js(&obj, "nodeName", &JsValue::from_str(&channel.node_name));
    set_js(&obj, "path", &anim_channel_path_to_js(&channel.path));
    if let Some(index) = channel.morph_target_index {
        set_u32(&obj, "morphTargetIndex", index);
    }
    set_js(
        &obj,
        "sampler",
        &anim_sampler_to_js(&channel.sampler).into(),
    );
    obj
}

#[cfg(feature = "read")]
fn animation_to_js(animation: &AnimationOutput) -> Object {
    let obj = Object::new();
    set_opt_string_null(&obj, "name", &animation.name);
    set_f64(&obj, "duration", animation.duration as f64);
    let channels = Array::new();
    for channel in &animation.channels {
        channels.push(&anim_channel_to_js(channel).into());
    }
    set_js(&obj, "channels", &channels.into());
    obj
}

#[cfg(feature = "read")]
fn global_settings_to_js(settings: &GlobalSettingsOutput) -> Object {
    let obj = Object::new();
    set_skipped_opt_i32_nullable(&obj, "upAxis", &settings.up_axis);
    set_skipped_opt_i32_nullable(&obj, "upAxisSign", &settings.up_axis_sign);
    set_skipped_opt_i32_nullable(&obj, "frontAxis", &settings.front_axis);
    set_skipped_opt_i32_nullable(&obj, "frontAxisSign", &settings.front_axis_sign);
    set_skipped_opt_i32_nullable(&obj, "coordAxis", &settings.coord_axis);
    set_skipped_opt_i32_nullable(&obj, "coordAxisSign", &settings.coord_axis_sign);
    set_skipped_opt_f64_nullable(&obj, "unitScaleFactor", &settings.unit_scale_factor);
    set_skipped_opt_f64_nullable(
        &obj,
        "originalUnitScaleFactor",
        &settings.original_unit_scale_factor,
    );
    set_skipped_opt_i32_nullable(&obj, "timeMode", &settings.time_mode);
    obj
}

/// An `Option<i32>` the serde contract always emitted, as `null` when absent.
#[cfg(feature = "read")]
fn set_skipped_opt_i32_nullable(obj: &Object, key: &str, value: &Option<i32>) {
    match value {
        Some(value) => set_i32(obj, key, *value),
        None => set_js(obj, key, &JsValue::NULL),
    }
}

#[cfg(feature = "read")]
fn set_skipped_opt_f64_nullable(obj: &Object, key: &str, value: &Option<f64>) {
    match value {
        Some(value) => set_f64(obj, key, *value),
        None => set_js(obj, key, &JsValue::NULL),
    }
}

#[cfg(feature = "read")]
fn transform_stack_to_js(stack: &TransformStackOutput) -> Object {
    let obj = Object::new();
    set_opt_vec3(&obj, "translation", &stack.translation);
    set_opt_vec3(&obj, "rotation", &stack.rotation);
    set_opt_vec3(&obj, "scaling", &stack.scaling);
    set_skipped_opt_i32_nullable(&obj, "rotationOrder", &stack.rotation_order);
    set_skipped_opt_bool_nullable(&obj, "rotationActive", &stack.rotation_active);
    set_opt_vec3(&obj, "preRotation", &stack.pre_rotation);
    set_opt_vec3(&obj, "postRotation", &stack.post_rotation);
    set_opt_vec3(&obj, "rotationOffset", &stack.rotation_offset);
    set_opt_vec3(&obj, "rotationPivot", &stack.rotation_pivot);
    set_opt_vec3(&obj, "scalingOffset", &stack.scaling_offset);
    set_opt_vec3(&obj, "scalingPivot", &stack.scaling_pivot);
    set_skipped_opt_i32_nullable(&obj, "inheritType", &stack.inherit_type);
    obj
}

#[cfg(feature = "read")]
fn set_skipped_opt_bool_nullable(obj: &Object, key: &str, value: &Option<bool>) {
    match value {
        Some(value) => set_bool(obj, key, *value),
        None => set_js(obj, key, &JsValue::NULL),
    }
}

#[cfg(feature = "read")]
fn scene_node_to_js(node: &SceneNodeOutput) -> Object {
    let obj = Object::new();
    set_u32(&obj, "id", node.id);
    set_opt_string_null(&obj, "name", &node.name);
    match &node.matrix {
        Some(matrix) => set_plain_f32_array(&obj, "matrix", matrix),
        None => set_js(&obj, "matrix", &JsValue::NULL),
    }
    if let Some(stack) = &node.transform_stack {
        set_js(&obj, "transformStack", &transform_stack_to_js(stack).into());
    }
    set_bool(
        &obj,
        "hasComplexTransformStack",
        node.has_complex_transform_stack,
    );
    let meshes = Array::new();
    for mesh in &node.meshes {
        meshes.push(&mesh_data_to_js(mesh).into());
    }
    set_js(&obj, "meshes", &meshes.into());
    let children = Array::new();
    for child in &node.children {
        children.push(&scene_node_to_js(child).into());
    }
    set_js(&obj, "children", &children.into());
    obj
}

#[cfg(feature = "read")]
fn mesh_data_to_js(mesh: &MeshData) -> Object {
    let obj = Object::new();
    set_opt_string_null(&obj, "name", &mesh.name);
    set_f32_array(&obj, "positions", &mesh.positions);
    set_u32_array(&obj, "indices", &mesh.indices);
    set_f32_array(&obj, "normals", &mesh.normals);
    set_f32_array(&obj, "uvs", &mesh.uvs);
    set_skipped_f32_array(&obj, "colors", &mesh.colors);
    set_skipped_f32_array(&obj, "tangents", &mesh.tangents);
    if !mesh.uv_layers.is_empty() {
        let layers = Array::new();
        for layer in &mesh.uv_layers {
            layers.push(&f32_array_to_js(layer));
        }
        set_js(&obj, "uvLayers", &layers.into());
    }
    set_skipped_plain_i32_array(&obj, "materialIndices", &mesh.material_indices);
    if let Some(material) = &mesh.material {
        set_u32(&obj, "material", *material as u32);
    }
    if let Some(skin) = &mesh.skin {
        set_js(&obj, "skin", &skin_to_js(skin).into());
    }
    if !mesh.morph_targets.is_empty() {
        let targets = Array::new();
        for target in &mesh.morph_targets {
            targets.push(&morph_target_to_js(target).into());
        }
        set_js(&obj, "morphTargets", &targets.into());
    }
    set_skipped_u16_array(&obj, "joints0", &mesh.joints0);
    set_skipped_f32_array(&obj, "weights0", &mesh.weights0);
    set_skipped_u16_array(&obj, "joints1", &mesh.joints1);
    set_skipped_f32_array(&obj, "weights1", &mesh.weights1);
    set_skipped_f32_array(&obj, "controlPoints", &mesh.control_points);
    set_skipped_i32_array(&obj, "polygonVertexIndices", &mesh.polygon_vertex_indices);
    if !mesh.uv_sets.is_empty() {
        let sets = Array::new();
        for set in &mesh.uv_sets {
            sets.push(&uv_set_to_js(set).into());
        }
        set_js(&obj, "uvSets", &sets.into());
    }
    if !mesh.normal_sets.is_empty() {
        let sets = Array::new();
        for set in &mesh.normal_sets {
            sets.push(&normal_set_to_js(set).into());
        }
        set_js(&obj, "normalSets", &sets.into());
    }
    if !mesh.color_sets.is_empty() {
        let sets = Array::new();
        for set in &mesh.color_sets {
            sets.push(&color_set_to_js(set).into());
        }
        set_js(&obj, "colorSets", &sets.into());
    }
    if !mesh.tangent_sets.is_empty() {
        let sets = Array::new();
        for set in &mesh.tangent_sets {
            sets.push(&tangent_set_to_js(set).into());
        }
        set_js(&obj, "tangentSets", &sets.into());
    }
    if !mesh.binormal_sets.is_empty() {
        let sets = Array::new();
        for set in &mesh.binormal_sets {
            sets.push(&tangent_set_to_js(set).into());
        }
        set_js(&obj, "binormalSets", &sets.into());
    }
    if !mesh.smoothing_layers.is_empty() {
        let layers = Array::new();
        for layer in &mesh.smoothing_layers {
            layers.push(&smoothing_layer_to_js(layer).into());
        }
        set_js(&obj, "smoothingLayers", &layers.into());
    }
    if !mesh.crease_layers.is_empty() {
        let layers = Array::new();
        for layer in &mesh.crease_layers {
            layers.push(&crease_layer_to_js(layer).into());
        }
        set_js(&obj, "creaseLayers", &layers.into());
    }
    obj
}

#[cfg(feature = "read")]
fn set_skipped_u16_array(obj: &Object, key: &str, values: &[u16]) {
    if !values.is_empty() {
        set_js(obj, key, &u16_array_to_js(values));
    }
}

#[cfg(feature = "read")]
fn scene_output_to_js(scene: &SceneOutput) -> Object {
    let obj = Object::new();
    if let Some(settings) = &scene.global_settings {
        set_js(
            &obj,
            "globalSettings",
            &global_settings_to_js(settings).into(),
        );
    }
    let root_nodes = Array::new();
    for node in &scene.root_nodes {
        root_nodes.push(&scene_node_to_js(node).into());
    }
    set_js(&obj, "rootNodes", &root_nodes.into());
    let materials = Array::new();
    for material in &scene.materials {
        materials.push(&material_to_js(material).into());
    }
    set_js(&obj, "materials", &materials.into());
    let textures = Array::new();
    for texture in &scene.textures {
        textures.push(&texture_to_js(texture).into());
    }
    set_js(&obj, "textures", &textures.into());
    let animations = Array::new();
    for animation in &scene.animations {
        animations.push(&animation_to_js(animation).into());
    }
    set_js(&obj, "animations", &animations.into());
    obj
}

#[cfg(feature = "read")]
fn parse_result_to_js(result: &ParseResult) -> JsValue {
    let obj = Object::new();
    set_bool(&obj, "success", result.success);
    let meshes = Array::new();
    for mesh in &result.meshes {
        meshes.push(&mesh_data_to_js(mesh).into());
    }
    set_js(&obj, "meshes", &meshes.into());
    set_opt_string_null(&obj, "error", &result.error);
    set_skipped_string_array(&obj, "warnings", &result.warnings);
    match result.version {
        Some(version) => set_u32(&obj, "version", version),
        None => set_js(&obj, "version", &JsValue::NULL),
    }
    match &result.scene {
        Some(scene) => set_js(&obj, "scene", &scene_output_to_js(scene).into()),
        None => set_js(&obj, "scene", &JsValue::NULL),
    }
    let materials = Array::new();
    for material in &result.materials {
        materials.push(&material_to_js(material).into());
    }
    set_js(&obj, "materials", &materials.into());
    let textures = Array::new();
    for texture in &result.textures {
        textures.push(&texture_to_js(texture).into());
    }
    set_js(&obj, "textures", &textures.into());
    let animations = Array::new();
    for animation in &result.animations {
        animations.push(&animation_to_js(animation).into());
    }
    set_js(&obj, "animations", &animations.into());
    obj.into()
}

/// Parse FBX binary file content.
#[cfg(feature = "read")]
#[wasm_bindgen]
pub fn parse_fbx(data: &[u8]) -> JsValue {
    let result = parse_fbx_scene(data);
    parse_result_to_js(&result)
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
                global_settings: scene
                    .global_settings
                    .as_ref()
                    .map(global_settings_to_output),
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
                // The JS side shows these as plain strings; `Display` renders
                // the message and, for repeats, the occurrence count.
                warnings: scene.warnings.iter().map(ToString::to_string).collect(),
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
fn global_settings_to_output(settings: &FbxGlobalSettings) -> GlobalSettingsOutput {
    GlobalSettingsOutput {
        up_axis: settings.up_axis,
        up_axis_sign: settings.up_axis_sign,
        front_axis: settings.front_axis,
        front_axis_sign: settings.front_axis_sign,
        coord_axis: settings.coord_axis,
        coord_axis_sign: settings.coord_axis_sign,
        unit_scale_factor: settings.unit_scale_factor,
        original_unit_scale_factor: settings.original_unit_scale_factor,
        time_mode: settings.time_mode,
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
        transform_stack: node.transform_stack.as_ref().map(transform_stack_to_output),
        has_complex_transform_stack: node.has_complex_transform_stack,
        meshes: node
            .mesh_instances
            .iter()
            .map(mesh_instance_to_data)
            .collect(),
        children: node.children.iter().map(scene_node_to_output).collect(),
    }
}

#[cfg(feature = "read")]
fn transform_stack_to_output(stack: &FbxTransformStack) -> TransformStackOutput {
    TransformStackOutput {
        translation: stack.translation,
        rotation: stack.rotation,
        scaling: stack.scaling,
        rotation_order: stack.rotation_order,
        rotation_active: stack.rotation_active,
        pre_rotation: stack.pre_rotation,
        post_rotation: stack.post_rotation,
        rotation_offset: stack.rotation_offset,
        rotation_pivot: stack.rotation_pivot,
        scaling_offset: stack.scaling_offset,
        scaling_pivot: stack.scaling_pivot,
        inherit_type: stack.inherit_type,
    }
}

#[cfg(feature = "read")]
fn tangent_set_to_output(set: &draco_io::FbxTangentSet) -> TangentSetOutput {
    TangentSetOutput {
        name: set.layer.name.clone(),
        mapping: set.layer.mapping.clone(),
        reference: set.layer.reference.clone(),
        values: set
            .layer
            .values
            .iter()
            .flat_map(|value| value.iter().copied())
            .collect(),
        indices: set.layer.indices.clone(),
        has_handedness: set.has_handedness,
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
        .layers
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
    mesh.tangent_sets = instance
        .layers
        .tangent_sets
        .iter()
        .map(tangent_set_to_output)
        .collect();
    mesh.binormal_sets = instance
        .layers
        .binormal_sets
        .iter()
        .map(tangent_set_to_output)
        .collect();
    mesh.smoothing_layers = instance
        .layers
        .smoothing_layers
        .iter()
        .map(|layer| SmoothingLayerOutput {
            mapping: layer.mapping.clone(),
            values: layer.values.clone(),
        })
        .collect();
    mesh.crease_layers = instance
        .layers
        .crease_layers
        .iter()
        .map(|layer| CreaseLayerOutput {
            kind: match layer.kind {
                draco_io::FbxCreaseKind::Edge => "edge".to_string(),
                draco_io::FbxCreaseKind::Vertex => "vertex".to_string(),
            },
            mapping: layer.mapping.clone(),
            values: layer.values.clone(),
        })
        .collect();
    mesh.color_sets = instance
        .layers
        .color_sets
        .iter()
        .map(|set| ColorSetOutput {
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
        .layers
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
    // Corner-domain expansion lives in draco-io so the Rust and WASM paths
    // cannot resolve layer elements differently.
    let render = instance.to_render_mesh();
    let render_control_points: Vec<u32> = if render.positions.is_empty() {
        (0..mesh.positions.len() as u32 / 3).collect()
    } else {
        mesh.positions = render
            .positions
            .iter()
            .flat_map(|point| point.iter().copied())
            .collect();
        mesh.indices = render.indices.clone();
        mesh.normals = render
            .normals
            .first()
            .map(|layer| {
                layer
                    .values
                    .iter()
                    .flat_map(|value| value.iter().copied())
                    .collect()
            })
            .unwrap_or_default();
        mesh.colors = render
            .colors
            .first()
            .map(|layer| {
                layer
                    .values
                    .iter()
                    .flat_map(|value| value.iter().copied())
                    .collect()
            })
            .unwrap_or_default();
        mesh.tangents = render
            .tangents
            .first()
            .map(|layer| {
                layer
                    .values
                    .iter()
                    .flat_map(|value| value.iter().copied())
                    .collect()
            })
            .unwrap_or_default();
        mesh.uv_layers = render
            .uvs
            .iter()
            .map(|layer| {
                layer
                    .values
                    .iter()
                    .flat_map(|value| value.iter().copied())
                    .collect()
            })
            .collect();
        mesh.uvs = mesh.uv_layers.first().cloned().unwrap_or_default();
        render.corner_to_control_point.clone()
    };
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
        mesh.joints1 = vec![0; point_count * 4];
        mesh.weights1 = vec![0.0; point_count * 4];
        for (point, entries) in influences.iter_mut().enumerate() {
            entries.sort_by(|left, right| right.1.total_cmp(&left.1));
            let sum: f32 = entries.iter().take(8).map(|entry| entry.1).sum();
            for (slot, &(joint, weight)) in entries.iter().take(8).enumerate() {
                if slot < 4 {
                    mesh.joints0[point * 4 + slot] = joint;
                    mesh.weights0[point * 4 + slot] = if sum > 0.0 { weight / sum } else { 0.0 };
                } else {
                    let second = slot - 4;
                    mesh.joints1[point * 4 + second] = joint;
                    mesh.weights1[point * 4 + second] = if sum > 0.0 { weight / sum } else { 0.0 };
                }
            }
        }
        if mesh.weights1.iter().all(|weight| *weight == 0.0) {
            mesh.joints1.clear();
            mesh.weights1.clear();
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
    let colors = read_attribute_as_f32(mesh, GeometryAttributeType::Color, 4);
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
        colors,
        tangents: Vec::new(),
        uv_layers: Vec::new(),
        material_indices: Vec::new(),
        material: None,
        skin: None,
        morph_targets: Vec::new(),
        joints0: Vec::new(),
        weights0: Vec::new(),
        joints1: Vec::new(),
        weights1: Vec::new(),
        control_points: Vec::new(),
        polygon_vertex_indices: Vec::new(),
        uv_sets: Vec::new(),
        normal_sets: Vec::new(),
        color_sets: Vec::new(),
        tangent_sets: Vec::new(),
        binormal_sets: Vec::new(),
        smoothing_layers: Vec::new(),
        crease_layers: Vec::new(),
    }
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

/// Input mesh data consumed by the FBX writer, from JavaScript.
#[cfg(feature = "write")]
#[derive(Clone)]
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
    pub control_points: Option<Vec<f32>>,
    pub polygon_vertex_indices: Option<Vec<i32>>,
    pub uv_sets: Vec<UvSetOutput>,
    pub normal_sets: Vec<NormalSetOutput>,
    /// Tangent layers to write, `xyzw` per value.
    pub tangent_sets: Vec<TangentSetOutput>,
    /// Binormal layers to write.
    pub binormal_sets: Vec<TangentSetOutput>,
    /// Smoothing layers to write.
    pub smoothing_layers: Vec<SmoothingLayerOutput>,
    /// Crease layers to write.
    pub crease_layers: Vec<CreaseLayerOutput>,
    pub color_sets: Vec<ColorSetOutput>,
    pub edges: Vec<i32>,
    /// Per-triangle indices into `SceneInput::materials`.
    pub material_indices: Vec<i32>,
    pub skin: Option<SkinInput>,
    pub morph_targets: Vec<MorphTargetInput>,
}

#[cfg(feature = "write")]
#[derive(Clone)]
pub struct MorphTargetInput {
    pub name: Option<String>,
    pub control_point_indices: Vec<u32>,
    pub position_deltas: Vec<f32>,
    pub normal_deltas: Option<Vec<f32>>,
    pub default_weight: f32,
    pub full_weight: f32,
}

#[cfg(feature = "write")]
fn default_full_morph_weight() -> f32 {
    100.0
}

#[cfg(feature = "write")]
#[derive(Clone)]
pub struct SkinClusterInput {
    pub joint_node_id: u32,
    pub control_point_indices: Vec<u32>,
    pub render_point_indices: Vec<u32>,
    pub weights: Vec<f32>,
    pub mesh_bind_transform: Vec<f32>,
    pub joint_bind_transform: Vec<f32>,
    pub armature_bind_transform: Option<Vec<f32>>,
}

#[cfg(feature = "write")]
#[derive(Clone)]
pub struct BindPoseInput {
    pub node_id: u32,
    pub matrix: Vec<f32>,
}

#[cfg(feature = "write")]
#[derive(Clone)]
pub struct SkinInput {
    pub clusters: Vec<SkinClusterInput>,
    pub bind_pose: Vec<BindPoseInput>,
}

/// A hierarchy-preserving FBX export scene supplied by JavaScript.
#[cfg(feature = "write")]
pub struct SceneInput {
    pub global_settings: Option<GlobalSettingsInput>,
    pub root_nodes: Vec<SceneNodeInput>,
    pub materials: Vec<MaterialInput>,
    pub textures: Vec<TextureInput>,
    pub animations: Vec<AnimationInput>,
}

#[cfg(feature = "write")]
#[derive(Clone, Default)]
pub struct GlobalSettingsInput {
    pub up_axis: Option<i32>,
    pub up_axis_sign: Option<i32>,
    pub front_axis: Option<i32>,
    pub front_axis_sign: Option<i32>,
    pub coord_axis: Option<i32>,
    pub coord_axis_sign: Option<i32>,
    pub unit_scale_factor: Option<f64>,
    pub original_unit_scale_factor: Option<f64>,
    pub time_mode: Option<i32>,
}

#[cfg(feature = "write")]
impl From<GlobalSettingsInput> for FbxGlobalSettings {
    fn from(value: GlobalSettingsInput) -> Self {
        Self {
            up_axis: value.up_axis,
            up_axis_sign: value.up_axis_sign,
            front_axis: value.front_axis,
            front_axis_sign: value.front_axis_sign,
            coord_axis: value.coord_axis,
            coord_axis_sign: value.coord_axis_sign,
            unit_scale_factor: value.unit_scale_factor,
            original_unit_scale_factor: value.original_unit_scale_factor,
            time_mode: value.time_mode,
        }
    }
}

/// One FBX model node supplied by JavaScript.
#[cfg(feature = "write")]
pub struct SceneNodeInput {
    /// Stable scene-local node id. Missing ids are assigned deterministically.
    pub id: u32,
    pub name: Option<String>,
    /// Row-major local affine transform, as used by `FbxTransform`.
    pub matrix: Option<Vec<f32>>,
    pub transform_stack: Option<TransformStackInput>,
    pub meshes: Vec<MeshInput>,
    /// Per-mesh material index list, mirroring `FbxMeshInstance::material_indices`.
    pub children: Vec<SceneNodeInput>,
}

/// Raw supported FBX Model stack supplied to the typed writer.
#[cfg(feature = "write")]
#[derive(Clone, Default)]
pub struct TransformStackInput {
    pub translation: Option<[f32; 3]>,
    pub rotation: Option<[f32; 3]>,
    pub scaling: Option<[f32; 3]>,
    pub rotation_order: Option<i32>,
    pub rotation_active: Option<bool>,
    pub pre_rotation: Option<[f32; 3]>,
    pub post_rotation: Option<[f32; 3]>,
    pub rotation_offset: Option<[f32; 3]>,
    pub rotation_pivot: Option<[f32; 3]>,
    pub scaling_offset: Option<[f32; 3]>,
    pub scaling_pivot: Option<[f32; 3]>,
    pub inherit_type: Option<i32>,
}

#[cfg(feature = "write")]
impl From<TransformStackInput> for FbxTransformStack {
    fn from(value: TransformStackInput) -> Self {
        Self {
            translation: value.translation,
            rotation: value.rotation,
            scaling: value.scaling,
            rotation_order: value.rotation_order,
            rotation_active: value.rotation_active,
            pre_rotation: value.pre_rotation,
            post_rotation: value.post_rotation,
            rotation_offset: value.rotation_offset,
            rotation_pivot: value.rotation_pivot,
            scaling_offset: value.scaling_offset,
            scaling_pivot: value.scaling_pivot,
            inherit_type: value.inherit_type,
        }
    }
}

/// Material input supplied by JavaScript for the FBX writer.
#[cfg(feature = "write")]
#[derive(Default)]
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
    pub textures: Vec<TextureBindingInput>,
}

#[cfg(feature = "write")]
#[derive(Clone, Copy)]
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
pub struct TextureBindingInput {
    pub slot: TextureSlotInput,
    pub texture_index: usize,
}

#[cfg(feature = "write")]
#[derive(Default)]
pub struct TextureInput {
    pub name: Option<String>,
    pub content: Option<Vec<u8>>,
    pub filename: Option<String>,
}

/// Animation input supplied by JavaScript.
#[cfg(feature = "write")]
pub struct AnimationInput {
    pub name: Option<String>,
    pub duration: f32,
    pub channels: Vec<AnimChannelInput>,
}

#[cfg(feature = "write")]
pub struct AnimChannelInput {
    pub node_id: u32,
    pub node_name: String,
    pub path: AnimChannelPathInput,
    pub morph_target_index: Option<u32>,
    pub sampler: AnimSamplerInput,
}

#[cfg(feature = "write")]
#[derive(Clone, Copy)]
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
pub struct AnimSamplerInput {
    pub input: Vec<f32>,
    pub output: Vec<f32>,
    pub interpolation: AnimInterpolationInput,
    pub in_tangents: Option<Vec<f32>>,
    pub out_tangents: Option<Vec<f32>>,
}

#[cfg(feature = "write")]
#[derive(Clone, Copy)]
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
#[derive(Default)]
pub struct ExportOptions {
    /// FBX version (default: 7500 for FBX 7.5)
    pub version: Option<u32>,
    /// The space the caller wrote the geometry in, declared verbatim.
    ///
    /// A flat mesh list carries no hierarchy and no statement about its own
    /// coordinates, so without this the file fell back to the writer's default
    /// axes -- which said one thing while the caller had written another.
    pub global_settings: Option<GlobalSettingsInput>,
    /// Whether binary FBX arrays should be zlib-compressed when that saves space.
    pub compression: bool,
}

/// Export result.
#[cfg(feature = "write")]
pub struct ExportResult {
    pub success: bool,
    pub binary_data: Option<Vec<u8>>,
    pub error: Option<String>,
    pub fbx_stats: Option<FbxCompressionStats>,
}

/// What the FBX writer actually compressed inside the binary container.
#[cfg(feature = "write")]
pub struct FbxCompressionStats {
    pub requested: bool,
    pub compressed_arrays: usize,
    pub compressed_raw_bytes: usize,
    pub compressed_stored_bytes: usize,
}

#[cfg(feature = "write")]
impl FbxCompressionStats {
    fn from_writer(requested: bool, stats: FbxWriteStats) -> Self {
        Self {
            requested,
            compressed_arrays: stats.compressed_arrays,
            compressed_raw_bytes: stats.compressed_raw_bytes,
            compressed_stored_bytes: stats.compressed_stored_bytes,
        }
    }
}

// ---------------------------------------------------------------------------
// Writer: JavaScript objects -> Rust input structures
//
// The shell feeds the writer back the reader's own output (through
// `structuredClone`), so every numeric field accepts both typed arrays and
// plain arrays. Required fields produce a descriptive error when missing.
// ---------------------------------------------------------------------------

#[cfg(feature = "write")]
fn fbx_compression_stats_to_js(stats: &FbxCompressionStats) -> Object {
    let obj = Object::new();
    set_bool(&obj, "requested", stats.requested);
    set_u32(&obj, "compressed_arrays", stats.compressed_arrays as u32);
    set_u32(
        &obj,
        "compressed_raw_bytes",
        stats.compressed_raw_bytes as u32,
    );
    set_u32(
        &obj,
        "compressed_stored_bytes",
        stats.compressed_stored_bytes as u32,
    );
    obj
}

#[cfg(feature = "write")]
fn export_result_to_js(result: &ExportResult) -> JsValue {
    let obj = Object::new();
    set_bool(&obj, "success", result.success);
    match &result.binary_data {
        Some(data) => set_js(&obj, "binary_data", &u8_array_to_js(data)),
        None => set_js(&obj, "binary_data", &JsValue::NULL),
    }
    set_opt_string_null(&obj, "error", &result.error);
    match &result.fbx_stats {
        Some(stats) => set_js(
            &obj,
            "fbx_stats",
            &fbx_compression_stats_to_js(stats).into(),
        ),
        None => set_js(&obj, "fbx_stats", &JsValue::NULL),
    }
    obj.into()
}

#[cfg(feature = "write")]
fn smoothing_layer_from_js(value: &JsValue) -> Result<SmoothingLayerOutput, String> {
    Ok(SmoothingLayerOutput {
        mapping: opt_string_from_js(value, "mapping"),
        values: required_i32_array(value, "values")?,
    })
}

#[cfg(feature = "write")]
fn crease_layer_from_js(value: &JsValue) -> Result<CreaseLayerOutput, String> {
    Ok(CreaseLayerOutput {
        kind: opt_string_from_js(value, "kind").unwrap_or_else(|| "edge".to_string()),
        mapping: opt_string_from_js(value, "mapping"),
        values: required_f64_array(value, "values")?,
    })
}

#[cfg(feature = "write")]
fn tangent_set_from_js(value: &JsValue) -> Result<TangentSetOutput, String> {
    Ok(TangentSetOutput {
        name: opt_string_from_js(value, "name"),
        mapping: opt_string_from_js(value, "mapping"),
        reference: opt_string_from_js(value, "reference"),
        values: required_f32_array(value, "values")?,
        indices: required_i32_array(value, "indices")?,
        has_handedness: opt_bool_from_js(value, "hasHandedness").unwrap_or(false),
    })
}

#[cfg(feature = "write")]
fn color_set_from_js(value: &JsValue) -> Result<ColorSetOutput, String> {
    Ok(ColorSetOutput {
        name: opt_string_from_js(value, "name"),
        mapping: opt_string_from_js(value, "mapping"),
        reference: opt_string_from_js(value, "reference"),
        values: required_f32_array(value, "values")?,
        indices: required_i32_array(value, "indices")?,
    })
}

#[cfg(feature = "write")]
fn uv_set_from_js(value: &JsValue) -> Result<UvSetOutput, String> {
    Ok(UvSetOutput {
        name: opt_string_from_js(value, "name"),
        mapping: opt_string_from_js(value, "mapping"),
        reference: opt_string_from_js(value, "reference"),
        values: required_f32_array(value, "values")?,
        indices: required_i32_array(value, "indices")?,
    })
}

#[cfg(feature = "write")]
fn normal_set_from_js(value: &JsValue) -> Result<NormalSetOutput, String> {
    Ok(NormalSetOutput {
        name: opt_string_from_js(value, "name"),
        mapping: opt_string_from_js(value, "mapping"),
        reference: opt_string_from_js(value, "reference"),
        values: required_f32_array(value, "values")?,
        indices: required_i32_array(value, "indices")?,
    })
}

/// Read a `Vec<Layer>` where each element is an object.
#[cfg(feature = "write")]
fn layers_from_js<T>(
    value: &JsValue,
    field: &str,
    reader: impl Fn(&JsValue) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    match get_field(value, field) {
        Some(field_value) => {
            let array = field_value
                .dyn_ref::<Array>()
                .ok_or_else(|| format!("{field} must be an array"))?;
            let mut out = Vec::with_capacity(array.length() as usize);
            for index in 0..array.length() {
                out.push(reader(&array.get(index))?);
            }
            Ok(out)
        }
        None => Ok(Vec::new()),
    }
}

#[cfg(feature = "write")]
fn morph_target_input_from_js(value: &JsValue) -> Result<MorphTargetInput, String> {
    Ok(MorphTargetInput {
        name: opt_string_from_js(value, "name"),
        control_point_indices: required_u32_array(value, "controlPointIndices")?,
        position_deltas: required_f32_array(value, "positionDeltas")?,
        normal_deltas: optional_f32_array(value, "normalDeltas")?,
        default_weight: opt_f64_from_js(value, "defaultWeight").unwrap_or(0.0) as f32,
        full_weight: opt_f64_from_js(value, "fullWeight")
            .unwrap_or_else(|| default_full_morph_weight() as f64) as f32,
    })
}

#[cfg(feature = "write")]
fn skin_cluster_input_from_js(value: &JsValue) -> Result<SkinClusterInput, String> {
    Ok(SkinClusterInput {
        joint_node_id: required_u32(value, "jointNodeId")?,
        control_point_indices: required_u32_array(value, "controlPointIndices")?,
        render_point_indices: optional_u32_array(value, "renderPointIndices")?.unwrap_or_default(),
        weights: required_f32_array(value, "weights")?,
        mesh_bind_transform: required_f32_array(value, "meshBindTransform")?,
        joint_bind_transform: required_f32_array(value, "jointBindTransform")?,
        armature_bind_transform: optional_f32_array(value, "armatureBindTransform")?,
    })
}

#[cfg(feature = "write")]
fn bind_pose_input_from_js(value: &JsValue) -> Result<BindPoseInput, String> {
    Ok(BindPoseInput {
        node_id: required_u32(value, "nodeId")?,
        matrix: required_f32_array(value, "matrix")?,
    })
}

#[cfg(feature = "write")]
fn skin_input_from_js(value: &JsValue) -> Result<SkinInput, String> {
    Ok(SkinInput {
        clusters: layers_from_js(value, "clusters", skin_cluster_input_from_js)?,
        bind_pose: layers_from_js(value, "bindPose", bind_pose_input_from_js)?,
    })
}

#[cfg(feature = "write")]
fn texture_slot_input_from_js(value: &JsValue) -> Result<TextureSlotInput, String> {
    let text = value
        .as_string()
        .ok_or_else(|| "texture slot must be a string".to_string())?;
    match text.as_str() {
        "diffuse" => Ok(TextureSlotInput::Diffuse),
        "normal" => Ok(TextureSlotInput::Normal),
        "emissive" => Ok(TextureSlotInput::Emissive),
        "specular" => Ok(TextureSlotInput::Specular),
        "roughness" => Ok(TextureSlotInput::Roughness),
        "metallic" => Ok(TextureSlotInput::Metallic),
        "ambient" => Ok(TextureSlotInput::Ambient),
        _ => Err(format!("unknown texture slot: {text}")),
    }
}

#[cfg(feature = "write")]
fn texture_binding_input_from_js(value: &JsValue) -> Result<TextureBindingInput, String> {
    Ok(TextureBindingInput {
        slot: texture_slot_input_from_js(&required_object(value, "slot")?)?,
        texture_index: required_u32(value, "textureIndex")? as usize,
    })
}

#[cfg(feature = "write")]
fn texture_input_from_js(value: &JsValue) -> Result<TextureInput, String> {
    Ok(TextureInput {
        name: opt_string_from_js(value, "name"),
        content: optional_u8_array(value, "content")?,
        filename: opt_string_from_js(value, "filename"),
    })
}

#[cfg(feature = "write")]
fn material_input_from_js(value: &JsValue) -> Result<MaterialInput, String> {
    Ok(MaterialInput {
        name: opt_string_from_js(value, "name"),
        shading_model: opt_string_from_js(value, "shadingModel"),
        diffuse: optional_vec3(value, "diffuse")?,
        specular: optional_vec3(value, "specular")?,
        emissive: optional_vec3(value, "emissive")?,
        ambient: optional_vec3(value, "ambient")?,
        diffuse_factor: opt_f64_from_js(value, "diffuseFactor").map(|v| v as f32),
        specular_factor: opt_f64_from_js(value, "specularFactor").map(|v| v as f32),
        shininess: opt_f64_from_js(value, "shininess").map(|v| v as f32),
        emissive_factor: opt_f64_from_js(value, "emissiveFactor").map(|v| v as f32),
        reflection_factor: opt_f64_from_js(value, "reflectionFactor").map(|v| v as f32),
        transparency_factor: opt_f64_from_js(value, "transparencyFactor").map(|v| v as f32),
        opacity: opt_f64_from_js(value, "opacity").map(|v| v as f32),
        bump_factor: opt_f64_from_js(value, "bumpFactor").map(|v| v as f32),
        textures: layers_from_js(value, "textures", texture_binding_input_from_js)?,
    })
}

#[cfg(feature = "write")]
fn anim_channel_path_input_from_js(value: &JsValue) -> Result<AnimChannelPathInput, String> {
    let text = value
        .as_string()
        .ok_or_else(|| "animation channel path must be a string".to_string())?;
    match text.as_str() {
        "translation" => Ok(AnimChannelPathInput::Translation),
        "rotation" => Ok(AnimChannelPathInput::Rotation),
        "scale" => Ok(AnimChannelPathInput::Scale),
        "morphweight" => Ok(AnimChannelPathInput::MorphWeight),
        _ => Err(format!("unknown animation channel path: {text}")),
    }
}

#[cfg(feature = "write")]
fn anim_interpolation_input_from_js(value: &JsValue) -> Result<AnimInterpolationInput, String> {
    let text = value
        .as_string()
        .ok_or_else(|| "animation interpolation must be a string".to_string())?;
    match text.as_str() {
        "step" => Ok(AnimInterpolationInput::Step),
        "linear" => Ok(AnimInterpolationInput::Linear),
        "cubic" => Ok(AnimInterpolationInput::Cubic),
        _ => Err(format!("unknown animation interpolation: {text}")),
    }
}

#[cfg(feature = "write")]
fn anim_sampler_input_from_js(value: &JsValue) -> Result<AnimSamplerInput, String> {
    Ok(AnimSamplerInput {
        input: required_f32_array(value, "input")?,
        output: required_f32_array(value, "output")?,
        interpolation: anim_interpolation_input_from_js(&required_object(value, "interpolation")?)?,
        in_tangents: optional_f32_array(value, "inTangents")?,
        out_tangents: optional_f32_array(value, "outTangents")?,
    })
}

#[cfg(feature = "write")]
fn anim_channel_input_from_js(value: &JsValue) -> Result<AnimChannelInput, String> {
    Ok(AnimChannelInput {
        node_id: opt_u32_from_js(value, "nodeId").unwrap_or(0),
        node_name: required_string(value, "nodeName")?,
        path: anim_channel_path_input_from_js(&required_object(value, "path")?)?,
        morph_target_index: opt_u32_from_js(value, "morphTargetIndex"),
        sampler: anim_sampler_input_from_js(&required_object(value, "sampler")?)?,
    })
}

#[cfg(feature = "write")]
fn animation_input_from_js(value: &JsValue) -> Result<AnimationInput, String> {
    Ok(AnimationInput {
        name: opt_string_from_js(value, "name"),
        duration: required_f64(value, "duration")? as f32,
        channels: layers_from_js(value, "channels", anim_channel_input_from_js)?,
    })
}

#[cfg(feature = "write")]
fn global_settings_input_from_js(value: &JsValue) -> Result<GlobalSettingsInput, String> {
    Ok(GlobalSettingsInput {
        up_axis: opt_i32_from_js(value, "upAxis"),
        up_axis_sign: opt_i32_from_js(value, "upAxisSign"),
        front_axis: opt_i32_from_js(value, "frontAxis"),
        front_axis_sign: opt_i32_from_js(value, "frontAxisSign"),
        coord_axis: opt_i32_from_js(value, "coordAxis"),
        coord_axis_sign: opt_i32_from_js(value, "coordAxisSign"),
        unit_scale_factor: opt_f64_from_js(value, "unitScaleFactor"),
        original_unit_scale_factor: opt_f64_from_js(value, "originalUnitScaleFactor"),
        time_mode: opt_i32_from_js(value, "timeMode"),
    })
}

#[cfg(feature = "write")]
fn transform_stack_input_from_js(value: &JsValue) -> Result<TransformStackInput, String> {
    Ok(TransformStackInput {
        translation: optional_vec3(value, "translation")?,
        rotation: optional_vec3(value, "rotation")?,
        scaling: optional_vec3(value, "scaling")?,
        rotation_order: opt_i32_from_js(value, "rotationOrder"),
        rotation_active: opt_bool_from_js(value, "rotationActive"),
        pre_rotation: optional_vec3(value, "preRotation")?,
        post_rotation: optional_vec3(value, "postRotation")?,
        rotation_offset: optional_vec3(value, "rotationOffset")?,
        rotation_pivot: optional_vec3(value, "rotationPivot")?,
        scaling_offset: optional_vec3(value, "scalingOffset")?,
        scaling_pivot: optional_vec3(value, "scalingPivot")?,
        inherit_type: opt_i32_from_js(value, "inheritType"),
    })
}

#[cfg(feature = "write")]
fn mesh_input_from_js(value: &JsValue) -> Result<MeshInput, String> {
    Ok(MeshInput {
        name: opt_string_from_js(value, "name"),
        positions: required_f32_array(value, "positions")?,
        indices: required_u32_array(value, "indices")?,
        normals: optional_f32_array(value, "normals")?,
        uvs: optional_f32_array(value, "uvs")?,
        control_points: optional_f32_array(value, "controlPoints")?,
        polygon_vertex_indices: optional_i32_array(value, "polygonVertexIndices")?,
        uv_sets: layers_from_js(value, "uvSets", uv_set_from_js)?,
        normal_sets: layers_from_js(value, "normalSets", normal_set_from_js)?,
        tangent_sets: layers_from_js(value, "tangentSets", tangent_set_from_js)?,
        binormal_sets: layers_from_js(value, "binormalSets", tangent_set_from_js)?,
        smoothing_layers: layers_from_js(value, "smoothingLayers", smoothing_layer_from_js)?,
        crease_layers: layers_from_js(value, "creaseLayers", crease_layer_from_js)?,
        color_sets: layers_from_js(value, "colorSets", color_set_from_js)?,
        edges: optional_i32_array(value, "edges")?.unwrap_or_default(),
        material_indices: optional_i32_array(value, "materialIndices")?.unwrap_or_default(),
        skin: match get_field(value, "skin") {
            Some(skin) => Some(skin_input_from_js(&skin)?),
            None => None,
        },
        morph_targets: layers_from_js(value, "morphTargets", morph_target_input_from_js)?,
    })
}

#[cfg(feature = "write")]
fn scene_node_input_from_js(value: &JsValue) -> Result<SceneNodeInput, String> {
    Ok(SceneNodeInput {
        id: opt_u32_from_js(value, "id").unwrap_or(0),
        name: opt_string_from_js(value, "name"),
        matrix: optional_f32_array(value, "matrix")?,
        transform_stack: match get_field(value, "transformStack") {
            Some(stack) => Some(transform_stack_input_from_js(&stack)?),
            None => None,
        },
        meshes: layers_from_js(value, "meshes", mesh_input_from_js)?,
        children: layers_from_js(value, "children", scene_node_input_from_js)?,
    })
}

#[cfg(feature = "write")]
fn scene_input_from_js(value: &JsValue) -> Result<SceneInput, String> {
    Ok(SceneInput {
        global_settings: match get_field(value, "globalSettings") {
            Some(settings) => Some(global_settings_input_from_js(&settings)?),
            None => None,
        },
        root_nodes: layers_from_js(value, "rootNodes", scene_node_input_from_js)?,
        materials: layers_from_js(value, "materials", material_input_from_js)?,
        textures: layers_from_js(value, "textures", texture_input_from_js)?,
        animations: layers_from_js(value, "animations", animation_input_from_js)?,
    })
}

/// Read `ExportOptions` into `(global_settings, compression)`.
#[cfg(feature = "write")]
fn export_options_from_js(
    options_js: &JsValue,
) -> Result<(Option<GlobalSettingsInput>, bool), String> {
    if !options_js.is_object() {
        return Ok((None, false));
    }
    let global_settings = match get_field(options_js, "globalSettings") {
        Some(settings) => Some(global_settings_input_from_js(&settings)?),
        None => None,
    };
    let compression = opt_bool_from_js(options_js, "compression").unwrap_or(false);
    Ok((global_settings, compression))
}

/// Create FBX binary content from mesh data.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub fn create_fbx(meshes_js: JsValue, options_js: JsValue) -> JsValue {
    let meshes: Result<Vec<MeshInput>, String> = (|| {
        let array = meshes_js
            .dyn_ref::<Array>()
            .ok_or_else(|| "meshes must be an array".to_string())?;
        let mut out = Vec::with_capacity(array.length() as usize);
        for index in 0..array.length() {
            out.push(mesh_input_from_js(&array.get(index))?);
        }
        Ok(out)
    })();
    let meshes = match meshes {
        Ok(m) => m,
        Err(e) => {
            let result = ExportResult {
                success: false,
                binary_data: None,
                error: Some(format!("Invalid mesh data: {e}")),
                fbx_stats: None,
            };
            return export_result_to_js(&result);
        }
    };

    let (global_settings, compression) =
        export_options_from_js(&options_js).unwrap_or((None, false));
    let result = create_fbx_internal(&meshes, global_settings, compression);
    export_result_to_js(&result)
}

/// Create FBX binary content while preserving model hierarchy, materials,
/// textures, animation, and local transforms.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub fn create_fbx_scene(scene_js: JsValue, options_js: JsValue) -> JsValue {
    let (_, compression) = export_options_from_js(&options_js).unwrap_or((None, false));
    let result = match scene_input_from_js(&scene_js) {
        Ok(input) => scene_input_to_fbx_scene(input)
            .and_then(|scene| {
                write_fbx_scene(&scene, compression).map_err(|error| error.to_string())
            })
            .map(|(binary_data, fbx_stats)| ExportResult {
                success: true,
                binary_data: Some(binary_data),
                error: None,
                fbx_stats: Some(fbx_stats),
            })
            .unwrap_or_else(|error| ExportResult {
                success: false,
                binary_data: None,
                error: Some(error),
                fbx_stats: None,
            }),
        Err(error) => ExportResult {
            success: false,
            binary_data: None,
            error: Some(format!("Invalid scene data: {error}")),
            fbx_stats: None,
        },
    };
    export_result_to_js(&result)
}

#[cfg(feature = "write")]
fn scene_input_to_fbx_scene(input: SceneInput) -> Result<FbxScene, String> {
    Ok(FbxScene {
        global_settings: input.global_settings.map(Into::into),
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
        transform_stack: input.transform_stack.map(Into::into),
        has_complex_transform_stack: false,
        mesh_instances: input
            .meshes
            .iter()
            .enumerate()
            .map(|(index, mesh)| mesh_input_to_instance(mesh, index))
            .collect::<Result<_, String>>()?,
        attribute: None,
        children: input
            .children
            .into_iter()
            .map(scene_node_to_fbx)
            .collect::<Result<_, _>>()?,
    })
}

/// Converts one JS mesh payload into the shared `draco-io` mesh instance.
///
/// Both the scene writer and the flat `create_fbx` entry point go through
/// this, so the two paths cannot drift apart.
#[cfg(feature = "write")]
fn tangent_output_to_fbx(set: &TangentSetOutput) -> draco_io::FbxTangentSet {
    draco_io::FbxTangentSet {
        layer: draco_io::FbxLayerSet {
            name: set.name.clone(),
            mapping: set.mapping.clone(),
            reference: set.reference.clone(),
            values: set
                .values
                .chunks_exact(4)
                .map(|v| [v[0], v[1], v[2], v[3]])
                .collect(),
            indices: set.indices.clone(),
        },
        has_handedness: set.has_handedness,
    }
}

#[cfg(feature = "write")]
fn mesh_input_to_instance(mesh: &MeshInput, index: usize) -> Result<FbxMeshInstance, String> {
    Ok(FbxMeshInstance {
        name: mesh.name.clone().or_else(|| Some(format!("mesh_{index}"))),
        mesh: mesh_input_to_core_mesh(mesh)?,
        control_points: mesh
            .control_points
            .as_deref()
            .unwrap_or(&[])
            .chunks_exact(3)
            .map(|value| [value[0], value[1], value[2]])
            .collect(),
        polygon_vertex_indices: mesh.polygon_vertex_indices.clone().unwrap_or_default(),
        edges: mesh.edges.clone(),
        layers: draco_io::FbxMeshLayers {
            smoothing_layers: mesh
                .smoothing_layers
                .iter()
                .map(|layer| draco_io::FbxSmoothingLayer {
                    mapping: layer.mapping.clone(),
                    values: layer.values.clone(),
                })
                .collect(),
            crease_layers: mesh
                .crease_layers
                .iter()
                .map(|layer| draco_io::FbxCreaseLayer {
                    kind: if layer.kind == "vertex" {
                        draco_io::FbxCreaseKind::Vertex
                    } else {
                        draco_io::FbxCreaseKind::Edge
                    },
                    mapping: layer.mapping.clone(),
                    values: layer.values.clone(),
                })
                .collect(),
            tangent_sets: mesh
                .tangent_sets
                .iter()
                .map(tangent_output_to_fbx)
                .collect(),
            binormal_sets: mesh
                .binormal_sets
                .iter()
                .map(tangent_output_to_fbx)
                .collect(),
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
            color_sets: mesh
                .color_sets
                .iter()
                .map(|set| draco_io::FbxColorSet {
                    name: set.name.clone(),
                    mapping: set.mapping.clone(),
                    reference: set.reference.clone(),
                    values: set
                        .values
                        .chunks_exact(4)
                        .map(|value| [value[0], value[1], value[2], value[3]])
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
        },
        material_indices: mesh.material_indices.clone(),
        skin: mesh.skin.as_ref().map(skin_input_to_fbx).transpose()?,
        morph_targets: mesh
            .morph_targets
            .iter()
            .map(morph_target_input_to_fbx)
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

#[cfg(feature = "write")]
fn create_fbx_internal(
    meshes: &[MeshInput],
    global_settings: Option<GlobalSettingsInput>,
    compression: bool,
) -> ExportResult {
    // The legacy `create_fbx` entry point receives a flat mesh list with no
    // hierarchy, so every mesh becomes its own root Model. Materials,
    // textures and animation are not expressible here; callers that need them
    // go through `create_fbx_scene`. Serialization itself is shared with that
    // path so the two cannot emit divergent FBX.
    let scene = match flat_meshes_to_scene(meshes) {
        Ok(mut scene) => {
            scene.global_settings = global_settings.map(Into::into);
            scene
        }
        Err(error) => {
            return ExportResult {
                success: false,
                binary_data: None,
                error: Some(error),
                fbx_stats: None,
            }
        }
    };
    match write_fbx_scene(&scene, compression) {
        Ok((binary_data, fbx_stats)) => ExportResult {
            success: true,
            binary_data: Some(binary_data),
            error: None,
            fbx_stats: Some(fbx_stats),
        },
        Err(error) => ExportResult {
            success: false,
            binary_data: None,
            error: Some(error.to_string()),
            fbx_stats: None,
        },
    }
}

#[cfg(feature = "write")]
fn write_fbx_scene(
    scene: &FbxScene,
    compression: bool,
) -> Result<(Vec<u8>, FbxCompressionStats), std::io::Error> {
    let mut writer = FbxWriter::new().with_compression(compression);
    writer.add_scene(scene)?;
    let (binary_data, stats) = writer.write_to_vec_with_stats()?;
    Ok((
        binary_data,
        FbxCompressionStats::from_writer(compression, stats),
    ))
}

#[cfg(feature = "write")]
fn flat_meshes_to_scene(meshes: &[MeshInput]) -> Result<FbxScene, String> {
    let root_nodes = meshes
        .iter()
        .enumerate()
        .map(|(index, mesh)| {
            Ok(FbxSceneNode {
                id: FbxNodeId(index as u32),
                name: mesh.name.clone().or_else(|| Some(format!("mesh_{index}"))),
                transform: None,
                transform_stack: None,
                has_complex_transform_stack: false,
                mesh_instances: vec![mesh_input_to_instance(mesh, index)?],
                attribute: None,
                children: Vec::new(),
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(FbxScene {
        root_nodes,
        ..FbxScene::default()
    })
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
            global_settings: None,
            root_nodes: vec![FbxSceneNode {
                id: FbxNodeId(1),
                name: Some("Root".to_string()),
                transform: None,
                transform_stack: None,
                has_complex_transform_stack: false,
                mesh_instances: vec![FbxMeshInstance {
                    name: Some("Triangle".to_string()),
                    mesh: triangle_mesh(),
                    ..Default::default()
                }],
                attribute: None,
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
            global_settings: None,
            root_nodes: vec![FbxSceneNode {
                id: FbxNodeId(1),
                name: Some("Root".to_string()),
                transform: None,
                transform_stack: None,
                has_complex_transform_stack: false,
                mesh_instances: Vec::new(),
                attribute: None,
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
            color_sets: Vec::new(),
            tangent_sets: Vec::new(),
            binormal_sets: Vec::new(),
            smoothing_layers: Vec::new(),
            crease_layers: Vec::new(),
            edges: Vec::new(),
            material_indices: Vec::new(),
            skin: None,
            morph_targets: Vec::new(),
        };

        // No `globalSettings`: the writer's own defaults, which is what a caller
        // that states no space gets.
        let result = create_fbx_internal(&[mesh], None, false);
        assert!(result.success);
        assert!(result.binary_data.is_some());

        let data = result.binary_data.unwrap();
        assert!(data.len() > 27);
        assert_eq!(&data[0..21], FBX_MAGIC);
    }
}
