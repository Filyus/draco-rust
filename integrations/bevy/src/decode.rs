//! The gltf-rs side of a decode: find the payload and the declared accessors
//! in the document Bevy parsed, and hand both to `draco-gltf`.

use draco_gltf::{
    DecodeLimits, DracoPrimitiveContract, DracoPrimitiveExtension, JsonValue, PackedGeometry,
    PrimitiveMode,
};

/// Why a Draco primitive could not be turned into geometry.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DecodeError {
    #[error("extension object is not valid JSON: {0}")]
    Json(String),
    #[error("bufferView {0} does not exist")]
    MissingBufferView(usize),
    #[error("bufferView {view} names buffer {buffer}, which was not loaded")]
    MissingBuffer { view: usize, buffer: usize },
    #[error("bufferView {view} spans bytes {start}..{end} of a {len}-byte buffer")]
    ViewOutOfBounds {
        view: usize,
        start: usize,
        end: usize,
        len: usize,
    },
    #[error(transparent)]
    Draco(#[from] draco_gltf::Error),
}

/// A decoded primitive and the extension object it was decoded against.
pub(crate) struct Decoded {
    pub extension: DracoPrimitiveExtension,
    pub geometry: PackedGeometry,
}

/// Decodes `primitive`, whose `KHR_draco_mesh_compression` object is
/// `extension`, out of the buffers Bevy loaded for `document`.
pub(crate) fn decode_primitive(
    document: &gltf::Document,
    primitive: &gltf::Primitive<'_>,
    extension: &serde_json::Value,
    buffers: &[Vec<u8>],
    limits: DecodeLimits,
) -> Result<Decoded, DecodeError> {
    // gltf-rs and draco-gltf each have their own JSON model. The object is a
    // handful of keys, so a round trip through text is cheaper than a second
    // parser for it would be to keep in step with the first.
    let text =
        serde_json::to_vec(extension).map_err(|error| DecodeError::Json(error.to_string()))?;
    let value = JsonValue::parse(&text).map_err(DecodeError::Json)?;
    let extension = DracoPrimitiveExtension::from_json(&value)?;

    let payload = payload(document, extension.buffer_view(), buffers)?;
    let geometry = extension.decode(payload, &contract(primitive, limits))?;
    Ok(Decoded {
        extension,
        geometry,
    })
}

/// Borrows the bytes of buffer view `index`, checking every bound.
fn payload<'a>(
    document: &gltf::Document,
    index: usize,
    buffers: &'a [Vec<u8>],
) -> Result<&'a [u8], DecodeError> {
    let view = document
        .views()
        .nth(index)
        .ok_or(DecodeError::MissingBufferView(index))?;
    let buffer = buffers
        .get(view.buffer().index())
        .ok_or(DecodeError::MissingBuffer {
            view: index,
            buffer: view.buffer().index(),
        })?;
    let start = view.offset();
    start
        .checked_add(view.length())
        .and_then(|end| buffer.get(start..end))
        .ok_or(DecodeError::ViewOutOfBounds {
            view: index,
            start,
            end: start.saturating_add(view.length()),
            len: buffer.len(),
        })
}

/// Collects what the primitive's accessors declare. Every attribute is
/// listed, compressed or not: `draco-gltf` refuses a declared count the stream
/// cannot supply, whichever accessor declares it.
fn contract(primitive: &gltf::Primitive<'_>, limits: DecodeLimits) -> DracoPrimitiveContract {
    let mut contract = DracoPrimitiveContract::new().with_limits(limits);
    if let Some(mode) = PrimitiveMode::from_gltf(primitive.mode().as_gl_enum()) {
        contract = contract.with_mode(mode);
    }
    for (semantic, accessor) in primitive.attributes() {
        contract = contract.with_attribute(
            semantic.to_string(),
            accessor.count() as u64,
            accessor.normalized(),
        );
    }
    if let Some(indices) = primitive.indices() {
        contract = contract.with_indices(indices.count() as u64);
    }
    contract
}
