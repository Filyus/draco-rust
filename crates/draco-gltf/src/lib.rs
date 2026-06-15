//! Load and save full glTF scenes with Draco-compressed geometry.
//!
//! `draco-gltf` is a thin bridge between [`gltf`](https://docs.rs/gltf) (which
//! models the whole glTF scene: materials, textures, nodes, animations, skins,
//! and arbitrary extensions) and the Draco crates:
//!
//! - **decode** uses [`draco_core`] to decompress `KHR_draco_mesh_compression`
//!   geometry,
//! - **encode** delegates to [`draco_io`]'s document-preserving compressor, so
//!   the compression logic lives in exactly one place.
//!
//! It exists because neither side does the whole job alone: `gltf-rs` does not
//! decode Draco (and its validator even *rejects* a Draco asset, since
//! `KHR_draco_mesh_compression` is a required extension it does not implement),
//! while the Draco crates intentionally do not model the rest of a glTF scene.
//!
//! # Example
//!
//! ```no_run
//! // Load a glTF/GLB that uses Draco (and anything else), then decode geometry.
//! let scene = draco_gltf::import("model.glb")?;
//! println!("{} materials", scene.document.materials().count());
//! for (mesh, prim) in scene.draco_primitives() {
//!     let geometry = scene.decode_primitive(&prim)?; // draco_core::Mesh
//!     println!("mesh {:?}: {} faces", mesh.name(), geometry.num_faces());
//! }
//!
//! // Compress a full scene back to a self-contained Draco glTF.
//! let bytes = draco_gltf::compress(&scene.document, &scene.buffers)?;
//! std::fs::write("model.draco.gltf", bytes)?;
//! # Ok::<(), draco_gltf::Error>(())
//! ```
//!
//! # Validation
//!
//! `gltf-rs`'s validator rejects Draco assets, so [`import`] loads without it.
//! The other content is still well-formed `gltf-rs` data; callers that need
//! strict validation of the non-Draco parts can run `gltf-rs` validation
//! themselves and ignore the `KHR_draco_mesh_compression`-related errors.

use std::path::Path;

use draco_core::{DecoderBuffer, Mesh, MeshDecoder};
use gltf::buffer;

/// Re-export so callers can use the scene model without depending on `gltf`
/// directly.
pub use gltf;

const KHR_DRACO: &str = "KHR_draco_mesh_compression";

/// Errors from loading, decoding, or compressing.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `gltf-rs` failed to parse the document.
    #[error("glTF error: {0}")]
    Gltf(#[from] gltf::Error),
    /// Filesystem or stream I/O failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// Serializing the document for compression failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// A Draco bitstream failed to decode.
    #[error("Draco decode error: {0}")]
    Decode(String),
    /// Compression (via `draco-io`) failed.
    #[error("compression error: {0}")]
    Compress(String),
    /// The `KHR_draco_mesh_compression` extension was malformed or absent.
    #[error("Draco extension error: {0}")]
    Extension(String),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// A loaded glTF document with its decoded buffers and images.
///
/// The `document` is a full [`gltf::Document`]: materials, textures, nodes,
/// animations, skins, lights, and extensions are all available through the
/// `gltf-rs` API. Use [`Self::decode_primitive`] to decompress the geometry of
/// any Draco primitive.
pub struct Import {
    /// The parsed glTF document (full scene model).
    pub document: gltf::Document,
    /// Buffer data, indexed by glTF buffer index.
    pub buffers: Vec<buffer::Data>,
    /// Decoded image data, indexed by glTF image index.
    pub images: Vec<gltf::image::Data>,
}

impl Import {
    /// Iterates `(mesh, primitive)` pairs whose geometry is Draco-compressed.
    pub fn draco_primitives(&self) -> impl Iterator<Item = (gltf::Mesh<'_>, gltf::Primitive<'_>)> {
        self.document
            .meshes()
            .flat_map(|mesh| mesh.primitives().map(move |prim| (mesh.clone(), prim)))
            .filter(|(_, prim)| is_draco(prim))
    }

    /// Decodes a Draco primitive's geometry into a [`draco_core::Mesh`].
    ///
    /// Errors if the primitive is not Draco-compressed or the extension is
    /// malformed.
    pub fn decode_primitive(&self, primitive: &gltf::Primitive<'_>) -> Result<Mesh> {
        decode_primitive(&self.document, &self.buffers, primitive)
    }
}

/// Loads a glTF/GLB file that may use Draco (and any other extensions).
///
/// Loads without `gltf-rs` validation (which rejects Draco assets); see the
/// [module docs](crate#validation).
pub fn import<P: AsRef<Path>>(path: P) -> Result<Import> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    import_slice(&bytes, path.parent())
}

/// Loads glTF JSON or GLB bytes, resolving external resources relative to
/// `base` when present.
pub fn import_slice(bytes: &[u8], base: Option<&Path>) -> Result<Import> {
    let gltf::Gltf { document, blob } = gltf::Gltf::from_slice_without_validation(bytes)?;
    let buffers = gltf::import_buffers(&document, base, blob)?;
    let images = gltf::import_images(&document, base, &buffers)?;
    Ok(Import {
        document,
        buffers,
        images,
    })
}

/// Returns `true` if the primitive's geometry is Draco-compressed.
pub fn is_draco(primitive: &gltf::Primitive<'_>) -> bool {
    primitive.extension_value(KHR_DRACO).is_some()
}

/// Decodes a Draco primitive's geometry into a [`draco_core::Mesh`].
pub fn decode_primitive(
    document: &gltf::Document,
    buffers: &[buffer::Data],
    primitive: &gltf::Primitive<'_>,
) -> Result<Mesh> {
    let ext = primitive
        .extension_value(KHR_DRACO)
        .ok_or_else(|| Error::Extension("primitive is not Draco-compressed".into()))?;
    let view_index =
        ext.get("bufferView")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Extension("missing bufferView".into()))? as usize;

    let view = document
        .views()
        .nth(view_index)
        .ok_or_else(|| Error::Extension(format!("bufferView {view_index} out of range")))?;
    let buffer = buffers
        .get(view.buffer().index())
        .ok_or_else(|| Error::Extension("buffer not resolved".into()))?;
    let start = view.offset();
    let end = start
        .checked_add(view.length())
        .filter(|&e| e <= buffer.0.len())
        .ok_or_else(|| Error::Extension("Draco bufferView out of range".into()))?;

    let mut mesh = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(&buffer.0[start..end]), &mut mesh)
        .map_err(|e| Error::Decode(format!("{e:?}")))?;
    Ok(mesh)
}

/// Returns the extension's `glTF semantic -> Draco attribute id` map for a
/// Draco primitive, or `None` if it is not Draco-compressed.
///
/// The ids match the unique ids of the attributes in the decoded
/// [`draco_core::Mesh`], so callers can line up decoded data with glTF
/// semantics (`POSITION`, `JOINTS_0`, `TANGENT`, …).
pub fn draco_attribute_map(primitive: &gltf::Primitive<'_>) -> Option<Vec<(String, u32)>> {
    let ext = primitive.extension_value(KHR_DRACO)?;
    let map = ext.get("attributes")?.as_object()?;
    Some(
        map.iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_u64()? as u32)))
            .collect(),
    )
}

/// Compresses a full glTF scene to a self-contained Draco glTF (embedded
/// buffer), preserving materials, textures, nodes, animations, skins, and any
/// other content.
///
/// This delegates the actual compression to [`draco_io::compress_gltf_bytes`],
/// so there is a single compression implementation. It re-emits the `gltf-rs`
/// document with its buffers embedded as data URIs and hands the bytes to the
/// document-preserving compressor.
pub fn compress(document: &gltf::Document, buffers: &[buffer::Data]) -> Result<Vec<u8>> {
    let mut root = document.clone().into_json();
    for (i, buf) in buffers.iter().enumerate() {
        let entry = root
            .buffers
            .get_mut(i)
            .ok_or_else(|| Error::Compress(format!("buffer {i} missing in document")))?;
        entry.uri = Some(format!(
            "data:application/octet-stream;base64,{}",
            base64_encode(&buf.0)
        ));
        entry.byte_length = gltf::json::validation::USize64(buf.0.len() as u64);
    }
    let bytes = serde_json::to_vec(&root)?;
    draco_io::compress_gltf_bytes(&bytes, None).map_err(|e| Error::Compress(e.to_string()))
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
