use super::*;

/// Loads a glTF/GLB file that may use Draco (and any other extensions).
///
/// Filesystem-only (not available on `wasm32`); on the web use [`import_slice`]
/// with bytes you have already fetched.
#[cfg(not(target_arch = "wasm32"))]
pub fn import<P: AsRef<Path>>(path: P) -> Result<Import> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    let options = ImportOptions {
        base_path: path.parent(),
        external_file_policy: ExternalFilePolicy::ConfineToBase,
        resolver: None,
        limits: ResourceLimits::default(),
    };
    import_slice_with_options(&bytes, &options)
}

/// Loads glTF JSON or GLB bytes, resolving external resources relative to
/// `base` when present.
///
/// The document is validated with [`validate`] (gltf-rs validation minus the
/// expected Draco "unsupported extension" error), so a structurally invalid
/// asset is rejected even though gltf-rs's own validator cannot be used on a
/// Draco file directly.
pub fn import_slice(bytes: &[u8], base: Option<&Path>) -> Result<Import> {
    let options = ImportOptions {
        base_path: base,
        external_file_policy: if base.is_some() {
            ExternalFilePolicy::Allow
        } else {
            ExternalFilePolicy::Deny
        },
        resolver: None,
        limits: ResourceLimits::default(),
    };
    import_slice_with_options(bytes, &options)
}

/// Loads glTF/GLB with an explicit resolver, external-file policy, and optional
/// quotas. The resolver is synchronous so the API works in native callers and
/// in WASM wrappers that already hold companion-resource bytes.
pub fn import_slice_with_options(bytes: &[u8], options: &ImportOptions<'_>) -> Result<Import> {
    let container = draco_io::parse_gltf_container(bytes)?;
    let raw_document: Value = serde_json::from_slice(container.json)?;
    let root = gltf::json::Root::deserialize(&raw_document)?;
    let document = gltf::Document::from_json_without_validation(root);
    validate(&document)?;
    let document_snapshot = serde_json::to_value(document.clone().into_json())?;

    let mut references = Vec::new();
    references
        .try_reserve_exact(document.buffers().len())
        .map_err(|_| Error::ResourceLimit("failed to allocate buffer reference table".into()))?;
    for buffer in document.buffers() {
        let uri = match buffer.source() {
            gltf::buffer::Source::Bin => None,
            gltf::buffer::Source::Uri(uri) => Some(uri),
        };
        references.push(draco_io::GltfBufferReference {
            uri,
            byte_length: buffer.length(),
        });
    }

    let file_resolver = options
        .base_path
        .map(|base| FileResourceResolver::new(base, options.external_file_policy));
    let resolver: Option<&dyn ResourceResolver> = options.resolver.or_else(|| {
        file_resolver
            .as_ref()
            .map(|resolver| resolver as &dyn ResourceResolver)
    });
    let buffers = draco_io::resolve_gltf_buffers(
        &references,
        container.format,
        container.bin,
        resolver,
        &options.limits,
    )?;
    draco_io::validate_gltf_document_for_repacking(&raw_document, &buffers)?;

    #[cfg(feature = "image")]
    {
        let images = load_images(&document, &buffers, resolver, &options.limits)?;
        Ok(Import {
            document,
            buffers,
            images,
            input_format: container.format,
            raw_document,
            document_snapshot,
        })
    }
    #[cfg(not(feature = "image"))]
    {
        Ok(Import {
            document,
            buffers,
            input_format: container.format,
            raw_document,
            document_snapshot,
        })
    }
}

#[cfg(feature = "image")]
fn load_images(
    document: &gltf::Document,
    buffers: &[Vec<u8>],
    resolver: Option<&dyn ResourceResolver>,
    limits: &ResourceLimits,
) -> Result<Vec<gltf::image::Data>> {
    let mut out = Vec::new();
    out.try_reserve_exact(document.images().len())
        .map_err(|_| Error::ResourceLimit("failed to allocate image list".into()))?;
    for source in document.images().map(|image| image.source()) {
        let encoded = match source {
            gltf::image::Source::Uri { uri, .. } => {
                draco_io::resolve_resource_uri(uri, resolver, limits.max_resource_bytes)?
            }
            gltf::image::Source::View { view, .. } => {
                let buffer = buffers
                    .get(view.buffer().index())
                    .ok_or_else(|| Error::Extension("image buffer was not resolved".into()))?;
                let end = view
                    .offset()
                    .checked_add(view.length())
                    .filter(|end| *end <= buffer.len())
                    .ok_or_else(|| Error::Extension("image bufferView out of range".into()))?;
                let bytes = &buffer[view.offset()..end];
                let mut encoded = Vec::new();
                encoded
                    .try_reserve_exact(bytes.len())
                    .map_err(|_| Error::ResourceLimit("failed to allocate encoded image".into()))?;
                encoded.extend_from_slice(bytes);
                encoded
            }
        };
        out.push(decode_image(&encoded, limits.max_image_pixels)?);
    }
    Ok(out)
}

#[cfg(feature = "image")]
fn decode_image(bytes: &[u8], max_pixels: Option<u64>) -> Result<gltf::image::Data> {
    use image::GenericImageView;
    use std::io::Cursor;

    let reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let format = reader
        .format()
        .ok_or_else(|| Error::Extension("image encoding could not be detected".into()))?;
    let (width, height) = reader.into_dimensions()?;
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| Error::ResourceLimit("image pixel count overflow".into()))?;
    if max_pixels.is_some_and(|limit| pixels > limit) {
        return Err(Error::ResourceLimit(format!("image has {pixels} pixels")));
    }

    let mut reader = image::ImageReader::new(Cursor::new(bytes));
    reader.set_format(format);
    let mut limits = image::Limits::no_limits();
    if let Some(max_pixels) = max_pixels {
        let per_dimension = u32::try_from(max_pixels).unwrap_or(u32::MAX);
        limits.max_image_width = Some(per_dimension);
        limits.max_image_height = Some(per_dimension);
        limits.max_alloc = max_pixels.checked_mul(16);
    }
    reader.limits(limits);
    let decoded = reader.decode()?;
    let format = match &decoded {
        image::DynamicImage::ImageLuma8(_) => gltf::image::Format::R8,
        image::DynamicImage::ImageLumaA8(_) => gltf::image::Format::R8G8,
        image::DynamicImage::ImageRgb8(_) => gltf::image::Format::R8G8B8,
        image::DynamicImage::ImageRgba8(_) => gltf::image::Format::R8G8B8A8,
        image::DynamicImage::ImageLuma16(_) => gltf::image::Format::R16,
        image::DynamicImage::ImageLumaA16(_) => gltf::image::Format::R16G16,
        image::DynamicImage::ImageRgb16(_) => gltf::image::Format::R16G16B16,
        image::DynamicImage::ImageRgba16(_) => gltf::image::Format::R16G16B16A16,
        image::DynamicImage::ImageRgb32F(_) => gltf::image::Format::R32G32B32FLOAT,
        image::DynamicImage::ImageRgba32F(_) => gltf::image::Format::R32G32B32A32FLOAT,
        _ => {
            return Err(Error::Extension(
                "decoded image format is unsupported by glTF".into(),
            ))
        }
    };
    let (width, height) = decoded.dimensions();
    Ok(gltf::image::Data {
        pixels: decoded.into_bytes(),
        format,
        width,
        height,
    })
}
