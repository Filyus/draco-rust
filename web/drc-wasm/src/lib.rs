//! Draco `.drc` reader and writer WASM module.
//!
//! The codec itself lives in `draco-core`; this is the JavaScript surface for
//! the standalone container, the one `draco_encoder`/`draco_decoder` write.
//! Draco inside glTF is a different route entirely — there the payload is a
//! `KHR_draco_mesh_compression` extension and the glTF module owns it.
//!
//! The two halves are independent: build with `--features read` or
//! `--features write` (both are on by default).

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// Install the panic hook, so a panic reads as a message rather than a trap.
#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// The version of this WASM module.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// File extensions this module reads and writes.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["drc".to_string()]
}

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;

// ===========================================================================
// Reader
// ===========================================================================

/// Mesh data produced by the decoder, for JavaScript interop.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize)]
pub struct MeshData {
    /// Vertex positions as a flat array `[x0, y0, z0, x1, ...]`.
    pub positions: Vec<f32>,
    /// Triangle indices as a flat array.
    pub indices: Vec<u32>,
    /// Vertex normals, empty when the payload carried none.
    pub normals: Vec<f32>,
    /// Texture coordinates, empty when the payload carried none.
    pub uvs: Vec<f32>,
    /// Vertex colors as `[r, g, b, a, ...]`, 0-255, empty when absent.
    pub colors: Vec<u8>,
    /// Everything else the payload carried, unread and unchanged.
    pub extras: Vec<ExtraAttribute>,
}

/// An attribute the flat mesh has no slot for, kept whole so it can be written
/// back exactly as it arrived.
///
/// Draco records enough to reconstruct one without knowing what it means: the
/// type, the component count, the component type, and the id a consumer
/// addresses it by. Nothing here is interpreted — a second texture-coordinate
/// set and a generic attribute holding skin weights travel the same way.
///
/// Values are per point rather than per unique entry, and `f64` rather than
/// bytes: every Draco component type fits a double exactly, so the numbers
/// survive the crossing into JavaScript and back with the declared type still
/// deciding what is written.
#[cfg(any(feature = "read", feature = "write"))]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExtraAttribute {
    /// `POSITION`, `NORMAL`, `COLOR`, `TEX_COORD` or `GENERIC`.
    #[serde(rename = "type")]
    pub attribute_type: String,
    /// Scalar components per value.
    pub components: u8,
    /// Component type, as Draco names it.
    pub data_type: String,
    /// The id the payload assigned it.
    pub unique_id: u32,
    /// Whether integer values are to be read as normalized.
    pub normalized: bool,
    /// One tuple per point, `components` long.
    pub values: Vec<f64>,
}

/// What the decoder hands back: the mesh, or why there is none.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize)]
pub struct ParseResult {
    /// Whether a mesh was decoded.
    pub success: bool,
    /// The decoded mesh; a `.drc` holds one.
    pub meshes: Vec<MeshData>,
    /// Why the decode failed, when it did.
    pub error: Option<String>,
    /// Non-fatal remarks about the payload.
    pub warnings: Vec<String>,
    /// Every attribute the payload declared, in the order it declared them.
    pub attributes: Vec<AttributeInfo>,
}

/// One attribute as the payload declares it.
///
/// Draco names four types and an open-ended `GENERIC`, and puts no limit on how
/// many of each a payload carries: two texture-coordinate sets are ordinary,
/// and glTF's own Draco extension stores joints and weights as generics. The
/// flat mesh the shell works in has room for one of each named type and nothing
/// generic at all, so this list is what makes the difference visible instead of
/// the extra attributes simply not arriving.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize)]
pub struct AttributeInfo {
    /// `POSITION`, `NORMAL`, `COLOR`, `TEX_COORD` or `GENERIC`.
    #[serde(rename = "type")]
    pub attribute_type: String,
    /// Scalar components per value.
    pub components: u8,
    /// Component type, as Draco names it.
    pub data_type: String,
    /// The id the payload assigned, which is how a consumer addresses it.
    pub unique_id: u32,
    /// Unique values stored, before the point mapping is applied.
    pub values: usize,
    /// Whether the flat mesh interpreted this attribute. The rest are carried
    /// through unchanged rather than dropped, but nothing reads their meaning.
    pub read: bool,
}

/// Decode a standalone Draco payload.
#[cfg(feature = "read")]
#[wasm_bindgen]
pub fn parse_drc_bytes(data: &[u8]) -> JsValue {
    let result = parse_drc_internal(data);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

#[cfg(feature = "read")]
fn parse_drc_internal(data: &[u8]) -> ParseResult {
    use draco_core::decoder_buffer::DecoderBuffer;
    use draco_core::mesh_decoder::MeshDecoder;

    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let mut buffer = DecoderBuffer::new(data);
    // Nothing catches a panic here. The decoder returns errors for every
    // malformed payload tried against it -- truncation at each of a payload's
    // byte offsets, and garbage -- and on wasm32 a catch would be theatre
    // anyway: the target has no unwinding, and the release profile aborts on
    // top of that. What actually stands between a trap and a dead page is the
    // shell's own guard around this call.
    match decoder.decode(&mut buffer, &mut mesh) {
        Ok(()) => {
            let mut warnings = Vec::new();
            if mesh.num_faces() == 0 {
                warnings.push(
                    "Draco payload decoded to a point cloud: it carries no faces".to_string(),
                );
            }
            let attributes = describe_attributes(&mesh);
            for attribute in attributes.iter().filter(|attribute| !attribute.read) {
                warnings.push(format!(
                    "Draco payload carries a {} attribute ({} components, id {}) that nothing                      here interprets: it is not shown in the preview, and only a .drc export                      keeps it",
                    attribute.attribute_type, attribute.components, attribute.unique_id,
                ));
            }
            ParseResult {
                success: true,
                attributes,
                meshes: vec![mesh_to_js_data(&mesh)],
                error: None,
                warnings,
            }
        }
        Err(error) => ParseResult {
            success: false,
            meshes: vec![],
            error: Some(error.to_string()),
            warnings: vec![],
            attributes: vec![],
        },
    }
}

#[cfg(feature = "read")]
fn describe_attributes(mesh: &Mesh) -> Vec<AttributeInfo> {
    // `named_attribute_id` answers with the first attribute of a type, which is
    // exactly what the flat mesh reads. So the first of each named type is the
    // one marked as received, and every attribute after it -- a second texture
    // coordinate set, a second colour set, anything generic -- is recorded as
    // present and not read rather than not recorded at all.
    let mut seen = Vec::new();
    (0..mesh.num_attributes())
        .map(|id| {
            let attribute = mesh.attribute(id);
            let attribute_type = attribute.attribute_type();
            let first = !seen.contains(&attribute_type);
            seen.push(attribute_type);
            AttributeInfo {
                attribute_type: attribute_type_name(attribute_type).to_string(),
                components: attribute.num_components(),
                data_type: data_type_name(attribute.data_type()).to_string(),
                unique_id: attribute.unique_id(),
                values: attribute.size(),
                read: first && attribute_type != GeometryAttributeType::Generic,
            }
        })
        .collect()
}

#[cfg(feature = "read")]
fn attribute_type_name(attribute_type: GeometryAttributeType) -> &'static str {
    match attribute_type {
        GeometryAttributeType::Position => "POSITION",
        GeometryAttributeType::Normal => "NORMAL",
        GeometryAttributeType::Color => "COLOR",
        GeometryAttributeType::TexCoord => "TEX_COORD",
        GeometryAttributeType::Generic => "GENERIC",
        GeometryAttributeType::Invalid => "INVALID",
    }
}

#[cfg(feature = "read")]
fn mesh_to_js_data(mesh: &Mesh) -> MeshData {
    let mut indices = Vec::with_capacity(mesh.num_faces() * 3);
    for index in 0..mesh.num_faces() {
        let face = mesh.face(FaceIndex(index as u32));
        indices.extend([face[0].0, face[1].0, face[2].0]);
    }
    MeshData {
        positions: read_attribute_as_f32(mesh, GeometryAttributeType::Position, 3),
        indices,
        normals: read_attribute_as_f32(mesh, GeometryAttributeType::Normal, 3),
        uvs: read_attribute_as_f32(mesh, GeometryAttributeType::TexCoord, 2),
        colors: read_colors(mesh),
        extras: read_extra_attributes(mesh),
    }
}

/// Every attribute past the one of each named type the flat mesh reads.
///
/// Which those are is decided the same way `describe_attributes` decides it,
/// and by the same rule the reader itself follows: `named_attribute_id` answers
/// with the first of a type, so the first of each named type is the one that
/// lands in a slot and everything after it comes through here instead.
#[cfg(feature = "read")]
fn read_extra_attributes(mesh: &Mesh) -> Vec<ExtraAttribute> {
    let mut seen: Vec<GeometryAttributeType> = Vec::new();
    let mut extras = Vec::new();
    for id in 0..mesh.num_attributes() {
        let attribute = mesh.attribute(id);
        let attribute_type = attribute.attribute_type();
        let interpreted =
            !seen.contains(&attribute_type) && attribute_type != GeometryAttributeType::Generic;
        seen.push(attribute_type);
        if interpreted {
            continue;
        }
        let components = attribute.num_components();
        let stride = attribute.byte_stride() as usize;
        let width = attribute.data_type().byte_length();
        let data = attribute.buffer().data();
        let mut values = Vec::with_capacity(mesh.num_points() * components as usize);
        for point in 0..mesh.num_points() {
            let value_index = attribute.mapped_index(PointIndex(point as u32)).0 as usize;
            for component in 0..components as usize {
                let offset = value_index * stride + component * width;
                values.push(if offset + width > data.len() {
                    0.0
                } else {
                    scalar_as_f64(attribute.data_type(), &data[offset..offset + width])
                });
            }
        }
        extras.push(ExtraAttribute {
            attribute_type: attribute_type_name(attribute_type).to_string(),
            components,
            data_type: data_type_name(attribute.data_type()).to_string(),
            unique_id: attribute.unique_id(),
            normalized: attribute.normalized(),
            values,
        });
    }
    extras
}

/// Read an attribute as one float tuple per point, whatever it decoded to.
///
/// Two things make this more than a copy. A Draco attribute is addressed by its
/// own value index rather than by the point id — the encoder deduplicates
/// values and records the mapping, so reading the buffer in point order returns
/// another vertex's data whenever the mapping is not the identity. And a
/// payload whose quantization transform was not applied hands back integers, so
/// the component ladder is not decoration either.
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
    let available = (attribute.num_components() as usize).min(components);
    let stride = attribute.byte_stride() as usize;
    let width = attribute.data_type().byte_length();
    let data = attribute.buffer().data();
    let mut values = Vec::with_capacity(mesh.num_points() * components);
    for point in 0..mesh.num_points() {
        let value_index = attribute.mapped_index(PointIndex(point as u32)).0 as usize;
        for component in 0..components {
            let offset = value_index * stride + component * width;
            if component >= available || offset + width > data.len() {
                values.push(0.0);
                continue;
            }
            values.push(scalar_as_f32(
                attribute.data_type(),
                &data[offset..offset + width],
            ));
        }
    }
    values
}

/// A component's name, and the same name read back.
///
/// The pair is written together because they are one contract: a name this side
/// cannot parse means an attribute that decoded and cannot be written again.
#[cfg(any(feature = "read", feature = "write"))]
fn data_type_name(data_type: DataType) -> &'static str {
    match data_type {
        DataType::Int8 => "int8",
        DataType::Uint8 => "uint8",
        DataType::Int16 => "int16",
        DataType::Uint16 => "uint16",
        DataType::Int32 => "int32",
        DataType::Uint32 => "uint32",
        DataType::Int64 => "int64",
        DataType::Uint64 => "uint64",
        DataType::Float32 => "float32",
        DataType::Float64 => "float64",
        DataType::Bool => "bool",
        DataType::Invalid => "invalid",
    }
}

#[cfg(feature = "write")]
fn data_type_from_name(name: &str) -> Option<DataType> {
    Some(match name {
        "int8" => DataType::Int8,
        "uint8" => DataType::Uint8,
        "int16" => DataType::Int16,
        "uint16" => DataType::Uint16,
        "int32" => DataType::Int32,
        "uint32" => DataType::Uint32,
        "int64" => DataType::Int64,
        "uint64" => DataType::Uint64,
        "float32" => DataType::Float32,
        "float64" => DataType::Float64,
        "bool" => DataType::Bool,
        _ => return None,
    })
}

#[cfg(feature = "write")]
fn attribute_type_from_name(name: &str) -> Option<GeometryAttributeType> {
    Some(match name {
        "POSITION" => GeometryAttributeType::Position,
        "NORMAL" => GeometryAttributeType::Normal,
        "COLOR" => GeometryAttributeType::Color,
        "TEX_COORD" => GeometryAttributeType::TexCoord,
        "GENERIC" => GeometryAttributeType::Generic,
        _ => return None,
    })
}

/// One component as a double, which every Draco type fits without loss.
#[cfg(feature = "read")]
fn scalar_as_f64(data_type: DataType, bytes: &[u8]) -> f64 {
    match data_type {
        DataType::Float32 => f32::from_le_bytes(bytes.try_into().unwrap()) as f64,
        DataType::Float64 => f64::from_le_bytes(bytes.try_into().unwrap()),
        DataType::Int8 => bytes[0] as i8 as f64,
        DataType::Uint8 | DataType::Bool => bytes[0] as f64,
        DataType::Int16 => i16::from_le_bytes(bytes.try_into().unwrap()) as f64,
        DataType::Uint16 => u16::from_le_bytes(bytes.try_into().unwrap()) as f64,
        DataType::Int32 => i32::from_le_bytes(bytes.try_into().unwrap()) as f64,
        DataType::Uint32 => u32::from_le_bytes(bytes.try_into().unwrap()) as f64,
        DataType::Int64 => i64::from_le_bytes(bytes.try_into().unwrap()) as f64,
        DataType::Uint64 => u64::from_le_bytes(bytes.try_into().unwrap()) as f64,
        DataType::Invalid => 0.0,
    }
}

/// The same value back in the component type it came from.
#[cfg(feature = "write")]
fn scalar_to_bytes(data_type: DataType, value: f64) -> Vec<u8> {
    match data_type {
        DataType::Float32 => (value as f32).to_le_bytes().to_vec(),
        DataType::Float64 => value.to_le_bytes().to_vec(),
        DataType::Int8 => (value as i8).to_le_bytes().to_vec(),
        DataType::Uint8 | DataType::Bool => (value as u8).to_le_bytes().to_vec(),
        DataType::Int16 => (value as i16).to_le_bytes().to_vec(),
        DataType::Uint16 => (value as u16).to_le_bytes().to_vec(),
        DataType::Int32 => (value as i32).to_le_bytes().to_vec(),
        DataType::Uint32 => (value as u32).to_le_bytes().to_vec(),
        DataType::Int64 => (value as i64).to_le_bytes().to_vec(),
        DataType::Uint64 => (value as u64).to_le_bytes().to_vec(),
        DataType::Invalid => Vec::new(),
    }
}

#[cfg(feature = "read")]
fn scalar_as_f32(data_type: DataType, bytes: &[u8]) -> f32 {
    match data_type {
        DataType::Float32 => f32::from_le_bytes(bytes.try_into().unwrap()),
        DataType::Float64 => f64::from_le_bytes(bytes.try_into().unwrap()) as f32,
        DataType::Int8 => bytes[0] as i8 as f32,
        DataType::Uint8 => bytes[0] as f32,
        DataType::Int16 => i16::from_le_bytes(bytes.try_into().unwrap()) as f32,
        DataType::Uint16 => u16::from_le_bytes(bytes.try_into().unwrap()) as f32,
        DataType::Int32 => i32::from_le_bytes(bytes.try_into().unwrap()) as f32,
        DataType::Uint32 => u32::from_le_bytes(bytes.try_into().unwrap()) as f32,
        _ => 0.0,
    }
}

#[cfg(feature = "read")]
fn read_colors(mesh: &Mesh) -> Vec<u8> {
    let attribute_id = mesh.named_attribute_id(GeometryAttributeType::Color);
    if attribute_id < 0 {
        return Vec::new();
    }
    // The shell wants RGBA bytes; anything wider is scaled here rather than in
    // four places downstream.
    let channels = read_attribute_as_f32(mesh, GeometryAttributeType::Color, 4);
    let attribute = mesh.attribute(attribute_id);
    let float_source = matches!(attribute.data_type(), DataType::Float32 | DataType::Float64);
    let opaque = if attribute.num_components() >= 4 {
        None
    } else {
        Some(255u8)
    };
    let mut colors = Vec::with_capacity(channels.len());
    for (index, value) in channels.iter().enumerate() {
        if index % 4 == 3 {
            if let Some(alpha) = opaque {
                colors.push(alpha);
                continue;
            }
        }
        let scaled = if float_source { value * 255.0 } else { *value };
        colors.push(scaled.clamp(0.0, 255.0) as u8);
    }
    colors
}

// ===========================================================================
// Writer
// ===========================================================================

/// Input mesh data consumed by the encoder, from JavaScript.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct MeshInput {
    /// Vertex positions as a flat array `[x0, y0, z0, x1, ...]`.
    pub positions: Vec<f32>,
    /// Triangle indices as a flat array.
    pub indices: Vec<u32>,
    /// Vertex normals (optional).
    pub normals: Option<Vec<f32>>,
    /// Texture coordinates (optional).
    pub uvs: Option<Vec<f32>>,
    /// Vertex colors as `[r, g, b, a, ...]`, 0-255 (optional).
    pub colors: Option<Vec<u8>>,
    /// Attributes to write back exactly as they were read, uninterpreted.
    pub extras: Option<Vec<ExtraAttribute>>,
}

/// Encoder options, named as the export panel's controls are.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize, Default)]
pub struct ExportOptions {
    /// 0 is the smallest file, 10 the fastest encode. Draco's own scale.
    pub encoding_speed: Option<i32>,
    /// Kept equal to `encoding_speed` when unset, which is Draco's default.
    pub decoding_speed: Option<i32>,
    /// Quantization bits for positions.
    pub position_bits: Option<i32>,
    /// Quantization bits for normals.
    pub normal_bits: Option<i32>,
    /// Quantization bits for texture coordinates.
    pub texcoord_bits: Option<i32>,
    /// Whether to write normals, when the mesh has them.
    pub include_normals: Option<bool>,
    /// Whether to write texture coordinates, when the mesh has them.
    pub include_uvs: Option<bool>,
    /// Whether to write colors, when the mesh has them.
    pub include_colors: Option<bool>,
}

/// Export result. `binary_data` is the payload; `.drc` has no text container.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize)]
pub struct ExportResult {
    /// Whether the payload was encoded.
    pub success: bool,
    /// The encoded payload.
    pub binary_data: Option<Vec<u8>>,
    /// Why the encode failed, when it did.
    pub error: Option<String>,
    /// What the encoder did, for the compression panel.
    pub draco_stats: Option<DracoStats>,
}

/// What the encoder did, in the shape the export panel already reads.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize)]
pub struct DracoStats {
    /// Always 1 here: a `.drc` holds one mesh.
    pub primitives: usize,
    /// The speed setting the payload was written at.
    pub speed: i32,
    /// Bytes of Draco payload.
    pub compressed_size: usize,
    /// `edgebreaker` or `sequential`, as the encoder settled it.
    pub method: Option<String>,
    /// The schemes selected per attribute, including their semantic names.
    pub prediction_scheme: Option<String>,
}

#[cfg(feature = "write")]
fn prediction_summary(info: &draco_core::EncodedMeshInfo) -> Option<String> {
    let schemes: Vec<String> = info
        .attributes
        .iter()
        .filter_map(|attribute| {
            let (method, transform) = attribute.prediction?;
            let semantic = match attribute.attribute_type {
                draco_core::geometry_attribute::GeometryAttributeType::Position => "POSITION",
                draco_core::geometry_attribute::GeometryAttributeType::Normal => "NORMAL",
                draco_core::geometry_attribute::GeometryAttributeType::Color => "COLOR",
                draco_core::geometry_attribute::GeometryAttributeType::TexCoord => "TEX_COORD",
                draco_core::geometry_attribute::GeometryAttributeType::Generic => "GENERIC",
                draco_core::geometry_attribute::GeometryAttributeType::Invalid => "INVALID",
            };
            Some(format!(
                "{semantic}: {}",
                prediction_scheme_name(method, transform),
            ))
        })
        .collect();
    (!schemes.is_empty()).then(|| schemes.join("; "))
}

#[cfg(feature = "write")]
fn prediction_scheme_name(
    method: draco_core::prediction_scheme::PredictionSchemeMethod,
    transform: draco_core::prediction_scheme::PredictionSchemeTransformType,
) -> String {
    let method = match method {
        draco_core::prediction_scheme::PredictionSchemeMethod::None => "None",
        draco_core::prediction_scheme::PredictionSchemeMethod::Undefined => "Undefined",
        draco_core::prediction_scheme::PredictionSchemeMethod::Difference => "Difference",
        draco_core::prediction_scheme::PredictionSchemeMethod::MeshPredictionParallelogram => {
            "Parallelogram"
        }
        draco_core::prediction_scheme::PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
            "Multi-parallelogram"
        }
        draco_core::prediction_scheme::PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
            "TexCoords (legacy)"
        }
        draco_core::prediction_scheme::PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram => {
            "Constrained multi-parallelogram"
        }
        draco_core::prediction_scheme::PredictionSchemeMethod::MeshPredictionTexCoordsPortable => {
            "TexCoords"
        }
        draco_core::prediction_scheme::PredictionSchemeMethod::MeshPredictionGeometricNormal => {
            "Geometric normal"
        }
    };
    let transform = match transform {
        draco_core::prediction_scheme::PredictionSchemeTransformType::None => "None",
        draco_core::prediction_scheme::PredictionSchemeTransformType::Delta => "Delta",
        draco_core::prediction_scheme::PredictionSchemeTransformType::Wrap => "Wrap",
        draco_core::prediction_scheme::PredictionSchemeTransformType::NormalOctahedron => {
            "Normal octahedron"
        }
        draco_core::prediction_scheme::PredictionSchemeTransformType::NormalOctahedronCanonicalized => {
            "Canonicalized normal octahedron"
        }
        draco_core::prediction_scheme::PredictionSchemeTransformType::Parallelogram => {
            "Parallelogram"
        }
        draco_core::prediction_scheme::PredictionSchemeTransformType::TexCoordsPortable => {
            "TexCoords"
        }
        draco_core::prediction_scheme::PredictionSchemeTransformType::GeometricNormal => {
            "Geometric normal"
        }
        draco_core::prediction_scheme::PredictionSchemeTransformType::MultiParallelogram => {
            "Multi-parallelogram"
        }
        draco_core::prediction_scheme::PredictionSchemeTransformType::ConstrainedMultiParallelogram => {
            "Constrained multi-parallelogram"
        }
    };
    format!("{method} ({transform})")
}

/// Encode mesh data into a standalone Draco payload.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub fn create_drc(mesh_js: JsValue, options_js: JsValue) -> JsValue {
    let mesh: MeshInput = match serde_wasm_bindgen::from_value(mesh_js) {
        Ok(mesh) => mesh,
        Err(error) => {
            return to_js(&ExportResult {
                success: false,
                binary_data: None,
                error: Some(format!("Invalid mesh data: {error}")),
                draco_stats: None,
            });
        }
    };
    let options: ExportOptions = serde_wasm_bindgen::from_value(options_js).unwrap_or_default();
    to_js(&create_drc_internal(&mesh, &options))
}

#[cfg(feature = "write")]
fn to_js(result: &ExportResult) -> JsValue {
    serde_wasm_bindgen::to_value(result).unwrap_or(JsValue::NULL)
}

#[cfg(feature = "write")]
fn create_drc_internal(input: &MeshInput, options: &ExportOptions) -> ExportResult {
    use draco_core::encoder_buffer::EncoderBuffer;
    use draco_core::encoder_options::EncoderOptions;
    use draco_core::mesh_encoder::MeshEncoder;

    let (mesh, quantization) = match mesh_input_to_core_mesh(input, options) {
        Ok(built) => built,
        Err(error) => {
            return ExportResult {
                success: false,
                binary_data: None,
                error: Some(error),
                draco_stats: None,
            }
        }
    };

    let speed = options.encoding_speed.unwrap_or(0).clamp(0, 10);
    let mut settings = EncoderOptions::new();
    settings.set_global_int("encoding_speed", speed);
    settings.set_global_int("decoding_speed", options.decoding_speed.unwrap_or(speed));
    for (attribute_id, bits) in quantization {
        settings.set_attribute_int(attribute_id, "quantization_bits", bits);
    }

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut output = EncoderBuffer::new();
    match encoder.encode(&settings, &mut output) {
        Ok(()) => {
            let bytes = output.data().to_vec();
            // Which method it settled on is a decision, not an echo of the
            // request: with no explicit choice the encoder takes sequential at
            // speed 10 and edgebreaker below it.
            let info = encoder.encoded_mesh_info();
            let method = info.map(|info| match info.encoding_method {
                1 => "edgebreaker".to_string(),
                0 => "sequential".to_string(),
                other => format!("method {other}"),
            });
            let actual_speed = info.map_or(speed, |info| info.speed);
            ExportResult {
                success: true,
                draco_stats: Some(DracoStats {
                    primitives: 1,
                    speed: actual_speed,
                    compressed_size: bytes.len(),
                    method,
                    prediction_scheme: info.and_then(prediction_summary),
                }),
                binary_data: Some(bytes),
                error: None,
            }
        }
        Err(error) => ExportResult {
            success: false,
            binary_data: None,
            error: Some(error.to_string()),
            draco_stats: None,
        },
    }
}

/// Build the mesh, and say which attribute id each bit setting belongs to.
///
/// The ids are only knowable here: `set_attribute_int` addresses attributes by
/// the order they were added, so the caller's per-channel bit counts cannot be
/// turned into encoder options until the mesh exists.
#[cfg(feature = "write")]
fn mesh_input_to_core_mesh(
    input: &MeshInput,
    options: &ExportOptions,
) -> Result<(Mesh, Vec<(i32, i32)>), String> {
    if !input.positions.len().is_multiple_of(3) {
        return Err("positions length must be divisible by 3".to_string());
    }
    if !input.indices.len().is_multiple_of(3) {
        return Err("indices length must be divisible by 3".to_string());
    }
    let vertex_count = input.positions.len() / 3;
    if vertex_count == 0 {
        return Err("mesh has no vertices".to_string());
    }
    if let Some(&highest) = input.indices.iter().max() {
        if highest as usize >= vertex_count {
            return Err("an index points past the last vertex".to_string());
        }
    }

    let mut mesh = Mesh::new();
    mesh.set_num_points(vertex_count);
    let mut quantization = Vec::new();

    let position_id = mesh.add_attribute(float_attribute(
        GeometryAttributeType::Position,
        3,
        &input.positions,
        vertex_count,
    ));
    if let Some(bits) = options.position_bits {
        quantization.push((position_id, bits));
    }

    if options.include_normals.unwrap_or(true) {
        if let Some(normals) = channel(&input.normals, vertex_count * 3) {
            let id = mesh.add_attribute(float_attribute(
                GeometryAttributeType::Normal,
                3,
                normals,
                vertex_count,
            ));
            if let Some(bits) = options.normal_bits {
                quantization.push((id, bits));
            }
        }
    }

    if options.include_uvs.unwrap_or(true) {
        if let Some(uvs) = channel(&input.uvs, vertex_count * 2) {
            let id = mesh.add_attribute(float_attribute(
                GeometryAttributeType::TexCoord,
                2,
                uvs,
                vertex_count,
            ));
            if let Some(bits) = options.texcoord_bits {
                quantization.push((id, bits));
            }
        }
    }

    if options.include_colors.unwrap_or(true) {
        if let Some(colors) = channel(&input.colors, vertex_count * 4) {
            let mut attribute = PointAttribute::new();
            attribute.init(
                GeometryAttributeType::Color,
                4,
                DataType::Uint8,
                true,
                vertex_count,
            );
            attribute.buffer_mut().write(0, &colors[..vertex_count * 4]);
            mesh.add_attribute(attribute);
        }
    }

    // Whatever the flat mesh could not name, put back with the type, component
    // count, component type and id it arrived with. Nothing here decides what
    // the attribute means, which is the point: a .drc that came in with a
    // second UV set or a generic goes out with it still there.
    //
    // The id is preserved rather than reassigned, because it is how a consumer
    // addresses the attribute -- glTF's Draco extension names attributes by it,
    // and renumbering would break a payload extracted from one. `add_attribute`
    // overwrites it with the attribute's position, so the ids the named
    // attributes above already took are the only ones an extra cannot keep.
    let mut taken: Vec<u32> = (0..mesh.num_attributes())
        .map(|id| mesh.attribute(id).unique_id())
        .collect();
    for extra in input.extras.iter().flatten() {
        let attribute_type = attribute_type_from_name(&extra.attribute_type)
            .ok_or_else(|| format!("unknown attribute type {}", extra.attribute_type))?;
        let data_type = data_type_from_name(&extra.data_type)
            .ok_or_else(|| format!("unknown component type {}", extra.data_type))?;
        let components = extra.components as usize;
        if components == 0 || extra.values.len() < vertex_count * components {
            return Err(format!(
                "attribute {} states {components} components and {} values for {vertex_count}                  vertices",
                extra.unique_id,
                extra.values.len(),
            ));
        }
        let width = data_type.byte_length();
        let mut attribute = PointAttribute::new();
        attribute.init(
            attribute_type,
            extra.components,
            data_type,
            extra.normalized,
            vertex_count,
        );
        for (index, chunk) in extra
            .values
            .chunks_exact(components)
            .take(vertex_count)
            .enumerate()
        {
            let bytes: Vec<u8> = chunk
                .iter()
                .flat_map(|value| scalar_to_bytes(data_type, *value))
                .collect();
            attribute
                .buffer_mut()
                .write(index * components * width, &bytes);
        }
        let unique_id = if taken.contains(&extra.unique_id) {
            (0u32..).find(|id| !taken.contains(id)).unwrap()
        } else {
            extra.unique_id
        };
        attribute.set_unique_id(unique_id);
        taken.push(unique_id);
        mesh.add_attribute_preserve_unique_id(attribute);
    }

    mesh.set_num_faces(input.indices.len() / 3);
    for (index, chunk) in input.indices.chunks_exact(3).enumerate() {
        mesh.set_face(
            FaceIndex(index as u32),
            [
                PointIndex(chunk[0]),
                PointIndex(chunk[1]),
                PointIndex(chunk[2]),
            ],
        );
    }
    Ok((mesh, quantization))
}

/// An optional channel, only when it covers every vertex.
///
/// A short channel is dropped rather than padded: the encoder would otherwise
/// write zeros that look like data, and a partial normal set is a bug upstream
/// rather than something to preserve.
#[cfg(feature = "write")]
fn channel<T>(values: &Option<Vec<T>>, required: usize) -> Option<&[T]> {
    values
        .as_deref()
        .filter(|values| values.len() >= required && required > 0)
}

#[cfg(feature = "write")]
fn float_attribute(
    attribute_type: GeometryAttributeType,
    components: usize,
    values: &[f32],
    vertex_count: usize,
) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(
        attribute_type,
        components as u8,
        DataType::Float32,
        false,
        vertex_count,
    );
    let width = components * 4;
    for (index, chunk) in values
        .chunks_exact(components)
        .take(vertex_count)
        .enumerate()
    {
        let bytes: Vec<u8> = chunk.iter().flat_map(|value| value.to_le_bytes()).collect();
        attribute.buffer_mut().write(index * width, &bytes);
    }
    attribute
}

#[cfg(all(test, feature = "read", feature = "write"))]
mod tests {
    use super::*;

    fn declares(attributes: &[AttributeInfo], name: &str) -> bool {
        attributes
            .iter()
            .any(|attribute| attribute.attribute_type == name)
    }

    fn quad() -> MeshInput {
        MeshInput {
            positions: vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                1.0, 1.0, 0.0, //
                0.0, 1.0, 0.0,
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            normals: Some([0.0, 0.0, 1.0].repeat(4)),
            uvs: Some(vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]),
            colors: None,
            extras: None,
        }
    }

    /// The decoded corners, rounded to something quantization cannot move, and
    /// sorted: Draco reorders points as it encodes, so the vertex list comes
    /// back as a set rather than as a sequence.
    fn corner_set(positions: &[f32], indices: &[u32]) -> Vec<[i32; 3]> {
        let mut corners: Vec<[i32; 3]> = indices
            .iter()
            .map(|index| {
                let base = *index as usize * 3;
                [
                    (positions[base] * 1000.0).round() as i32,
                    (positions[base + 1] * 1000.0).round() as i32,
                    (positions[base + 2] * 1000.0).round() as i32,
                ]
            })
            .collect();
        corners.sort_unstable();
        corners
    }

    /// Encode and decode across the two entry points the shell calls. Positions
    /// are quantized, so they come back near rather than equal; the topology
    /// does not get that latitude.
    #[test]
    fn test_roundtrip_through_the_js_entry_points() {
        let exported = create_drc_internal(
            &quad(),
            &ExportOptions {
                position_bits: Some(16),
                texcoord_bits: Some(12),
                ..Default::default()
            },
        );
        assert!(exported.success, "{:?}", exported.error);
        let payload = exported.binary_data.unwrap();
        assert_eq!(exported.draco_stats.unwrap().compressed_size, payload.len());

        let parsed = parse_drc_internal(&payload);
        assert!(parsed.success, "{:?}", parsed.error);
        let mesh = &parsed.meshes[0];
        assert_eq!(mesh.indices.len(), 6);
        assert_eq!(mesh.positions.len(), 12);
        assert!(declares(&parsed.attributes, "POSITION"));
        assert!(declares(&parsed.attributes, "TEX_COORD"));
        // The same six corners at the same places, however the encoder chose to
        // number them.
        assert_eq!(
            corner_set(&mesh.positions, &mesh.indices),
            corner_set(&quad().positions, &quad().indices),
        );
    }

    /// A `.drc` arrives over the network and the decoder trusts lengths the
    /// payload states. Garbage and every truncation of a real payload have to
    /// come back as errors: nothing above this catches a panic, so a panic is a
    /// trap that takes the page with it.
    #[test]
    fn test_malformed_payloads_report_rather_than_panic() {
        let garbage = parse_drc_internal(b"not a draco payload at all, but long enough to try");
        assert!(!garbage.success);
        assert!(garbage.error.is_some());

        let payload = create_drc_internal(&quad(), &ExportOptions::default())
            .binary_data
            .unwrap();
        for cut in 1..payload.len() {
            let result = parse_drc_internal(&payload[..cut]);
            assert!(!result.success, "a payload cut to {cut} bytes decoded");
            assert!(
                result.error.is_some(),
                "cut to {cut} bytes reported nothing"
            );
        }
    }

    /// Colours are the attribute the flat export path used to drop on the
    /// floor, and the encoding method is what the statistics panel had no way
    /// to name. Both are checked where they are produced.
    #[test]
    fn test_colours_survive_and_the_method_is_named() {
        let mut input = quad();
        input.colors = Some(vec![
            255, 0, 0, 255, //
            0, 255, 0, 255, //
            0, 0, 255, 255, //
            255, 255, 0, 255,
        ]);
        let exported = create_drc_internal(&input, &ExportOptions::default());
        assert!(exported.success, "{:?}", exported.error);
        let stats = exported.draco_stats.unwrap();
        assert_eq!(stats.method.as_deref(), Some("edgebreaker"));

        let parsed = parse_drc_internal(&exported.binary_data.unwrap());
        assert!(parsed.success, "{:?}", parsed.error);
        assert!(declares(&parsed.attributes, "COLOR"));
        let colors = &parsed.meshes[0].colors;
        assert_eq!(colors.len(), 16);
        // Draco renumbers the points, so the colours come back as a set.
        let mut seen: Vec<[u8; 4]> = colors
            .chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect();
        seen.sort_unstable();
        assert_eq!(
            seen,
            vec![
                [0, 0, 255, 255],
                [0, 255, 0, 255],
                [255, 0, 0, 255],
                [255, 255, 0, 255],
            ],
        );
    }

    /// Sequential is not a request the caller makes here; it is what the
    /// encoder settles on at the fastest speed, and the panel reports the
    /// decision rather than the ask.
    #[test]
    fn test_the_fastest_speed_reports_sequential() {
        let exported = create_drc_internal(
            &quad(),
            &ExportOptions {
                encoding_speed: Some(10),
                ..Default::default()
            },
        );
        assert!(exported.success, "{:?}", exported.error);
        assert_eq!(
            exported.draco_stats.unwrap().method.as_deref(),
            Some("sequential"),
        );
    }

    /// A payload may carry more than the flat mesh has room for: Draco puts no
    /// limit on how many attributes of a type it stores, and glTF's own Draco
    /// extension keeps joints and weights as generics. Those used to arrive and
    /// vanish without a word; now the file says what it contains and the shell
    /// is told what it did not get.
    #[test]
    fn test_extra_attributes_are_declared_and_reported() {
        use draco_core::encoder_buffer::EncoderBuffer;
        use draco_core::encoder_options::EncoderOptions;
        use draco_core::mesh_encoder::MeshEncoder;

        let input = quad();
        let (mut mesh, _) = mesh_input_to_core_mesh(&input, &ExportOptions::default()).unwrap();
        let vertices = input.positions.len() / 3;
        // A second texture coordinate set and one generic attribute, which is
        // the shape a lightmapped, skinned asset arrives in.
        mesh.add_attribute(float_attribute(
            GeometryAttributeType::TexCoord,
            2,
            &vec![0.25f32; vertices * 2],
            vertices,
        ));
        mesh.add_attribute(float_attribute(
            GeometryAttributeType::Generic,
            4,
            &vec![0.5f32; vertices * 4],
            vertices,
        ));

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh);
        let mut output = EncoderBuffer::new();
        encoder.encode(&EncoderOptions::new(), &mut output).unwrap();

        let parsed = parse_drc_internal(output.data());
        assert!(parsed.success, "{:?}", parsed.error);
        assert_eq!(parsed.attributes.len(), 5, "{:?}", parsed.attributes.len());
        let unread: Vec<&str> = parsed
            .attributes
            .iter()
            .filter(|attribute| !attribute.read)
            .map(|attribute| attribute.attribute_type.as_str())
            .collect();
        assert_eq!(unread, vec!["TEX_COORD", "GENERIC"]);
        assert_eq!(parsed.warnings.len(), 2, "{:?}", parsed.warnings);
        assert!(parsed
            .warnings
            .iter()
            .all(|warning| warning.contains("interprets")));
        // And the one of each type the flat mesh does read still arrives.
        assert_eq!(parsed.meshes[0].uvs.len(), vertices * 2);

        // The uninterpreted ones came through whole, and go back out the same.
        let extras = &parsed.meshes[0].extras;
        assert_eq!(extras.len(), 2);
        assert_eq!(extras[0].attribute_type, "TEX_COORD");
        assert_eq!(extras[0].components, 2);
        assert_eq!(extras[0].data_type, "float32");
        assert_eq!(extras[0].values, vec![0.25f64; vertices * 2]);
        assert_eq!(extras[1].attribute_type, "GENERIC");
        assert_eq!(extras[1].components, 4);
        assert_eq!(extras[1].values, vec![0.5f64; vertices * 4]);

        // Ids well past the attribute count, so a writer that renumbers by
        // position rather than preserving them fails here.
        let mut relabelled = extras.clone();
        relabelled[0].unique_id = 11;
        relabelled[1].unique_id = 12;
        let mut again = quad();
        again.extras = Some(relabelled.clone());
        let rewritten = create_drc_internal(&again, &ExportOptions::default());
        assert!(rewritten.success, "{:?}", rewritten.error);
        let reparsed = parse_drc_internal(&rewritten.binary_data.unwrap());
        assert!(reparsed.success, "{:?}", reparsed.error);
        let round_tripped = &reparsed.meshes[0].extras;
        assert_eq!(round_tripped.len(), 2);
        for (before, after) in relabelled.iter().zip(round_tripped.iter()) {
            assert_eq!(after.attribute_type, before.attribute_type);
            assert_eq!(after.components, before.components);
            assert_eq!(after.data_type, before.data_type);
            assert_eq!(after.unique_id, before.unique_id);
            assert_eq!(after.values, before.values);
        }
    }

    #[test]
    fn test_encoder_rejects_an_index_past_the_last_vertex() {
        let mut mesh = quad();
        mesh.indices = vec![0, 1, 99];
        let result = create_drc_internal(&mesh, &ExportOptions::default());
        assert!(!result.success);
        assert!(result.error.unwrap().contains("past the last vertex"));
    }
}
