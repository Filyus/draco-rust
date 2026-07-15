//! Hardened glTF/GLB reader for the browser.
//!
//! Container parsing, resource resolution, accessor decoding, and Draco
//! extension validation are delegated to `draco-io`. This crate only adapts
//! decoded meshes and scene metadata to the compact JavaScript result shape.

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_io::{GltfError, GltfReader, ResourceLimits, ResourceResolver};
use nanoserde::SerJson;
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

impl ParseResult {
    fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(message.into()),
            ..Self::default()
        }
    }
}

struct CompanionResolver {
    resources: Vec<(String, Vec<u8>)>,
}

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

fn copy_uint8_array(array: &js_sys::Uint8Array, uri: &str) -> Result<Vec<u8>, String> {
    let len = usize::try_from(array.length())
        .map_err(|_| format!("companion resource {uri} length does not fit usize"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|_| format!("failed to allocate companion resource {uri} ({len} bytes)"))?;
    bytes.resize(len, 0);
    array.copy_to(&mut bytes);
    Ok(bytes)
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
    to_js_value(&parse_document(json_content.as_bytes(), None))
}

/// Parse a GLB document.
#[wasm_bindgen]
pub fn parse_glb(data: &[u8]) -> JsValue {
    to_js_value(&parse_document(data, None))
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
            return to_js_value(&ParseResult::error(format!(
                "Invalid companion resource map: {error}"
            )));
        }
    };
    let resolver = CompanionResolver { resources };
    to_js_value(&parse_document(data, Some(&resolver)))
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
        .map_err(|_| "failed to allocate companion resource map".to_string())?;
    for index in 0..entries.length() {
        let pair = js_sys::Array::from(&entries.get(index));
        if pair.length() != 2 {
            return Err("invalid companion resource entry".to_string());
        }
        let uri = pair
            .get(0)
            .as_string()
            .ok_or("companion resource key is not a string")?;
        let value = pair.get(1);
        if !value.is_instance_of::<js_sys::Uint8Array>() {
            return Err(format!("companion resource {uri} is not a Uint8Array"));
        }
        let bytes = copy_uint8_array(&js_sys::Uint8Array::new(&value), &uri)?;
        if resources.iter().any(|(candidate, _)| candidate == &uri) {
            return Err(format!("duplicate companion resource URI: {uri}"));
        }
        resources.push((uri, bytes));
    }
    Ok(resources)
}

fn parse_document(data: &[u8], resolver: Option<&dyn ResourceResolver>) -> ParseResult {
    match parse_document_result(data, resolver) {
        Ok(result) => result,
        Err(error) => ParseResult::error(error.to_string()),
    }
}

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

fn exact_bytes<const N: usize>(bytes: &[u8]) -> Result<[u8; N], GltfError> {
    bytes.try_into().map_err(|_| {
        GltfError::InvalidGltf(format!(
            "decoded scalar has {} bytes, expected {N}",
            bytes.len()
        ))
    })
}

fn normalize_signed(value: i32, max: i32, normalized: bool) -> f32 {
    if normalized {
        ((value as f32) / (max as f32)).max(-1.0)
    } else {
        value as f32
    }
}

fn normalize_unsigned(value: u32, max: u32, normalized: bool) -> f32 {
    if normalized {
        (value as f32) / (max as f32)
    } else {
        value as f32
    }
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
}
