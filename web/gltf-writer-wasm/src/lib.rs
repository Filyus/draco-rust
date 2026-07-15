//! glTF/GLB Writer WASM module.
//!
//! Provides glTF 2.0 file generation functionality for web applications.
//! Supports both .gltf (JSON) and .glb (binary) formats with optional Draco compression.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use draco_io::{
    compress_gltf_bytes_with_options, compress_gltf_bytes_with_resolver, serialize_gltf_document,
    CompressionReport, EncodingMethod, GltfCompressionOptions, GltfContainerFormat, GltfError,
    OutputFormat, PreserveReason, QuantizationOptions, ResourceLimits, ResourceResolver,
};

/// Input mesh data from JavaScript.
#[derive(Serialize, Deserialize, Clone)]
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
}

/// Scene node input.
#[derive(Serialize, Deserialize, Clone)]
pub struct NodeInput {
    pub name: Option<String>,
    pub mesh_index: Option<usize>,
    pub translation: Option<[f32; 3]>,
    pub rotation: Option<[f32; 4]>,
    pub scale: Option<[f32; 3]>,
    pub children: Vec<usize>,
}

/// Export options.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct ExportOptions {
    /// Use Draco compression
    pub use_draco: Option<bool>,
    /// Draco quantization bits for positions (default: 14)
    pub position_quantization: Option<i32>,
    /// Draco quantization bits for normals (default: 10)
    pub normal_quantization: Option<i32>,
    /// Draco quantization bits for UVs (default: 12)
    pub texcoord_quantization: Option<i32>,
    /// Draco quantization bits for colors (default: 8)
    pub color_quantization: Option<i32>,
    /// Draco quantization bits for generic/custom attributes (default: 8)
    pub generic_quantization: Option<i32>,
    /// Output format: "glb" or "gltf"
    pub format: Option<String>,
    /// Draco encoding speed (0-10, default: 5). Lower = better compression, slower. Higher = faster, worse compression.
    pub encoding_speed: Option<i32>,
    /// Draco decoding speed (0-10, default: 5).
    pub decoding_speed: Option<i32>,
    /// Draco encoding method: 0 = sequential, 1 = edgebreaker, -1 = auto (default)
    pub encoding_method: Option<i32>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            use_draco: Some(false),
            position_quantization: Some(14),
            normal_quantization: Some(10),
            texcoord_quantization: Some(12),
            color_quantization: Some(8),
            generic_quantization: Some(8),
            format: Some("glb".to_string()),
            encoding_speed: Some(5),
            decoding_speed: Some(5),
            encoding_method: Some(-1),
        }
    }
}

/// Draco compression statistics.
#[derive(Serialize, Deserialize, Default)]
pub struct DracoStats {
    /// Compression method used: "sequential" or "edgebreaker"
    pub method: String,
    /// Encoding speed used (0-10)
    pub speed: i32,
    /// Compressed size in bytes
    pub compressed_size: usize,
    /// Prediction scheme used for position attribute
    pub prediction_scheme: String,
}

/// Stable primitive location returned to JavaScript.
#[derive(Serialize, Deserialize)]
pub struct PrimitiveLocationData {
    pub mesh: usize,
    pub primitive: usize,
}

/// One preserved primitive and its typed reason.
#[derive(Serialize, Deserialize)]
pub struct PreservedPrimitiveData {
    pub mesh: usize,
    pub primitive: usize,
    pub reason: String,
    pub detail: Option<String>,
    pub accessor: Option<usize>,
    pub mode: Option<u32>,
}

/// Primitive-by-primitive document compression report.
#[derive(Serialize, Deserialize)]
pub struct CompressionReportData {
    pub compressed_primitives: Vec<PrimitiveLocationData>,
    pub preserved_primitives: Vec<PreservedPrimitiveData>,
}

/// Export result.
#[derive(Serialize, Deserialize)]
pub struct ExportResult {
    pub success: bool,
    /// JSON content (for .gltf format or embedded)
    pub json_data: Option<String>,
    /// Binary data (for .glb format)
    pub binary_data: Option<Vec<u8>>,
    pub error: Option<String>,
    /// Draco compression statistics (if Draco was used)
    pub draco_stats: Option<DracoStats>,
    /// Detailed report for document-preserving compression.
    pub compression_report: Option<CompressionReportData>,
}

impl ExportResult {
    fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            json_data: None,
            binary_data: None,
            error: Some(message.into()),
            draco_stats: None,
            compression_report: None,
        }
    }
}

struct CompanionResolver {
    resources: HashMap<String, Vec<u8>>,
}

impl ResourceResolver for CompanionResolver {
    fn resolve(&self, uri: &str) -> Result<Vec<u8>, GltfError> {
        let resource = self
            .resources
            .get(uri)
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

fn parse_companion_resources(resources_js: JsValue) -> Result<HashMap<String, Vec<u8>>, String> {
    if !resources_js.is_object() || resources_js.is_null() {
        return Err("expected an object whose values are Uint8Array instances".to_string());
    }
    let object: js_sys::Object = resources_js.unchecked_into();
    let entries = js_sys::Object::entries(&object);
    let mut resources = HashMap::new();
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
        if resources.insert(uri.clone(), bytes).is_some() {
            return Err(format!("duplicate companion resource URI: {uri}"));
        }
    }
    Ok(resources)
}

fn parse_options(options_js: JsValue) -> Result<ExportOptions, String> {
    if options_js.is_null() || options_js.is_undefined() {
        return Ok(ExportOptions::default());
    }
    serde_wasm_bindgen::from_value(options_js)
        .map_err(|e| format!("Invalid glTF export options: {e}"))
}

fn validate_quantization(name: &str, bits: Option<i32>, min: i32, max: i32) -> Result<(), String> {
    if let Some(bits) = bits {
        if !(min..=max).contains(&bits) {
            return Err(format!(
                "{name} quantization must be between {min} and {max} bits, got {bits}"
            ));
        }
    }
    Ok(())
}

fn validate_options(options: &ExportOptions) -> Result<(), String> {
    match options.format.as_deref().unwrap_or("glb") {
        "glb" | "gltf" => {}
        format => return Err(format!("Unsupported glTF output format: {format}")),
    }

    validate_quantization("Position", options.position_quantization, 1, 31)?;
    validate_quantization("Normal", options.normal_quantization, 2, 30)?;
    validate_quantization("Texture coordinate", options.texcoord_quantization, 1, 31)?;
    validate_quantization("Color", options.color_quantization, 1, 31)?;
    validate_quantization("Generic", options.generic_quantization, 1, 31)?;

    for (name, speed) in [
        ("encoding_speed", options.encoding_speed.unwrap_or(5)),
        ("decoding_speed", options.decoding_speed.unwrap_or(5)),
    ] {
        if !(0..=10).contains(&speed) {
            return Err(format!("{name} must be between 0 and 10, got {speed}"));
        }
    }

    let method = options.encoding_method.unwrap_or(-1);
    if !(-1..=1).contains(&method) {
        return Err(format!(
            "encoding_method must be -1 (auto), 0 (sequential), or 1 (edgebreaker), got {method}"
        ));
    }
    Ok(())
}

fn optional_u8(value: Option<i32>, name: &str) -> Result<Option<u8>, String> {
    value
        .map(|value| {
            u8::try_from(value).map_err(|_| format!("{name} does not fit into an unsigned byte"))
        })
        .transpose()
}

fn compression_options(options: &ExportOptions) -> Result<GltfCompressionOptions, String> {
    validate_options(options)?;
    let encoding_method = match options.encoding_method.unwrap_or(-1) {
        -1 => EncodingMethod::Auto,
        0 => EncodingMethod::Sequential,
        1 => EncodingMethod::Edgebreaker,
        method => return Err(format!("Unsupported encoding method: {method}")),
    };
    let output_format = match options.format.as_deref().unwrap_or("glb") {
        "glb" => OutputFormat::Glb,
        "gltf" => OutputFormat::GltfEmbeddedBuffers,
        format => return Err(format!("Unsupported glTF output format: {format}")),
    };
    let canonical = GltfCompressionOptions {
        quantization: QuantizationOptions {
            position: optional_u8(options.position_quantization, "position_quantization")?,
            normal: optional_u8(options.normal_quantization, "normal_quantization")?,
            color: optional_u8(options.color_quantization, "color_quantization")?,
            texcoord: optional_u8(options.texcoord_quantization, "texcoord_quantization")?,
            generic: optional_u8(options.generic_quantization, "generic_quantization")?,
        },
        encoding_speed: u8::try_from(options.encoding_speed.unwrap_or(5))
            .map_err(|_| "encoding_speed does not fit into an unsigned byte".to_string())?,
        decoding_speed: u8::try_from(options.decoding_speed.unwrap_or(5))
            .map_err(|_| "decoding_speed does not fit into an unsigned byte".to_string())?,
        encoding_method,
        output_format,
    };
    canonical.validate().map_err(|error| error.to_string())?;
    Ok(canonical)
}

fn compression_report(report: CompressionReport) -> CompressionReportData {
    CompressionReportData {
        compressed_primitives: report
            .compressed_primitives
            .into_iter()
            .map(|location| PrimitiveLocationData {
                mesh: location.mesh,
                primitive: location.primitive,
            })
            .collect(),
        preserved_primitives: report
            .preserved_primitives
            .into_iter()
            .map(|preserved| {
                let (reason, detail, accessor, mode) = match preserved.reason {
                    PreserveReason::AlreadyDraco => ("alreadyDraco", None, None, None),
                    PreserveReason::UnsupportedMode { mode } => {
                        ("unsupportedMode", None, None, Some(mode))
                    }
                    PreserveReason::UnsupportedLayout { detail } => {
                        ("unsupportedLayout", Some(detail), None, None)
                    }
                    PreserveReason::SparseAccessor { accessor } => {
                        ("sparseAccessor", None, Some(accessor), None)
                    }
                    PreserveReason::MorphTargets => ("morphTargets", None, None, None),
                    PreserveReason::SharedAccessor { accessor } => {
                        ("sharedAccessor", None, Some(accessor), None)
                    }
                };
                PreservedPrimitiveData {
                    mesh: preserved.primitive.mesh,
                    primitive: preserved.primitive.primitive,
                    reason: reason.to_string(),
                    detail,
                    accessor,
                    mode,
                }
            })
            .collect(),
    }
}

fn method_name(method: EncodingMethod) -> &'static str {
    match method {
        EncodingMethod::Auto => "auto",
        EncodingMethod::Sequential => "sequential",
        EncodingMethod::Edgebreaker => "edgebreaker",
    }
}

fn validate_mesh(mesh: &MeshInput, mesh_index: usize) -> Result<(), String> {
    if mesh.positions.is_empty() || !mesh.positions.len().is_multiple_of(3) {
        return Err(format!(
            "Mesh {mesh_index} positions length must be a non-zero multiple of 3, got {}",
            mesh.positions.len()
        ));
    }
    if mesh.indices.is_empty() || !mesh.indices.len().is_multiple_of(3) {
        return Err(format!(
            "Mesh {mesh_index} indices length must be a non-zero multiple of 3, got {}",
            mesh.indices.len()
        ));
    }
    if mesh.positions.iter().any(|value| !value.is_finite()) {
        return Err(format!(
            "Mesh {mesh_index} positions contain a non-finite value"
        ));
    }

    let vertex_count = mesh.positions.len() / 3;
    if vertex_count > u32::MAX as usize {
        return Err(format!(
            "Mesh {mesh_index} has {vertex_count} vertices, exceeding the u32 index limit"
        ));
    }
    if let Some(index) = mesh
        .indices
        .iter()
        .copied()
        .find(|&index| index as usize >= vertex_count)
    {
        return Err(format!(
            "Mesh {mesh_index} index {index} is out of range for {vertex_count} vertices"
        ));
    }

    if let Some(normals) = &mesh.normals {
        if !normals.is_empty() && normals.len() != mesh.positions.len() {
            return Err(format!(
                "Mesh {mesh_index} normals length {} does not match positions length {}",
                normals.len(),
                mesh.positions.len()
            ));
        }
        if normals.iter().any(|value| !value.is_finite()) {
            return Err(format!(
                "Mesh {mesh_index} normals contain a non-finite value"
            ));
        }
    }

    if let Some(uvs) = &mesh.uvs {
        let expected = vertex_count
            .checked_mul(2)
            .ok_or_else(|| format!("Mesh {mesh_index} texture coordinate count overflow"))?;
        if !uvs.is_empty() && uvs.len() != expected {
            return Err(format!(
                "Mesh {mesh_index} texture coordinate length {} does not match vertex count {vertex_count}",
                uvs.len()
            ));
        }
        if uvs.iter().any(|value| !value.is_finite()) {
            return Err(format!(
                "Mesh {mesh_index} texture coordinates contain a non-finite value"
            ));
        }
    }
    Ok(())
}

fn validate_meshes(meshes: &[MeshInput]) -> Result<(), String> {
    if meshes.is_empty() {
        return Err("At least one mesh is required".to_string());
    }
    for (index, mesh) in meshes.iter().enumerate() {
        validate_mesh(mesh, index)?;
    }
    Ok(())
}

const ROTATION_LENGTH_TOLERANCE: f64 = 1.0e-5;

fn validate_nodes(nodes: &[NodeInput], mesh_count: usize) -> Result<(), String> {
    let mut parents = Vec::new();
    parents
        .try_reserve_exact(nodes.len())
        .map_err(|_| "failed to allocate node parent table".to_string())?;
    parents.resize(nodes.len(), None::<usize>);

    for (node_index, node) in nodes.iter().enumerate() {
        if let Some(mesh_index) = node.mesh_index {
            if mesh_index >= mesh_count {
                return Err(format!(
                    "Node {node_index} references mesh {mesh_index}, but only {mesh_count} meshes exist"
                ));
            }
        }
        if let Some(value) = node
            .translation
            .iter()
            .flatten()
            .chain(node.rotation.iter().flatten())
            .chain(node.scale.iter().flatten())
            .find(|value| !value.is_finite())
        {
            return Err(format!(
                "Node {node_index} transform contains non-finite value {value}"
            ));
        }
        if let Some(rotation) = node.rotation {
            let length = rotation
                .iter()
                .map(|&component| {
                    let component = f64::from(component);
                    component * component
                })
                .sum::<f64>()
                .sqrt();
            if (length - 1.0).abs() > ROTATION_LENGTH_TOLERANCE {
                return Err(format!(
                    "Node {node_index} rotation must be a unit quaternion, but has length {length}"
                ));
            }
        }
        for &child in &node.children {
            if child >= nodes.len() {
                return Err(format!(
                    "Node {node_index} references child {child}, but only {} nodes exist",
                    nodes.len()
                ));
            }
            if child == node_index {
                return Err(format!("Node {node_index} cannot be its own child"));
            }
            if let Some(existing_parent) = parents[child] {
                if existing_parent == node_index {
                    return Err(format!(
                        "Node {node_index} lists child {child} more than once"
                    ));
                }
                return Err(format!(
                    "Node {child} has multiple parents: {existing_parent} and {node_index}"
                ));
            }
            parents[child] = Some(node_index);
        }
    }

    let mut state = Vec::new();
    state
        .try_reserve_exact(nodes.len())
        .map_err(|_| "failed to allocate node validation state".to_string())?;
    state.resize(nodes.len(), 0u8);
    let mut stack = Vec::new();
    stack
        .try_reserve_exact(nodes.len())
        .map_err(|_| "failed to allocate node validation stack".to_string())?;
    for start in 0..nodes.len() {
        if state[start] != 0 {
            continue;
        }
        state[start] = 1;
        stack.push((start, 0usize));
        while let Some((node_index, next_child)) = stack.last_mut() {
            if *next_child == nodes[*node_index].children.len() {
                state[*node_index] = 2;
                stack.pop();
                continue;
            }
            let child = nodes[*node_index].children[*next_child];
            *next_child += 1;
            match state[child] {
                0 => {
                    state[child] = 1;
                    stack.push((child, 0));
                }
                1 => return Err(format!("Node hierarchy contains a cycle at node {child}")),
                _ => {}
            }
        }
    }
    Ok(())
}

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

/// Get the module name.
#[wasm_bindgen]
pub fn module_name() -> String {
    "glTF Writer".to_string()
}

/// Get supported file extensions.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["gltf".to_string(), "glb".to_string()]
}

/// Create glTF/GLB content from mesh data.
#[wasm_bindgen]
pub fn create_gltf(meshes_js: JsValue, options_js: JsValue) -> JsValue {
    let meshes: Vec<MeshInput> = match serde_wasm_bindgen::from_value::<Vec<MeshInput>>(meshes_js) {
        Ok(meshes) => meshes,
        Err(e) => {
            let result = ExportResult::error(format!("Invalid mesh data: {e}"));
            return serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL);
        }
    };

    let options = match parse_options(options_js) {
        Ok(options) => options,
        Err(error) => {
            return serde_wasm_bindgen::to_value(&ExportResult::error(error))
                .unwrap_or(JsValue::NULL);
        }
    };

    if let Err(error) = validate_meshes(&meshes).and_then(|()| validate_options(&options)) {
        return serde_wasm_bindgen::to_value(&ExportResult::error(error)).unwrap_or(JsValue::NULL);
    }

    // Catch any panics and convert to error result
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        create_gltf_internal(&meshes, &options)
    }));

    let result = match result {
        Ok(r) => r,
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown panic".to_string()
            };
            ExportResult::error(format!("Internal error: {msg}"))
        }
    };

    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Compress a complete glTF/GLB document while preserving non-geometry content.
#[wasm_bindgen]
pub fn compress_gltf_document(data: &[u8], options_js: JsValue) -> JsValue {
    let options = match parse_options(options_js) {
        Ok(options) => options,
        Err(error) => {
            return serde_wasm_bindgen::to_value(&ExportResult::error(error))
                .unwrap_or(JsValue::NULL);
        }
    };
    let canonical = match compression_options(&options) {
        Ok(options) => options,
        Err(error) => {
            return serde_wasm_bindgen::to_value(&ExportResult::error(error))
                .unwrap_or(JsValue::NULL);
        }
    };
    let result = compress_gltf_bytes_with_options(data, &canonical)
        .map(|output| document_compression_result(output, canonical))
        .unwrap_or_else(|error| {
            ExportResult::error(format!(
                "Document-preserving glTF compression failed: {error}"
            ))
        });

    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Compress a glTF/GLB document using an in-memory companion resource map.
#[wasm_bindgen]
pub fn compress_gltf_document_with_resources(
    data: &[u8],
    resources_js: JsValue,
    options_js: JsValue,
) -> JsValue {
    let resources = match parse_companion_resources(resources_js) {
        Ok(resources) => resources,
        Err(error) => {
            return serde_wasm_bindgen::to_value(&ExportResult::error(format!(
                "Invalid companion resource map: {error}"
            )))
            .unwrap_or(JsValue::NULL);
        }
    };
    let options = match parse_options(options_js) {
        Ok(options) => options,
        Err(error) => {
            return serde_wasm_bindgen::to_value(&ExportResult::error(error))
                .unwrap_or(JsValue::NULL);
        }
    };
    let canonical = match compression_options(&options) {
        Ok(options) => options,
        Err(error) => {
            return serde_wasm_bindgen::to_value(&ExportResult::error(error))
                .unwrap_or(JsValue::NULL);
        }
    };
    let resolver = CompanionResolver { resources };
    let result =
        compress_gltf_bytes_with_resolver(data, &resolver, &ResourceLimits::default(), &canonical)
            .map(|output| document_compression_result(output, canonical))
            .unwrap_or_else(|error| {
                ExportResult::error(format!(
                    "Document-preserving glTF compression failed: {error}"
                ))
            });
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

fn document_compression_result(
    output: draco_io::CompressionOutput<Vec<u8>>,
    options: GltfCompressionOptions,
) -> ExportResult {
    let compressed_size = output.data.len();
    let is_glb = match options.output_format {
        OutputFormat::Glb => true,
        OutputFormat::GltfEmbeddedBuffers => false,
        OutputFormat::SameAsInput => output.data.starts_with(b"glTF"),
    };
    let (json_data, binary_data) = if is_glb {
        (None, Some(output.data))
    } else {
        match String::from_utf8(output.data) {
            Ok(json) => (Some(json), None),
            Err(_) => {
                return ExportResult::error(
                    "Document-preserving compressor returned invalid UTF-8 glTF JSON",
                );
            }
        }
    };
    ExportResult {
        success: true,
        json_data,
        binary_data,
        error: None,
        draco_stats: Some(DracoStats {
            method: method_name(options.encoding_method).to_string(),
            speed: i32::from(options.encoding_speed),
            compressed_size,
            prediction_scheme: "document".to_string(),
        }),
        compression_report: Some(compression_report(output.report)),
    }
}

/// Create glTF with scene graph.
#[wasm_bindgen]
pub fn create_gltf_with_scene(
    meshes_js: JsValue,
    nodes_js: JsValue,
    options_js: JsValue,
) -> JsValue {
    let meshes: Vec<MeshInput> = match serde_wasm_bindgen::from_value(meshes_js) {
        Ok(m) => m,
        Err(e) => {
            let result = ExportResult::error(format!("Invalid mesh data: {e}"));
            return serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL);
        }
    };

    let nodes: Vec<NodeInput> = match serde_wasm_bindgen::from_value(nodes_js) {
        Ok(n) => n,
        Err(e) => {
            let result = ExportResult::error(format!("Invalid node data: {e}"));
            return serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL);
        }
    };

    let options = match parse_options(options_js) {
        Ok(options) => options,
        Err(error) => {
            return serde_wasm_bindgen::to_value(&ExportResult::error(error))
                .unwrap_or(JsValue::NULL);
        }
    };
    if let Err(error) = validate_meshes(&meshes)
        .and_then(|()| validate_nodes(&nodes, meshes.len()))
        .and_then(|()| validate_options(&options))
    {
        return serde_wasm_bindgen::to_value(&ExportResult::error(error)).unwrap_or(JsValue::NULL);
    }
    let result = create_gltf_with_scene_internal(&meshes, &nodes, &options);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

fn create_gltf_internal(meshes: &[MeshInput], options: &ExportOptions) -> ExportResult {
    // Create default nodes for each mesh
    let nodes: Vec<NodeInput> = meshes
        .iter()
        .enumerate()
        .map(|(i, m)| NodeInput {
            name: m.name.clone(),
            mesh_index: Some(i),
            translation: None,
            rotation: None,
            scale: None,
            children: vec![],
        })
        .collect();

    create_gltf_with_scene_internal(meshes, &nodes, options)
}

fn append_f32s(buffer: &mut Vec<u8>, values: &[f32], what: &str) -> Result<(usize, usize), String> {
    let byte_length = values
        .len()
        .checked_mul(std::mem::size_of::<f32>())
        .ok_or_else(|| format!("{what} byte length overflow"))?;
    buffer
        .try_reserve_exact(byte_length)
        .map_err(|_| format!("failed to allocate {what} ({byte_length} bytes)"))?;
    let offset = buffer.len();
    for value in values {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
    Ok((offset, byte_length))
}

fn append_u32s(buffer: &mut Vec<u8>, values: &[u32], what: &str) -> Result<(usize, usize), String> {
    let byte_length = values
        .len()
        .checked_mul(std::mem::size_of::<u32>())
        .ok_or_else(|| format!("{what} byte length overflow"))?;
    buffer
        .try_reserve_exact(byte_length)
        .map_err(|_| format!("failed to allocate {what} ({byte_length} bytes)"))?;
    let offset = buffer.len();
    for value in values {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
    Ok((offset, byte_length))
}

fn create_gltf_with_scene_internal(
    meshes: &[MeshInput],
    nodes: &[NodeInput],
    options: &ExportOptions,
) -> ExportResult {
    if let Err(error) = validate_meshes(meshes)
        .and_then(|()| validate_nodes(nodes, meshes.len()))
        .and_then(|()| validate_options(options))
    {
        return ExportResult::error(error);
    }

    let format = options.format.as_deref().unwrap_or("glb");

    // Build one canonical, uncompressed document. Optional Draco compression is
    // applied to this document through draco-io below; the WASM wrapper does
    // not maintain its own KHR_draco_mesh_compression implementation.
    let mut binary_data: Vec<u8> = Vec::new();
    let mut buffer_views: Vec<serde_json::Value> = Vec::new();
    let mut accessors: Vec<serde_json::Value> = Vec::new();
    let mut gltf_meshes: Vec<serde_json::Value> = Vec::new();
    let estimated_entries = match meshes.len().checked_mul(4) {
        Some(count) => count,
        None => return ExportResult::error("glTF metadata entry count overflow"),
    };
    if buffer_views.try_reserve(estimated_entries).is_err()
        || accessors.try_reserve(estimated_entries).is_err()
        || gltf_meshes.try_reserve_exact(meshes.len()).is_err()
    {
        return ExportResult::error("failed to allocate glTF metadata");
    }

    for mesh in meshes {
        let vertex_count = mesh.positions.len() / 3;
        let mut attributes = HashMap::new();

        let (pos_bv_offset, pos_byte_length) =
            match append_f32s(&mut binary_data, &mesh.positions, "position buffer") {
                Ok(range) => range,
                Err(error) => return ExportResult::error(error),
            };
        let mut pos_min = [f32::INFINITY; 3];
        let mut pos_max = [f32::NEG_INFINITY; 3];
        for position in mesh.positions.chunks_exact(3) {
            for component in 0..3 {
                pos_min[component] = pos_min[component].min(position[component]);
                pos_max[component] = pos_max[component].max(position[component]);
            }
        }
        let pos_bv_idx = buffer_views.len();
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": pos_bv_offset,
            "byteLength": pos_byte_length
        }));
        let pos_acc_idx = accessors.len();
        accessors.push(serde_json::json!({
            "bufferView": pos_bv_idx,
            "componentType": 5126,
            "count": vertex_count,
            "type": "VEC3",
            "min": pos_min,
            "max": pos_max
        }));
        attributes.insert("POSITION", pos_acc_idx);

        if let Some(normals) = mesh.normals.as_deref().filter(|values| !values.is_empty()) {
            let (offset, byte_length) =
                match append_f32s(&mut binary_data, normals, "normal buffer") {
                    Ok(range) => range,
                    Err(error) => return ExportResult::error(error),
                };
            let view = buffer_views.len();
            buffer_views.push(serde_json::json!({
                "buffer": 0,
                "byteOffset": offset,
                "byteLength": byte_length
            }));
            let accessor = accessors.len();
            accessors.push(serde_json::json!({
                "bufferView": view,
                "componentType": 5126,
                "count": vertex_count,
                "type": "VEC3"
            }));
            attributes.insert("NORMAL", accessor);
        }

        if let Some(uvs) = mesh.uvs.as_deref().filter(|values| !values.is_empty()) {
            let (offset, byte_length) =
                match append_f32s(&mut binary_data, uvs, "texture coordinate buffer") {
                    Ok(range) => range,
                    Err(error) => return ExportResult::error(error),
                };
            let view = buffer_views.len();
            buffer_views.push(serde_json::json!({
                "buffer": 0,
                "byteOffset": offset,
                "byteLength": byte_length
            }));
            let accessor = accessors.len();
            accessors.push(serde_json::json!({
                "bufferView": view,
                "componentType": 5126,
                "count": vertex_count,
                "type": "VEC2"
            }));
            attributes.insert("TEXCOORD_0", accessor);
        }

        let (idx_bv_offset, idx_byte_length) =
            match append_u32s(&mut binary_data, &mesh.indices, "index buffer") {
                Ok(range) => range,
                Err(error) => return ExportResult::error(error),
            };
        let idx_bv_idx = buffer_views.len();
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": idx_bv_offset,
            "byteLength": idx_byte_length
        }));
        let idx_acc_idx = accessors.len();
        accessors.push(serde_json::json!({
            "bufferView": idx_bv_idx,
            "componentType": 5125,
            "count": mesh.indices.len(),
            "type": "SCALAR"
        }));

        gltf_meshes.push(serde_json::json!({
            "name": mesh.name,
            "primitives": [{
                "attributes": attributes,
                "indices": idx_acc_idx
            }]
        }));
    }

    // Build nodes
    let gltf_nodes: Vec<serde_json::Value> = nodes
        .iter()
        .map(|n| {
            let mut node = serde_json::json!({});
            if let Some(ref name) = n.name {
                node["name"] = serde_json::json!(name);
            }
            if let Some(mesh_idx) = n.mesh_index {
                node["mesh"] = serde_json::json!(mesh_idx);
            }
            if let Some(t) = n.translation {
                node["translation"] = serde_json::json!(t);
            }
            if let Some(r) = n.rotation {
                node["rotation"] = serde_json::json!(r);
            }
            if let Some(s) = n.scale {
                node["scale"] = serde_json::json!(s);
            }
            if !n.children.is_empty() {
                node["children"] = serde_json::json!(n.children);
            }
            node
        })
        .collect();

    // Root node indices for scene. Child nodes must not be instantiated twice.
    let mut is_child = vec![false; nodes.len()];
    for node in nodes {
        for &child in &node.children {
            is_child[child] = true;
        }
    }
    let root_nodes: Vec<usize> = is_child
        .iter()
        .enumerate()
        .filter_map(|(index, &child)| (!child).then_some(index))
        .collect();

    let gltf_json = serde_json::json!({
        "asset": {
            "version": "2.0",
            "generator": "draco-io WASM"
        },
        "scene": 0,
        "scenes": [{
            "nodes": root_nodes
        }],
        "nodes": gltf_nodes,
        "meshes": gltf_meshes,
        "accessors": accessors,
        "bufferViews": buffer_views,
        "buffers": [{
            "byteLength": binary_data.len()
        }]
    });

    let output_format = if format == "glb" {
        OutputFormat::Glb
    } else {
        OutputFormat::GltfEmbeddedBuffers
    };

    if options.use_draco.unwrap_or(false) {
        let canonical = match compression_options(options) {
            Ok(options) => options,
            Err(error) => return ExportResult::error(error),
        };
        let uncompressed = match serialize_gltf_document(
            &gltf_json,
            &binary_data,
            GltfContainerFormat::Gltf,
            OutputFormat::Glb,
        ) {
            Ok(bytes) => bytes,
            Err(error) => {
                return ExportResult::error(format!(
                    "failed to build intermediate glTF document: {error}"
                ));
            }
        };
        return match compress_gltf_bytes_with_options(&uncompressed, &canonical) {
            Ok(output) => document_compression_result(output, canonical),
            Err(error) => {
                ExportResult::error(format!("Draco document compression failed: {error}"))
            }
        };
    }

    let bytes = match serialize_gltf_document(
        &gltf_json,
        &binary_data,
        GltfContainerFormat::Gltf,
        output_format,
    ) {
        Ok(bytes) => bytes,
        Err(error) => return ExportResult::error(format!("glTF serialization failed: {error}")),
    };
    if output_format == OutputFormat::Glb {
        ExportResult {
            success: true,
            json_data: None,
            binary_data: Some(bytes),
            error: None,
            draco_stats: None,
            compression_report: None,
        }
    } else {
        let json_data = match String::from_utf8(bytes) {
            Ok(json) => json,
            Err(_) => return ExportResult::error("serialized glTF JSON is not valid UTF-8"),
        };
        ExportResult {
            success: true,
            json_data: Some(json_data),
            binary_data: None,
            error: None,
            draco_stats: None,
            compression_report: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> MeshInput {
        MeshInput {
            name: Some("triangle".to_string()),
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0],
            indices: vec![0, 1, 2],
            normals: None,
            uvs: None,
        }
    }

    fn node(children: Vec<usize>, rotation: Option<[f32; 4]>) -> NodeInput {
        NodeInput {
            name: None,
            mesh_index: None,
            translation: None,
            rotation,
            scale: None,
            children,
        }
    }

    #[test]
    fn test_create_simple_gltf() {
        let result = create_gltf_internal(&[triangle()], &ExportOptions::default());
        assert!(result.success);
        assert!(result.binary_data.is_some());
    }

    #[test]
    fn rejects_invalid_mesh_arrays_and_non_finite_values() {
        let mut mesh = triangle();
        mesh.indices.push(0);
        assert!(validate_mesh(&mesh, 0).is_err());

        let mut mesh = triangle();
        mesh.positions[0] = f32::NAN;
        assert!(validate_mesh(&mesh, 0).is_err());

        let mut mesh = triangle();
        mesh.indices[2] = 3;
        assert!(validate_mesh(&mesh, 0).is_err());

        let mut mesh = triangle();
        mesh.normals = Some(vec![0.0; 6]);
        assert!(validate_mesh(&mesh, 0).is_err());
    }

    #[test]
    fn rejects_invalid_options_without_clamping() {
        let options = ExportOptions {
            encoding_speed: Some(11),
            ..ExportOptions::default()
        };
        assert!(validate_options(&options).is_err());

        let options = ExportOptions {
            normal_quantization: Some(1),
            ..ExportOptions::default()
        };
        assert!(validate_options(&options).is_err());

        let options = ExportOptions {
            encoding_method: Some(2),
            ..ExportOptions::default()
        };
        assert!(validate_options(&options).is_err());
    }

    #[test]
    fn rejects_invalid_node_references_and_cycles() {
        let invalid_mesh = NodeInput {
            name: None,
            mesh_index: Some(1),
            translation: None,
            rotation: None,
            scale: None,
            children: vec![],
        };
        assert!(validate_nodes(&[invalid_mesh], 1).is_err());

        let cycle = vec![
            NodeInput {
                name: None,
                mesh_index: None,
                translation: None,
                rotation: None,
                scale: None,
                children: vec![1],
            },
            NodeInput {
                name: None,
                mesh_index: None,
                translation: None,
                rotation: None,
                scale: None,
                children: vec![0],
            },
        ];
        assert!(validate_nodes(&cycle, 1).is_err());
    }

    #[test]
    fn rejects_duplicate_children_and_multiple_parents() {
        let duplicate = vec![node(vec![1, 1], None), node(vec![], None)];
        let error = validate_nodes(&duplicate, 1).unwrap_err();
        assert!(error.contains("more than once"), "{error}");

        let multiple_parents = vec![node(vec![2], None), node(vec![2], None), node(vec![], None)];
        let error = validate_nodes(&multiple_parents, 1).unwrap_err();
        assert!(error.contains("multiple parents"), "{error}");
    }

    #[test]
    fn rotation_must_be_finite_and_unit_length() {
        assert!(validate_nodes(&[node(vec![], Some([0.0, 0.0, 0.0, 1.0]))], 1).is_ok());
        assert!(validate_nodes(
            &[node(
                vec![],
                Some([0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2, 0.707_106_77]),
            )],
            1,
        )
        .is_ok());

        for invalid in [
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 2.0],
            [0.0, 0.0, 0.0, f32::INFINITY],
        ] {
            assert!(
                validate_nodes(&[node(vec![], Some(invalid))], 1).is_err(),
                "rotation {invalid:?} must be rejected"
            );
        }
    }
}
