//! Document-preserving glTF Draco compression.
//!
//! [`compress_gltf_bytes`] takes a self-contained glTF or GLB document and
//! returns a copy whose triangle-mesh geometry is compressed with
//! `KHR_draco_mesh_compression`, while **everything else in the document is
//! carried through untouched**: materials, textures, images, samplers, cameras,
//! nodes, animations, skins, `extras`, and unknown extension JSON that has no
//! opaque binary references. Unknown buffer/view/offset-like extension fields
//! are rejected because they cannot be remapped safely.
//!
//! This is the key difference from the `read meshes -> write fresh glTF` path,
//! which only models geometry and therefore drops materials and other content.
//! Here we mutate the original JSON document in place and only touch the parts
//! that change: the compressed primitives, their geometry accessors, the
//! buffer, and the buffer views.
//!
//! # What gets compressed
//!
//! A primitive is compressed only when its structure can be reproduced. The
//! default quantization is intentionally lossy; set an attribute class to
//! `None` in [`QuantizationOptions`] to disable its quantization:
//!
//! - triangle list (`mode` 4 or absent), indexed or non-indexed (a fresh
//!   indices accessor is generated for the non-indexed case),
//! - not already Draco-compressed,
//! - its geometry accessors are not shared with any other primitive,
//! - decoding and re-encoding succeed and reproduce the exact attribute set.
//!
//! All standard attribute semantics are compressed, including `TANGENT`,
//! `JOINTS_n`, `WEIGHTS_n`, multiple `TEXCOORD_n`/`COLOR_n`, and custom `_*`
//! attributes: non-standard ones ride along inside the Draco stream as generic
//! attributes and are named by the extension's attribute map (the glTF semantic
//! lives in the map, not in the Draco attribute). Skinned and tangent-bearing
//! meshes are therefore compressed, not just preserved.
//!
//! Primitives that fall outside this scope — already Draco, sharing geometry
//! accessors, sparse accessors, or an attribute layout the encoder rejects —
//! are left uncompressed but fully preserved, along with the rest of the
//! document.
//!
//! Non-triangle primitives are likewise left uncompressed, and this is required
//! by the spec, not a limitation: `KHR_draco_mesh_compression` restricts the
//! primitive `mode` to `TRIANGLES` or `TRIANGLE_STRIP` ("Restrictions on
//! geometry type"), so point clouds (`POINTS`) and line modes cannot be
//! Draco-compressed in glTF at all. (Only `TRIANGLES` is compressed here;
//! `TRIANGLE_STRIP` is allowed by the spec but uncommon and left as-is.)

use std::collections::{BTreeSet, HashMap};
#[cfg(feature = "gltf-reader")]
use std::path::Path;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use serde_json::{Map, Value};

pub use crate::gltf_container::OutputFormat;
#[cfg(feature = "gltf-reader")]
use crate::gltf_container::{
    parse_gltf_container, serialize_gltf_document, FileResourceResolver, ResourceLimits,
    ResourceResolver,
};
use crate::gltf_geometry::{
    component_type_for_data_type, gltf_type_for_num_components, validate_semantic_accessor,
    GltfError,
};
// The byte API parses + resolves buffers through the reader; the in-memory
// `compress_gltf_value` does not need it.
use crate::gltf_khr_draco::{parse_khr_draco_mesh_compression, validate_khr_draco_document};
#[cfg(feature = "gltf-reader")]
use crate::gltf_reader::GltfReader;
use crate::gltf_writer::encode_draco_mesh_with_info;

type Result<T> = std::result::Result<T, GltfError>;

const KHR_DRACO: &str = "KHR_draco_mesh_compression";
const MODE_TRIANGLES: u64 = 4;
const MODE_TRIANGLE_STRIP: u64 = 5;

/// Draco mesh encoding method selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EncodingMethod {
    /// Let the encoder choose based on the geometry and speed settings.
    #[default]
    Auto,
    /// Force the sequential mesh encoder.
    Sequential,
    /// Force the EdgeBreaker mesh encoder.
    Edgebreaker,
}

/// Per-attribute quantization. `None` disables quantization for that class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuantizationOptions {
    pub position: Option<u8>,
    pub normal: Option<u8>,
    pub color: Option<u8>,
    pub texcoord: Option<u8>,
    pub generic: Option<u8>,
}

impl Default for QuantizationOptions {
    fn default() -> Self {
        Self {
            position: Some(14),
            normal: Some(10),
            color: Some(8),
            texcoord: Some(12),
            generic: Some(8),
        }
    }
}

/// Options shared by the document compressor and geometry writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GltfCompressionOptions {
    pub quantization: QuantizationOptions,
    pub encoding_speed: u8,
    pub decoding_speed: u8,
    pub encoding_method: EncodingMethod,
    pub output_format: OutputFormat,
}

impl GltfCompressionOptions {
    /// Validate all numeric ranges without clamping.
    pub fn validate(&self) -> Result<()> {
        if self.encoding_speed > 10 {
            return Err(GltfError::InvalidOptions(format!(
                "encoding_speed {} is outside 0..=10",
                self.encoding_speed
            )));
        }
        if self.decoding_speed > 10 {
            return Err(GltfError::InvalidOptions(format!(
                "decoding_speed {} is outside 0..=10",
                self.decoding_speed
            )));
        }
        validate_quantization("position", self.quantization.position, 1, 31)?;
        validate_quantization("normal", self.quantization.normal, 2, 30)?;
        validate_quantization("color", self.quantization.color, 1, 31)?;
        validate_quantization("texcoord", self.quantization.texcoord, 1, 31)?;
        validate_quantization("generic", self.quantization.generic, 1, 31)?;
        Ok(())
    }
}

impl Default for GltfCompressionOptions {
    fn default() -> Self {
        Self {
            quantization: QuantizationOptions::default(),
            encoding_speed: 5,
            decoding_speed: 5,
            encoding_method: EncodingMethod::Auto,
            output_format: OutputFormat::SameAsInput,
        }
    }
}

fn validate_quantization(name: &str, bits: Option<u8>, min: u8, max: u8) -> Result<()> {
    if bits.is_some_and(|bits| !(min..=max).contains(&bits)) {
        return Err(GltfError::InvalidOptions(format!(
            "{name} quantization must be None or {min}..={max}"
        )));
    }
    Ok(())
}

/// Stable location of a primitive in a glTF document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PrimitiveLocation {
    pub mesh: usize,
    pub primitive: usize,
}

/// Why a valid primitive was preserved instead of compressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreserveReason {
    AlreadyDraco,
    UnsupportedMode { mode: u32 },
    UnsupportedLayout { detail: String },
    SparseAccessor { accessor: usize },
    MorphTargets,
    SharedAccessor { accessor: usize },
}

/// One preserved primitive and its typed reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreservedPrimitive {
    pub primitive: PrimitiveLocation,
    pub reason: PreserveReason,
}

/// Primitive-by-primitive result of compression.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompressionReport {
    pub compressed_primitives: Vec<PrimitiveLocation>,
    pub preserved_primitives: Vec<PreservedPrimitive>,
}

/// Data plus its compression report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressionOutput<T> {
    pub data: T,
    pub report: CompressionReport,
}

fn json_index(value: &Value, label: &str) -> Result<usize> {
    value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| GltfError::InvalidGltf(format!("{label} is not a valid index")))
}

/// Compress the geometry of a self-contained glTF/GLB document with Draco,
/// preserving all other document content.
///
/// `input` may be GLB bytes or glTF JSON whose buffers/images are embedded as
/// data URIs (use [`compress_gltf_bytes_with_base_path`] for external files).
/// The output container matches the input (GLB in -> GLB out, glTF in -> glTF
/// out with an embedded buffer).
#[cfg(feature = "gltf-reader")]
pub fn compress_gltf_bytes(input: &[u8]) -> Result<CompressionOutput<Vec<u8>>> {
    compress_gltf_bytes_with_options(input, &GltfCompressionOptions::default())
}

/// Compress with explicit options and no external file resolver.
#[cfg(feature = "gltf-reader")]
pub fn compress_gltf_bytes_with_options(
    input: &[u8],
    options: &GltfCompressionOptions,
) -> Result<CompressionOutput<Vec<u8>>> {
    compress_gltf_bytes_impl(input, None, &ResourceLimits::default(), options)
}

/// Like [`compress_gltf_bytes`], but resolves external buffers/`.bin` files
/// relative to `base_path`.
#[cfg(feature = "gltf-reader")]
pub fn compress_gltf_bytes_with_base_path(
    input: &[u8],
    base_path: Option<&Path>,
    options: &GltfCompressionOptions,
) -> Result<CompressionOutput<Vec<u8>>> {
    let resolver =
        base_path.map(|base| FileResourceResolver::new(base, crate::ExternalFilePolicy::Allow));
    compress_gltf_bytes_impl(
        input,
        resolver
            .as_ref()
            .map(|resolver| resolver as &dyn ResourceResolver),
        &ResourceLimits::default(),
        options,
    )
}

/// Compress using a caller-provided resource resolver and quotas.
#[cfg(feature = "gltf-reader")]
pub fn compress_gltf_bytes_with_resolver(
    input: &[u8],
    resolver: &dyn ResourceResolver,
    limits: &ResourceLimits,
    options: &GltfCompressionOptions,
) -> Result<CompressionOutput<Vec<u8>>> {
    compress_gltf_bytes_impl(input, Some(resolver), limits, options)
}

#[cfg(feature = "gltf-reader")]
fn compress_gltf_bytes_impl(
    input: &[u8],
    resolver: Option<&dyn ResourceResolver>,
    limits: &ResourceLimits,
    options: &GltfCompressionOptions,
) -> Result<CompressionOutput<Vec<u8>>> {
    options.validate()?;
    let container = parse_gltf_container(input)?;
    let doc = serde_json::from_slice::<Value>(container.json)?;

    // Reuse the reader (lenient: do not reject skins/animations/morph targets,
    // we only preserve them) for geometry decoding and resolved buffer bytes.
    let reader = if let Some(resolver) = resolver {
        GltfReader::from_bytes_lenient_with_resolver(input, resolver, limits)?
    } else {
        GltfReader::from_bytes_lenient(input)?
    };
    let compressed = compress_gltf_value(doc, reader.buffers(), options, |mesh, prim| {
        reader.decode_primitive_with_semantics(mesh, prim)
    })?;
    let (doc, bin) = compressed.data;
    let data = serialize_gltf_document(&doc, &bin, container.format, options.output_format)?;
    Ok(CompressionOutput {
        data,
        report: compressed.report,
    })
}

/// The document-preserving compression core: transforms a parsed glTF document
/// (`doc`) in place, returning the mutated document plus the new single binary
/// blob. Geometry decoding is supplied by `decode`, so callers that already have
/// a parsed scene (e.g. a `gltf-rs` document) can compress without re-parsing
/// through the byte API.
///
/// `decode(mesh_index, primitive_index)` returns the primitive's geometry as a
/// [`draco_core::Mesh`] plus the `(glTF semantic, Draco attribute id)` mapping;
/// an `Err` marks the primitive as not compressible (it is preserved). `buffers`
/// holds the resolved bytes for each glTF buffer (used for repacking the
/// non-compressed buffer views).
///
/// The returned document's single buffer carries `byteLength` but no URI; the
/// caller embeds the returned `bin` (as a GLB BIN chunk or a data URI).
pub fn compress_gltf_value<F>(
    mut doc: Value,
    buffers: &[Vec<u8>],
    options: &GltfCompressionOptions,
    decode: F,
) -> Result<CompressionOutput<(Value, Vec<u8>)>>
where
    F: Fn(usize, usize) -> Result<(draco_core::Mesh, Vec<(String, u32)>)>,
{
    if !doc.is_object() {
        return Err(GltfError::InvalidGltf("glTF root is not an object".into()));
    }
    options.validate()?;
    validate_gltf_document_for_repacking(&doc, buffers)?;

    // Reference-count accessor usage across every primitive so we only mutate
    // accessors that belong exclusively to a single primitive we compress.
    let accessor_users = count_accessor_users(&doc)?;

    let (plans, mut report) = build_plans(&doc, buffers, &decode, &accessor_users, options)?;

    // Mutate accessors of compressed primitives: drop their buffer view and set
    // the count to the Draco-encoded value. Done before scanning for orphans so
    // the now-unreferenced geometry buffer views fall out naturally.
    apply_accessor_mutations(&mut doc, &plans)?;

    // Non-indexed primitives need a generated indices accessor (Draco glTF
    // primitives are indexed).
    add_generated_indices(&mut doc, &plans)?;

    // Repack the binary: keep only buffer views still referenced by the JSON,
    // append one Draco buffer view per compressed primitive, and reindex every
    // buffer-view reference in the document.
    let repack = repack_buffers(&mut doc, buffers, &plans)?;

    // Write the Draco extension onto each compressed primitive (after reindex,
    // so the freshly appended buffer-view indices are not remapped).
    for (i, plan) in plans.iter().enumerate() {
        let draco_bv = repack.draco_buffer_views[i];
        set_primitive_draco_extension(&mut doc, plan, draco_bv)?;
    }

    if !plans.is_empty() {
        ensure_extension_listed(&mut doc, "extensionsUsed")?;
        ensure_extension_listed(&mut doc, "extensionsRequired")?;
    }

    set_single_buffer(&mut doc, repack.bin.len())?;

    report.compressed_primitives = plans
        .iter()
        .map(|plan| PrimitiveLocation {
            mesh: plan.mesh_idx,
            primitive: plan.prim_idx,
        })
        .collect();
    Ok(CompressionOutput {
        data: (doc, repack.bin),
        report,
    })
}

/// Consolidate resolved glTF buffers using the same known-reference and opaque
/// extension policy as the compressor.
pub fn consolidate_gltf_buffers(
    mut document: Value,
    buffers: &[Vec<u8>],
) -> Result<(Value, Vec<u8>)> {
    if !document.is_object() {
        return Err(GltfError::InvalidGltf("glTF root is not an object".into()));
    }
    validate_gltf_document_for_repacking(&document, buffers)?;
    let repack = repack_buffers(&mut document, buffers, &[])?;
    set_single_buffer(&mut document, repack.bin.len())?;
    Ok((document, repack.bin))
}

#[derive(Clone, Copy)]
struct ValidatedBufferView {
    buffer: usize,
    byte_offset: usize,
    byte_length: usize,
    byte_stride: Option<usize>,
}

/// Validate binary declarations and every accessor reference before deciding
/// whether a primitive is compressible. Preserve reasons are only for valid
/// but unsupported data; malformed sparse/morph/shared geometry must never be
/// converted into a successful report entry.
/// Validate a parsed document before any binary consolidation or compression.
///
/// This combines strict KHR Draco validation, conservative opaque-reference
/// rejection, accessor/reference validation, and checked buffer-view bounds.
pub fn validate_gltf_document_for_repacking(document: &Value, buffers: &[Vec<u8>]) -> Result<()> {
    validate_khr_draco_document(document)?;
    reject_opaque_binary_references(document)?;
    validate_gltf_document_binary_layout(document, buffers)
}

/// Validate all glTF binary declarations and accessor references.
pub fn validate_gltf_document_binary_layout(document: &Value, buffers: &[Vec<u8>]) -> Result<()> {
    let declared_buffers = optional_array(document, "buffers")?;
    if declared_buffers.len() != buffers.len() {
        return Err(GltfError::InvalidGltf(format!(
            "document declares {} buffers but {} were resolved",
            declared_buffers.len(),
            buffers.len()
        )));
    }
    for (index, declaration) in declared_buffers.iter().enumerate() {
        let declaration = declaration
            .as_object()
            .ok_or_else(|| GltfError::InvalidGltf(format!("buffer {index} is not an object")))?;
        let byte_length = required_usize(declaration, "byteLength", "buffer")?;
        let actual = buffers[index].len();
        if actual < byte_length {
            return Err(GltfError::InvalidGltf(format!(
                "buffer {index} byteLength {byte_length} exceeds resolved length {actual}"
            )));
        }
    }

    let view_values = optional_array(document, "bufferViews")?;
    let mut views = Vec::new();
    views.try_reserve_exact(view_values.len()).map_err(|_| {
        GltfError::ResourceLimitExceeded("bufferView validation allocation failed".into())
    })?;
    for (index, value) in view_values.iter().enumerate() {
        let view = value.as_object().ok_or_else(|| {
            GltfError::InvalidGltf(format!("bufferView {index} is not an object"))
        })?;
        let buffer = required_usize(view, "buffer", "bufferView")?;
        let byte_offset = optional_usize(view, "byteOffset", "bufferView")?.unwrap_or(0);
        let byte_length = required_usize(view, "byteLength", "bufferView")?;
        let byte_stride = optional_usize(view, "byteStride", "bufferView")?;
        if let Some(stride) = byte_stride {
            if !(4..=252).contains(&stride) || !stride.is_multiple_of(4) {
                return Err(GltfError::InvalidGltf(format!(
                    "bufferView {index} byteStride must be a multiple of 4 in 4..=252"
                )));
            }
        }
        let buffer_data = buffers.get(buffer).ok_or_else(|| {
            GltfError::InvalidGltf(format!(
                "bufferView {index} references invalid buffer {buffer}"
            ))
        })?;
        let end = byte_offset
            .checked_add(byte_length)
            .filter(|end| *end <= buffer_data.len())
            .ok_or_else(|| GltfError::InvalidGltf(format!("bufferView {index} is out of range")))?;
        let _ = end;
        views.push(ValidatedBufferView {
            buffer,
            byte_offset,
            byte_length,
            byte_stride,
        });
    }

    let accessor_values = optional_array(document, "accessors")?;
    for (index, value) in accessor_values.iter().enumerate() {
        validate_accessor(index, value, &views, buffers)?;
    }
    validate_primitive_accessor_contracts(document, accessor_values, &views, buffers)?;
    validate_accessor_references(document, accessor_values.len())
}

fn optional_array<'a>(document: &'a Value, key: &str) -> Result<&'a [Value]> {
    match document.get(key) {
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| GltfError::InvalidGltf(format!("{key} is not an array"))),
        None => Ok(&[]),
    }
}

fn required_usize(object: &Map<String, Value>, key: &str, label: &str) -> Result<usize> {
    let value = object
        .get(key)
        .ok_or_else(|| GltfError::InvalidGltf(format!("{label} is missing {key}")))?;
    json_index(value, &format!("{label}.{key}"))
}

fn optional_usize(object: &Map<String, Value>, key: &str, label: &str) -> Result<Option<usize>> {
    object
        .get(key)
        .map(|value| json_index(value, &format!("{label}.{key}")))
        .transpose()
}

fn component_size(component_type: u64, label: &str) -> Result<usize> {
    match component_type {
        5120 | 5121 => Ok(1),
        5122 | 5123 | 5131 => Ok(2),
        5124..=5126 => Ok(4),
        5130 | 5132 | 5133 => Ok(8),
        _ => Err(GltfError::InvalidGltf(format!(
            "{label} has invalid componentType {component_type}"
        ))),
    }
}

fn accessor_element_size(accessor_type: &str, component_size: usize) -> Result<usize> {
    let (columns, rows) = match accessor_type {
        "SCALAR" => (1usize, 1usize),
        "VEC2" => (1, 2),
        "VEC3" => (1, 3),
        "VEC4" => (1, 4),
        "MAT2" => (2, 2),
        "MAT3" => (3, 3),
        "MAT4" => (4, 4),
        _ => {
            return Err(GltfError::InvalidGltf(format!(
                "accessor has invalid type {accessor_type}"
            )));
        }
    };
    let column_size = rows
        .checked_mul(component_size)
        .ok_or_else(|| GltfError::InvalidGltf("accessor element size overflow".into()))?;
    let column_stride = if columns > 1 && component_size < 4 {
        column_size
            .checked_add(3)
            .map(|size| size / 4 * 4)
            .ok_or_else(|| GltfError::InvalidGltf("matrix element size overflow".into()))?
    } else {
        column_size
    };
    columns
        .checked_mul(column_stride)
        .ok_or_else(|| GltfError::InvalidGltf("accessor element size overflow".into()))
}

fn validate_range_in_view(
    view: ValidatedBufferView,
    byte_offset: usize,
    count: usize,
    element_size: usize,
    stride: usize,
    label: &str,
) -> Result<()> {
    if stride < element_size {
        return Err(GltfError::InvalidGltf(format!(
            "{label} stride {stride} is smaller than element size {element_size}"
        )));
    }
    let byte_length = count
        .checked_sub(1)
        .and_then(|prefix| prefix.checked_mul(stride))
        .and_then(|prefix| prefix.checked_add(element_size))
        .ok_or_else(|| GltfError::InvalidGltf(format!("{label} byte range overflow")))?;
    let end = byte_offset
        .checked_add(byte_length)
        .ok_or_else(|| GltfError::InvalidGltf(format!("{label} byte range overflow")))?;
    if end > view.byte_length {
        return Err(GltfError::InvalidGltf(format!(
            "{label} does not fit its bufferView"
        )));
    }
    Ok(())
}

fn validate_accessor(
    index: usize,
    value: &Value,
    views: &[ValidatedBufferView],
    buffers: &[Vec<u8>],
) -> Result<()> {
    let accessor = value
        .as_object()
        .ok_or_else(|| GltfError::InvalidGltf(format!("accessor {index} is not an object")))?;
    let component_type = accessor
        .get("componentType")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            GltfError::InvalidGltf(format!("accessor {index} has invalid componentType"))
        })?;
    let component_size = component_size(component_type, &format!("accessor {index}"))?;
    let accessor_type = accessor
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| GltfError::InvalidGltf(format!("accessor {index} has invalid type")))?;
    let element_size = accessor_element_size(accessor_type, component_size)?;
    let count = required_usize(accessor, "count", &format!("accessor {index}"))?;
    if count == 0 {
        return Err(GltfError::InvalidGltf(format!(
            "accessor {index} count must be greater than zero"
        )));
    }
    if accessor
        .get("normalized")
        .is_some_and(|normalized| !normalized.is_boolean())
    {
        return Err(GltfError::InvalidGltf(format!(
            "accessor {index}.normalized is not a boolean"
        )));
    }
    let byte_offset = optional_usize(accessor, "byteOffset", "accessor")?.unwrap_or(0);
    if !byte_offset.is_multiple_of(component_size) {
        return Err(GltfError::InvalidGltf(format!(
            "accessor {index} byteOffset is not component-aligned"
        )));
    }
    if let Some(view_index) = optional_usize(accessor, "bufferView", "accessor")? {
        let view = *views.get(view_index).ok_or_else(|| {
            GltfError::InvalidGltf(format!(
                "accessor {index} references invalid bufferView {view_index}"
            ))
        })?;
        let stride = view.byte_stride.unwrap_or(element_size);
        validate_range_in_view(
            view,
            byte_offset,
            count,
            element_size,
            stride,
            &format!("accessor {index}"),
        )?;
    } else if byte_offset != 0 {
        return Err(GltfError::InvalidGltf(format!(
            "accessor {index} has byteOffset without bufferView"
        )));
    }

    if let Some(sparse) = accessor.get("sparse") {
        validate_sparse_accessor(index, sparse, count, element_size, views, buffers)?;
    }
    Ok(())
}

fn validate_sparse_accessor(
    accessor_index: usize,
    sparse: &Value,
    accessor_count: usize,
    element_size: usize,
    views: &[ValidatedBufferView],
    buffers: &[Vec<u8>],
) -> Result<()> {
    let sparse = sparse.as_object().ok_or_else(|| {
        GltfError::InvalidGltf(format!("accessor {accessor_index}.sparse is not an object"))
    })?;
    let count = required_usize(sparse, "count", "sparse accessor")?;
    if count == 0 || count > accessor_count {
        return Err(GltfError::InvalidGltf(format!(
            "accessor {accessor_index} has invalid sparse count {count}"
        )));
    }

    let indices = sparse
        .get("indices")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            GltfError::InvalidGltf(format!(
                "accessor {accessor_index}.sparse.indices is not an object"
            ))
        })?;
    let indices_view_index = required_usize(indices, "bufferView", "sparse indices")?;
    let indices_view = *views.get(indices_view_index).ok_or_else(|| {
        GltfError::InvalidGltf(format!(
            "sparse indices references invalid bufferView {indices_view_index}"
        ))
    })?;
    if indices_view.byte_stride.is_some() {
        return Err(GltfError::InvalidGltf(
            "sparse indices bufferView must not define byteStride".into(),
        ));
    }
    let index_component = indices
        .get("componentType")
        .and_then(Value::as_u64)
        .ok_or_else(|| GltfError::InvalidGltf("sparse indices componentType is invalid".into()))?;
    let index_size = match index_component {
        5121 => 1,
        5123 => 2,
        5125 => 4,
        _ => {
            return Err(GltfError::InvalidGltf(format!(
                "sparse indices has invalid componentType {index_component}"
            )));
        }
    };
    let indices_offset = optional_usize(indices, "byteOffset", "sparse indices")?.unwrap_or(0);
    if !indices_offset.is_multiple_of(index_size) {
        return Err(GltfError::InvalidGltf(
            "sparse indices byteOffset is not component-aligned".into(),
        ));
    }
    validate_range_in_view(
        indices_view,
        indices_offset,
        count,
        index_size,
        index_size,
        "sparse indices",
    )?;
    let indices_buffer = buffers
        .get(indices_view.buffer)
        .ok_or_else(|| GltfError::InvalidGltf("sparse indices buffer is out of range".into()))?;
    let indices_start = indices_view
        .byte_offset
        .checked_add(indices_offset)
        .ok_or_else(|| GltfError::InvalidGltf("sparse indices offset overflow".into()))?;
    let mut previous = None;
    for sparse_index in 0..count {
        let offset = sparse_index
            .checked_mul(index_size)
            .and_then(|offset| indices_start.checked_add(offset))
            .ok_or_else(|| GltfError::InvalidGltf("sparse indices offset overflow".into()))?;
        let end = offset
            .checked_add(index_size)
            .ok_or_else(|| GltfError::InvalidGltf("sparse indices offset overflow".into()))?;
        let bytes = indices_buffer
            .get(offset..end)
            .ok_or_else(|| GltfError::InvalidGltf("sparse indices are out of range".into()))?;
        let value = match index_component {
            5121 => bytes[0] as u32,
            5123 => u16::from_le_bytes([bytes[0], bytes[1]]) as u32,
            5125 => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            _ => {
                return Err(GltfError::InvalidGltf(
                    "sparse indices component type changed after validation".into(),
                ));
            }
        } as usize;
        if value >= accessor_count {
            return Err(GltfError::InvalidGltf(format!(
                "accessor {accessor_index} sparse index {value} is out of range"
            )));
        }
        if previous.is_some_and(|previous| value <= previous) {
            return Err(GltfError::InvalidGltf(format!(
                "accessor {accessor_index} sparse indices are not strictly increasing"
            )));
        }
        previous = Some(value);
    }

    let values = sparse
        .get("values")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            GltfError::InvalidGltf(format!(
                "accessor {accessor_index}.sparse.values is not an object"
            ))
        })?;
    let values_view_index = required_usize(values, "bufferView", "sparse values")?;
    let values_view = *views.get(values_view_index).ok_or_else(|| {
        GltfError::InvalidGltf(format!(
            "sparse values references invalid bufferView {values_view_index}"
        ))
    })?;
    if values_view.byte_stride.is_some() {
        return Err(GltfError::InvalidGltf(
            "sparse values bufferView must not define byteStride".into(),
        ));
    }
    let values_offset = optional_usize(values, "byteOffset", "sparse values")?.unwrap_or(0);
    validate_range_in_view(
        values_view,
        values_offset,
        count,
        element_size,
        element_size,
        "sparse values",
    )
}

fn validate_primitive_accessor_contracts(
    document: &Value,
    accessors: &[Value],
    views: &[ValidatedBufferView],
    buffers: &[Vec<u8>],
) -> Result<()> {
    for (mesh_index, mesh) in optional_array(document, "meshes")?.iter().enumerate() {
        let primitives = mesh
            .get("primitives")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                GltfError::InvalidGltf(format!("mesh {mesh_index}.primitives is not an array"))
            })?;
        for (primitive_index, primitive) in primitives.iter().enumerate() {
            let primitive = primitive.as_object().ok_or_else(|| {
                GltfError::InvalidGltf(format!(
                    "primitive {mesh_index}:{primitive_index} is not an object"
                ))
            })?;
            let attributes = primitive
                .get("attributes")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    GltfError::InvalidGltf(format!(
                        "primitive {mesh_index}:{primitive_index}.attributes is not an object"
                    ))
                })?;
            let mut vertex_count = None;
            for (semantic, accessor_index) in attributes {
                let accessor_index = json_index(accessor_index, "primitive attribute accessor")?;
                let accessor = accessors
                    .get(accessor_index)
                    .and_then(Value::as_object)
                    .ok_or_else(|| {
                        GltfError::InvalidGltf(format!(
                            "primitive {mesh_index}:{primitive_index} accessor {accessor_index} is out of range"
                        ))
                    })?;
                let count = required_usize(accessor, "count", "primitive attribute accessor")?;
                if let Some(expected) = vertex_count {
                    if expected != count {
                        return Err(GltfError::InvalidGltf(format!(
                            "primitive {mesh_index}:{primitive_index} attribute counts do not match"
                        )));
                    }
                } else {
                    vertex_count = Some(count);
                }
                let component_type = accessor
                    .get("componentType")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| {
                        GltfError::InvalidGltf(format!(
                            "accessor {accessor_index} has invalid componentType"
                        ))
                    })?;
                // glTF 2.1 component types are outside the current codec scope
                // and are preserved later. Core glTF 2.0 layouts must satisfy
                // the semantic contract even when another preserve reason wins.
                if matches!(component_type, 5120 | 5121 | 5122 | 5123 | 5126) {
                    let accessor_type =
                        accessor
                            .get("type")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                GltfError::InvalidGltf(format!(
                                    "accessor {accessor_index} has invalid type"
                                ))
                            })?;
                    let normalized = match accessor.get("normalized") {
                        None => false,
                        Some(Value::Bool(normalized)) => *normalized,
                        Some(_) => {
                            return Err(GltfError::InvalidGltf(format!(
                                "accessor {accessor_index}.normalized is not a boolean"
                            )));
                        }
                    };
                    validate_semantic_accessor(
                        semantic,
                        accessor_type,
                        component_type,
                        normalized,
                    )?;
                }
            }
            let vertex_count = vertex_count.unwrap_or(0);
            let mode = primitive
                .get("mode")
                .and_then(Value::as_u64)
                .unwrap_or(MODE_TRIANGLES);
            let element_count = if let Some(indices) = primitive.get("indices") {
                let accessor_index = json_index(indices, "primitive indices accessor")?;
                let values =
                    read_unsigned_scalar_accessor(accessor_index, accessors, views, buffers)?;
                if values.iter().any(|&index| index as usize >= vertex_count) {
                    return Err(GltfError::InvalidGltf(format!(
                        "primitive {mesh_index}:{primitive_index} index is out of range for {vertex_count} vertices"
                    )));
                }
                values.len()
            } else {
                vertex_count
            };
            validate_primitive_element_count(mode, element_count, mesh_index, primitive_index)?;

            if let Some(targets) = primitive.get("targets") {
                let targets = targets.as_array().ok_or_else(|| {
                    GltfError::InvalidGltf("primitive.targets is not an array".into())
                })?;
                for (target_index, target) in targets.iter().enumerate() {
                    let target = target.as_object().ok_or_else(|| {
                        GltfError::InvalidGltf("morph target is not an object".into())
                    })?;
                    if target.is_empty() {
                        return Err(GltfError::InvalidGltf("morph target is empty".into()));
                    }
                    for (semantic, accessor_index) in target {
                        let accessor_index = json_index(accessor_index, "morph target accessor")?;
                        let accessor = accessors
                            .get(accessor_index)
                            .and_then(Value::as_object)
                            .ok_or_else(|| {
                                GltfError::InvalidGltf(format!(
                                    "morph target accessor {accessor_index} is out of range"
                                ))
                            })?;
                        let count = required_usize(accessor, "count", "morph target accessor")?;
                        if count != vertex_count
                            || !matches!(semantic.as_str(), "POSITION" | "NORMAL" | "TANGENT")
                            || accessor.get("type").and_then(Value::as_str) != Some("VEC3")
                            || accessor.get("componentType").and_then(Value::as_u64) != Some(5126)
                            || accessor
                                .get("normalized")
                                .is_some_and(|normalized| normalized != &Value::Bool(false))
                        {
                            return Err(GltfError::InvalidGltf(format!(
                                "primitive {mesh_index}:{primitive_index} morph target {target_index} {semantic} accessor has an invalid contract"
                            )));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_primitive_element_count(
    mode: u64,
    count: usize,
    mesh: usize,
    primitive: usize,
) -> Result<()> {
    let valid = match mode {
        0 => count >= 1,
        1 => count >= 2 && count.is_multiple_of(2),
        2 | 3 => count >= 2,
        4 => count >= 3 && count.is_multiple_of(3),
        5 | 6 => count >= 3,
        _ => false,
    };
    if !valid {
        return Err(GltfError::InvalidGltf(format!(
            "primitive {mesh}:{primitive} has invalid element count {count} for mode {mode}"
        )));
    }
    Ok(())
}

fn read_unsigned_scalar_accessor(
    accessor_index: usize,
    accessors: &[Value],
    views: &[ValidatedBufferView],
    buffers: &[Vec<u8>],
) -> Result<Vec<u32>> {
    let accessor = accessors
        .get(accessor_index)
        .and_then(Value::as_object)
        .ok_or_else(|| {
            GltfError::InvalidGltf(format!("accessor {accessor_index} is out of range"))
        })?;
    if accessor.get("type").and_then(Value::as_str) != Some("SCALAR")
        || accessor
            .get("normalized")
            .is_some_and(|normalized| normalized != &Value::Bool(false))
    {
        return Err(GltfError::InvalidGltf(format!(
            "indices accessor {accessor_index} has an invalid contract"
        )));
    }
    let component_type = accessor
        .get("componentType")
        .and_then(Value::as_u64)
        .ok_or_else(|| GltfError::InvalidGltf("indices componentType is invalid".into()))?;
    let value_size = match component_type {
        5121 => 1,
        5123 => 2,
        5125 => 4,
        _ => {
            return Err(GltfError::InvalidGltf(format!(
                "indices accessor {accessor_index} has invalid componentType {component_type}"
            )));
        }
    };
    let count = required_usize(accessor, "count", "indices accessor")?;
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| {
        GltfError::ResourceLimitExceeded("indices validation allocation failed".into())
    })?;
    values.resize(count, 0);
    if let Some(view_index) = optional_usize(accessor, "bufferView", "indices accessor")? {
        let view = *views.get(view_index).ok_or_else(|| {
            GltfError::InvalidGltf(format!("indices bufferView {view_index} is out of range"))
        })?;
        if view.byte_stride.is_some() {
            return Err(GltfError::InvalidGltf(
                "indices bufferView must not define byteStride".into(),
            ));
        }
        let byte_offset = optional_usize(accessor, "byteOffset", "indices accessor")?.unwrap_or(0);
        for (index, value) in values.iter_mut().enumerate() {
            *value = read_unsigned_from_view(
                view,
                byte_offset,
                index,
                value_size,
                component_type,
                buffers,
                "indices accessor",
            )?;
        }
    }
    if let Some(sparse) = accessor.get("sparse").and_then(Value::as_object) {
        let sparse_count = required_usize(sparse, "count", "sparse accessor")?;
        let sparse_indices = sparse
            .get("indices")
            .and_then(Value::as_object)
            .ok_or_else(|| GltfError::InvalidGltf("sparse indices are invalid".into()))?;
        let sparse_values = sparse
            .get("values")
            .and_then(Value::as_object)
            .ok_or_else(|| GltfError::InvalidGltf("sparse values are invalid".into()))?;
        let sparse_index_component = sparse_indices
            .get("componentType")
            .and_then(Value::as_u64)
            .ok_or_else(|| GltfError::InvalidGltf("sparse index type is invalid".into()))?;
        let sparse_index_size = component_size(sparse_index_component, "sparse indices")?;
        let index_view = *views
            .get(required_usize(
                sparse_indices,
                "bufferView",
                "sparse indices",
            )?)
            .ok_or_else(|| GltfError::InvalidGltf("sparse indices view is invalid".into()))?;
        let value_view = *views
            .get(required_usize(
                sparse_values,
                "bufferView",
                "sparse values",
            )?)
            .ok_or_else(|| GltfError::InvalidGltf("sparse values view is invalid".into()))?;
        let index_offset =
            optional_usize(sparse_indices, "byteOffset", "sparse indices")?.unwrap_or(0);
        let value_offset =
            optional_usize(sparse_values, "byteOffset", "sparse values")?.unwrap_or(0);
        for sparse_index in 0..sparse_count {
            let destination = read_unsigned_from_view(
                index_view,
                index_offset,
                sparse_index,
                sparse_index_size,
                sparse_index_component,
                buffers,
                "sparse indices",
            )? as usize;
            let value = read_unsigned_from_view(
                value_view,
                value_offset,
                sparse_index,
                value_size,
                component_type,
                buffers,
                "sparse values",
            )?;
            *values
                .get_mut(destination)
                .ok_or_else(|| GltfError::InvalidGltf("sparse index is out of range".into()))? =
                value;
        }
    }
    Ok(values)
}

fn read_unsigned_from_view(
    view: ValidatedBufferView,
    byte_offset: usize,
    index: usize,
    component_size: usize,
    component_type: u64,
    buffers: &[Vec<u8>],
    label: &str,
) -> Result<u32> {
    let start = index
        .checked_mul(component_size)
        .and_then(|offset| byte_offset.checked_add(offset))
        .and_then(|offset| view.byte_offset.checked_add(offset))
        .ok_or_else(|| GltfError::InvalidGltf(format!("{label} offset overflow")))?;
    let end = start
        .checked_add(component_size)
        .ok_or_else(|| GltfError::InvalidGltf(format!("{label} offset overflow")))?;
    let bytes = buffers
        .get(view.buffer)
        .and_then(|buffer| buffer.get(start..end))
        .ok_or_else(|| GltfError::InvalidGltf(format!("{label} is out of range")))?;
    match component_type {
        5121 => Ok(bytes[0] as u32),
        5123 => Ok(u16::from_le_bytes([bytes[0], bytes[1]]) as u32),
        5125 => Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        _ => Err(GltfError::InvalidGltf(format!(
            "{label} has invalid componentType {component_type}"
        ))),
    }
}

fn validate_accessor_reference(value: &Value, count: usize, label: &str) -> Result<()> {
    let index = json_index(value, label)?;
    if index >= count {
        return Err(GltfError::InvalidGltf(format!(
            "{label} {index} is out of range"
        )));
    }
    Ok(())
}

fn validate_accessor_references(document: &Value, accessor_count: usize) -> Result<()> {
    for (mesh_index, mesh) in optional_array(document, "meshes")?.iter().enumerate() {
        let mesh = mesh
            .as_object()
            .ok_or_else(|| GltfError::InvalidGltf(format!("mesh {mesh_index} is not an object")))?;
        let primitives = mesh
            .get("primitives")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                GltfError::InvalidGltf(format!("mesh {mesh_index}.primitives is not an array"))
            })?;
        if primitives.is_empty() {
            return Err(GltfError::InvalidGltf(format!(
                "mesh {mesh_index} has no primitives"
            )));
        }
        for (primitive_index, primitive) in primitives.iter().enumerate() {
            let primitive = primitive.as_object().ok_or_else(|| {
                GltfError::InvalidGltf(format!(
                    "primitive {mesh_index}:{primitive_index} is not an object"
                ))
            })?;
            let attributes = primitive
                .get("attributes")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    GltfError::InvalidGltf(format!(
                        "primitive {mesh_index}:{primitive_index}.attributes is not an object"
                    ))
                })?;
            if attributes.is_empty() {
                return Err(GltfError::InvalidGltf(format!(
                    "primitive {mesh_index}:{primitive_index}.attributes is empty"
                )));
            }
            for accessor in attributes.values() {
                validate_accessor_reference(accessor, accessor_count, "primitive attribute")?;
            }
            if let Some(indices) = primitive.get("indices") {
                validate_accessor_reference(indices, accessor_count, "primitive indices")?;
            }
            if let Some(mode) = primitive.get("mode") {
                let mode = mode.as_u64().ok_or_else(|| {
                    GltfError::InvalidGltf("primitive.mode is not an integer".into())
                })?;
                if mode > 6 {
                    return Err(GltfError::InvalidGltf(format!(
                        "primitive mode {mode} is outside the glTF enum"
                    )));
                }
            }
            if let Some(targets) = primitive.get("targets") {
                let targets = targets.as_array().ok_or_else(|| {
                    GltfError::InvalidGltf("primitive.targets is not an array".into())
                })?;
                for target in targets {
                    let target = target.as_object().ok_or_else(|| {
                        GltfError::InvalidGltf("morph target is not an object".into())
                    })?;
                    if target.is_empty() {
                        return Err(GltfError::InvalidGltf("morph target is empty".into()));
                    }
                    for accessor in target.values() {
                        validate_accessor_reference(
                            accessor,
                            accessor_count,
                            "morph target accessor",
                        )?;
                    }
                }
            }
        }
    }

    for animation in optional_array(document, "animations")? {
        if let Some(samplers) = animation.get("samplers") {
            let samplers = samplers.as_array().ok_or_else(|| {
                GltfError::InvalidGltf("animation.samplers is not an array".into())
            })?;
            for sampler in samplers {
                let sampler = sampler.as_object().ok_or_else(|| {
                    GltfError::InvalidGltf("animation sampler is not an object".into())
                })?;
                for key in ["input", "output"] {
                    let accessor = sampler.get(key).ok_or_else(|| {
                        GltfError::InvalidGltf(format!("animation sampler is missing {key}"))
                    })?;
                    validate_accessor_reference(accessor, accessor_count, "animation accessor")?;
                }
            }
        }
    }
    for skin in optional_array(document, "skins")? {
        if let Some(accessor) = skin.get("inverseBindMatrices") {
            validate_accessor_reference(accessor, accessor_count, "inverseBindMatrices")?;
        }
    }
    for node in optional_array(document, "nodes")? {
        if let Some(attributes) = node
            .get("extensions")
            .and_then(|extensions| extensions.get("EXT_mesh_gpu_instancing"))
            .and_then(|extension| extension.get("attributes"))
        {
            let attributes = attributes.as_object().ok_or_else(|| {
                GltfError::InvalidGltf("EXT_mesh_gpu_instancing.attributes is not an object".into())
            })?;
            for accessor in attributes.values() {
                validate_accessor_reference(accessor, accessor_count, "instancing accessor")?;
            }
        }
    }
    Ok(())
}

/// A primitive that will be compressed, with everything needed to rewrite it.
struct CompressPlan {
    mesh_idx: usize,
    prim_idx: usize,
    draco_bytes: Vec<u8>,
    /// `(glTF semantic, Draco attribute id)` for the extension's attribute map.
    semantic_to_id: Vec<(String, u32)>,
    /// Accessor index for each attribute.
    attribute_accessors: Vec<usize>,
    /// The source indices accessor, or `None` for a non-indexed primitive (a
    /// fresh indices accessor is generated, since Draco glTF primitives are
    /// always indexed).
    indices_accessor: Option<usize>,
    num_points: usize,
    num_indices: usize,
}

fn build_plans<F>(
    doc: &Value,
    buffers: &[Vec<u8>],
    decode: &F,
    accessor_users: &HashMap<usize, usize>,
    options: &GltfCompressionOptions,
) -> Result<(Vec<CompressPlan>, CompressionReport)>
where
    F: Fn(usize, usize) -> Result<(draco_core::Mesh, Vec<(String, u32)>)>,
{
    let mut plans = Vec::new();
    let mut report = CompressionReport::default();
    let Some(meshes) = doc.get("meshes").and_then(Value::as_array) else {
        return Ok((plans, report));
    };

    for (mesh_idx, mesh) in meshes.iter().enumerate() {
        let Some(primitives) = mesh.get("primitives").and_then(Value::as_array) else {
            continue;
        };
        for (prim_idx, prim) in primitives.iter().enumerate() {
            let location = PrimitiveLocation {
                mesh: mesh_idx,
                primitive: prim_idx,
            };
            match plan_for_primitive(
                doc,
                prim,
                buffers,
                location,
                decode,
                accessor_users,
                options,
            )? {
                PlanDecision::Compress(plan) => plans.push(plan),
                PlanDecision::Preserve(reason) => {
                    report.preserved_primitives.push(PreservedPrimitive {
                        primitive: location,
                        reason,
                    })
                }
            }
        }
    }
    Ok((plans, report))
}

enum PlanDecision {
    Compress(CompressPlan),
    Preserve(PreserveReason),
}

fn validate_existing_draco_primitive(
    document: &Value,
    primitive: &Value,
    buffers: &[Vec<u8>],
    mode: u32,
) -> Result<()> {
    let extension = parse_khr_draco_mesh_compression(document, primitive)?.ok_or_else(|| {
        GltfError::InvalidGltf("primitive has no KHR_draco_mesh_compression payload".into())
    })?;
    let views = document
        .get("bufferViews")
        .and_then(Value::as_array)
        .ok_or_else(|| GltfError::InvalidGltf("missing bufferViews array".into()))?;
    let view = views
        .get(extension.buffer_view)
        .and_then(Value::as_object)
        .ok_or_else(|| {
            GltfError::InvalidGltf(format!(
                "Draco bufferView {} is out of range",
                extension.buffer_view
            ))
        })?;
    let data = buffer_view_bytes(view, buffers)?;
    let mut decoder_buffer = DecoderBuffer::new(data);
    let mut mesh = Mesh::new();
    MeshDecoder::new()
        .decode(&mut decoder_buffer, &mut mesh)
        .map_err(GltfError::DracoDecode)?;

    let primitive_attributes = primitive
        .get("attributes")
        .and_then(Value::as_object)
        .ok_or_else(|| GltfError::InvalidGltf("primitive.attributes is not an object".into()))?;
    let accessors = document
        .get("accessors")
        .and_then(Value::as_array)
        .ok_or_else(|| GltfError::InvalidGltf("missing accessors array".into()))?;

    for (semantic, &unique_id) in &extension.attributes {
        let accessor_index = primitive_attributes
            .get(semantic)
            .ok_or_else(|| {
                GltfError::InvalidGltf(format!(
                    "Draco semantic {semantic} is absent from primitive.attributes"
                ))
            })
            .and_then(|value| json_index(value, "Draco attribute accessor"))?;
        let accessor = accessors
            .get(accessor_index)
            .and_then(Value::as_object)
            .ok_or_else(|| {
                GltfError::InvalidGltf(format!("accessor {accessor_index} is out of range"))
            })?;
        let accessor_type = accessor
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                GltfError::InvalidGltf(format!("accessor {accessor_index} has invalid type"))
            })?;
        let component_type = accessor
            .get("componentType")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                GltfError::InvalidGltf(format!(
                    "accessor {accessor_index} has invalid componentType"
                ))
            })?;
        let normalized = match accessor.get("normalized") {
            None => false,
            Some(Value::Bool(normalized)) => *normalized,
            Some(_) => {
                return Err(GltfError::InvalidGltf("normalized is not a boolean".into()));
            }
        };
        let semantic_spec =
            validate_semantic_accessor(semantic, accessor_type, component_type, normalized)?;
        let attribute = mesh.attribute_by_unique_id(unique_id).ok_or_else(|| {
            GltfError::InvalidGltf(format!(
                "Draco unique attribute id {unique_id} for {semantic} is absent"
            ))
        })?;
        if attribute.attribute_type() != semantic_spec.attribute_type
            || gltf_type_for_num_components(attribute.num_components())? != accessor_type
            || component_type_for_data_type(attribute.data_type())? != component_type
            || attribute.normalized() != normalized
        {
            return Err(GltfError::InvalidGltf(format!(
                "decoded Draco attribute {semantic} does not match accessor {accessor_index}"
            )));
        }
        let count = required_usize(accessor, "count", "Draco attribute accessor")?;
        if attribute.size() != count || count != mesh.num_points() {
            return Err(GltfError::InvalidGltf(format!(
                "decoded Draco attribute {semantic} count {} does not match accessor count {count}",
                attribute.size()
            )));
        }
    }

    if let Some(indices) = primitive.get("indices") {
        let accessor_index = json_index(indices, "Draco indices accessor")?;
        let accessor = accessors
            .get(accessor_index)
            .and_then(Value::as_object)
            .ok_or_else(|| {
                GltfError::InvalidGltf(format!("accessor {accessor_index} is out of range"))
            })?;
        if accessor.get("type").and_then(Value::as_str) != Some("SCALAR")
            || accessor
                .get("componentType")
                .and_then(Value::as_u64)
                .is_none_or(|component| !matches!(component, 5121 | 5123 | 5125))
            || accessor
                .get("normalized")
                .is_some_and(|normalized| normalized != &Value::Bool(false))
        {
            return Err(GltfError::InvalidGltf(
                "Draco indices accessor has an invalid contract".into(),
            ));
        }
        let expected_count = if mode == MODE_TRIANGLES as u32 {
            mesh.num_faces().checked_mul(3)
        } else {
            mesh.num_faces().checked_add(2)
        }
        .ok_or_else(|| GltfError::InvalidGltf("decoded Draco index count overflow".into()))?;
        let count = required_usize(accessor, "count", "Draco indices accessor")?;
        if count != expected_count {
            return Err(GltfError::InvalidGltf(format!(
                "Draco indices accessor count {count} does not match decoded count {expected_count}"
            )));
        }
    }
    Ok(())
}

fn plan_for_primitive<F>(
    doc: &Value,
    prim: &Value,
    buffers: &[Vec<u8>],
    location: PrimitiveLocation,
    decode: &F,
    accessor_users: &HashMap<usize, usize>,
    options: &GltfCompressionOptions,
) -> Result<PlanDecision>
where
    F: Fn(usize, usize) -> Result<(draco_core::Mesh, Vec<(String, u32)>)>,
{
    let mesh_idx = location.mesh;
    let prim_idx = location.primitive;
    // KHR_draco_mesh_compression restricts compressed primitives to TRIANGLES
    // or TRIANGLE_STRIP ("Restrictions on geometry type"), so point clouds
    // (POINTS) and line modes can never be Draco-compressed in glTF. We compress
    // the triangle list (mode 4 / default); everything else is preserved as-is.
    let mode = prim
        .get("mode")
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| GltfError::InvalidGltf("primitive.mode is not an integer".into()))
        })
        .transpose()?
        .unwrap_or(MODE_TRIANGLES);
    let mode_u32 = u32::try_from(mode)
        .map_err(|_| GltfError::InvalidGltf("primitive.mode exceeds u32".into()))?;
    // A repeated compression request must prove that an existing Draco payload
    // and all of its accessor contracts are valid before reporting it as
    // preserved. Schema-only validation would turn corrupt input into a
    // successful AlreadyDraco result.
    if prim
        .get("extensions")
        .and_then(|extensions| extensions.get(KHR_DRACO))
        .is_some()
    {
        validate_existing_draco_primitive(doc, prim, buffers, mode_u32)?;
        return Ok(PlanDecision::Preserve(PreserveReason::AlreadyDraco));
    }
    if mode != MODE_TRIANGLES && mode != MODE_TRIANGLE_STRIP {
        return Ok(PlanDecision::Preserve(PreserveReason::UnsupportedMode {
            mode: mode_u32,
        }));
    }
    if mode == MODE_TRIANGLE_STRIP {
        return Ok(PlanDecision::Preserve(PreserveReason::UnsupportedLayout {
            detail: "TRIANGLE_STRIP compression is not implemented".into(),
        }));
    }
    if let Some(targets) = prim.get("targets") {
        let targets = targets
            .as_array()
            .ok_or_else(|| GltfError::InvalidGltf("primitive.targets is not an array".into()))?;
        if !targets.is_empty() {
            return Ok(PlanDecision::Preserve(PreserveReason::MorphTargets));
        }
    }
    // Indexed and non-indexed triangle lists are both supported. Non-indexed
    // primitives get a freshly generated indices accessor below, since Draco
    // glTF primitives are always indexed.
    let indices_accessor = prim
        .get("indices")
        .map(|value| json_index(value, "indices accessor"))
        .transpose()?;

    // Collect attribute semantics + accessors; require a round-trippable set.
    let Some(attributes) = prim.get("attributes").and_then(Value::as_object) else {
        return Err(GltfError::InvalidGltf(
            "primitive.attributes must be a non-empty object".into(),
        ));
    };
    if attributes.is_empty() || !attributes.contains_key("POSITION") {
        return Ok(PlanDecision::Preserve(PreserveReason::UnsupportedLayout {
            detail: "primitive has no POSITION attribute".into(),
        }));
    }
    let mut attribute_accessors = Vec::new();
    for accessor in attributes.values() {
        attribute_accessors.push(json_index(accessor, "attribute accessor")?);
    }

    let accessors = doc
        .get("accessors")
        .and_then(Value::as_array)
        .ok_or_else(|| GltfError::InvalidGltf("missing accessors array".into()))?;
    for &accessor in attribute_accessors.iter().chain(indices_accessor.iter()) {
        let value = accessors
            .get(accessor)
            .and_then(Value::as_object)
            .ok_or_else(|| {
                GltfError::InvalidGltf(format!("accessor {accessor} is out of range"))
            })?;
        if value.contains_key("sparse") {
            return Ok(PlanDecision::Preserve(PreserveReason::SparseAccessor {
                accessor,
            }));
        }
    }

    // All geometry accessors must be used by exactly this primitive, so that
    // dropping their buffer view / changing their count cannot corrupt another
    // primitive that shares them.
    let exclusive = |acc: usize| accessor_users.get(&acc).copied().unwrap_or(0) == 1;
    if let Some(accessor) = indices_accessor.filter(|accessor| !exclusive(*accessor)) {
        return Ok(PlanDecision::Preserve(PreserveReason::SharedAccessor {
            accessor,
        }));
    }
    if let Some(accessor) = attribute_accessors
        .iter()
        .copied()
        .find(|accessor| !exclusive(*accessor))
    {
        return Ok(PlanDecision::Preserve(PreserveReason::SharedAccessor {
            accessor,
        }));
    }

    // Decode geometry with the original glTF semantic names. An unsupported
    // attribute/layout means "leave this primitive uncompressed".
    let (mesh, semantic_to_uid) = match decode(mesh_idx, prim_idx) {
        Ok(out) => out,
        Err(GltfError::Unsupported(detail)) => {
            return Ok(PlanDecision::Preserve(PreserveReason::UnsupportedLayout {
                detail,
            }))
        }
        Err(error) => return Err(error),
    };
    let (draco_bytes, info) = match encode_draco_mesh_with_info(&mesh, options) {
        Ok(out) => out,
        Err(crate::gltf_writer::GltfWriteError::Unsupported(detail)) => {
            return Ok(PlanDecision::Preserve(PreserveReason::UnsupportedLayout {
                detail,
            }))
        }
        Err(crate::gltf_writer::GltfWriteError::InvalidMesh(detail)) => {
            return Err(GltfError::InvalidGltf(detail))
        }
        Err(crate::gltf_writer::GltfWriteError::InvalidOptions(detail)) => {
            return Err(GltfError::InvalidOptions(detail))
        }
        Err(crate::gltf_writer::GltfWriteError::ResourceLimit(detail)) => {
            return Err(GltfError::ResourceLimitExceeded(detail))
        }
        Err(crate::gltf_writer::GltfWriteError::DracoEncode(source)) => {
            return Err(GltfError::DracoEncode(source))
        }
        Err(error) => {
            return Err(GltfError::InvalidGltf(format!(
                "Draco writer failed: {error}"
            )))
        }
    };

    // The decoded attribute set must match the source primitive exactly, so the
    // extension's attribute map is faithful (no dropped or renamed attribute).
    let produced: BTreeSet<&str> = semantic_to_uid.iter().map(|(s, _)| s.as_str()).collect();
    let original: BTreeSet<&str> = attributes.keys().map(String::as_str).collect();
    if produced != original {
        return Err(GltfError::InvalidGltf(
            "decoded attribute set does not match primitive.attributes".into(),
        ));
    }
    let semantic_to_id = semantic_to_uid;

    let num_indices = info
        .num_encoded_faces
        .checked_mul(3)
        .ok_or_else(|| GltfError::ResourceLimitExceeded("index count overflow".into()))?;

    Ok(PlanDecision::Compress(CompressPlan {
        mesh_idx,
        prim_idx,
        draco_bytes,
        semantic_to_id,
        attribute_accessors,
        indices_accessor,
        num_points: info.num_encoded_points,
        num_indices,
    }))
}

/// Counts, for each accessor index, how many primitives reference it (via
/// attributes or indices) across the whole document.
fn count_accessor_users(doc: &Value) -> Result<HashMap<usize, usize>> {
    let mut users: HashMap<usize, usize> = HashMap::new();
    let mut add = |value: &Value, label: &str| -> Result<()> {
        let accessor = json_index(value, label)?;
        let count = users.entry(accessor).or_default();
        *count = count
            .checked_add(1)
            .ok_or_else(|| GltfError::InvalidGltf("accessor use count overflow".into()))?;
        Ok(())
    };

    if let Some(meshes) = doc.get("meshes") {
        let meshes = meshes
            .as_array()
            .ok_or_else(|| GltfError::InvalidGltf("meshes is not an array".into()))?;
        for mesh in meshes {
            let primitives = mesh
                .get("primitives")
                .and_then(Value::as_array)
                .ok_or_else(|| GltfError::InvalidGltf("mesh.primitives is not an array".into()))?;
            for prim in primitives {
                let attrs = prim
                    .get("attributes")
                    .and_then(Value::as_object)
                    .ok_or_else(|| {
                        GltfError::InvalidGltf("primitive.attributes is not an object".into())
                    })?;
                for accessor in attrs.values() {
                    add(accessor, "primitive attribute accessor")?;
                }
                if let Some(accessor) = prim.get("indices") {
                    add(accessor, "primitive indices accessor")?;
                }
                if let Some(targets) = prim.get("targets") {
                    let targets = targets.as_array().ok_or_else(|| {
                        GltfError::InvalidGltf("primitive.targets is not an array".into())
                    })?;
                    for target in targets {
                        let target = target.as_object().ok_or_else(|| {
                            GltfError::InvalidGltf("morph target is not an object".into())
                        })?;
                        for accessor in target.values() {
                            add(accessor, "morph target accessor")?;
                        }
                    }
                }
            }
        }
    }

    if let Some(animations) = doc.get("animations") {
        for animation in animations
            .as_array()
            .ok_or_else(|| GltfError::InvalidGltf("animations is not an array".into()))?
        {
            if let Some(samplers) = animation.get("samplers") {
                for sampler in samplers.as_array().ok_or_else(|| {
                    GltfError::InvalidGltf("animation.samplers is not an array".into())
                })? {
                    let sampler = sampler.as_object().ok_or_else(|| {
                        GltfError::InvalidGltf("animation sampler is not an object".into())
                    })?;
                    for key in ["input", "output"] {
                        let accessor = sampler.get(key).ok_or_else(|| {
                            GltfError::InvalidGltf(format!("animation sampler is missing {key}"))
                        })?;
                        add(accessor, "animation sampler accessor")?;
                    }
                }
            }
        }
    }

    if let Some(skins) = doc.get("skins") {
        for skin in skins
            .as_array()
            .ok_or_else(|| GltfError::InvalidGltf("skins is not an array".into()))?
        {
            if let Some(accessor) = skin.get("inverseBindMatrices") {
                add(accessor, "skin inverseBindMatrices accessor")?;
            }
        }
    }

    if let Some(nodes) = doc.get("nodes") {
        for node in nodes
            .as_array()
            .ok_or_else(|| GltfError::InvalidGltf("nodes is not an array".into()))?
        {
            if let Some(attributes) = node
                .get("extensions")
                .and_then(|extensions| extensions.get("EXT_mesh_gpu_instancing"))
                .and_then(|extension| extension.get("attributes"))
            {
                let attributes = attributes.as_object().ok_or_else(|| {
                    GltfError::InvalidGltf(
                        "EXT_mesh_gpu_instancing.attributes is not an object".into(),
                    )
                })?;
                for accessor in attributes.values() {
                    add(accessor, "EXT_mesh_gpu_instancing accessor")?;
                }
            }
        }
    }

    Ok(users)
}

fn apply_accessor_mutations(doc: &mut Value, plans: &[CompressPlan]) -> Result<()> {
    let accessors = doc
        .get_mut("accessors")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| GltfError::InvalidGltf("missing accessors array".into()))?;

    for plan in plans {
        for &acc in &plan.attribute_accessors {
            strip_geometry_accessor(accessors, acc, plan.num_points)?;
        }
        if let Some(indices) = plan.indices_accessor {
            strip_geometry_accessor(accessors, indices, plan.num_indices)?;
        }
    }
    Ok(())
}

/// For each non-indexed plan, append a fresh `SCALAR`/`UNSIGNED_INT` indices
/// accessor (no buffer view — the indices live in the Draco stream) and point
/// the primitive at it. Appending keeps existing accessor indices stable, and
/// the new accessor has no buffer view so it does not affect buffer repacking.
fn add_generated_indices(doc: &mut Value, plans: &[CompressPlan]) -> Result<()> {
    for plan in plans {
        if plan.indices_accessor.is_some() {
            continue;
        }
        let accessors = doc
            .get_mut("accessors")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| GltfError::InvalidGltf("missing accessors array".into()))?;
        let new_idx = accessors.len();
        accessors.push(serde_json::json!({
            "componentType": 5125u64, // UNSIGNED_INT
            "count": plan.num_indices as u64,
            "type": "SCALAR",
        }));
        let prim = primitive_mut(doc, plan)?;
        prim.insert("indices".into(), Value::from(new_idx as u64));
    }
    Ok(())
}

/// Mutable access to a plan's primitive JSON object.
fn primitive_mut<'a>(
    doc: &'a mut Value,
    plan: &CompressPlan,
) -> Result<&'a mut Map<String, Value>> {
    doc.get_mut("meshes")
        .and_then(Value::as_array_mut)
        .and_then(|m| m.get_mut(plan.mesh_idx))
        .and_then(|m| m.get_mut("primitives"))
        .and_then(Value::as_array_mut)
        .and_then(|p| p.get_mut(plan.prim_idx))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| GltfError::InvalidGltf("primitive vanished during rewrite".into()))
}

/// Removes an accessor's buffer view (its data now lives in Draco) and sets the
/// count to the Draco-encoded element count. Other fields (type, componentType,
/// min/max, normalized) are preserved.
fn strip_geometry_accessor(accessors: &mut [Value], idx: usize, count: usize) -> Result<()> {
    let accessor = accessors
        .get_mut(idx)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| GltfError::InvalidGltf(format!("accessor {} out of range", idx)))?;
    accessor.remove("bufferView");
    accessor.remove("byteOffset");
    accessor.insert("count".into(), Value::from(count));
    Ok(())
}

struct Repack {
    bin: Vec<u8>,
    /// New buffer-view index for each plan's Draco stream, in plan order.
    draco_buffer_views: Vec<usize>,
}

fn repack_buffers(
    doc: &mut Value,
    source_buffers: &[Vec<u8>],
    plans: &[CompressPlan],
) -> Result<Repack> {
    // Which buffer views are still referenced anywhere in the JSON (accessors,
    // images, surviving Draco extensions, and any unknown extension)? Scanning
    // by key name covers known and unknown referrers uniformly.
    let mut referenced = BTreeSet::new();
    collect_buffer_view_refs(doc, &mut referenced)?;

    // Move the old table out of the document. Kept entries are then moved into
    // the new table rather than deep-cloning arbitrary extension JSON.
    let old_views = doc
        .as_object_mut()
        .ok_or_else(|| GltfError::InvalidGltf("glTF root is not an object".into()))?
        .remove("bufferViews");
    let mut old_views = match old_views {
        Some(Value::Array(views)) => views,
        Some(_) => {
            return Err(GltfError::InvalidGltf("bufferViews is not an array".into()));
        }
        None => Vec::new(),
    };

    // Build the new binary: kept views (remapped), then one view per Draco blob.
    let mut bin: Vec<u8> = Vec::new();
    let mut new_views: Vec<Value> = Vec::new();
    let new_view_count = referenced
        .len()
        .checked_add(plans.len())
        .ok_or_else(|| GltfError::ResourceLimitExceeded("bufferView table size overflow".into()))?;
    new_views.try_reserve_exact(new_view_count).map_err(|_| {
        GltfError::ResourceLimitExceeded("bufferView table allocation failed".into())
    })?;
    let mut remap: HashMap<usize, usize> = HashMap::new();
    remap.try_reserve(referenced.len()).map_err(|_| {
        GltfError::ResourceLimitExceeded("bufferView remap allocation failed".into())
    })?;

    for &old_idx in &referenced {
        let view = old_views
            .get_mut(old_idx)
            .ok_or_else(|| GltfError::InvalidGltf(format!("buffer view {old_idx} invalid")))?;
        let mut new_view = match std::mem::take(view) {
            Value::Object(view) => view,
            _ => {
                return Err(GltfError::InvalidGltf(format!(
                    "buffer view {old_idx} is not an object"
                )));
            }
        };
        let bytes = buffer_view_bytes(&new_view, source_buffers)?;
        let byte_length = bytes.len();
        align_to_4(&mut bin)?;
        let offset = bin.len();
        append_bytes(&mut bin, bytes, "buffer view")?;

        new_view.insert("buffer".into(), Value::from(0u64));
        new_view.insert("byteOffset".into(), Value::from(offset as u64));
        new_view.insert("byteLength".into(), Value::from(byte_length as u64));
        let new_idx = new_views.len();
        new_views.push(Value::Object(new_view));
        remap.insert(old_idx, new_idx);
    }

    // Reindex every buffer-view reference in the document to the kept set.
    remap_buffer_view_refs(doc, &remap)?;

    // Append the Draco buffer views (not present in the JSON yet, so they are
    // intentionally added after the remap pass).
    let mut draco_buffer_views = Vec::new();
    draco_buffer_views
        .try_reserve_exact(plans.len())
        .map_err(|_| {
            GltfError::ResourceLimitExceeded("Draco bufferView table allocation failed".into())
        })?;
    for plan in plans {
        align_to_4(&mut bin)?;
        let offset = bin.len();
        append_bytes(&mut bin, &plan.draco_bytes, "Draco bitstream")?;
        let new_idx = new_views.len();
        new_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": offset as u64,
            "byteLength": plan.draco_bytes.len() as u64,
        }));
        draco_buffer_views.push(new_idx);
    }

    let root = doc
        .as_object_mut()
        .ok_or_else(|| GltfError::InvalidGltf("glTF root is not an object".into()))?;
    if new_views.is_empty() {
        root.remove("bufferViews");
    } else {
        root.insert("bufferViews".into(), Value::Array(new_views));
    }

    Ok(Repack {
        bin,
        draco_buffer_views,
    })
}

fn buffer_view_bytes<'a>(view: &Map<String, Value>, buffers: &'a [Vec<u8>]) -> Result<&'a [u8]> {
    let buffer_idx: usize = view
        .get("buffer")
        .and_then(Value::as_u64)
        .ok_or_else(|| GltfError::InvalidGltf("buffer view missing buffer index".into()))?
        .try_into()
        .map_err(|_| GltfError::InvalidGltf("buffer index cannot fit usize".into()))?;
    let offset_u64 = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0);
    let offset = usize::try_from(offset_u64)
        .map_err(|_| GltfError::InvalidGltf("buffer view offset cannot fit usize".into()))?;
    let length: usize = view
        .get("byteLength")
        .and_then(Value::as_u64)
        .ok_or_else(|| GltfError::InvalidGltf("buffer view missing byteLength".into()))?
        .try_into()
        .map_err(|_| GltfError::InvalidGltf("buffer view length cannot fit usize".into()))?;
    let buffer = buffers
        .get(buffer_idx)
        .ok_or_else(|| GltfError::InvalidGltf(format!("buffer {} not resolved", buffer_idx)))?;
    let end = offset
        .checked_add(length)
        .filter(|&e| e <= buffer.len())
        .ok_or_else(|| GltfError::InvalidGltf("buffer view out of range".into()))?;
    Ok(&buffer[offset..end])
}

/// Recursively collect every integer found under a `"bufferView"` key.
fn collect_buffer_view_refs(value: &Value, out: &mut BTreeSet<usize>) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "extras" {
                    continue;
                }
                if key == "extensions" {
                    if let Some(extensions) = child.as_object() {
                        if let Some(draco) = extensions.get(KHR_DRACO) {
                            collect_buffer_view_refs(draco, out)?;
                        }
                        if let Some(metadata) = extensions.get("EXT_structural_metadata") {
                            collect_structural_metadata_refs(metadata, out)?;
                        }
                    }
                    continue;
                }
                if key == "bufferView" {
                    out.insert(json_index(child, "bufferView")?);
                }
                collect_buffer_view_refs(child, out)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_buffer_view_refs(item, out)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Recursively remap every integer under a `"bufferView"` key using `remap`.
fn remap_buffer_view_refs(value: &mut Value, remap: &HashMap<usize, usize>) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "extras" {
                    continue;
                }
                if key == "extensions" {
                    if let Some(extensions) = child.as_object_mut() {
                        if let Some(draco) = extensions.get_mut(KHR_DRACO) {
                            remap_buffer_view_refs(draco, remap)?;
                        }
                        if let Some(metadata) = extensions.get_mut("EXT_structural_metadata") {
                            remap_structural_metadata_refs(metadata, remap)?;
                        }
                    }
                    continue;
                }
                if key == "bufferView" {
                    let old = json_index(child, "bufferView")?;
                    let new = remap.get(&old).ok_or_else(|| {
                        GltfError::InvalidGltf(format!("bufferView {old} has no repack mapping"))
                    })?;
                    *child = Value::from(*new as u64);
                }
                remap_buffer_view_refs(child, remap)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                remap_buffer_view_refs(item, remap)?;
            }
        }
        _ => {}
    }
    Ok(())
}

const STRUCTURAL_METADATA_BUFFER_VIEW_KEYS: &[&str] = &["values", "arrayOffsets", "stringOffsets"];

fn structural_metadata_properties(value: &Value) -> Result<Vec<&Map<String, Value>>> {
    let Some(tables) = value.get("propertyTables") else {
        return Ok(Vec::new());
    };
    let tables = tables.as_array().ok_or_else(|| {
        GltfError::InvalidGltf("EXT_structural_metadata.propertyTables is not an array".into())
    })?;
    let mut properties = Vec::new();
    for table in tables {
        let Some(table_properties) = table.get("properties") else {
            continue;
        };
        let table_properties = table_properties.as_object().ok_or_else(|| {
            GltfError::InvalidGltf(
                "EXT_structural_metadata property table properties is not an object".into(),
            )
        })?;
        for property in table_properties.values() {
            properties.push(property.as_object().ok_or_else(|| {
                GltfError::InvalidGltf("EXT_structural_metadata property is not an object".into())
            })?);
        }
    }
    Ok(properties)
}

fn collect_structural_metadata_refs(metadata: &Value, out: &mut BTreeSet<usize>) -> Result<()> {
    for property in structural_metadata_properties(metadata)? {
        for key in STRUCTURAL_METADATA_BUFFER_VIEW_KEYS {
            if let Some(value) = property.get(*key) {
                out.insert(json_index(value, "EXT_structural_metadata bufferView")?);
            }
        }
    }
    Ok(())
}

fn remap_structural_metadata_refs(
    metadata: &mut Value,
    remap: &HashMap<usize, usize>,
) -> Result<()> {
    let Some(tables) = metadata.get_mut("propertyTables") else {
        return Ok(());
    };
    let tables = tables.as_array_mut().ok_or_else(|| {
        GltfError::InvalidGltf("EXT_structural_metadata.propertyTables is not an array".into())
    })?;
    for table in tables {
        let Some(properties) = table.get_mut("properties") else {
            continue;
        };
        let properties = properties.as_object_mut().ok_or_else(|| {
            GltfError::InvalidGltf(
                "EXT_structural_metadata property table properties is not an object".into(),
            )
        })?;
        for property in properties.values_mut() {
            let property = property.as_object_mut().ok_or_else(|| {
                GltfError::InvalidGltf("EXT_structural_metadata property is not an object".into())
            })?;
            for key in STRUCTURAL_METADATA_BUFFER_VIEW_KEYS {
                if let Some(value) = property.get_mut(*key) {
                    let old = json_index(value, "EXT_structural_metadata bufferView")?;
                    let new = remap.get(&old).ok_or_else(|| {
                        GltfError::InvalidGltf(format!(
                            "EXT_structural_metadata bufferView {old} has no repack mapping"
                        ))
                    })?;
                    *value = Value::from(*new as u64);
                }
            }
        }
    }
    Ok(())
}

fn known_non_binary_extension(name: &str) -> bool {
    name == "KHR_texture_transform"
        || name == "EXT_mesh_features"
        || name == "EXT_mesh_gpu_instancing"
        || name == "KHR_lights_punctual"
        || name == "KHR_animation_pointer"
        || matches!(
            name,
            "KHR_materials_anisotropy"
                | "KHR_materials_clearcoat"
                | "KHR_materials_diffuse_transmission"
                | "KHR_materials_dispersion"
                | "KHR_materials_emissive_strength"
                | "KHR_materials_ior"
                | "KHR_materials_iridescence"
                | "KHR_materials_pbrSpecularGlossiness"
                | "KHR_materials_sheen"
                | "KHR_materials_specular"
                | "KHR_materials_transmission"
                | "KHR_materials_unlit"
                | "KHR_materials_variants"
                | "KHR_materials_volume"
                | "KHR_texture_basisu"
                | "EXT_texture_webp"
                | "EXT_texture_avif"
                | "MSFT_lod"
        )
}

fn reject_opaque_binary_references(document: &Value) -> Result<()> {
    fn scan(value: &Value, path: &str) -> Result<()> {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    let normalized: String = key
                        .chars()
                        .filter(|character| character.is_ascii_alphanumeric())
                        .flat_map(char::to_lowercase)
                        .collect();
                    let looks_binary = normalized.contains("buffer")
                        || normalized.contains("offset")
                        || normalized.contains("stride")
                        || (normalized.contains("byte") && normalized.contains("length"));
                    if looks_binary {
                        return Err(GltfError::OpaqueBinaryReference(format!("{path}.{key}")));
                    }
                    scan(child, &format!("{path}.{key}"))?;
                }
            }
            Value::Array(values) => {
                for (index, child) in values.iter().enumerate() {
                    scan(child, &format!("{path}[{index}]"))?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn walk(value: &Value, path: &str) -> Result<()> {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    if key == "extras" {
                        continue;
                    }
                    if key == "extensions" {
                        let extensions = child.as_object().ok_or_else(|| {
                            GltfError::InvalidGltf(format!("{path}.extensions is not an object"))
                        })?;
                        for (name, extension) in extensions {
                            let extension_path = format!("{path}.extensions.{name}");
                            if name == KHR_DRACO
                                || name == "EXT_structural_metadata"
                                || known_non_binary_extension(name)
                            {
                                // The extension's own binary layout is known (or
                                // known not to contain binary references), but
                                // nested extension objects are independent and
                                // must still be classified recursively.
                                walk(extension, &extension_path)?;
                            } else {
                                scan(extension, &extension_path)?;
                            }
                        }
                        continue;
                    }
                    walk(child, &format!("{path}.{key}"))?;
                }
            }
            Value::Array(values) => {
                for (index, child) in values.iter().enumerate() {
                    walk(child, &format!("{path}[{index}]"))?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    walk(document, "$")
}

fn set_primitive_draco_extension(
    doc: &mut Value,
    plan: &CompressPlan,
    draco_buffer_view: usize,
) -> Result<()> {
    let prim = primitive_mut(doc, plan)?;

    let mut attributes = Map::new();
    for (semantic, id) in &plan.semantic_to_id {
        attributes.insert(semantic.clone(), Value::from(*id as u64));
    }
    let draco = serde_json::json!({
        "bufferView": draco_buffer_view as u64,
        "attributes": Value::Object(attributes),
    });

    let extensions = prim
        .entry("extensions")
        .or_insert_with(|| Value::Object(Map::new()));
    if !extensions.is_object() {
        return Err(GltfError::InvalidGltf(
            "primitive.extensions is not an object".into(),
        ));
    }
    let extensions = extensions
        .as_object_mut()
        .ok_or_else(|| GltfError::InvalidGltf("primitive.extensions is not an object".into()))?;
    extensions.insert(KHR_DRACO.into(), draco);
    Ok(())
}

/// Adds `KHR_draco_mesh_compression` to a root string array (creating it if
/// absent) without duplicating it.
fn ensure_extension_listed(doc: &mut Value, key: &str) -> Result<()> {
    let root = doc
        .as_object_mut()
        .ok_or_else(|| GltfError::InvalidGltf("glTF root is not an object".into()))?;
    let list = root.entry(key).or_insert_with(|| Value::Array(Vec::new()));
    let arr = list
        .as_array_mut()
        .ok_or_else(|| GltfError::InvalidGltf(format!("{key} is not an array")))?;
    if !arr.iter().any(|v| v.as_str() == Some(KHR_DRACO)) {
        arr.push(Value::from(KHR_DRACO));
    }
    Ok(())
}

/// Collapses the document to a single buffer of `bin_len` bytes (carrying
/// `byteLength` but no URI). The caller embeds the bytes: `serialize` fills a
/// data URI for glTF output, or writes a GLB BIN chunk.
fn set_single_buffer(doc: &mut Value, bin_len: usize) -> Result<()> {
    let root = doc
        .as_object_mut()
        .ok_or_else(|| GltfError::InvalidGltf("glTF root is not an object".into()))?;
    if bin_len == 0 {
        root.remove("buffers");
        return Ok(());
    }
    let mut buffer = Map::new();
    let bin_len = u64::try_from(bin_len)
        .map_err(|_| GltfError::ResourceLimitExceeded("buffer exceeds u64".into()))?;
    buffer.insert("byteLength".into(), Value::from(bin_len));
    root.insert("buffers".into(), Value::Array(vec![Value::Object(buffer)]));
    Ok(())
}

fn align_to_4(buf: &mut Vec<u8>) -> Result<()> {
    let padding = (4 - buf.len() % 4) % 4;
    let new_len = buf
        .len()
        .checked_add(padding)
        .ok_or_else(|| GltfError::ResourceLimitExceeded("alignment size overflow".into()))?;
    buf.try_reserve(padding)
        .map_err(|_| GltfError::ResourceLimitExceeded("alignment allocation failed".into()))?;
    buf.resize(new_len, 0);
    Ok(())
}

fn append_bytes(buf: &mut Vec<u8>, bytes: &[u8], label: &str) -> Result<()> {
    buf.len()
        .checked_add(bytes.len())
        .ok_or_else(|| GltfError::ResourceLimitExceeded(format!("{label} size overflow")))?;
    buf.try_reserve(bytes.len())
        .map_err(|_| GltfError::ResourceLimitExceeded(format!("{label} allocation failed")))?;
    buf.extend_from_slice(bytes);
    Ok(())
}
