//! Shared glTF/GLB container and resource handling.

use std::fs::{self, File};
use std::io::Read;
#[cfg(feature = "gltf-writer")]
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(any(feature = "gltf-writer", test))]
use serde_json::Value;

use crate::gltf_geometry::{GltfError, Result};

const GLB_MAGIC: u32 = 0x4654_6c67;
const GLB_VERSION: u32 = 2;
const GLB_CHUNK_JSON: u32 = 0x4e4f_534a;
const GLB_CHUNK_BIN: u32 = 0x004e_4942;

/// Container used by an input glTF document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GltfContainerFormat {
    /// JSON glTF document.
    Gltf,
    /// Binary GLB 2.0 document.
    Glb,
}

/// Borrowed, strictly parsed glTF container.
#[derive(Clone, Copy, Debug)]
pub struct GltfContainer<'a> {
    /// Input container kind.
    pub format: GltfContainerFormat,
    /// JSON document bytes (including legal GLB JSON padding).
    pub json: &'a [u8],
    /// Optional GLB BIN chunk.
    pub bin: Option<&'a [u8]>,
}

/// Output selection shared by the native and WASM glTF APIs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputFormat {
    /// Preserve the input container kind.
    #[default]
    SameAsInput,
    /// Emit JSON glTF with its buffer embedded as a data URI.
    GltfEmbeddedBuffers,
    /// Emit a binary GLB 2.0 container.
    Glb,
}

/// Optional resource quotas. `None` means unlimited.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Maximum decoded size of any one resource.
    pub max_resource_bytes: Option<usize>,
    /// Maximum decoded size of all glTF buffers combined.
    pub max_total_buffer_bytes: Option<usize>,
    /// Maximum decoded image pixel count. Image decoders enforce this limit.
    pub max_image_pixels: Option<u64>,
}

/// One glTF `buffers[]` declaration, independent of a JSON front end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GltfBufferReference<'a> {
    /// Optional data or companion-resource URI.
    pub uri: Option<&'a str>,
    /// Declared `byteLength` of the logical buffer, excluding GLB padding.
    pub byte_length: usize,
}

/// Synchronous resolver for non-data resource URIs.
pub trait ResourceResolver {
    /// Resolve `uri` to its exact bytes.
    fn resolve(&self, uri: &str) -> Result<Vec<u8>>;

    /// Resolve with a decoded-byte limit when the implementation can preflight
    /// it. The default preserves compatibility for custom resolvers and checks
    /// their returned bytes; filesystem resolvers override this to check file
    /// metadata before allocating.
    fn resolve_with_limit(&self, uri: &str, max_bytes: Option<usize>) -> Result<Vec<u8>> {
        let data = self.resolve(uri)?;
        check_limit(data.len(), max_bytes, uri)?;
        Ok(data)
    }
}

/// Resolve a data or companion-resource URI with an optional byte quota.
pub fn resolve_resource_uri(
    uri: &str,
    resolver: Option<&dyn ResourceResolver>,
    max_bytes: Option<usize>,
) -> Result<Vec<u8>> {
    if uri.starts_with("data:") {
        return decode_data_uri(uri, max_bytes);
    }
    let resolver = resolver.ok_or_else(|| GltfError::ExternalResourceDenied(uri.to_owned()))?;
    resolver.resolve_with_limit(uri, max_bytes)
}

impl<F> ResourceResolver for F
where
    F: Fn(&str) -> Result<Vec<u8>>,
{
    fn resolve(&self, uri: &str) -> Result<Vec<u8>> {
        self(uri)
    }
}

/// Policy used by [`FileResourceResolver`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExternalFilePolicy {
    /// Reject every external file URI.
    #[default]
    Deny,
    /// Allow paths outside the base directory.
    Allow,
    /// Require resolved paths to remain below the base directory.
    ConfineToBase,
}

/// Native filesystem resolver used by convenience file-loading APIs.
#[derive(Clone, Debug)]
pub struct FileResourceResolver {
    base: PathBuf,
    policy: ExternalFilePolicy,
}

impl FileResourceResolver {
    /// Create a resolver rooted at `base`.
    pub fn new(base: impl Into<PathBuf>, policy: ExternalFilePolicy) -> Self {
        Self {
            base: base.into(),
            policy,
        }
    }
}

impl ResourceResolver for FileResourceResolver {
    fn resolve(&self, uri: &str) -> Result<Vec<u8>> {
        self.resolve_with_limit(uri, None)
    }

    fn resolve_with_limit(&self, uri: &str, max_bytes: Option<usize>) -> Result<Vec<u8>> {
        if self.policy == ExternalFilePolicy::Deny {
            return Err(GltfError::ExternalResourceDenied(uri.to_owned()));
        }
        if uri.contains("://") || uri.starts_with("data:") {
            return Err(GltfError::Unsupported(format!(
                "unsupported external resource URI: {uri}"
            )));
        }

        let decoded = percent_decode(uri)?;
        let decoded = std::str::from_utf8(&decoded).map_err(|_| {
            GltfError::InvalidGltf("external resource path is not valid UTF-8".into())
        })?;
        let candidate = self.base.join(Path::new(decoded));
        if self.policy == ExternalFilePolicy::ConfineToBase {
            let base = self.base.canonicalize()?;
            let path = candidate.canonicalize()?;
            if !path.starts_with(&base) {
                return Err(GltfError::ExternalResourceDenied(uri.to_owned()));
            }
            return read_file_fallibly(&path, max_bytes);
        }
        read_file_fallibly(&candidate, max_bytes)
    }
}

fn read_file_fallibly(path: &Path, max_bytes: Option<usize>) -> Result<Vec<u8>> {
    let length_u64 = fs::metadata(path)?.len();
    let length = usize::try_from(length_u64).map_err(|_| {
        GltfError::ResourceLimitExceeded(format!(
            "{} is too large for this platform",
            path.display()
        ))
    })?;
    check_limit(length, max_bytes, &path.display().to_string())?;

    let mut data = Vec::new();
    data.try_reserve_exact(length).map_err(|_| {
        GltfError::ResourceLimitExceeded(format!("{} allocation failed", path.display()))
    })?;
    data.resize(length, 0);
    let mut file = File::open(path)?;
    file.read_exact(&mut data)?;
    let mut extra = [0u8; 1];
    if file.read(&mut extra)? != 0 {
        return Err(GltfError::InvalidGltf(format!(
            "resource {} grew while it was read",
            path.display()
        )));
    }
    Ok(data)
}

/// Strictly parse JSON glTF or a GLB 2.0 container.
pub fn parse_gltf_container(data: &[u8]) -> Result<GltfContainer<'_>> {
    if data.len() < 4 || read_u32(data, 0)? != GLB_MAGIC {
        return Ok(GltfContainer {
            format: GltfContainerFormat::Gltf,
            json: data,
            bin: None,
        });
    }
    if data.len() < 12 {
        return Err(GltfError::InvalidGlb(
            "file is too small for a GLB header".into(),
        ));
    }
    if read_u32(data, 4)? != GLB_VERSION {
        return Err(GltfError::InvalidGlb(format!(
            "unsupported GLB version {}",
            read_u32(data, 4)?
        )));
    }
    let declared = usize::try_from(read_u32(data, 8)?)
        .map_err(|_| GltfError::InvalidGlb("GLB length cannot fit usize".into()))?;
    if declared != data.len() {
        return Err(GltfError::InvalidGlb(
            "GLB header length does not match file length".into(),
        ));
    }

    let mut offset = 12usize;
    let mut chunk_index = 0usize;
    let mut json = None;
    let mut bin = None;
    while offset < declared {
        let header_end = offset
            .checked_add(8)
            .filter(|end| *end <= declared)
            .ok_or_else(|| GltfError::InvalidGlb("partial GLB chunk header".into()))?;
        let length = usize::try_from(read_u32(data, offset)?)
            .map_err(|_| GltfError::InvalidGlb("chunk length cannot fit usize".into()))?;
        let kind = read_u32(data, offset + 4)?;
        if !length.is_multiple_of(4) {
            return Err(GltfError::InvalidGlb(
                "GLB chunk length is not 4-byte aligned".into(),
            ));
        }
        let end = header_end
            .checked_add(length)
            .filter(|end| *end <= declared)
            .ok_or_else(|| GltfError::InvalidGlb("GLB chunk extends past file end".into()))?;
        let bytes = &data[header_end..end];
        match kind {
            GLB_CHUNK_JSON => {
                if chunk_index != 0 || json.replace(bytes).is_some() {
                    return Err(GltfError::InvalidGlb(
                        "JSON must be the first and only JSON chunk".into(),
                    ));
                }
                // JSON permits trailing whitespace, which is indistinguishable
                // from the space bytes used to pad a GLB JSON chunk. Keep the
                // full chunk for the JSON parser and reject only an empty chunk
                // here; malformed trailing bytes are rejected during parsing.
                if !bytes
                    .iter()
                    .any(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
                {
                    return Err(GltfError::InvalidGlb("JSON chunk is empty".into()));
                }
            }
            GLB_CHUNK_BIN => {
                if chunk_index != 1 || bin.replace(bytes).is_some() {
                    return Err(GltfError::InvalidGlb(
                        "BIN must be the second and only BIN chunk".into(),
                    ));
                }
            }
            _ => {
                if chunk_index == 0 {
                    return Err(GltfError::InvalidGlb("JSON chunk must be first".into()));
                }
            }
        }
        offset = end;
        chunk_index += 1;
    }

    Ok(GltfContainer {
        format: GltfContainerFormat::Glb,
        json: json.ok_or_else(|| GltfError::InvalidGlb("GLB has no JSON chunk".into()))?,
        bin,
    })
}

/// Resolve every declared glTF buffer through the shared resource policy.
///
/// Returned buffers have exactly their declared `byteLength`; legal GLB BIN
/// padding is validated and removed. Resource and aggregate quotas are checked
/// before buffers are exposed to accessor readers.
pub fn resolve_gltf_buffers(
    references: &[GltfBufferReference<'_>],
    format: GltfContainerFormat,
    glb_bin: Option<&[u8]>,
    resolver: Option<&dyn ResourceResolver>,
    limits: &ResourceLimits,
) -> Result<Vec<Vec<u8>>> {
    if format == GltfContainerFormat::Gltf && glb_bin.is_some() {
        return Err(GltfError::InvalidGltf(
            "JSON glTF input cannot have a GLB BIN chunk".into(),
        ));
    }
    if references.is_empty() && glb_bin.is_some() {
        return Err(GltfError::InvalidGlb(
            "GLB has a BIN chunk but declares no buffer".into(),
        ));
    }
    if glb_bin.is_some()
        && references
            .first()
            .is_some_and(|buffer| buffer.uri.is_some())
    {
        return Err(GltfError::InvalidGlb(
            "GLB BIN chunk requires buffer 0 without a URI".into(),
        ));
    }

    let mut buffers = Vec::new();
    buffers
        .try_reserve_exact(references.len())
        .map_err(|_| GltfError::ResourceLimitExceeded("buffer table allocation failed".into()))?;
    let mut total = 0usize;
    for (index, reference) in references.iter().enumerate() {
        let remaining_total = limits
            .max_total_buffer_bytes
            .map(|limit| {
                limit.checked_sub(total).ok_or_else(|| {
                    GltfError::ResourceLimitExceeded(
                        "glTF buffers exceed the configured total".into(),
                    )
                })
            })
            .transpose()?;
        if remaining_total.is_some_and(|remaining| reference.byte_length > remaining) {
            return Err(GltfError::ResourceLimitExceeded(format!(
                "buffer {index} byteLength {} exceeds the remaining total quota",
                reference.byte_length
            )));
        }
        let effective_limit = match (limits.max_resource_bytes, remaining_total) {
            (Some(resource), Some(total)) => Some(resource.min(total)),
            (Some(resource), None) => Some(resource),
            (None, Some(total)) => Some(total),
            (None, None) => None,
        };
        let effective_limits = ResourceLimits {
            max_resource_bytes: effective_limit,
            ..*limits
        };
        let data = resolve_gltf_buffer(
            index,
            *reference,
            format,
            glb_bin,
            resolver,
            &effective_limits,
        )?;
        total = total
            .checked_add(data.len())
            .ok_or_else(|| GltfError::ResourceLimitExceeded("total buffer size overflow".into()))?;
        check_limit(total, limits.max_total_buffer_bytes, "glTF buffers total")?;
        buffers.push(data);
    }
    Ok(buffers)
}

fn resolve_gltf_buffer(
    index: usize,
    reference: GltfBufferReference<'_>,
    format: GltfContainerFormat,
    glb_bin: Option<&[u8]>,
    resolver: Option<&dyn ResourceResolver>,
    limits: &ResourceLimits,
) -> Result<Vec<u8>> {
    if let Some(uri) = reference.uri {
        let mut data = resolve_resource_uri(uri, resolver, limits.max_resource_bytes)?;
        validate_declared_buffer_length(index, reference.byte_length, data.len(), false)?;
        data.truncate(reference.byte_length);
        return Ok(data);
    }

    if format != GltfContainerFormat::Glb {
        return Err(GltfError::InvalidGltf(format!(
            "Buffer {index} has no URI in JSON glTF"
        )));
    }
    if index != 0 {
        return Err(GltfError::InvalidGlb(format!(
            "Buffer {index} has no URI and is not buffer 0"
        )));
    }
    let bin = glb_bin.ok_or_else(|| {
        GltfError::InvalidGlb("Buffer 0 has no URI but GLB has no BIN chunk".into())
    })?;
    check_limit(bin.len(), limits.max_resource_bytes, "GLB BIN chunk")?;
    validate_declared_buffer_length(index, reference.byte_length, bin.len(), true)?;
    if bin[reference.byte_length..]
        .iter()
        .any(|&padding| padding != 0)
    {
        return Err(GltfError::InvalidGlb(
            "GLB BIN padding must contain only zero bytes".into(),
        ));
    }
    copy_prefix(bin, reference.byte_length, "GLB BIN chunk")
}

fn validate_declared_buffer_length(
    index: usize,
    declared: usize,
    actual: usize,
    glb_bin: bool,
) -> Result<()> {
    if actual < declared {
        return Err(GltfError::InvalidGltf(format!(
            "Buffer {index} byteLength {declared} exceeds resource length {actual}"
        )));
    }
    if glb_bin {
        let padded_limit = declared
            .checked_add(3)
            .ok_or_else(|| GltfError::InvalidGlb("buffer byteLength overflow".into()))?;
        if actual > padded_limit {
            return Err(GltfError::InvalidGlb(format!(
                "GLB BIN chunk length {actual} is more than 3 bytes larger than buffer[0].byteLength {declared}"
            )));
        }
    }
    Ok(())
}

fn copy_prefix(data: &[u8], length: usize, label: &str) -> Result<Vec<u8>> {
    let prefix = data
        .get(..length)
        .ok_or_else(|| GltfError::InvalidGltf(format!("{label} is truncated")))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| GltfError::ResourceLimitExceeded(format!("{label} allocation failed")))?;
    output.extend_from_slice(prefix);
    Ok(output)
}

/// Serialize a document and one consolidated binary buffer.
#[cfg(feature = "gltf-writer")]
pub fn serialize_gltf_document(
    document: &Value,
    bin: &[u8],
    input_format: GltfContainerFormat,
    output_format: OutputFormat,
) -> Result<Vec<u8>> {
    let format = match output_format {
        OutputFormat::SameAsInput => input_format,
        OutputFormat::GltfEmbeddedBuffers => GltfContainerFormat::Gltf,
        OutputFormat::Glb => GltfContainerFormat::Glb,
    };
    let mut document = document.clone();
    normalize_single_buffer(&mut document, bin, format)?;
    match format {
        GltfContainerFormat::Gltf => serialize_json(&document),
        GltfContainerFormat::Glb => build_glb_container(&document, bin),
    }
}

#[cfg(feature = "gltf-writer")]
fn normalize_single_buffer(
    document: &mut Value,
    bin: &[u8],
    format: GltfContainerFormat,
) -> Result<()> {
    let root = document
        .as_object_mut()
        .ok_or_else(|| GltfError::InvalidGltf("glTF root is not an object".into()))?;
    if bin.is_empty() {
        let has_views = root
            .get("bufferViews")
            .and_then(Value::as_array)
            .is_some_and(|views| !views.is_empty());
        if has_views {
            return Err(GltfError::InvalidGltf(
                "empty consolidated buffer cannot back bufferViews".into(),
            ));
        }
        root.remove("buffers");
        return Ok(());
    }

    let buffers = root
        .entry("buffers")
        .or_insert_with(|| Value::Array(vec![Value::Object(Default::default())]))
        .as_array_mut()
        .ok_or_else(|| GltfError::InvalidGltf("buffers is not an array".into()))?;
    if buffers.len() != 1 {
        return Err(GltfError::InvalidGltf(format!(
            "consolidated serializer requires exactly one buffer, got {}",
            buffers.len()
        )));
    }
    let buffer = buffers[0]
        .as_object_mut()
        .ok_or_else(|| GltfError::InvalidGltf("buffer 0 is not an object".into()))?;
    buffer.insert("byteLength".into(), Value::from(bin.len() as u64));
    match format {
        GltfContainerFormat::Gltf => {
            buffer.insert(
                "uri".into(),
                Value::String(encode_data_uri("application/octet-stream", bin)?),
            );
        }
        GltfContainerFormat::Glb => {
            buffer.remove("uri");
        }
    }
    Ok(())
}

/// Decode a `data:` URI with an optional decoded-byte quota.
pub fn decode_data_uri(uri: &str, max_bytes: Option<usize>) -> Result<Vec<u8>> {
    let body = uri
        .strip_prefix("data:")
        .ok_or_else(|| GltfError::InvalidGltf("URI is not a data URI".into()))?;
    let comma = body
        .find(',')
        .ok_or_else(|| GltfError::InvalidGltf("data URI has no comma".into()))?;
    let metadata = &body[..comma];
    let payload = &body[comma + 1..];
    let is_base64 = metadata
        .split(';')
        .skip(1)
        .any(|part| part.eq_ignore_ascii_case("base64"));
    let decoded = if is_base64 {
        decode_base64(payload, max_bytes)?
    } else {
        percent_decode_with_limit(payload, max_bytes, "data URI")?
    };
    Ok(decoded)
}

/// Encode bytes as a base64 `data:` URI.
#[cfg(feature = "gltf-writer")]
pub fn encode_data_uri(media_type: &str, data: &[u8]) -> Result<String> {
    let encoded_len = data
        .len()
        .checked_add(2)
        .and_then(|length| length.checked_div(3))
        .and_then(|length| length.checked_mul(4))
        .ok_or_else(|| GltfError::ResourceLimitExceeded("base64 size overflow".into()))?;
    let prefix_len = "data:;base64,"
        .len()
        .checked_add(media_type.len())
        .ok_or_else(|| GltfError::ResourceLimitExceeded("data URI size overflow".into()))?;
    let capacity = prefix_len
        .checked_add(encoded_len)
        .ok_or_else(|| GltfError::ResourceLimitExceeded("data URI size overflow".into()))?;
    let mut output = String::new();
    output
        .try_reserve_exact(capacity)
        .map_err(|_| GltfError::ResourceLimitExceeded("data URI allocation failed".into()))?;
    output.push_str("data:");
    output.push_str(media_type);
    output.push_str(";base64,");
    encode_base64_into(data, &mut output);
    Ok(output)
}

/// Build a strict GLB 2.0 container without rewriting the JSON document.
///
/// Most callers should use [`serialize_gltf_document`], which also normalizes
/// the consolidated buffer declaration. This lower-level helper is useful for
/// fixtures whose JSON intentionally references companion resources.
#[cfg(any(feature = "gltf-writer", test))]
pub fn build_glb_container(document: &Value, bin: &[u8]) -> Result<Vec<u8>> {
    // Reject an impossible BIN chunk before attempting to copy it. The padded
    // chunk length and the complete GLB both have to fit GLB's u32 fields.
    let bin_padding = (4 - bin.len() % 4) % 4;
    let padded_bin_len = bin
        .len()
        .checked_add(bin_padding)
        .ok_or_else(|| GltfError::InvalidGlb("BIN chunk size overflow".into()))?;
    u32::try_from(padded_bin_len)
        .map_err(|_| GltfError::InvalidGlb("BIN chunk exceeds the 32-bit limit".into()))?;

    let mut json = serialize_json(document)?;
    pad_to_four(&mut json, b' ')?;
    let mut bin_copy = Vec::new();
    bin_copy
        .try_reserve_exact(bin.len())
        .map_err(|_| GltfError::ResourceLimitExceeded("BIN chunk allocation failed".into()))?;
    bin_copy.extend_from_slice(bin);
    let mut bin = bin_copy;
    pad_to_four(&mut bin, 0)?;

    let mut total = 12usize
        .checked_add(8)
        .and_then(|value| value.checked_add(json.len()))
        .ok_or_else(|| GltfError::InvalidGlb("GLB size overflow".into()))?;
    if !bin.is_empty() {
        total = total
            .checked_add(8)
            .and_then(|value| value.checked_add(bin.len()))
            .ok_or_else(|| GltfError::InvalidGlb("GLB size overflow".into()))?;
    }
    let total_u32 = u32::try_from(total)
        .map_err(|_| GltfError::InvalidGlb("GLB exceeds the 32-bit length limit".into()))?;
    let json_len = u32::try_from(json.len())
        .map_err(|_| GltfError::InvalidGlb("JSON chunk exceeds the 32-bit limit".into()))?;
    let bin_len = u32::try_from(bin.len())
        .map_err(|_| GltfError::InvalidGlb("BIN chunk exceeds the 32-bit limit".into()))?;

    let mut output = Vec::new();
    output
        .try_reserve_exact(total)
        .map_err(|_| GltfError::ResourceLimitExceeded("GLB allocation failed".into()))?;
    output.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    output.extend_from_slice(&GLB_VERSION.to_le_bytes());
    output.extend_from_slice(&total_u32.to_le_bytes());
    output.extend_from_slice(&json_len.to_le_bytes());
    output.extend_from_slice(&GLB_CHUNK_JSON.to_le_bytes());
    output.extend_from_slice(&json);
    if !bin.is_empty() {
        output.extend_from_slice(&bin_len.to_le_bytes());
        output.extend_from_slice(&GLB_CHUNK_BIN.to_le_bytes());
        output.extend_from_slice(&bin);
    }
    Ok(output)
}

#[cfg(any(feature = "gltf-writer", test))]
fn pad_to_four(bytes: &mut Vec<u8>, padding: u8) -> Result<()> {
    let padding_len = (4 - bytes.len() % 4) % 4;
    let padded_len = bytes
        .len()
        .checked_add(padding_len)
        .ok_or_else(|| GltfError::ResourceLimitExceeded("padding size overflow".into()))?;
    bytes
        .try_reserve(padding_len)
        .map_err(|_| GltfError::ResourceLimitExceeded("padding allocation failed".into()))?;
    bytes.resize(padded_len, padding);
    Ok(())
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .filter(|end| *end <= data.len())
        .ok_or_else(|| GltfError::InvalidGlb("truncated u32".into()))?;
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&data[offset..end]);
    Ok(u32::from_le_bytes(bytes))
}

#[cfg(feature = "gltf-writer")]
struct FallibleJsonBuffer {
    bytes: Vec<u8>,
    allocation_failed: bool,
}

#[cfg(feature = "gltf-writer")]
impl Write for FallibleJsonBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.try_reserve(bytes.len()).is_err() {
            self.allocation_failed = true;
            return Err(io::Error::other("JSON allocation failed"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "gltf-writer")]
fn serialize_json(document: &Value) -> Result<Vec<u8>> {
    let mut output = FallibleJsonBuffer {
        bytes: Vec::new(),
        allocation_failed: false,
    };
    if let Err(error) = serde_json::to_writer(&mut output, document) {
        if output.allocation_failed {
            return Err(GltfError::ResourceLimitExceeded(
                "JSON allocation failed".into(),
            ));
        }
        return Err(GltfError::Json(error));
    }
    Ok(output.bytes)
}

#[cfg(all(test, not(feature = "gltf-writer")))]
fn serialize_json(document: &Value) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(document)?)
}

fn check_limit(length: usize, limit: Option<usize>, resource: &str) -> Result<()> {
    if limit.is_some_and(|limit| length > limit) {
        return Err(GltfError::ResourceLimitExceeded(format!(
            "{resource} is {length} bytes"
        )));
    }
    Ok(())
}

fn decode_base64(input: &str, limit: Option<usize>) -> Result<Vec<u8>> {
    if input.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err(GltfError::InvalidGltf(
            "base64 data must not contain whitespace".into(),
        ));
    }
    if !input.len().is_multiple_of(4) {
        return Err(GltfError::InvalidGltf(
            "base64 length must be divisible by four".into(),
        ));
    }
    let padding = input
        .as_bytes()
        .iter()
        .rev()
        .take_while(|b| **b == b'=')
        .count();
    if padding > 2 || input.as_bytes()[..input.len().saturating_sub(padding)].contains(&b'=') {
        return Err(GltfError::InvalidGltf("invalid base64 padding".into()));
    }
    let decoded_len = input
        .len()
        .checked_div(4)
        .and_then(|length| length.checked_mul(3))
        .and_then(|length| length.checked_sub(padding))
        .ok_or_else(|| GltfError::ResourceLimitExceeded("base64 size overflow".into()))?;
    check_limit(decoded_len, limit, "data URI")?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(decoded_len)
        .map_err(|_| GltfError::ResourceLimitExceeded("base64 allocation failed".into()))?;
    for chunk in input.as_bytes().chunks_exact(4) {
        let a = base64_value(chunk[0])? as u32;
        let b = base64_value(chunk[1])? as u32;
        let c = if chunk[2] == b'=' {
            0
        } else {
            base64_value(chunk[2])? as u32
        };
        let d = if chunk[3] == b'=' {
            0
        } else {
            base64_value(chunk[3])? as u32
        };
        if (chunk[2] == b'=' && b & 0x0f != 0) || (chunk[3] == b'=' && c & 0x03 != 0) {
            return Err(GltfError::InvalidGltf(
                "base64 has non-zero padding bits".into(),
            ));
        }
        let value = (a << 18) | (b << 12) | (c << 6) | d;
        output.push((value >> 16) as u8);
        if chunk[2] != b'=' {
            output.push((value >> 8) as u8);
        }
        if chunk[3] != b'=' {
            output.push(value as u8);
        }
    }
    debug_assert_eq!(output.len(), decoded_len);
    Ok(output)
}

fn base64_value(byte: u8) -> Result<u8> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(GltfError::InvalidGltf("invalid base64 character".into())),
    }
}

#[cfg(feature = "gltf-writer")]
fn encode_base64_into(data: &[u8], output: &mut String) {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in data.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let value = (a << 16) | (b << 8) | c;
        output.push(TABLE[((value >> 18) & 63) as usize] as char);
        output.push(TABLE[((value >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(value & 63) as usize] as char
        } else {
            '='
        });
    }
}

fn percent_decode(input: &str) -> Result<Vec<u8>> {
    percent_decode_with_limit(input, None, "percent-encoded URI")
}

fn percent_decode_with_limit(input: &str, limit: Option<usize>, label: &str) -> Result<Vec<u8>> {
    let bytes = input.as_bytes();
    let mut decoded_len = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let end = index
                .checked_add(3)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| GltfError::InvalidGltf("truncated percent escape".into()))?;
            let _ = hex(bytes[index + 1])?;
            let _ = hex(bytes[index + 2])?;
            index = end;
        } else {
            index += 1;
        }
        decoded_len = decoded_len.checked_add(1).ok_or_else(|| {
            GltfError::ResourceLimitExceeded("percent-decoded size overflow".into())
        })?;
    }
    check_limit(decoded_len, limit, label)?;

    let mut output = Vec::new();
    output
        .try_reserve_exact(decoded_len)
        .map_err(|_| GltfError::ResourceLimitExceeded("percent decode allocation failed".into()))?;
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let end = index
                .checked_add(3)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| GltfError::InvalidGltf("truncated percent escape".into()))?;
            let high = hex(bytes[index + 1])?;
            let low = hex(bytes[index + 2])?;
            output.push((high << 4) | low);
            index = end;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    Ok(output)
}

fn hex(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(GltfError::InvalidGltf("invalid percent escape".into())),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn raw_glb(chunks: &[(u32, &[u8])]) -> Vec<u8> {
        let total = 12
            + chunks
                .iter()
                .map(|(_, bytes)| 8 + bytes.len())
                .sum::<usize>();
        let mut output = Vec::with_capacity(total);
        output.extend_from_slice(&GLB_MAGIC.to_le_bytes());
        output.extend_from_slice(&GLB_VERSION.to_le_bytes());
        output.extend_from_slice(&(total as u32).to_le_bytes());
        for (kind, bytes) in chunks {
            output.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            output.extend_from_slice(&kind.to_le_bytes());
            output.extend_from_slice(bytes);
        }
        output
    }

    #[test]
    fn strict_data_uri_rejects_malformed_input() {
        assert_eq!(decode_data_uri("data:,a%20b", None).unwrap(), b"a b");
        assert_eq!(decode_data_uri("data:;base64,YQ==", None).unwrap(), b"a");
        assert!(decode_data_uri("data:;base64,YQ", None).is_err());
        assert!(decode_data_uri("data:;base64,YR==", None).is_err());
        assert!(decode_data_uri("data:;base64,YWF=", None).is_err());
        assert!(decode_data_uri("data:,a%2", None).is_err());
        assert!(decode_data_uri("data:;base64,YQ==", Some(0)).is_err());
        assert!(decode_data_uri("data:,abcd", Some(2)).is_err());
    }

    #[test]
    fn buffer_total_quota_is_forwarded_before_resolution() {
        struct LimitAwareResolver(Cell<Option<usize>>);

        impl ResourceResolver for LimitAwareResolver {
            fn resolve(&self, _: &str) -> Result<Vec<u8>> {
                panic!("resolve_with_limit must be used")
            }

            fn resolve_with_limit(&self, _: &str, max_bytes: Option<usize>) -> Result<Vec<u8>> {
                self.0.set(max_bytes);
                Ok(vec![1, 2])
            }
        }

        let resolver = LimitAwareResolver(Cell::new(None));
        let buffers = resolve_gltf_buffers(
            &[GltfBufferReference {
                uri: Some("mesh.bin"),
                byte_length: 2,
            }],
            GltfContainerFormat::Gltf,
            None,
            Some(&resolver),
            &ResourceLimits {
                max_total_buffer_bytes: Some(3),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
        assert_eq!(buffers, [vec![1, 2]]);
        assert_eq!(resolver.0.get(), Some(3));

        assert!(resolve_gltf_buffers(
            &[GltfBufferReference {
                uri: Some("data:,abcd"),
                byte_length: 4,
            }],
            GltfContainerFormat::Gltf,
            None,
            None,
            &ResourceLimits {
                max_total_buffer_bytes: Some(2),
                ..ResourceLimits::default()
            },
        )
        .is_err());
    }

    #[test]
    fn glb_builder_and_parser_are_strict() {
        let document = serde_json::json!({
            "asset": {"version": "2.0"},
            "buffers": [{"byteLength": 3}]
        });
        let bytes = build_glb_container(&document, &[1, 2, 3]).unwrap();
        let parsed = parse_gltf_container(&bytes).unwrap();
        assert_eq!(parsed.format, GltfContainerFormat::Glb);
        assert_eq!(&parsed.bin.unwrap()[..3], &[1, 2, 3]);
        let parsed_json: Value = serde_json::from_slice(parsed.json).unwrap();
        assert_eq!(parsed_json["buffers"][0]["byteLength"], 3);
        assert!(parsed_json["buffers"][0].get("uri").is_none());

        let mut trailing = bytes;
        trailing.push(0);
        assert!(parse_gltf_container(&trailing).is_err());

        let json = b"{\"asset\":{\"version\":\"2.0\"}} ";
        let bin = [0u8; 4];
        let bin_first = raw_glb(&[(GLB_CHUNK_BIN, &bin), (GLB_CHUNK_JSON, json)]);
        assert!(parse_gltf_container(&bin_first).is_err());
        let duplicate_bin = raw_glb(&[
            (GLB_CHUNK_JSON, json),
            (GLB_CHUNK_BIN, &bin),
            (GLB_CHUNK_BIN, &bin),
        ]);
        assert!(parse_gltf_container(&duplicate_bin).is_err());
        let unaligned = raw_glb(&[(GLB_CHUNK_JSON, b"{}")]);
        assert!(parse_gltf_container(&unaligned).is_err());
        let trailing_json_whitespace = raw_glb(&[(GLB_CHUNK_JSON, b"{}\t\t")]);
        let parsed = parse_gltf_container(&trailing_json_whitespace).unwrap();
        let parsed_json: Value = serde_json::from_slice(parsed.json).unwrap();
        assert_eq!(parsed_json, serde_json::json!({}));

        let invalid_bin_padding = [1u8, 0xff, 0, 0];
        assert!(resolve_gltf_buffers(
            &[GltfBufferReference {
                uri: None,
                byte_length: 1,
            }],
            GltfContainerFormat::Glb,
            Some(&invalid_bin_padding),
            None,
            &ResourceLimits::default(),
        )
        .is_err());

        let mut wrong_length = build_glb_container(&document, &[]).unwrap();
        let declared = (wrong_length.len() as u32 - 1).to_le_bytes();
        wrong_length[8..12].copy_from_slice(&declared);
        assert!(parse_gltf_container(&wrong_length).is_err());
    }

    #[test]
    #[cfg(feature = "gltf-writer")]
    fn embedded_serializer_sets_uri_and_exact_length() {
        let document = serde_json::json!({
            "asset": {"version": "2.0"},
            "buffers": [{"byteLength": 1, "uri": "old.bin"}]
        });
        let bytes = serialize_gltf_document(
            &document,
            &[1, 2, 3],
            GltfContainerFormat::Glb,
            OutputFormat::GltfEmbeddedBuffers,
        )
        .unwrap();
        let output: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(output["buffers"][0]["byteLength"], 3);
        assert_eq!(
            output["buffers"][0]["uri"],
            "data:application/octet-stream;base64,AQID"
        );
    }
}
