//! Compact glTF/GLB reader for the browser.
//!
//! The WASM path uses a checked `nanoserde` schema and shares strict GLB
//! container validation with `draco-io`. Native callers use the full
//! `draco_io::GltfReader`; keeping the browser schema local avoids pulling the
//! serde-backed document model into the 100 KiB reader budget.

#![allow(clippy::question_mark)]

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::GeometryAttributeType;
#[cfg(test)]
use draco_core::geometry_attribute::PointAttribute;
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
#[cfg(test)]
use draco_io::{GltfError, GltfReader, ResourceLimits, ResourceResolver};
use nanoserde::{DeJson, SerJson};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// Mesh data structure for JavaScript interop.
#[derive(SerJson, Clone, Default)]
pub struct MeshData {
    /// Mesh name.
    pub name: Option<String>,
    /// Vertex positions as `[x0, y0, z0, ...]`.
    pub positions: Vec<f32>,
    /// Triangle indices as `[i0, i1, i2, ...]`.
    pub indices: Vec<u32>,
    /// Vertex normals, when present.
    pub normals: Vec<f32>,
    /// First texture-coordinate set, when present.
    pub uvs: Vec<f32>,
    /// First color set, when present.
    pub colors: Vec<f32>,
}

/// Node in the scene graph.
#[derive(SerJson, Clone, Default)]
pub struct SceneNode {
    pub name: Option<String>,
    #[nserde(rename = "meshIndex")]
    pub mesh_index: Option<usize>,
    pub translation: Option<Vec<f32>>,
    pub rotation: Option<Vec<f32>>,
    pub scale: Option<Vec<f32>>,
    pub children: Vec<usize>,
}

/// Scene data.
#[derive(SerJson, Clone, Default)]
pub struct SceneData {
    pub name: Option<String>,
    pub nodes: Vec<usize>,
}

/// Parse result containing decoded geometry and scene metadata.
#[derive(SerJson, Default)]
pub struct ParseResult {
    pub success: bool,
    pub meshes: Vec<MeshData>,
    pub scenes: Vec<SceneData>,
    pub nodes: Vec<SceneNode>,
    #[nserde(rename = "defaultScene")]
    pub default_scene: Option<usize>,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    #[nserde(rename = "usesDraco")]
    pub uses_draco: bool,
}

#[cfg(test)]
impl ParseResult {
    fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(message.into()),
            ..Self::default()
        }
    }
}

fn copy_uint8_array(array: &js_sys::Uint8Array) -> Result<Vec<u8>, String> {
    let len = usize::try_from(array.length())
        .map_err(|_| "Companion resource is too large".to_string())?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|_| "Companion resource is too large".to_string())?;
    bytes.resize(len, 0);
    array.copy_to(&mut bytes);
    Ok(bytes)
}

#[cfg(test)]
struct CompanionResolver {
    resources: Vec<(String, Vec<u8>)>,
}

#[cfg(test)]
impl ResourceResolver for CompanionResolver {
    fn resolve(&self, uri: &str) -> Result<Vec<u8>, GltfError> {
        let resource = self
            .resources
            .iter()
            .find_map(|(candidate, bytes)| (candidate == uri).then_some(bytes))
            .ok_or_else(|| GltfError::InvalidGltf(format!("missing external resource: {uri}")))?;
        copy_bytes(resource, &format!("external resource {uri}"))
    }
}

#[cfg(test)]
fn copy_bytes(data: &[u8], what: &str) -> Result<Vec<u8>, GltfError> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(data.len()).map_err(|_| {
        GltfError::ResourceLimitExceeded(format!(
            "failed to allocate {what} ({} bytes)",
            data.len()
        ))
    })?;
    copy.extend_from_slice(data);
    Ok(copy)
}

/// Initialize the WASM module.
#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Get the crate version used to build this module.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Get the module name.
#[wasm_bindgen]
pub fn module_name() -> String {
    "glTF Reader".to_string()
}

/// Get supported file extensions.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["gltf".to_string(), "glb".to_string()]
}

fn to_js_value(result: &ParseResult) -> JsValue {
    let json = SerJson::serialize_json(result);
    js_sys::JSON::parse(&json).unwrap_or(JsValue::NULL)
}

/// Parse an embedded/self-contained glTF JSON document.
#[wasm_bindgen]
pub fn parse_gltf(json_content: &str) -> JsValue {
    to_js_value(&parse_gltf_json_with_resources(json_content, None, &[]))
}

/// Parse a GLB document.
#[wasm_bindgen]
pub fn parse_glb(data: &[u8]) -> JsValue {
    to_js_value(&parse_glb_internal_compact(data))
}

/// Parse glTF/GLB bytes with a map of companion URI to exact resource bytes.
///
/// JavaScript should pass an object such as
/// `{ "model.bin": Uint8Array, "albedo.png": Uint8Array }`. Missing external
/// buffers are reported as controlled errors; the resolver never reads files.
#[wasm_bindgen]
pub fn parse_gltf_with_resources(data: &[u8], resources_js: JsValue) -> JsValue {
    let resources = match parse_companion_resources(resources_js) {
        Ok(resources) => resources,
        Err(error) => {
            return to_js_value(&ParseResult {
                success: false,
                error: Some(error),
                ..Default::default()
            });
        }
    };
    let result = match draco_io::parse_glb_json_and_bin(data) {
        Ok((json, bin)) => match std::str::from_utf8(json) {
            Ok(json) => parse_gltf_json_with_resources(json, bin, &resources),
            Err(_) => ParseResult {
                success: false,
                error: Some("Invalid glTF JSON".to_string()),
                ..Default::default()
            },
        },
        Err(_) => match std::str::from_utf8(data) {
            Ok(json) => parse_gltf_json_with_resources(json, None, &resources),
            Err(_) => ParseResult {
                success: false,
                error: Some("Invalid glTF JSON".to_string()),
                ..Default::default()
            },
        },
    };
    to_js_value(&result)
}

fn parse_companion_resources(resources_js: JsValue) -> Result<Vec<(String, Vec<u8>)>, String> {
    if !resources_js.is_object() || resources_js.is_null() {
        return Err("expected an object whose values are Uint8Array instances".to_string());
    }
    let object: js_sys::Object = resources_js.unchecked_into();
    let entries = js_sys::Object::entries(&object);
    let mut resources = Vec::new();
    resources
        .try_reserve(entries.length() as usize)
        .map_err(|_| "Companion resource map is too large".to_string())?;
    for index in 0..entries.length() {
        let pair = js_sys::Array::from(&entries.get(index));
        if pair.length() != 2 {
            return Err("Invalid companion resource map".to_string());
        }
        let uri = pair
            .get(0)
            .as_string()
            .ok_or("Companion resource URI is not a string")?;
        let value = pair.get(1);
        if !value.is_instance_of::<js_sys::Uint8Array>() {
            return Err("Invalid companion resource map".to_string());
        }
        let bytes = copy_uint8_array(&js_sys::Uint8Array::new(&value))?;
        if resources.iter().any(|(candidate, _)| candidate == &uri) {
            return Err("Duplicate companion resource URI".to_string());
        }
        resources.push((uri, bytes));
    }
    Ok(resources)
}

#[cfg(test)]
fn parse_document(data: &[u8], resolver: Option<&dyn ResourceResolver>) -> ParseResult {
    match parse_document_result(data, resolver) {
        Ok(result) => result,
        Err(error) => ParseResult::error(error.to_string()),
    }
}

#[cfg(test)]
fn parse_document_result(
    data: &[u8],
    resolver: Option<&dyn ResourceResolver>,
) -> Result<ParseResult, GltfError> {
    let reader = match resolver {
        Some(resolver) => GltfReader::from_bytes_lenient_with_resolver(
            data,
            resolver,
            &ResourceLimits::default(),
        )?,
        None => GltfReader::from_bytes_lenient(data)?,
    };
    let metadata = reader.document_metadata();
    let decoded = reader.decode_all_meshes()?;
    if decoded.is_empty() {
        return Err(GltfError::InvalidGltf(
            "document contains no decodable mesh primitives".into(),
        ));
    }

    let names = metadata.primitive_names;
    if names.len() != decoded.len() {
        return Err(GltfError::InvalidGltf(format!(
            "decoded primitive count {} does not match document primitive count {}",
            decoded.len(),
            names.len()
        )));
    }

    let mut meshes = Vec::new();
    meshes
        .try_reserve_exact(decoded.len())
        .map_err(|_| GltfError::InvalidGltf("failed to allocate mesh result".into()))?;
    for (mesh, name) in decoded.iter().zip(names) {
        meshes.push(mesh_to_data(mesh, name)?);
    }

    let nodes = metadata
        .nodes
        .into_iter()
        .map(|node| SceneNode {
            name: node.name,
            mesh_index: node.mesh,
            translation: node.translation.map(|value| value.to_vec()),
            rotation: node.rotation.map(|value| value.to_vec()),
            scale: node.scale.map(|value| value.to_vec()),
            children: node.children,
        })
        .collect();
    let scenes = metadata
        .scenes
        .into_iter()
        .map(|scene| SceneData {
            name: scene.name,
            nodes: scene.nodes,
        })
        .collect();
    Ok(ParseResult {
        success: true,
        meshes,
        scenes,
        nodes,
        default_scene: metadata.default_scene,
        error: None,
        warnings: Vec::new(),
        uses_draco: metadata.uses_draco,
    })
}

#[cfg(test)]
fn mesh_to_data(mesh: &Mesh, name: Option<String>) -> Result<MeshData, GltfError> {
    let positions = read_named_attribute(mesh, GeometryAttributeType::Position, &[3])?
        .ok_or_else(|| GltfError::InvalidGltf("decoded mesh has no POSITION attribute".into()))?;
    if positions.is_empty() {
        return Err(GltfError::InvalidGltf(
            "decoded mesh has an empty POSITION attribute".into(),
        ));
    }

    let normals =
        read_named_attribute(mesh, GeometryAttributeType::Normal, &[3])?.unwrap_or_default();
    let uvs =
        read_named_attribute(mesh, GeometryAttributeType::TexCoord, &[2])?.unwrap_or_default();
    let colors =
        read_named_attribute(mesh, GeometryAttributeType::Color, &[3, 4])?.unwrap_or_default();

    let index_count = mesh
        .num_faces()
        .checked_mul(3)
        .ok_or_else(|| GltfError::InvalidGltf("triangle index count overflow".into()))?;
    let mut indices = Vec::new();
    indices
        .try_reserve_exact(index_count)
        .map_err(|_| GltfError::InvalidGltf("failed to allocate triangle indices".into()))?;
    for face_index in 0..mesh.num_faces() {
        let face = mesh
            .face(FaceIndex(u32::try_from(face_index).map_err(|_| {
                GltfError::InvalidGltf("face index exceeds u32".into())
            })?));
        for point in face {
            if point.0 as usize >= mesh.num_points() {
                return Err(GltfError::InvalidGltf(format!(
                    "decoded face references point {} but mesh has {} points",
                    point.0,
                    mesh.num_points()
                )));
            }
            indices.push(point.0);
        }
    }

    Ok(MeshData {
        name,
        positions,
        indices,
        normals,
        uvs,
        colors,
    })
}

#[cfg(test)]
fn read_named_attribute(
    mesh: &Mesh,
    attribute_type: GeometryAttributeType,
    allowed_components: &[u8],
) -> Result<Option<Vec<f32>>, GltfError> {
    let id = mesh.named_attribute_id(attribute_type);
    if id < 0 {
        return Ok(None);
    }
    let attribute = mesh
        .try_attribute(id)
        .map_err(|error| GltfError::InvalidGltf(format!("invalid decoded attribute: {error}")))?;
    if !allowed_components.contains(&attribute.num_components()) {
        return Err(GltfError::InvalidGltf(format!(
            "decoded {:?} attribute has {} components; expected one of {:?}",
            attribute_type,
            attribute.num_components(),
            allowed_components
        )));
    }

    let components = usize::from(attribute.num_components());
    let value_count = mesh
        .num_points()
        .checked_mul(components)
        .ok_or_else(|| GltfError::InvalidGltf("attribute result size overflow".into()))?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(value_count)
        .map_err(|_| GltfError::InvalidGltf("failed to allocate attribute result".into()))?;

    for point_index in 0..mesh.num_points() {
        let point = PointIndex(
            u32::try_from(point_index)
                .map_err(|_| GltfError::InvalidGltf("point index exceeds u32".into()))?,
        );
        let mapped = attribute.mapped_index(point);
        if mapped.0 == u32::MAX {
            return Err(GltfError::InvalidGltf(format!(
                "decoded {:?} attribute has no value for point {point_index}",
                attribute_type
            )));
        }
        read_attribute_value(attribute, mapped.0 as usize, &mut values)?;
    }
    Ok(Some(values))
}

#[cfg(test)]
fn read_attribute_value(
    attribute: &PointAttribute,
    value_index: usize,
    output: &mut Vec<f32>,
) -> Result<(), GltfError> {
    let stride = usize::try_from(attribute.byte_stride())
        .map_err(|_| GltfError::InvalidGltf("decoded attribute has a negative stride".into()))?;
    let component_size = match attribute.data_type() {
        DataType::Int8 | DataType::Uint8 => 1,
        DataType::Int16 | DataType::Uint16 => 2,
        DataType::Float32 => 4,
        DataType::Int32
        | DataType::Uint32
        | DataType::Int64
        | DataType::Uint64
        | DataType::Float64
        | DataType::Bool
        | DataType::Invalid => {
            return Err(GltfError::InvalidGltf(
                "decoded attribute uses a component type that glTF 2.0 vertex attributes cannot represent"
                    .into(),
            ));
        }
    };
    let row_size = usize::from(attribute.num_components())
        .checked_mul(component_size)
        .ok_or_else(|| GltfError::InvalidGltf("decoded attribute row size overflow".into()))?;
    if stride < row_size {
        return Err(GltfError::InvalidGltf(format!(
            "decoded attribute stride {stride} is smaller than row size {row_size}"
        )));
    }
    let start = value_index
        .checked_mul(stride)
        .ok_or_else(|| GltfError::InvalidGltf("decoded attribute offset overflow".into()))?;
    let end = start
        .checked_add(row_size)
        .ok_or_else(|| GltfError::InvalidGltf("decoded attribute range overflow".into()))?;
    let row = attribute.buffer().data().get(start..end).ok_or_else(|| {
        GltfError::InvalidGltf("decoded attribute value extends past its buffer".into())
    })?;
    for component in row.chunks_exact(component_size) {
        output.push(scalar_to_f32(
            attribute.data_type(),
            attribute.normalized(),
            component,
        )?);
    }
    Ok(())
}

#[cfg(test)]
fn scalar_to_f32(data_type: DataType, normalized: bool, bytes: &[u8]) -> Result<f32, GltfError> {
    let value = match data_type {
        DataType::Int8 => normalize_signed(
            i8::from_le_bytes(exact_bytes(bytes)?) as i32,
            i8::MAX as i32,
            normalized,
        ),
        DataType::Uint8 => normalize_unsigned(
            u8::from_le_bytes(exact_bytes(bytes)?) as u32,
            u8::MAX as u32,
            normalized,
        ),
        DataType::Int16 => normalize_signed(
            i16::from_le_bytes(exact_bytes(bytes)?) as i32,
            i16::MAX as i32,
            normalized,
        ),
        DataType::Uint16 => normalize_unsigned(
            u16::from_le_bytes(exact_bytes(bytes)?) as u32,
            u16::MAX as u32,
            normalized,
        ),
        DataType::Float32 => f32::from_le_bytes(exact_bytes(bytes)?),
        DataType::Int32
        | DataType::Uint32
        | DataType::Int64
        | DataType::Uint64
        | DataType::Float64
        | DataType::Bool
        | DataType::Invalid => {
            return Err(GltfError::InvalidGltf(
                "decoded attribute uses a component type that glTF 2.0 vertex attributes cannot represent"
                    .into(),
            ));
        }
    };
    if !value.is_finite() {
        return Err(GltfError::InvalidGltf(
            "decoded attribute contains a non-finite value".into(),
        ));
    }
    Ok(value)
}

#[cfg(test)]
fn exact_bytes<const N: usize>(bytes: &[u8]) -> Result<[u8; N], GltfError> {
    bytes.try_into().map_err(|_| {
        GltfError::InvalidGltf(format!(
            "decoded scalar has {} bytes, expected {N}",
            bytes.len()
        ))
    })
}

#[cfg(test)]
fn normalize_signed(value: i32, max: i32, normalized: bool) -> f32 {
    if normalized {
        ((value as f32) / (max as f32)).max(-1.0)
    } else {
        value as f32
    }
}

#[cfg(test)]
fn normalize_unsigned(value: u32, max: u32, normalized: bool) -> f32 {
    if normalized {
        (value as f32) / (max as f32)
    } else {
        value as f32
    }
}

// The WASM path deliberately keeps a compact nanoserde schema.  Native
// callers still use the full draco-io reader; this path performs the checks
// needed before touching any buffer or Draco attribute data.
#[derive(DeJson, Default)]
struct LiteRoot {
    #[nserde(default)]
    accessors: Vec<LiteAccessor>,
    #[nserde(default, rename = "bufferViews")]
    buffer_views: Vec<LiteBufferView>,
    #[nserde(default)]
    buffers: Vec<LiteBuffer>,
    #[nserde(default)]
    images: Vec<LiteImage>,
    #[nserde(default)]
    meshes: Vec<LiteMesh>,
    #[nserde(default)]
    nodes: Vec<LiteNode>,
    #[nserde(default)]
    scenes: Vec<LiteScene>,
    #[nserde(default)]
    scene: Option<usize>,
    #[nserde(default, rename = "extensionsUsed")]
    extensions_used: Vec<String>,
    #[nserde(default, rename = "extensionsRequired")]
    extensions_required: Vec<String>,
}

#[derive(DeJson, Default)]
struct LiteAccessor {
    #[nserde(default, rename = "bufferView")]
    buffer_view: Option<usize>,
    #[nserde(default, rename = "byteOffset")]
    byte_offset: Option<usize>,
    #[nserde(default, rename = "componentType")]
    component_type: u32,
    #[nserde(default)]
    count: usize,
    #[nserde(default, rename = "type")]
    accessor_type: String,
    #[nserde(default)]
    normalized: bool,
    #[nserde(default)]
    sparse: Option<LiteSparse>,
}

#[derive(DeJson, Default)]
struct LiteSparse {}

#[derive(DeJson, Default)]
struct LiteBufferView {
    #[nserde(default)]
    buffer: usize,
    #[nserde(default, rename = "byteOffset")]
    byte_offset: Option<usize>,
    #[nserde(default, rename = "byteLength")]
    byte_length: usize,
    #[nserde(default, rename = "byteStride")]
    byte_stride: Option<usize>,
}

#[derive(DeJson, Default)]
struct LiteBuffer {
    #[nserde(default, rename = "byteLength")]
    byte_length: usize,
    #[nserde(default)]
    uri: Option<String>,
}

#[derive(DeJson, Default)]
struct LiteImage {
    #[nserde(default)]
    uri: Option<String>,
}

#[derive(DeJson, Default)]
struct LiteMesh {
    #[nserde(default)]
    name: Option<String>,
    #[nserde(default)]
    primitives: Vec<LitePrimitive>,
}

#[derive(DeJson, Default)]
struct LitePrimitive {
    #[nserde(default)]
    attributes: HashMap<String, u32>,
    #[nserde(default)]
    indices: Option<u32>,
    #[nserde(default)]
    mode: Option<u32>,
    #[nserde(default)]
    extensions: Option<LitePrimitiveExtensions>,
}

#[derive(DeJson, Default)]
struct LitePrimitiveExtensions {
    #[nserde(default, rename = "KHR_draco_mesh_compression")]
    khr_draco: Option<LiteDracoExtension>,
}

#[derive(DeJson, Default)]
struct LiteDracoExtension {
    #[nserde(default, rename = "bufferView")]
    buffer_view: Option<usize>,
    #[nserde(default)]
    attributes: HashMap<String, u32>,
}

#[derive(DeJson, Default)]
struct LiteNode {
    #[nserde(default)]
    name: Option<String>,
    #[nserde(default)]
    mesh: Option<usize>,
    #[nserde(default)]
    translation: Option<Vec<f32>>,
    #[nserde(default)]
    rotation: Option<Vec<f32>>,
    #[nserde(default)]
    scale: Option<Vec<f32>>,
    #[nserde(default)]
    children: Vec<usize>,
}

#[derive(DeJson, Default)]
struct LiteScene {
    #[nserde(default)]
    name: Option<String>,
    #[nserde(default)]
    nodes: Vec<usize>,
}

fn lite_error(message: &'static str) -> ParseResult {
    ParseResult {
        success: false,
        error: Some(message.to_string()),
        ..Default::default()
    }
}

fn parse_glb_internal_compact(data: &[u8]) -> ParseResult {
    match draco_io::parse_glb_json_and_bin(data) {
        Ok((json, bin)) => match std::str::from_utf8(json) {
            Ok(json) => parse_gltf_json_with_resources(json, bin, &[]),
            Err(_) => lite_error("Invalid GLB JSON"),
        },
        Err(_) => lite_error("Invalid GLB"),
    }
}

fn parse_gltf_json_with_resources(
    json_content: &str,
    bin_buffer: Option<&[u8]>,
    resources: &[(String, Vec<u8>)],
) -> ParseResult {
    let root: LiteRoot = match DeJson::deserialize_json(json_content) {
        Ok(root) => root,
        Err(_) => return lite_error("Failed to parse glTF JSON"),
    };
    let resolved = if bin_buffer.is_none() {
        root.buffers
            .iter()
            .find_map(|buffer| buffer.uri.as_deref())
            .map(|uri| resolve_buffer_uri(uri, resources))
            .transpose()
    } else {
        Ok(None)
    };
    let resolved = match resolved {
        Ok(value) => value,
        Err(error) => {
            return ParseResult {
                success: false,
                error: Some(error),
                ..Default::default()
            }
        }
    };
    let buffer = bin_buffer.or(resolved.as_deref());
    for image in &root.images {
        if let Some(uri) = image.uri.as_deref() {
            if uri.starts_with("data:") {
                if decode_lite_data_uri(uri).is_err() {
                    return lite_error("Invalid image data URI");
                }
            } else if !resources.iter().any(|(candidate, _)| candidate == uri) {
                return ParseResult {
                    success: false,
                    error: Some(uri.to_string()),
                    ..Default::default()
                };
            }
        }
    }
    if validate_lite_document(&root, buffer).is_err() {
        return lite_error("Invalid glTF buffer or accessor contract");
    }

    let uses_draco = root
        .extensions_used
        .iter()
        .any(|name| name == "KHR_draco_mesh_compression");
    let mut meshes = Vec::new();
    if meshes
        .try_reserve_exact(root.meshes.iter().map(|mesh| mesh.primitives.len()).sum())
        .is_err()
    {
        return lite_error("Mesh result is too large");
    }
    for gltf_mesh in &root.meshes {
        for primitive in &gltf_mesh.primitives {
            let mut mesh = MeshData {
                name: gltf_mesh.name.clone(),
                ..Default::default()
            };
            if let Some(draco) = primitive
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.khr_draco.as_ref())
            {
                let view = match draco.buffer_view {
                    Some(view) => view,
                    None => return lite_error("KHR Draco bufferView is missing"),
                };
                let bytes = match buffer
                    .and_then(|data| lite_buffer_view(&root.buffer_views, view, data))
                {
                    Some(bytes) => bytes,
                    None => return lite_error("KHR Draco bufferView is out of range"),
                };
                match decode_draco_mesh_compact(bytes, &draco.attributes) {
                    Ok(mut decoded) => {
                        decoded.name = gltf_mesh.name.clone();
                        meshes.push(decoded);
                        continue;
                    }
                    Err(_) => return lite_error("Failed to decode Draco mesh"),
                }
            }
            let data = match buffer {
                Some(data) => data,
                None => return lite_error("Missing buffer"),
            };
            if let Some(&index) = primitive.attributes.get("POSITION") {
                mesh.positions =
                    match read_lite_vec3(&root.accessors, &root.buffer_views, data, index) {
                        Ok(values) => values,
                        Err(_) => return lite_error("Invalid POSITION accessor"),
                    };
            }
            if let Some(&index) = primitive.attributes.get("NORMAL") {
                mesh.normals =
                    match read_lite_vec3(&root.accessors, &root.buffer_views, data, index) {
                        Ok(values) => values,
                        Err(_) => return lite_error("Invalid NORMAL accessor"),
                    };
            }
            if let Some(&index) = primitive.attributes.get("TEXCOORD_0") {
                mesh.uvs = match read_lite_vec2(&root.accessors, &root.buffer_views, data, index) {
                    Ok(values) => values,
                    Err(_) => return lite_error("Invalid TEXCOORD accessor"),
                };
            }
            if let Some(index) = primitive.indices {
                mesh.indices =
                    match read_lite_indices(&root.accessors, &root.buffer_views, data, index) {
                        Ok(values) => values,
                        Err(_) => return lite_error("Invalid index accessor"),
                    };
            }
            if primitive.mode.unwrap_or(4) == 5 {
                if mesh.indices.is_empty() {
                    let count = mesh.positions.len() / 3;
                    mesh.indices = match (0..count).map(u32::try_from).collect() {
                        Ok(values) => values,
                        Err(_) => return lite_error("Vertex count exceeds u32"),
                    };
                }
                mesh.indices = triangulate_strip_compact(&mesh.indices);
            } else if mesh.indices.is_empty() {
                let count = mesh.positions.len() / 3;
                if !count.is_multiple_of(3) {
                    return lite_error("Non-indexed TRIANGLES count is not divisible by three");
                }
                mesh.indices = match (0..count).map(u32::try_from).collect() {
                    Ok(values) => values,
                    Err(_) => return lite_error("Vertex count exceeds u32"),
                };
            }
            if !mesh.indices.len().is_multiple_of(3)
                || mesh
                    .indices
                    .iter()
                    .any(|&index| index as usize >= mesh.positions.len() / 3)
            {
                return lite_error("Invalid triangle indices");
            }
            meshes.push(mesh);
        }
    }
    if meshes.is_empty() {
        return lite_error("Document contains no mesh primitives");
    }
    let nodes = root
        .nodes
        .into_iter()
        .map(|node| SceneNode {
            name: node.name,
            mesh_index: node.mesh,
            translation: node.translation,
            rotation: node.rotation,
            scale: node.scale,
            children: node.children,
        })
        .collect();
    let scenes = root
        .scenes
        .into_iter()
        .map(|scene| SceneData {
            name: scene.name,
            nodes: scene.nodes,
        })
        .collect();
    ParseResult {
        success: true,
        meshes,
        scenes,
        nodes,
        default_scene: root.scene,
        error: None,
        warnings: Vec::new(),
        uses_draco,
    }
}

fn resolve_buffer_uri(uri: &str, resources: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    if uri.starts_with("data:") {
        return decode_lite_data_uri(uri).map_err(|_| "Invalid buffer data URI".to_string());
    }
    resources
        .iter()
        .find(|(candidate, _)| candidate == uri)
        .map(|(_, bytes)| bytes.clone())
        .ok_or_else(|| uri.to_string())
}

fn decode_lite_data_uri(uri: &str) -> Result<Vec<u8>, ()> {
    let (header, payload) = uri.split_once(',').ok_or(())?;
    if header
        .as_bytes()
        .windows(7)
        .any(|window| window.eq_ignore_ascii_case(b";base64"))
    {
        if !payload.len().is_multiple_of(4) {
            return Err(());
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(payload.len().saturating_mul(3) / 4)
            .map_err(|_| ())?;
        let bytes = payload.as_bytes();
        for (chunk_index, chunk) in bytes.chunks_exact(4).enumerate() {
            let last = chunk_index * 4 + 4 == bytes.len();
            let a = lite_base64_value(chunk[0])?;
            let b = lite_base64_value(chunk[1])?;
            output.push((a << 2) | (b >> 4));
            if chunk[2] == b'=' {
                if !last || chunk[3] != b'=' || b & 0x0f != 0 {
                    return Err(());
                }
                continue;
            }
            let c = lite_base64_value(chunk[2])?;
            output.push((b << 4) | (c >> 2));
            if chunk[3] == b'=' {
                if !last || c & 0x03 != 0 {
                    return Err(());
                }
            } else {
                let d = lite_base64_value(chunk[3])?;
                output.push((c << 6) | d);
            }
        }
        return Ok(output);
    }
    let bytes = payload.as_bytes();
    let mut output = Vec::new();
    output.try_reserve_exact(bytes.len()).map_err(|_| ())?;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index
                .checked_add(2)
                .filter(|end| *end < bytes.len())
                .is_none()
            {
                return Err(());
            }
            let high = lite_hex(bytes[index + 1])?;
            let low = lite_hex(bytes[index + 2])?;
            output.push((high << 4) | low);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    Ok(output)
}

fn lite_base64_value(byte: u8) -> Result<u8, ()> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(()),
    }
}

fn lite_hex(byte: u8) -> Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(()),
    }
}

fn lite_buffer_view<'a>(
    views: &[LiteBufferView],
    index: usize,
    data: &'a [u8],
) -> Option<&'a [u8]> {
    let view = views.get(index)?;
    let start = view.byte_offset.unwrap_or(0);
    let end = start.checked_add(view.byte_length)?;
    data.get(start..end)
}

fn validate_lite_document(root: &LiteRoot, data: Option<&[u8]>) -> Result<(), ()> {
    let draco_used = root
        .extensions_used
        .iter()
        .any(|name| name == "KHR_draco_mesh_compression");
    if root
        .extensions_required
        .iter()
        .any(|name| name != "KHR_draco_mesh_compression")
    {
        return Err(());
    }
    if root
        .extensions_required
        .iter()
        .any(|name| name == "KHR_draco_mesh_compression")
        && !draco_used
    {
        return Err(());
    }
    let data = match data {
        Some(data) => data,
        None if root.buffer_views.is_empty()
            && root.meshes.iter().all(|mesh| mesh.primitives.is_empty()) =>
        {
            return Ok(())
        }
        None => return Err(()),
    };
    if root
        .buffers
        .first()
        .map(|buffer| buffer.byte_length > data.len())
        .unwrap_or(false)
    {
        return Err(());
    }
    for view in &root.buffer_views {
        if view.buffer != 0
            || view
                .byte_offset
                .unwrap_or(0)
                .checked_add(view.byte_length)
                .filter(|end| *end <= data.len())
                .is_none()
        {
            return Err(());
        }
        if let Some(stride) = view.byte_stride {
            if !(4..=252).contains(&stride) || stride % 4 != 0 {
                return Err(());
            }
        }
    }
    for (node_index, node) in root.nodes.iter().enumerate() {
        if node.mesh.is_some_and(|mesh| mesh >= root.meshes.len())
            || node
                .children
                .iter()
                .any(|&child| child >= root.nodes.len() || child == node_index)
        {
            return Err(());
        }
        for value in [&node.translation, &node.scale] {
            if value.as_ref().is_some_and(|values| {
                values.len() != 3 || values.iter().any(|value| !value.is_finite())
            }) {
                return Err(());
            }
        }
        if node.rotation.as_ref().is_some_and(|values| {
            values.len() != 4 || values.iter().any(|value| !value.is_finite())
        }) {
            return Err(());
        }
    }
    for scene in &root.scenes {
        if scene.nodes.iter().any(|&node| node >= root.nodes.len()) {
            return Err(());
        }
    }
    for mesh in &root.meshes {
        for primitive in &mesh.primitives {
            if !primitive.attributes.contains_key("POSITION")
                || !matches!(primitive.mode.unwrap_or(4), 4 | 5)
            {
                return Err(());
            }
            let has_draco = primitive
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.khr_draco.as_ref())
                .is_some();
            for &accessor in primitive.attributes.values() {
                validate_lite_accessor(root, data, accessor, has_draco)?;
            }
            if let Some(index) = primitive.indices {
                validate_lite_accessor(root, data, index, has_draco)?;
            }
            if let Some(draco) = primitive
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.khr_draco.as_ref())
            {
                let view = draco.buffer_view.ok_or(())?;
                if draco.attributes.is_empty()
                    || lite_buffer_view(&root.buffer_views, view, data).is_none()
                {
                    return Err(());
                }
                if draco
                    .attributes
                    .keys()
                    .any(|semantic| !primitive.attributes.contains_key(semantic))
                {
                    return Err(());
                }
            }
        }
    }
    Ok(())
}

fn validate_lite_accessor(
    root: &LiteRoot,
    data: &[u8],
    index: u32,
    allow_missing_view: bool,
) -> Result<(), ()> {
    let accessor = root
        .accessors
        .get(usize::try_from(index).map_err(|_| ())?)
        .ok_or(())?;
    if accessor.sparse.is_some() {
        return Err(());
    }
    let components: usize = match accessor.accessor_type.as_str() {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        _ => return Err(()),
    };
    let width = match accessor.component_type {
        5121 => 1,
        5123 => 2,
        5125 | 5126 => 4,
        _ => return Err(()),
    };
    let view = match accessor.buffer_view {
        Some(index) => root.buffer_views.get(index).ok_or(())?,
        None if allow_missing_view => return Ok(()),
        None => return Err(()),
    };
    let row = components.checked_mul(width).ok_or(())?;
    let stride = view.byte_stride.unwrap_or(row);
    if stride < row {
        return Err(());
    }
    let bytes = if accessor.count == 0 {
        0
    } else {
        accessor
            .count
            .checked_sub(1)
            .ok_or(())?
            .checked_mul(stride)
            .ok_or(())?
            .checked_add(row)
            .ok_or(())?
    };
    let start = view
        .byte_offset
        .unwrap_or(0)
        .checked_add(accessor.byte_offset.unwrap_or(0))
        .ok_or(())?;
    if start
        .checked_add(bytes)
        .filter(|end| *end <= data.len())
        .is_none()
    {
        return Err(());
    }
    Ok(())
}

fn lite_accessor<'a>(
    accessors: &'a [LiteAccessor],
    views: &'a [LiteBufferView],
    _data: &[u8],
    index: u32,
) -> Result<(&'a LiteAccessor, &'a LiteBufferView, usize), ()> {
    let accessor = accessors
        .get(usize::try_from(index).map_err(|_| ())?)
        .ok_or(())?;
    let view = views.get(accessor.buffer_view.ok_or(())?).ok_or(())?;
    let start = view
        .byte_offset
        .unwrap_or(0)
        .checked_add(accessor.byte_offset.unwrap_or(0))
        .ok_or(())?;
    Ok((accessor, view, start))
}

fn read_lite_vec3(
    accessors: &[LiteAccessor],
    views: &[LiteBufferView],
    data: &[u8],
    index: u32,
) -> Result<Vec<f32>, ()> {
    let (accessor, view, start) = lite_accessor(accessors, views, data, index)?;
    if accessor.accessor_type != "VEC3" || accessor.component_type != 5126 || accessor.normalized {
        return Err(());
    }
    let stride = view.byte_stride.unwrap_or(12);
    let mut output = Vec::new();
    output
        .try_reserve_exact(accessor.count.checked_mul(3).ok_or(())?)
        .map_err(|_| ())?;
    for row in 0..accessor.count {
        let offset = start
            .checked_add(row.checked_mul(stride).ok_or(())?)
            .ok_or(())?;
        let end = offset.checked_add(12).ok_or(())?;
        let bytes = data.get(offset..end).ok_or(())?;
        let x = f32::from_le_bytes(bytes[0..4].try_into().map_err(|_| ())?);
        let y = f32::from_le_bytes(bytes[4..8].try_into().map_err(|_| ())?);
        let z = f32::from_le_bytes(bytes[8..12].try_into().map_err(|_| ())?);
        if !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return Err(());
        }
        output.extend_from_slice(&[x, y, z]);
    }
    Ok(output)
}

fn read_lite_vec2(
    accessors: &[LiteAccessor],
    views: &[LiteBufferView],
    data: &[u8],
    index: u32,
) -> Result<Vec<f32>, ()> {
    let (accessor, view, start) = lite_accessor(accessors, views, data, index)?;
    if accessor.accessor_type != "VEC2" || accessor.component_type != 5126 || accessor.normalized {
        return Err(());
    }
    let stride = view.byte_stride.unwrap_or(8);
    let mut output = Vec::new();
    output
        .try_reserve_exact(accessor.count.checked_mul(2).ok_or(())?)
        .map_err(|_| ())?;
    for row in 0..accessor.count {
        let offset = start
            .checked_add(row.checked_mul(stride).ok_or(())?)
            .ok_or(())?;
        let end = offset.checked_add(8).ok_or(())?;
        let bytes = data.get(offset..end).ok_or(())?;
        let u = f32::from_le_bytes(bytes[0..4].try_into().map_err(|_| ())?);
        let v = f32::from_le_bytes(bytes[4..8].try_into().map_err(|_| ())?);
        if !u.is_finite() || !v.is_finite() {
            return Err(());
        }
        output.extend_from_slice(&[u, v]);
    }
    Ok(output)
}

fn read_lite_indices(
    accessors: &[LiteAccessor],
    views: &[LiteBufferView],
    data: &[u8],
    index: u32,
) -> Result<Vec<u32>, ()> {
    let (accessor, view, start) = lite_accessor(accessors, views, data, index)?;
    if accessor.accessor_type != "SCALAR" || accessor.normalized {
        return Err(());
    }
    let size = match accessor.component_type {
        5121 => 1,
        5123 => 2,
        5125 => 4,
        _ => return Err(()),
    };
    let stride = view.byte_stride.unwrap_or(size);
    let mut output = Vec::new();
    output.try_reserve_exact(accessor.count).map_err(|_| ())?;
    for row in 0..accessor.count {
        let offset = start
            .checked_add(row.checked_mul(stride).ok_or(())?)
            .ok_or(())?;
        let end = offset.checked_add(size).ok_or(())?;
        let bytes = data.get(offset..end).ok_or(())?;
        output.push(match size {
            1 => bytes[0] as u32,
            2 => u16::from_le_bytes(bytes.try_into().map_err(|_| ())?) as u32,
            _ => u32::from_le_bytes(bytes.try_into().map_err(|_| ())?),
        });
    }
    Ok(output)
}

fn triangulate_strip_compact(indices: &[u32]) -> Vec<u32> {
    if indices.len() < 3 {
        return Vec::new();
    }
    let Some(capacity) = (indices.len() - 2).checked_mul(3) else {
        return Vec::new();
    };
    let mut output = Vec::new();
    if output.try_reserve_exact(capacity).is_err() {
        return Vec::new();
    }
    for index in 2..indices.len() {
        if index % 2 == 0 {
            output.extend_from_slice(&[indices[index - 2], indices[index - 1], indices[index]]);
        } else {
            output.extend_from_slice(&[indices[index - 1], indices[index - 2], indices[index]]);
        }
    }
    output
}

fn decode_draco_mesh_compact(
    data: &[u8],
    extension_attributes: &HashMap<String, u32>,
) -> Result<MeshData, ()> {
    use draco_core::decoder_buffer::DecoderBuffer;
    use draco_core::geometry_attribute::GeometryAttributeType;
    use draco_core::mesh::Mesh;
    use draco_core::mesh_decoder::MeshDecoder;
    let mut buffer = DecoderBuffer::new(data);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder.decode(&mut buffer, &mut mesh).map_err(|_| ())?;
    if extension_attributes
        .values()
        .any(|&id| mesh.attribute_by_unique_id(id).is_none())
    {
        return Err(());
    }
    let positions = lite_draco_attribute(&mesh, GeometryAttributeType::Position, 3)?.ok_or(())?;
    let normals =
        lite_draco_attribute(&mesh, GeometryAttributeType::Normal, 3)?.unwrap_or_default();
    let uvs = lite_draco_attribute(&mesh, GeometryAttributeType::TexCoord, 2)?.unwrap_or_default();
    let colors =
        lite_draco_attribute_range(&mesh, GeometryAttributeType::Color, 3, 4)?.unwrap_or_default();
    let count = mesh.num_faces().checked_mul(3).ok_or(())?;
    let mut indices = Vec::new();
    indices.try_reserve_exact(count).map_err(|_| ())?;
    for face_index in 0..mesh.num_faces() {
        let face = mesh.face(FaceIndex(u32::try_from(face_index).map_err(|_| ())?));
        for point in face {
            if point.0 as usize >= mesh.num_points() {
                return Err(());
            }
            indices.push(point.0);
        }
    }
    Ok(MeshData {
        name: None,
        positions,
        indices,
        normals,
        uvs,
        colors,
    })
}

fn lite_draco_attribute(
    mesh: &Mesh,
    kind: GeometryAttributeType,
    components: u8,
) -> Result<Option<Vec<f32>>, ()> {
    lite_draco_attribute_range(mesh, kind, components, components)
}

fn lite_draco_attribute_range(
    mesh: &Mesh,
    kind: GeometryAttributeType,
    min_components: u8,
    max_components: u8,
) -> Result<Option<Vec<f32>>, ()> {
    let id = mesh.named_attribute_id(kind);
    if id < 0 {
        return Ok(None);
    }
    let attribute = mesh.try_attribute(id).map_err(|_| ())?;
    let components = attribute.num_components();
    if components < min_components
        || components > max_components
        || attribute.data_type() != DataType::Float32
    {
        return Err(());
    }
    let stride = usize::try_from(attribute.byte_stride()).map_err(|_| ())?;
    let row_size = usize::from(components).checked_mul(4).ok_or(())?;
    if stride < row_size {
        return Err(());
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(
            mesh.num_points()
                .checked_mul(usize::from(components))
                .ok_or(())?,
        )
        .map_err(|_| ())?;
    for point in 0..mesh.num_points() {
        let value = attribute.mapped_index(PointIndex(u32::try_from(point).map_err(|_| ())?));
        if value.0 == u32::MAX {
            return Err(());
        }
        let start = usize::try_from(value.0)
            .map_err(|_| ())?
            .checked_mul(stride)
            .ok_or(())?;
        let end = start.checked_add(row_size).ok_or(())?;
        let bytes = attribute.buffer().data().get(start..end).ok_or(())?;
        for component in bytes.chunks_exact(4) {
            let value = f32::from_le_bytes(component.try_into().map_err(|_| ())?);
            if !value.is_finite() {
                return Err(());
            }
            output.push(value);
        }
    }
    Ok(Some(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle_resource() -> Vec<u8> {
        [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect()
    }

    fn external_triangle_json() -> Vec<u8> {
        br#"{
          "asset":{"version":"2.0"},
          "buffers":[{"uri":"triangle.bin","byteLength":36}],
          "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36}],
          "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],
          "meshes":[{"name":"Triangle","primitives":[{"attributes":{"POSITION":0}}]}],
          "nodes":[{"mesh":0}],
          "scenes":[{"nodes":[0]}],
          "scene":0
        }"#
        .to_vec()
    }

    #[test]
    fn common_reader_decodes_external_resource_and_materializes_faces() {
        let resources = vec![("triangle.bin".to_string(), triangle_resource())];
        let resolver = CompanionResolver { resources };

        let result = parse_document_result(&external_triangle_json(), Some(&resolver)).unwrap();
        assert!(result.success);
        assert_eq!(result.meshes.len(), 1);
        assert_eq!(result.meshes[0].positions.len(), 9);
        assert_eq!(result.meshes[0].indices, vec![0, 1, 2]);
    }

    #[test]
    fn missing_external_resource_is_a_controlled_error() {
        let resolver = CompanionResolver {
            resources: Vec::new(),
        };
        let error = match parse_document_result(&external_triangle_json(), Some(&resolver)) {
            Ok(_) => panic!("missing companion resource unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("triangle.bin"));
    }

    #[test]
    fn zero_geometry_is_not_reported_as_success() {
        let input = br#"{"asset":{"version":"2.0"},"meshes":[]}"#;
        let result = parse_document(input, None);
        assert!(!result.success);
        assert!(result.meshes.is_empty());
    }

    #[test]
    fn compact_reader_decodes_external_resource() {
        let resources = vec![("triangle.bin".to_string(), triangle_resource())];
        let input = external_triangle_json();
        let json = std::str::from_utf8(&input).unwrap();
        let result = parse_gltf_json_with_resources(json, None, &resources);
        assert!(result.success);
        assert_eq!(result.meshes.len(), 1);
        assert_eq!(result.meshes[0].positions.len(), 9);
        assert_eq!(result.meshes[0].indices, vec![0, 1, 2]);
    }

    #[test]
    fn compact_reader_rejects_malformed_accessor() {
        let json = r#"{
            "asset":{"version":"2.0"},
            "buffers":[{"byteLength":4}],
            "bufferViews":[{"buffer":0,"byteLength":4}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]
        }"#;
        assert!(!parse_gltf_json_with_resources(json, Some(&[0; 4]), &[]).success);
    }

    #[test]
    fn compact_data_uri_decoder_rejects_non_canonical_base64() {
        assert_eq!(decode_lite_data_uri("data:;base64,YQ==").unwrap(), b"a");
        assert!(decode_lite_data_uri("data:;base64,YQ").is_err());
        assert!(decode_lite_data_uri("data:;base64,YR==").is_err());
        assert!(decode_lite_data_uri("data:,a%2").is_err());
    }
}
