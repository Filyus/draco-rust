//! Shared glTF/GLB container and resource handling.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::gltf_error::{GltfError, Result};

const GLB_MAGIC: u32 = 0x4654_6c67;
const GLB_VERSION_V2: u32 = 2;
const GLB_VERSION_V3: u32 = 3;
const GLB_CHUNK_JSON: u32 = 0x4e4f_534a;
const GLB_CHUNK_BIN: u32 = 0x004e_4942;
/// Version 3 starts every chunk on this boundary, counted from the file start.
const V3_CHUNK_ALIGNMENT: u64 = 8;

/// Container used by an input glTF document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GltfContainerFormat {
    /// JSON glTF document.
    Gltf,
    /// Binary GLB 2.0 document.
    GlbV2,
    /// Draft glTF 2.1 binary container (GLB version 3).
    GlbV3,
}

/// Builds a GLB v2 or draft v3 container from serialized JSON and binary data.
///
/// ```
/// # use draco_gltf::{parse_gltf_container, GltfContainerFormat};
/// # use draco_gltf::container::build_glb_from_json;
/// let json = br#"{"asset":{"version":"2.0"}}"#;
/// let glb = build_glb_from_json(json, &[1, 2, 3], GltfContainerFormat::GlbV2)?;
/// let container = parse_gltf_container(&glb)?;
/// assert_eq!(container.format, GltfContainerFormat::GlbV2);
/// assert_eq!(&container.bin.unwrap()[..3], &[1, 2, 3]);
/// # Ok::<(), draco_gltf::GltfError>(())
/// ```
pub fn build_glb_from_json(
    json: &[u8],
    bin: &[u8],
    format: GltfContainerFormat,
) -> Result<Vec<u8>> {
    if !matches!(
        format,
        GltfContainerFormat::GlbV2 | GltfContainerFormat::GlbV3
    ) {
        return Err(GltfError::InvalidGlb(
            "GLB output requires a GLB format".into(),
        ));
    }
    let v3 = format == GltfContainerFormat::GlbV3;
    // Version 3 starts every chunk on an 8-byte boundary; padding the JSON
    // chunk to 8 keeps the BIN chunk aligned without a gap between them.
    let json_alignment = if v3 { 8 } else { 4 };
    let mut json = json.to_vec();
    while !json.len().is_multiple_of(json_alignment) {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let header = if v3 { 16 } else { 12 };
    let chunk_header = if v3 { 16 } else { 8 };
    let total = header
        + chunk_header
        + json.len()
        + if bin.is_empty() {
            0
        } else {
            chunk_header + bin.len()
        };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    out.extend_from_slice(&(if v3 { 3u32 } else { 2u32 }).to_le_bytes());
    if v3 {
        out.extend_from_slice(&(total as u64).to_le_bytes());
    } else {
        out.extend_from_slice(
            &u32::try_from(total)
                .map_err(|_| GltfError::ResourceLimitExceeded("GLB v2 exceeds u32".into()))?
                .to_le_bytes(),
        );
    }
    for (kind, bytes) in [(GLB_CHUNK_JSON, &json), (GLB_CHUNK_BIN, &bin)] {
        if kind == GLB_CHUNK_BIN && bytes.is_empty() {
            continue;
        }
        if v3 {
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        } else {
            out.extend_from_slice(
                &u32::try_from(bytes.len())
                    .map_err(|_| {
                        GltfError::ResourceLimitExceeded("GLB v2 chunk exceeds u32".into())
                    })?
                    .to_le_bytes(),
            );
            out.extend_from_slice(&kind.to_le_bytes());
        }
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

#[cfg(test)]
mod glb_tests {
    use super::*;

    #[test]
    fn glb_v3_builder_roundtrips_container_layout() {
        let bytes = build_glb_from_json(
            br#"{"asset":{"version":"2.1"}}"#,
            &[1, 2, 3],
            GltfContainerFormat::GlbV3,
        )
        .unwrap();
        let parsed = parse_gltf_container(&bytes).unwrap();
        assert_eq!(parsed.format, GltfContainerFormat::GlbV3);
        assert_eq!(parsed.bin.unwrap()[..3], [1, 2, 3]);
    }

    #[test]
    fn range_reader_materializes_only_selected_v3_chunk() {
        let bytes = build_glb_from_json(
            br#"{"asset":{"version":"2.1"}}"#,
            &[1, 2, 3, 4],
            GltfContainerFormat::GlbV3,
        )
        .unwrap();
        let mut reader = GlbRangeReader::open(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(reader.layout().chunks.len(), 2);
        let bin = reader.layout().chunks[1];
        assert_eq!(reader.read_chunk(bin, Some(4)).unwrap(), [1, 2, 3, 4]);
        assert!(reader.read_chunk(bin, Some(3)).is_err());
    }

    fn u32_at(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn u64_at(bytes: &[u8], offset: usize) -> u64 {
        u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
    }

    /// The draft's version 3 chunk header is `chunkType`, `chunkEncoding`,
    /// `chunkLength`, in that order, and every chunk starts 8-byte aligned.
    #[test]
    fn glb_v3_chunks_carry_the_header_the_draft_defines() {
        let bytes = build_glb_from_json(
            br#"{"asset":{"version":"2.1"}}"#,
            &[1, 2, 3, 4],
            GltfContainerFormat::GlbV3,
        )
        .unwrap();
        assert_eq!(u32_at(&bytes, 4), 3);
        assert_eq!(u64_at(&bytes, 8), bytes.len() as u64);

        assert_eq!(u32_at(&bytes, 16), GLB_CHUNK_JSON);
        assert_eq!(u32_at(&bytes, 20), 0, "chunkEncoding");
        let json_length = u64_at(&bytes, 24) as usize;
        let bin_header = 32 + json_length;
        assert_eq!(bin_header % 8, 0, "the BIN chunk starts 8-byte aligned");

        assert_eq!(u32_at(&bytes, bin_header), GLB_CHUNK_BIN);
        assert_eq!(u32_at(&bytes, bin_header + 4), 0, "chunkEncoding");
        assert_eq!(u64_at(&bytes, bin_header + 8), 4);
        assert_eq!(&bytes[bin_header + 16..], [1, 2, 3, 4]);
    }

    /// A v3 chunk's length is not padded to anything: the next chunk starts at
    /// the next 8-byte boundary after its end, with the gap outside both.
    #[test]
    fn glb_v3_reads_chunks_that_end_before_the_next_aligned_start() {
        let json = br#"{"asset":{"version":"2.1"}}"#;
        assert_eq!(json.len() % 8, 3);
        let bin = [9u8, 8, 7];
        let json_end = 32 + json.len();
        let bin_start = json_end.next_multiple_of(8);
        let total = bin_start + 16 + bin.len();

        let mut file = Vec::new();
        file.extend_from_slice(&GLB_MAGIC.to_le_bytes());
        file.extend_from_slice(&GLB_VERSION_V3.to_le_bytes());
        file.extend_from_slice(&(total as u64).to_le_bytes());
        file.extend_from_slice(&GLB_CHUNK_JSON.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&(json.len() as u64).to_le_bytes());
        file.extend_from_slice(json);
        file.resize(bin_start, 0);
        file.extend_from_slice(&GLB_CHUNK_BIN.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&(bin.len() as u64).to_le_bytes());
        file.extend_from_slice(&bin);
        assert_eq!(file.len(), total);

        let parsed = parse_gltf_container(&file).unwrap();
        assert_eq!(parsed.json, json);
        assert_eq!(parsed.bin.unwrap(), bin);

        let mut reader = GlbRangeReader::open(std::io::Cursor::new(file)).unwrap();
        let chunks = reader.layout().chunks.clone();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1].offset, bin_start as u64 + 16);
        assert_eq!(reader.read_chunk(chunks[1], None).unwrap(), bin);
    }

    #[test]
    fn glb_v3_refuses_a_chunk_encoding_it_does_not_know() {
        let mut bytes = build_glb_from_json(
            br#"{"asset":{"version":"2.1"}}"#,
            &[1, 2, 3, 4],
            GltfContainerFormat::GlbV3,
        )
        .unwrap();
        bytes[20] = 1;
        assert!(parse_gltf_container(&bytes).is_err());
        assert!(GlbRangeReader::open(std::io::Cursor::new(bytes)).is_err());
    }
}

impl GltfContainerFormat {
    /// Whether this is either binary GLB container version.
    pub const fn is_glb(self) -> bool {
        matches!(self, Self::GlbV2 | Self::GlbV3)
    }
}

/// A BIN chunk of a GLB container and its position among the chunks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlbBinChunk<'a> {
    /// Index of the chunk in the file, counting every chunk from zero.
    ///
    /// This is the number a glTF `buffer.chunk` property names.
    pub index: usize,
    /// Chunk payload, including any trailing padding.
    pub data: &'a [u8],
}

/// Borrowed, strictly parsed glTF container.
#[derive(Clone, Debug)]
pub struct GltfContainer<'a> {
    /// Input container kind.
    pub format: GltfContainerFormat,
    /// JSON document bytes (including legal GLB JSON padding).
    pub json: &'a [u8],
    /// The BIN chunk at chunk index 1, which buffer 0 uses when it names
    /// neither a `chunk` nor a `uri`. Also listed in [`Self::bin_chunks`].
    pub bin: Option<&'a [u8]>,
    /// Every BIN chunk, in file order. Version 2 has at most one.
    pub bin_chunks: Vec<GlbBinChunk<'a>>,
}

/// A chunk address in a seekable GLB input. No chunk bytes are materialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlbChunkDescriptor {
    /// Absolute byte offset of chunk payload.
    pub offset: u64,
    /// Payload length in bytes.
    pub length: u64,
    /// Four-byte chunk kind.
    pub kind: u32,
    /// Reserved GLB v3 encoding field; currently zero.
    pub encoding: u32,
}

/// Metadata for a seekable GLB input, suitable for range-based resource loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlbLayout {
    /// Container version represented by this layout.
    pub format: GltfContainerFormat,
    /// Declared total container length.
    pub length: u64,
    /// Ordered chunk descriptors.
    pub chunks: Vec<GlbChunkDescriptor>,
}

/// Seekable, range-backed GLB source.
///
/// Opening the source reads only headers and chunk descriptors. Callers choose
/// which chunk to materialize and can enforce an independent quota for each
/// range; this is the non-slice path for GLB v3 files larger than addressable
/// memory.
pub struct GlbRangeReader<R> {
    input: R,
    layout: GlbLayout,
}

impl<R: Read + Seek> GlbRangeReader<R> {
    /// Inspects a seekable source without materializing chunk payloads.
    pub fn open(mut input: R) -> Result<Self> {
        let layout = inspect_glb(&mut input)?;
        Ok(Self { input, layout })
    }

    /// Returns the inspected container layout.
    pub fn layout(&self) -> &GlbLayout {
        &self.layout
    }

    /// Materializes one described chunk after checking the caller's limit.
    pub fn read_chunk(
        &mut self,
        descriptor: GlbChunkDescriptor,
        max_bytes: Option<usize>,
    ) -> Result<Vec<u8>> {
        if !self.layout.chunks.contains(&descriptor) {
            return Err(GltfError::InvalidGlb(
                "GLB chunk does not belong to this source".into(),
            ));
        }
        let length = usize::try_from(descriptor.length).map_err(|_| {
            GltfError::ResourceLimitExceeded("GLB chunk exceeds this platform".into())
        })?;
        check_limit(length, max_bytes, "GLB chunk")?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| GltfError::ResourceLimitExceeded("GLB chunk allocation failed".into()))?;
        bytes.resize(length, 0);
        self.input.seek(SeekFrom::Start(descriptor.offset))?;
        self.input.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    /// Returns the underlying seekable source.
    pub fn into_inner(self) -> R {
        self.input
    }
}

/// Inspects GLB v2/v3 headers from a seekable source without allocating chunks.
pub fn inspect_glb<R: Read + Seek>(input: &mut R) -> Result<GlbLayout> {
    input.seek(SeekFrom::Start(0))?;
    let magic = read_u32_stream(input)?;
    if magic != GLB_MAGIC {
        return Err(GltfError::InvalidGlb("input is not a GLB container".into()));
    }
    let version = read_u32_stream(input)?;
    let (format, length, header, chunk_header) = match version {
        GLB_VERSION_V2 => (
            GltfContainerFormat::GlbV2,
            u64::from(read_u32_stream(input)?),
            12u64,
            8u64,
        ),
        GLB_VERSION_V3 => (
            GltfContainerFormat::GlbV3,
            read_u64_stream(input)?,
            16u64,
            16u64,
        ),
        _ => {
            return Err(GltfError::InvalidGlb(format!(
                "unsupported GLB version {version}"
            )))
        }
    };
    let actual = input.seek(SeekFrom::End(0))?;
    if actual != length {
        return Err(GltfError::InvalidGlb(
            "GLB header length does not match stream length".into(),
        ));
    }
    input.seek(SeekFrom::Start(header))?;
    let mut chunks = Vec::new();
    let mut offset = header;
    while offset < length {
        if length - offset < chunk_header {
            return Err(GltfError::InvalidGlb("partial GLB chunk header".into()));
        }
        let (chunk_length, kind, encoding) = if format == GltfContainerFormat::GlbV3 {
            let kind = read_u32_stream(input)?;
            let encoding = read_u32_stream(input)?;
            (read_u64_stream(input)?, kind, encoding)
        } else {
            let chunk_length = u64::from(read_u32_stream(input)?);
            (chunk_length, read_u32_stream(input)?, 0)
        };
        if encoding != 0 {
            return Err(GltfError::InvalidGlb(
                "GLB v3 chunk encoding is not plain and this crate cannot decode it".into(),
            ));
        }
        let aligned_length = format == GltfContainerFormat::GlbV3 || chunk_length % 4 == 0;
        if !aligned_length || chunk_length > length - offset - chunk_header {
            return Err(GltfError::InvalidGlb("invalid GLB chunk length".into()));
        }
        chunks.push(GlbChunkDescriptor {
            offset: offset + chunk_header,
            length: chunk_length,
            kind,
            encoding,
        });
        offset = offset
            .checked_add(chunk_header)
            .and_then(|value| value.checked_add(chunk_length))
            .ok_or_else(|| GltfError::InvalidGlb("GLB chunk offset overflow".into()))?;
        if format == GltfContainerFormat::GlbV3 {
            // The gap up to the next 8-byte boundary belongs to no chunk.
            offset = offset.next_multiple_of(V3_CHUNK_ALIGNMENT);
        }
        input.seek(SeekFrom::Start(offset))?;
    }
    Ok(GlbLayout {
        format,
        length,
        chunks,
    })
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
    /// Maximum number of explicit nested glTF assets on one `files` chain.
    ///
    /// The document layer owns graph traversal; this quota bounds each
    /// caller-directed chain without triggering implicit recursion.
    pub max_external_asset_depth: Option<usize>,
}

/// One glTF `buffers[]` declaration, independent of a JSON front end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GltfBufferReference<'a> {
    /// Optional data or companion-resource URI.
    pub uri: Option<&'a str>,
    /// Optional index of the GLB chunk holding the buffer (`buffer.chunk`).
    pub chunk: Option<usize>,
    /// Declared `byteLength` of the logical buffer, excluding GLB padding.
    pub byte_length: usize,
    /// `EXT_meshopt_compression` fallback buffer: it carries no stored bytes
    /// and starts out zeroed, waiting for its buffer views to be decoded.
    pub meshopt_fallback: bool,
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

/// Filesystem resolver used by convenience file-loading APIs.
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

/// Strictly parse JSON glTF, GLB 2.0, or the draft GLB 3 container.
pub fn parse_gltf_container(data: &[u8]) -> Result<GltfContainer<'_>> {
    if data.len() < 4 || read_u32(data, 0)? != GLB_MAGIC {
        return Ok(GltfContainer {
            format: GltfContainerFormat::Gltf,
            json: data,
            bin: None,
            bin_chunks: Vec::new(),
        });
    }
    if data.len() < 12 {
        return Err(GltfError::InvalidGlb(
            "file is too small for a GLB header".into(),
        ));
    }
    let version = read_u32(data, 4)?;
    if version == GLB_VERSION_V3 {
        return parse_glb_v3(data);
    }
    if version != GLB_VERSION_V2 {
        return Err(GltfError::InvalidGlb(format!(
            "unsupported GLB version {version}"
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
    let mut bin_chunks = Vec::new();
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
                bin_chunks.push(GlbBinChunk {
                    index: chunk_index,
                    data: bytes,
                });
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
        format: GltfContainerFormat::GlbV2,
        json: json.ok_or_else(|| GltfError::InvalidGlb("GLB has no JSON chunk".into()))?,
        bin,
        bin_chunks,
    })
}

/// Parses the draft GLB v3 wire format. Its header is
/// `magic:u32, version:u32, length:u64`; each chunk header is
/// `type:u32, encoding:u32, length:u64`, and every chunk starts on an 8-byte
/// boundary, so a chunk's length is not padded and the gap after it belongs to
/// no chunk. `encoding` zero means plain data; any other value needs a decoder
/// this crate does not have.
fn parse_glb_v3(data: &[u8]) -> Result<GltfContainer<'_>> {
    if data.len() < 16 {
        return Err(GltfError::InvalidGlb(
            "file is too small for a GLB v3 header".into(),
        ));
    }
    let declared = read_u64(data, 8)?;
    let actual = u64::try_from(data.len())
        .map_err(|_| GltfError::InvalidGlb("input length cannot fit u64".into()))?;
    if declared != actual {
        return Err(GltfError::InvalidGlb(
            "GLB v3 header length does not match file length".into(),
        ));
    }

    let mut offset = 16usize;
    let mut chunk_index = 0usize;
    let mut json = None;
    let mut bin = None;
    let mut bin_chunks = Vec::new();
    while offset < data.len() {
        let header_end = offset
            .checked_add(16)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| GltfError::InvalidGlb("partial GLB v3 chunk header".into()))?;
        let kind = read_u32(data, offset)?;
        let encoding = read_u32(data, offset + 4)?;
        let length = usize::try_from(read_u64(data, offset + 8)?).map_err(|_| {
            GltfError::ResourceLimitExceeded("GLB v3 chunk exceeds platform address space".into())
        })?;
        if encoding != 0 {
            return Err(GltfError::InvalidGlb(
                "GLB v3 chunk encoding is not plain and this crate cannot decode it".into(),
            ));
        }
        let end = header_end
            .checked_add(length)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| GltfError::InvalidGlb("GLB v3 chunk extends past file end".into()))?;
        let bytes = &data[header_end..end];
        match kind {
            // The glTF JSON is the first JSON chunk, wherever it sits; later
            // JSON chunks and chunks of unknown type belong to extensions.
            GLB_CHUNK_JSON if json.is_none() => {
                if !bytes
                    .iter()
                    .any(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
                {
                    return Err(GltfError::InvalidGlb("GLB v3 JSON chunk is empty".into()));
                }
                json = Some(bytes);
            }
            GLB_CHUNK_BIN => {
                if chunk_index == 1 {
                    bin = Some(bytes);
                }
                bin_chunks.push(GlbBinChunk {
                    index: chunk_index,
                    data: bytes,
                });
            }
            _ => {}
        }
        offset = end.next_multiple_of(V3_CHUNK_ALIGNMENT as usize);
        chunk_index = chunk_index
            .checked_add(1)
            .ok_or_else(|| GltfError::InvalidGlb("too many GLB v3 chunks".into()))?;
    }
    Ok(GltfContainer {
        format: GltfContainerFormat::GlbV3,
        json: json.ok_or_else(|| GltfError::InvalidGlb("GLB v3 has no JSON chunk".into()))?,
        bin,
        bin_chunks,
    })
}

/// Parses only the JSON and optional BIN slices from a GLB container.
///
/// This keeps strict GLB container checks available to callers that do not need
/// to construct a document model.
pub fn parse_glb_json_and_bin(data: &[u8]) -> Result<(&[u8], Option<&[u8]>)> {
    if data.len() < 4
        || u32::from_le_bytes(
            data[0..4]
                .try_into()
                .map_err(|_| GltfError::InvalidGlb("short GLB magic".into()))?,
        ) != GLB_MAGIC
    {
        return Err(GltfError::InvalidGlb("input is not a GLB container".into()));
    }
    if data.len() < 12 {
        return Err(GltfError::InvalidGlb("GLB header is truncated".into()));
    }
    let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if version != GLB_VERSION_V2 {
        return Err(GltfError::InvalidGlb("unsupported GLB version".into()));
    }
    let declared = usize::try_from(u32::from_le_bytes(data[8..12].try_into().unwrap()))
        .map_err(|_| GltfError::InvalidGlb("GLB length is too large".into()))?;
    if declared != data.len() {
        return Err(GltfError::InvalidGlb(
            "GLB length does not match input".into(),
        ));
    }
    let mut offset = 12usize;
    let mut chunks = 0usize;
    let mut json = None;
    let mut bin = None;
    while offset < declared {
        let header_end = offset
            .checked_add(8)
            .filter(|end| *end <= declared)
            .ok_or_else(|| GltfError::InvalidGlb("GLB chunk header is truncated".into()))?;
        let length = usize::try_from(u32::from_le_bytes(
            data[offset..offset + 4].try_into().unwrap(),
        ))
        .map_err(|_| GltfError::InvalidGlb("GLB chunk is too large".into()))?;
        if !length.is_multiple_of(4) {
            return Err(GltfError::InvalidGlb(
                "GLB chunk is not 4-byte aligned".into(),
            ));
        }
        let kind = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
        let end = header_end
            .checked_add(length)
            .filter(|end| *end <= declared)
            .ok_or_else(|| GltfError::InvalidGlb("GLB chunk exceeds input".into()))?;
        match kind {
            GLB_CHUNK_JSON if chunks == 0 && json.is_none() => {
                let bytes = &data[header_end..end];
                if bytes
                    .iter()
                    .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
                {
                    return Err(GltfError::InvalidGlb("GLB JSON chunk is empty".into()));
                }
                json = Some(bytes);
            }
            GLB_CHUNK_JSON => {
                return Err(GltfError::InvalidGlb(
                    "GLB JSON must be first and unique".into(),
                ))
            }
            GLB_CHUNK_BIN if bin.is_none() => bin = Some(&data[header_end..end]),
            GLB_CHUNK_BIN => {
                return Err(GltfError::InvalidGlb(
                    "GLB contains duplicate BIN chunks".into(),
                ))
            }
            _ => {}
        }
        offset = end;
        chunks = chunks
            .checked_add(1)
            .ok_or_else(|| GltfError::InvalidGlb("too many GLB chunks".into()))?;
    }
    json.map(|json| (json, bin))
        .ok_or_else(|| GltfError::InvalidGlb("GLB JSON chunk is missing".into()))
}

/// Resolves all declared buffer references under the configured quotas.
pub fn resolve_gltf_buffers(
    references: &[GltfBufferReference<'_>],
    format: GltfContainerFormat,
    glb_chunks: &[GlbBinChunk<'_>],
    resolver: Option<&dyn ResourceResolver>,
    limits: &ResourceLimits,
) -> Result<Vec<Vec<u8>>> {
    if format == GltfContainerFormat::Gltf && !glb_chunks.is_empty() {
        return Err(GltfError::InvalidGltf(
            "JSON glTF input cannot have a GLB BIN chunk".into(),
        ));
    }
    if references.is_empty() && !glb_chunks.is_empty() {
        return Err(GltfError::InvalidGlb(
            "GLB has a BIN chunk but declares no buffer".into(),
        ));
    }
    // Version 2 has one BIN chunk, and buffer 0 is its only possible owner.
    // Version 3 names chunks explicitly, so a chunk may sit unused.
    if format == GltfContainerFormat::GlbV2
        && !glb_chunks.is_empty()
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
            glb_chunks,
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
    glb_chunks: &[GlbBinChunk<'_>],
    resolver: Option<&dyn ResourceResolver>,
    limits: &ResourceLimits,
) -> Result<Vec<u8>> {
    let chunk = buffer_chunk(index, reference, format, glb_chunks)?;
    // A fallback buffer only carries stored bytes when the extension is
    // optional; when it is required the buffer starts out zeroed and its views
    // are filled in by the meshopt decoder.
    if reference.meshopt_fallback && reference.uri.is_none() && chunk.is_none() {
        check_limit(
            reference.byte_length,
            limits.max_resource_bytes,
            "meshopt fallback buffer",
        )?;
        let mut data = Vec::new();
        data.try_reserve_exact(reference.byte_length).map_err(|_| {
            GltfError::ResourceLimitExceeded("meshopt fallback buffer allocation failed".into())
        })?;
        data.resize(reference.byte_length, 0);
        return Ok(data);
    }

    if let Some(uri) = reference.uri {
        let mut data = resolve_resource_uri(uri, resolver, limits.max_resource_bytes)?;
        validate_declared_buffer_length(index, reference.byte_length, data.len(), None)?;
        data.truncate(reference.byte_length);
        return Ok(data);
    }

    if !format.is_glb() {
        return Err(GltfError::InvalidGltf(format!(
            "Buffer {index} has no URI in JSON glTF"
        )));
    }
    let Some(bin) = chunk else {
        return Err(GltfError::InvalidGlb(if index == 0 {
            "Buffer 0 has no URI or chunk but GLB has no BIN chunk at index 1".into()
        } else {
            format!("Buffer {index} names neither a URI nor a chunk")
        }));
    };
    check_limit(bin.len(), limits.max_resource_bytes, "GLB BIN chunk")?;
    // The last 7 bytes are 8-byte alignment padding in version 3, 3 in version 2.
    let padding = if format == GltfContainerFormat::GlbV3 {
        7
    } else {
        3
    };
    validate_declared_buffer_length(index, reference.byte_length, bin.len(), Some(padding))?;
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

/// The BIN chunk `reference` stores its bytes in, when it names one.
///
/// A buffer names its chunk with `chunk`. Buffer 0 with neither `chunk` nor
/// `uri` uses the chunk at index 1, as in glTF 2.0; a missing chunk is left to
/// the caller to report. Every other buffer without either is not GLB-stored.
fn buffer_chunk<'a>(
    index: usize,
    reference: GltfBufferReference<'_>,
    format: GltfContainerFormat,
    chunks: &[GlbBinChunk<'a>],
) -> Result<Option<&'a [u8]>> {
    let find = |chunk: usize| {
        chunks
            .iter()
            .find(|candidate| candidate.index == chunk)
            .map(|candidate| candidate.data)
    };
    match reference.chunk {
        Some(chunk) => {
            if reference.uri.is_some() {
                return Err(GltfError::InvalidGlb(format!(
                    "Buffer {index} names both a chunk and a URI"
                )));
            }
            if !format.is_glb() {
                return Err(GltfError::InvalidGltf(format!(
                    "Buffer {index} names chunk {chunk} but the input is not a GLB"
                )));
            }
            find(chunk).map(Some).ok_or_else(|| {
                GltfError::InvalidGlb(format!(
                    "Buffer {index} names chunk {chunk}, which is not a BIN chunk of this GLB"
                ))
            })
        }
        None if index == 0 && reference.uri.is_none() && format.is_glb() => Ok(find(1)),
        None => Ok(None),
    }
}

fn validate_declared_buffer_length(
    index: usize,
    declared: usize,
    actual: usize,
    padding: Option<usize>,
) -> Result<()> {
    if actual < declared {
        return Err(GltfError::InvalidGltf(format!(
            "Buffer {index} byteLength {declared} exceeds resource length {actual}"
        )));
    }
    if let Some(padding) = padding {
        let padded_limit = declared
            .checked_add(padding)
            .ok_or_else(|| GltfError::InvalidGlb("buffer byteLength overflow".into()))?;
        if actual > padded_limit {
            return Err(GltfError::InvalidGlb(format!(
                "GLB BIN chunk length {actual} is more than {padding} bytes larger than buffer[{index}].byteLength {declared}"
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

/// Decode a `data:` URI with an optional decoded-byte quota.
///
/// ```
/// # use draco_gltf::decode_data_uri;
/// assert_eq!(decode_data_uri("data:text/plain;base64,aGk=", None)?, b"hi");
/// # Ok::<(), draco_gltf::GltfError>(())
/// ```
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

fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .filter(|end| *end <= data.len())
        .ok_or_else(|| GltfError::InvalidGlb("truncated u32".into()))?;
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&data[offset..end]);
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64> {
    let end = offset
        .checked_add(8)
        .filter(|end| *end <= data.len())
        .ok_or_else(|| GltfError::InvalidGlb("truncated u64".into()))?;
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&data[offset..end]);
    Ok(u64::from_le_bytes(bytes))
}

fn read_u32_stream<R: Read>(input: &mut R) -> Result<u32> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn read_u64_stream<R: Read>(input: &mut R) -> Result<u64> {
    let mut bytes = [0; 8];
    input.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
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
    for chunk in input.as_bytes().as_chunks::<4>().0 {
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
        output.extend_from_slice(&GLB_VERSION_V2.to_le_bytes());
        output.extend_from_slice(&(total as u32).to_le_bytes());
        for (kind, bytes) in chunks {
            output.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            output.extend_from_slice(&kind.to_le_bytes());
            output.extend_from_slice(bytes);
        }
        output
    }

    /// A version 3 file from parts, each chunk starting on an 8-byte boundary.
    fn v3_file(chunks: &[(u32, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (kind, bytes) in chunks {
            body.resize(body.len().next_multiple_of(8), 0);
            body.extend_from_slice(&kind.to_le_bytes());
            body.extend_from_slice(&0u32.to_le_bytes());
            body.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            body.extend_from_slice(bytes);
        }
        let mut file = Vec::new();
        file.extend_from_slice(&GLB_MAGIC.to_le_bytes());
        file.extend_from_slice(&GLB_VERSION_V3.to_le_bytes());
        file.extend_from_slice(&((16 + body.len()) as u64).to_le_bytes());
        file.extend_from_slice(&body);
        file
    }

    const UNKNOWN_CHUNK: u32 = 0x1234_5678;
    const JSON: &[u8] = br#"{"asset":{"version":"2.1"}}"#;

    fn chunk_buffer(chunk: Option<usize>, byte_length: usize) -> GltfBufferReference<'static> {
        GltfBufferReference {
            uri: None,
            chunk,
            byte_length,
            meshopt_fallback: false,
        }
    }

    fn resolve(
        references: &[GltfBufferReference<'_>],
        container: &GltfContainer<'_>,
    ) -> Result<Vec<Vec<u8>>> {
        resolve_gltf_buffers(
            references,
            container.format,
            &container.bin_chunks,
            None,
            &ResourceLimits::default(),
        )
    }

    /// A version 3 buffer names its chunk, and the chunk index counts every
    /// chunk in the file, whatever its type.
    #[test]
    fn glb_v3_buffers_read_the_chunk_they_name() {
        let first = [1u8, 2, 3, 4];
        let second = [5u8, 6, 7, 8, 9, 10, 11, 12];
        let file = v3_file(&[
            (GLB_CHUNK_JSON, JSON),
            (UNKNOWN_CHUNK, &[0xEE; 5]),
            (GLB_CHUNK_BIN, &first),
            (GLB_CHUNK_BIN, &second),
        ]);
        let container = parse_gltf_container(&file).unwrap();
        assert_eq!(container.json, JSON);
        assert!(container.bin.is_none(), "chunk 1 is not a BIN chunk");
        let indices: Vec<_> = container
            .bin_chunks
            .iter()
            .map(|chunk| chunk.index)
            .collect();
        assert_eq!(indices, [2, 3]);

        let buffers = resolve(
            &[chunk_buffer(Some(3), 8), chunk_buffer(Some(2), 4)],
            &container,
        )
        .unwrap();
        assert_eq!(buffers, [second.to_vec(), first.to_vec()]);
    }

    /// Buffer 0 with neither `chunk` nor `uri` still means chunk 1, next to
    /// buffers that name theirs.
    #[test]
    fn glb_v3_buffer_zero_keeps_the_implicit_chunk() {
        let file = v3_file(&[
            (GLB_CHUNK_JSON, JSON),
            (GLB_CHUNK_BIN, &[1, 2, 3, 4]),
            (GLB_CHUNK_BIN, &[5, 6, 7, 8]),
        ]);
        let container = parse_gltf_container(&file).unwrap();
        assert_eq!(container.bin, Some([1u8, 2, 3, 4].as_slice()));
        let buffers = resolve(
            &[chunk_buffer(None, 4), chunk_buffer(Some(2), 4)],
            &container,
        )
        .unwrap();
        assert_eq!(buffers, [vec![1, 2, 3, 4], vec![5, 6, 7, 8]]);
    }

    /// The glTF JSON is the first JSON chunk wherever it is, and later JSON
    /// chunks and unknown chunks are not the reader's concern.
    #[test]
    fn glb_v3_finds_the_json_chunk_among_others() {
        let file = v3_file(&[
            (UNKNOWN_CHUNK, &[1, 2, 3]),
            (GLB_CHUNK_JSON, JSON),
            (GLB_CHUNK_JSON, b"not the gltf json"),
            (GLB_CHUNK_BIN, &[1, 2, 3, 4]),
        ]);
        let container = parse_gltf_container(&file).unwrap();
        assert_eq!(container.json, JSON);
        assert_eq!(container.bin_chunks.len(), 1);
        assert_eq!(container.bin_chunks[0].index, 3);
    }

    #[test]
    fn glb_v3_refuses_buffers_that_name_no_usable_chunk() {
        let file = v3_file(&[
            (GLB_CHUNK_JSON, JSON),
            (GLB_CHUNK_BIN, &[1, 2, 3, 4]),
            (UNKNOWN_CHUNK, &[0; 4]),
        ]);
        let container = parse_gltf_container(&file).unwrap();
        // Chunk 2 exists but is not a BIN chunk; chunk 9 does not exist.
        for chunk in [2, 9] {
            assert!(resolve(&[chunk_buffer(Some(chunk), 4)], &container).is_err());
        }
        // Only buffer 0 stores bytes implicitly.
        assert!(resolve(&[chunk_buffer(None, 4), chunk_buffer(None, 4)], &container).is_err());
        // A chunk and a URI are two answers to one question.
        let both = GltfBufferReference {
            uri: Some("data:,abcd"),
            ..chunk_buffer(Some(1), 4)
        };
        assert!(resolve(&[both], &container).is_err());
        // Plain JSON glTF has no chunks to name.
        assert!(resolve_gltf_buffers(
            &[chunk_buffer(Some(1), 4)],
            GltfContainerFormat::Gltf,
            &[],
            None,
            &ResourceLimits::default(),
        )
        .is_err());
    }

    /// Version 3 pads a BIN chunk to 8 bytes, so up to 7 bytes may follow the
    /// buffer's `byteLength`; version 2 pads to 4 and allows 3.
    #[test]
    fn glb_bin_padding_allowance_follows_the_container_version() {
        let bin = [1u8, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, 0, 0, 0, 0];
        let v3_bytes = v3_file(&[(GLB_CHUNK_JSON, JSON), (GLB_CHUNK_BIN, &bin)]);
        let v3 = parse_gltf_container(&v3_bytes).unwrap();
        assert!(resolve(&[chunk_buffer(None, 9)], &v3).is_ok());
        assert!(resolve(&[chunk_buffer(None, 8)], &v3).is_err());

        let v2_file = raw_glb(&[(GLB_CHUNK_JSON, b"{}  "), (GLB_CHUNK_BIN, &bin[..12])]);
        let v2 = parse_gltf_container(&v2_file).unwrap();
        assert!(resolve(&[chunk_buffer(None, 9)], &v2).is_ok());
        assert!(resolve(&[chunk_buffer(None, 8)], &v2).is_err());
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
    fn compact_glb_slice_parser_keeps_strict_container_checks() {
        let json = b"{}  ";
        let bin = [1u8, 2, 3, 4];
        let bytes = raw_glb(&[(GLB_CHUNK_JSON, json), (GLB_CHUNK_BIN, &bin)]);
        let (parsed_json, parsed_bin) = parse_glb_json_and_bin(&bytes).unwrap();
        assert_eq!(parsed_json, json);
        assert_eq!(parsed_bin, Some(bin.as_slice()));

        let mut malformed = bytes.clone();
        malformed[8..12].copy_from_slice(&(bytes.len() as u32 - 4).to_le_bytes());
        assert!(parse_glb_json_and_bin(&malformed).is_err());
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
                chunk: None,
                byte_length: 2,
                meshopt_fallback: false,
            }],
            GltfContainerFormat::Gltf,
            &[],
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
                chunk: None,
                byte_length: 4,
                meshopt_fallback: false,
            }],
            GltfContainerFormat::Gltf,
            &[],
            None,
            &ResourceLimits {
                max_total_buffer_bytes: Some(2),
                ..ResourceLimits::default()
            },
        )
        .is_err());
    }
}
