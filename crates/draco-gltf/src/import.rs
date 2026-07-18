use std::path::Path;

use crate::json::Value;

#[cfg(any(feature = "draco-decode", feature = "geometry"))]
use crate::PrimitiveRef;
use crate::{Document, Error, ExtensionRegistry, ResourceStore, Result, ValidationProfile};
#[cfg(feature = "resources")]
use crate::{ExternalAssetIndex, FileIndex};
use draco_io::{
    parse_gltf_container, resolve_gltf_buffers, GltfBufferReference, GltfContainerFormat,
    ResourceLimits, ResourceResolver,
};
#[cfg(not(target_arch = "wasm32"))]
use draco_io::{ExternalFilePolicy, FileResourceResolver};

/// Lossless glTF document plus its resolved resources.
#[derive(Clone)]
pub struct Import {
    pub document: Document,
    pub resources: ResourceStore,
    pub input_format: GltfContainerFormat,
    profile: ValidationProfile,
    extensions: ExtensionRegistry,
    #[cfg(feature = "resources")]
    provenance: Vec<String>,
}

/// A portable JSON glTF document and the companion resources it references.
///
/// Write `json` to the `.gltf` file and each [`GltfResource`] relative to it.
/// Data URIs remain embedded in `json`; every materialized buffer with a
/// non-data URI is returned exactly once in `resources`.
#[derive(Clone, Debug)]
pub struct GltfOutput {
    pub json: Vec<u8>,
    pub resources: Vec<GltfResource>,
}

/// One companion resource produced by [`Import::to_gltf_output`].
#[derive(Clone, Debug)]
pub struct GltfResource {
    pub uri: String,
    pub bytes: Vec<u8>,
}

/// Default maximum explicit nested-asset depth for [`Import::load_asset`].
pub const DEFAULT_EXTERNAL_ASSET_DEPTH: usize = 32;

/// Resolves URIs in an embedded child against the parent's virtual `files`
/// directory before falling back to the caller's resolver.
#[cfg(feature = "resources")]
struct PackagedResolver<'a> {
    import: &'a Import,
    fallback: &'a dyn ResourceResolver,
}

#[cfg(feature = "resources")]
impl ResourceResolver for PackagedResolver<'_> {
    fn resolve(&self, uri: &str) -> std::result::Result<Vec<u8>, draco_io::GltfError> {
        let Some(file) = self
            .import
            .document
            .files()
            .into_iter()
            .find(|file| file.name() == Some(uri))
        else {
            return self.fallback.resolve(uri);
        };
        if file.value().get("bufferView").is_some() {
            return self
                .import
                .embedded_file_bytes(file.value())
                .map_err(|error| draco_io::GltfError::InvalidGltf(error.to_string()));
        }
        if let Some(source) = file.value().get("uri").and_then(Value::as_str) {
            return draco_io::resolve_resource_uri(source, Some(self.fallback), None);
        }
        Err(draco_io::GltfError::InvalidGltf(format!(
            "packaged file {uri:?} has no source"
        )))
    }
}

impl Import {
    pub fn validate(&self, extensions: &ExtensionRegistry) -> Result<()> {
        self.document.validate(self.profile)?;
        extensions.validate(&self.document)?;
        Ok(())
    }

    #[cfg(feature = "transform")]
    pub(crate) fn ensure_transform_safe(&self, primitive: PrimitiveRef<'_>) -> Result<()> {
        let Some(extensions) = primitive
            .value()
            .get("extensions")
            .and_then(Value::as_object)
        else {
            return Ok(());
        };
        for (name, _) in extensions {
            if !self.extensions.allows_binary_transform(name) {
                return Err(Error::Extension(format!(
                    "cannot transform primitive with extension {name:?}: its binary-reference semantics are not registered as transform-safe"
                )));
            }
        }
        Ok(())
    }

    /// Iterates primitives carrying the built-in Draco extension.
    #[cfg(feature = "draco-decode")]
    pub fn draco_primitives(&self) -> impl Iterator<Item = PrimitiveRef<'_>> + '_ {
        self.document
            .meshes()
            .into_iter()
            .flat_map(move |mesh| {
                let count = mesh
                    .value()
                    .get("primitives")
                    .and_then(Value::as_array)
                    .map_or(0, |values| values.len());
                (0..count)
                    .filter_map(move |primitive| self.document.primitive(mesh.index(), primitive))
            })
            .filter(|primitive| {
                primitive
                    .extension(crate::KHR_DRACO_MESH_COMPRESSION)
                    .is_some()
            })
    }

    /// Decodes a primitive through the supplied extension registry.
    #[cfg(feature = "draco-decode")]
    pub fn decode_primitive(&self, primitive: PrimitiveRef<'_>) -> Result<draco_core::Mesh> {
        self.validate(&self.extensions)?;
        self.extensions
            .decode_primitive(&self.document, &self.resources, primitive)
    }

    /// Decodes an ordinary (non-Draco) triangle or point primitive through the
    /// same geometry contract used by compact consumers.
    #[cfg(feature = "geometry")]
    pub fn decode_geometry_primitive(
        &self,
        primitive: PrimitiveRef<'_>,
    ) -> Result<(draco_core::Mesh, Vec<(String, u32)>)> {
        if primitive
            .extension(crate::KHR_DRACO_MESH_COMPRESSION)
            .is_some()
        {
            return Err(Error::Extension("primitive uses Draco compression".into()));
        }
        let value = primitive.value();
        let attributes = value
            .get("attributes")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::Extension("primitive attributes are invalid".into()))?
            .iter()
            .map(|(semantic, value)| {
                value
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .map(|index| (semantic.clone(), index))
                    .ok_or_else(|| Error::Extension(format!("attribute {semantic} is invalid")))
            })
            .collect::<Result<Vec<_>>>()?;
        let indices = value
            .get("indices")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok());
        let mode = value.get("mode").and_then(Value::as_u64).unwrap_or(4) as u32;
        let source = crate::DocumentAccessorSource::new(&self.document, &self.resources);
        Ok(draco_io::decode_geometry(
            &source,
            mode,
            &attributes,
            indices,
        )?)
    }

    pub fn to_bytes(&self, output: crate::OutputFormat) -> Result<Vec<u8>> {
        let format = match output {
            crate::OutputFormat::GltfJson => return self.document.to_json_bytes(),
            crate::OutputFormat::SameAsInput => self.input_format,
            crate::OutputFormat::GlbV2 => draco_io::GltfContainerFormat::GlbV2,
            crate::OutputFormat::GlbV3 => draco_io::GltfContainerFormat::GlbV3,
        };
        if format.is_glb() {
            let (json, bin) = self.consolidated_glb_payload()?;
            return Ok(draco_io::gltf_container::build_glb_from_json(
                &json, &bin, format,
            )?);
        }
        self.document.to_json_bytes()
    }

    /// Serializes a self-contained `.gltf` output bundle.
    ///
    /// Unlike [`Import::to_bytes`] with [`crate::OutputFormat::GltfJson`],
    /// this method returns companion buffer payloads as well. Buffers without
    /// a URI (for example a Draco payload appended during compression) receive
    /// a deterministic `buffer-{index}.bin` URI in the returned JSON.
    pub fn to_gltf_output(&self) -> Result<GltfOutput> {
        let declared = self.document.buffers().len();
        if declared != self.resources.buffers.len() {
            return Err(Error::ResourceLimit(format!(
                "document declares {declared} buffers but resource store has {}",
                self.resources.buffers.len()
            )));
        }
        let mut document = self.document.clone();
        let mut resources = Vec::new();
        let buffers = document
            .as_value_mut()
            .get_mut("buffers")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| Error::Validation(vec!["buffers is not an array".into()]))?;
        for (index, (buffer, bytes)) in buffers.iter_mut().zip(&self.resources.buffers).enumerate()
        {
            let uri = buffer.get("uri").and_then(Value::as_str).map(str::to_owned);
            let uri = match uri {
                Some(uri) if uri.starts_with("data:") => continue,
                Some(uri) => uri,
                None => {
                    let uri = format!("buffer-{index}.bin");
                    buffer["uri"] = Value::from(uri.as_str());
                    uri
                }
            };
            resources.push(GltfResource {
                uri,
                bytes: bytes.clone(),
            });
        }
        Ok(GltfOutput {
            json: document.to_json_bytes()?,
            resources,
        })
    }

    /// Creates a GLB payload by consolidating resolved buffers while retaining
    /// every bufferView index and all non-resource JSON verbatim.
    fn consolidated_glb_payload(&self) -> Result<(Vec<u8>, Vec<u8>)> {
        let declared = self.document.buffers().len();
        if declared != self.resources.buffers.len() {
            return Err(Error::ResourceLimit(format!(
                "document declares {declared} buffers but resource store has {}",
                self.resources.buffers.len()
            )));
        }
        let mut offsets = Vec::with_capacity(declared);
        let mut bin = Vec::new();
        for resource in &self.resources.buffers {
            while !bin.len().is_multiple_of(4) {
                bin.push(0);
            }
            offsets.push(bin.len());
            bin.try_reserve(resource.len())
                .map_err(|_| Error::ResourceLimit("GLB consolidation allocation failed".into()))?;
            bin.extend_from_slice(resource);
        }
        let mut document = self.document.clone();
        let root = document.as_value_mut();
        if let Some(views) = root.get_mut("bufferViews").and_then(Value::as_array_mut) {
            for (index, view) in views.iter_mut().enumerate() {
                let buffer = view
                    .get("buffer")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| {
                        Error::Validation(vec![format!(
                            "bufferViews[{index}].buffer is not a valid index"
                        )])
                    })?;
                let prefix = *offsets.get(buffer).ok_or_else(|| {
                    Error::Validation(vec![format!(
                        "bufferViews[{index}].buffer references missing buffer {buffer}"
                    )])
                })?;
                let offset = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0);
                let offset = usize::try_from(offset)
                    .ok()
                    .and_then(|offset| prefix.checked_add(offset))
                    .ok_or_else(|| Error::ResourceLimit("GLB bufferView offset overflow".into()))?;
                view["buffer"] = Value::from(0usize);
                view["byteOffset"] = Value::from(offset);
            }
        }
        root["buffers"] = Value::Array(vec![Value::object([(
            "byteLength",
            Value::from(bin.len()),
        )])]);
        Ok((document.to_json_bytes()?, bin))
    }

    /// Materializes all Draco primitives as ordinary indexed triangle geometry.
    #[cfg(feature = "transform")]
    pub fn decompress_in_place(&mut self) -> Result<()> {
        let mut candidate = self.clone();
        candidate.decompress_in_place_inner()?;
        *self = candidate;
        Ok(())
    }

    #[cfg(feature = "transform")]
    fn decompress_in_place_inner(&mut self) -> Result<()> {
        let mut plans = Vec::new();
        for mesh in self.document.meshes() {
            let count = mesh
                .value()
                .get("primitives")
                .and_then(Value::as_array)
                .map_or(0, |values| values.len());
            for primitive_index in 0..count {
                let primitive = self
                    .document
                    .primitive(mesh.index(), primitive_index)
                    .unwrap();
                let Some(extension) = primitive.extension(crate::KHR_DRACO_MESH_COMPRESSION) else {
                    continue;
                };
                self.ensure_transform_safe(primitive)?;
                let mapping = crate::extensions::parse_draco_extension(Some(extension))?
                    .ok_or_else(|| Error::Extension("missing Draco extension".into()))?
                    .attributes;
                let decoded = self.decode_primitive(primitive)?;
                plans.push((mesh.index().0, primitive_index, mapping, decoded));
            }
        }
        if plans.is_empty() {
            return Ok(());
        }
        let buffer_index = self.resources.buffers.len();
        let mut bytes = Vec::new();
        {
            let root = self.document.as_value_mut();
            for (mesh_index, primitive_index, mapping, mesh) in &plans {
                for (semantic, unique_id) in mapping {
                    let source_accessor = root["meshes"][*mesh_index]["primitives"]
                        [*primitive_index]["attributes"][semantic]
                        .as_u64()
                        .and_then(|value| usize::try_from(value).ok())
                        .ok_or_else(|| {
                            Error::Extension(format!("Draco attribute {semantic} has no accessor"))
                        })?;
                    // A glTF accessor can be shared by another primitive,
                    // animation, skin, or morph target. Decompression only
                    // owns this primitive reference, so always detach it
                    // before replacing its binary layout.
                    let accessor = clone_accessor(root, source_accessor)?;
                    root["meshes"][*mesh_index]["primitives"][*primitive_index]["attributes"]
                        [semantic.as_str()] = Value::from(accessor as u64);
                    let data = decoded_attribute_bytes(mesh, *unique_id)?;
                    let view = append_view(root, buffer_index, &mut bytes, &data)?;
                    let target = root["accessors"]
                        .as_array_mut()
                        .and_then(|accessors| accessors.get_mut(accessor))
                        .ok_or_else(|| Error::Extension("Draco accessor out of range".into()))?;
                    target["bufferView"] = Value::from(view as u64);
                    target["byteOffset"] = Value::from(0u64);
                    target["count"] = Value::from(mesh.num_points() as u64);
                    if let Some(object) = target.as_object_mut() {
                        if let Some(index) = object.iter().position(|(name, _)| name == "sparse") {
                            object.remove(index);
                        }
                    }
                }
                let indices = decoded_index_bytes(mesh)?;
                let view = append_view(root, buffer_index, &mut bytes, &indices)?;
                let source_accessor = root["meshes"][*mesh_index]["primitives"][*primitive_index]
                    .get("indices")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok());
                let accessor = match source_accessor {
                    Some(index) => {
                        let index = clone_accessor(root, index)?;
                        root["meshes"][*mesh_index]["primitives"][*primitive_index]["indices"] =
                            Value::from(index as u64);
                        index
                    }
                    None => {
                        let accessors = root["accessors"]
                            .as_array_mut()
                            .ok_or_else(|| Error::Extension("accessors is not an array".into()))?;
                        let index = accessors.len();
                        accessors.push(Value::Object(Vec::new()));
                        root["meshes"][*mesh_index]["primitives"][*primitive_index]["indices"] =
                            Value::from(index as u64);
                        index
                    }
                };
                let target = root["accessors"]
                    .as_array_mut()
                    .and_then(|accessors| accessors.get_mut(accessor))
                    .unwrap();
                target["bufferView"] = Value::from(view as u64);
                target["byteOffset"] = Value::from(0u64);
                target["count"] = Value::from((mesh.num_faces() * 3) as u64);
                target["componentType"] = Value::from(5125u64);
                target["type"] = Value::from("SCALAR");
                let primitive = &mut root["meshes"][*mesh_index]["primitives"][*primitive_index];
                primitive["mode"] = Value::from(4u64);
                if let Some(extensions) = primitive
                    .get_mut("extensions")
                    .and_then(Value::as_object_mut)
                {
                    if let Some(index) = extensions
                        .iter()
                        .position(|(name, _)| name == crate::KHR_DRACO_MESH_COMPRESSION)
                    {
                        extensions.remove(index);
                    }
                }
            }
            root["buffers"]
                .as_array_mut()
                .ok_or_else(|| Error::Extension("buffers is not an array".into()))?
                .push(Value::object([("byteLength", Value::from(bytes.len()))]));
            let has_draco = root["meshes"]
                .as_array()
                .unwrap_or(&[])
                .iter()
                .flat_map(|mesh| {
                    mesh.get("primitives")
                        .and_then(Value::as_array)
                        .unwrap_or(&[])
                })
                .any(|primitive| {
                    primitive
                        .get("extensions")
                        .and_then(|extensions| extensions.get(crate::KHR_DRACO_MESH_COMPRESSION))
                        .is_some()
                });
            if !has_draco {
                for name in ["extensionsUsed", "extensionsRequired"] {
                    if let Some(values) = root.get_mut(name).and_then(Value::as_array_mut) {
                        values.retain(|value| {
                            value.as_str() != Some(crate::KHR_DRACO_MESH_COMPRESSION)
                        });
                    }
                }
            }
        }
        self.resources.buffers.push(bytes);
        Ok(())
    }

    /// Lists declared glTF 2.1 `files` entries without resolving them.
    #[cfg(feature = "resources")]
    pub fn external_files(&self) -> impl Iterator<Item = FileIndex> + '_ {
        self.document.files().into_iter().map(|file| file.index())
    }

    /// Explicitly resolves and parses an external-asset model declaration.
    #[cfg(feature = "resources")]
    pub fn load_external_asset(
        &self,
        asset: ExternalAssetIndex,
        resolver: &dyn ResourceResolver,
        limits: &ResourceLimits,
        profile: ValidationProfile,
        extensions: &ExtensionRegistry,
    ) -> Result<Self> {
        let file = self
            .document
            .external_asset(asset)
            .and_then(|asset| asset.file())
            .ok_or_else(|| {
                Error::Extension(format!("external asset {} is out of range", asset.0))
            })?;
        self.load_asset(file, resolver, limits, profile, extensions)
    }

    /// URI chain leading to this import. It is intended for diagnostics and
    /// explicit cycle detection; it never triggers recursive loading itself.
    #[cfg(feature = "resources")]
    pub fn provenance(&self) -> &[String] {
        &self.provenance
    }

    /// Explicitly resolves and parses one nested glTF file.
    #[cfg(feature = "resources")]
    pub fn load_asset(
        &self,
        file: FileIndex,
        resolver: &dyn ResourceResolver,
        limits: &ResourceLimits,
        profile: ValidationProfile,
        extensions: &ExtensionRegistry,
    ) -> Result<Self> {
        self.load_asset_with_depth(
            file,
            resolver,
            limits,
            profile,
            extensions,
            DEFAULT_EXTERNAL_ASSET_DEPTH,
        )
    }

    /// Explicitly loads one nested asset with a caller-selected graph depth limit.
    #[cfg(feature = "resources")]
    pub fn load_asset_with_depth(
        &self,
        file: FileIndex,
        resolver: &dyn ResourceResolver,
        limits: &ResourceLimits,
        profile: ValidationProfile,
        extensions: &ExtensionRegistry,
        max_depth: usize,
    ) -> Result<Self> {
        let max_depth = limits
            .max_external_asset_depth
            .map_or(max_depth, |limit| limit.min(max_depth));
        if self.provenance.len() >= max_depth {
            return Err(Error::ResourceLimit(format!(
                "nested glTF asset depth exceeds {max_depth}"
            )));
        }
        let entry = self
            .document
            .file(file)
            .ok_or_else(|| Error::Extension(format!("file {} is out of range", file.0)))?;
        let packaged = entry.buffer_view().is_some()
            || entry.uri().is_some_and(|uri| uri.starts_with("data:"));
        let source = entry.uri().map(str::to_owned).unwrap_or_else(|| {
            format!(
                "bufferView:{}",
                entry.value()["bufferView"].as_u64().unwrap_or(u64::MAX)
            )
        });
        if self.provenance.iter().any(|ancestor| ancestor == &source) {
            return Err(Error::Extension(format!(
                "cyclic external glTF asset reference: {source}"
            )));
        }
        let bytes = if let Some(uri) = entry.uri() {
            draco_io::resolve_resource_uri(uri, Some(resolver), limits.max_resource_bytes)?
        } else {
            self.embedded_file_bytes(entry.value())?
        };
        let mut loaded = if packaged {
            let packaged_resolver = PackagedResolver {
                import: self,
                fallback: resolver,
            };
            parse_with_options(
                &bytes,
                None,
                Some(&packaged_resolver),
                limits,
                profile,
                extensions,
            )?
        } else {
            parse_with_options(&bytes, None, Some(resolver), limits, profile, extensions)?
        };
        loaded.provenance = self.provenance.clone();
        loaded.provenance.push(source);
        Ok(loaded)
    }

    #[cfg(feature = "resources")]
    fn embedded_file_bytes(&self, file: &Value) -> Result<Vec<u8>> {
        let view = file
            .get("bufferView")
            .and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok())
            .ok_or_else(|| Error::Extension("file has neither uri nor bufferView".into()))?;
        let view = self
            .document
            .buffer_view(crate::BufferViewIndex(view))
            .ok_or_else(|| Error::Extension("file bufferView is out of range".into()))?;
        let buffer = view
            .buffer()
            .ok_or_else(|| Error::Extension("file bufferView has no buffer".into()))?;
        let bytes = self
            .resources
            .buffers
            .get(buffer.0)
            .ok_or_else(|| Error::ResourceLimit("file buffer is not materialized".into()))?;
        let start = usize::try_from(view.byte_offset())
            .map_err(|_| Error::ResourceLimit("file byteOffset exceeds this platform".into()))?;
        let length = view
            .byte_length()
            .and_then(|length| usize::try_from(length).ok())
            .ok_or_else(|| Error::Extension("file bufferView has no byteLength".into()))?;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| Error::Extension("file bufferView is outside its buffer".into()))?;
        Ok(bytes[start..end].to_vec())
    }
}

#[cfg(feature = "transform")]
pub(crate) fn decoded_attribute_bytes(mesh: &draco_core::Mesh, unique_id: u32) -> Result<Vec<u8>> {
    let attribute = mesh.attribute_by_unique_id(unique_id).ok_or_else(|| {
        Error::Extension(format!("decoded Draco attribute {unique_id} is missing"))
    })?;
    let stride = usize::try_from(attribute.byte_stride())
        .map_err(|_| Error::Extension("decoded attribute stride is invalid".into()))?;
    let mut out =
        vec![
            0;
            mesh.num_points()
                .checked_mul(stride)
                .ok_or_else(|| Error::ResourceLimit("decoded attribute size overflow".into()))?
        ];
    let mut row = vec![0; stride];
    for point in 0..mesh.num_points() {
        let index = attribute.mapped_index(draco_core::PointIndex(point as u32));
        if !attribute
            .buffer()
            .try_read(index.0 as usize * stride, &mut row)
        {
            return Err(Error::Extension(
                "decoded attribute is out of bounds".into(),
            ));
        }
        out[point * stride..(point + 1) * stride].copy_from_slice(&row);
    }
    Ok(out)
}

#[cfg(feature = "transform")]
pub(crate) fn decoded_index_bytes(mesh: &draco_core::Mesh) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(mesh.num_faces() * 12);
    for face in 0..mesh.num_faces() {
        for point in mesh.face(draco_core::FaceIndex(face as u32)) {
            out.extend_from_slice(&point.0.to_le_bytes());
        }
    }
    Ok(out)
}

#[cfg(feature = "transform")]
fn append_view(root: &mut Value, buffer: usize, bytes: &mut Vec<u8>, data: &[u8]) -> Result<usize> {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    let offset = bytes.len();
    bytes.extend_from_slice(data);
    let views = root["bufferViews"]
        .as_array_mut()
        .ok_or_else(|| Error::Extension("bufferViews is not an array".into()))?;
    let index = views.len();
    views.push(Value::object([
        ("buffer", Value::from(buffer)),
        ("byteOffset", Value::from(offset)),
        ("byteLength", Value::from(data.len())),
    ]));
    Ok(index)
}

#[cfg(feature = "transform")]
fn clone_accessor(root: &mut Value, index: usize) -> Result<usize> {
    let accessors = root["accessors"]
        .as_array_mut()
        .ok_or_else(|| Error::Extension("accessors is not an array".into()))?;
    let source = accessors
        .get(index)
        .cloned()
        .ok_or_else(|| Error::Extension("Draco accessor out of range".into()))?;
    let clone = accessors.len();
    accessors.push(source);
    Ok(clone)
}

pub fn parse(bytes: &[u8], profile: ValidationProfile) -> Result<Import> {
    parse_with_options(
        bytes,
        None,
        None,
        &ResourceLimits::default(),
        profile,
        &ExtensionRegistry::default(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub fn open(path: impl AsRef<Path>, profile: ValidationProfile) -> Result<Import> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    let resolver = FileResourceResolver::new(
        path.parent().unwrap_or_else(|| Path::new(".")),
        ExternalFilePolicy::ConfineToBase,
    );
    parse_with_options(
        &bytes,
        path.parent(),
        Some(&resolver),
        &ResourceLimits::default(),
        profile,
        &ExtensionRegistry::default(),
    )
}

pub fn parse_with_options(
    bytes: &[u8],
    _base: Option<&Path>,
    resolver: Option<&dyn ResourceResolver>,
    limits: &ResourceLimits,
    profile: ValidationProfile,
    extensions: &ExtensionRegistry,
) -> Result<Import> {
    let container = parse_gltf_container(bytes)?;
    let document = Document::from_json_bytes(container.json)?;
    document.validate(profile)?;
    extensions.validate(&document)?;
    let mut references = Vec::new();
    for buffer in document.buffers() {
        let uri = buffer.value().get("uri").and_then(Value::as_str);
        let byte_length = buffer
            .value()
            .get("byteLength")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                Error::Validation(vec![format!(
                    "buffer {} byteLength is invalid",
                    buffer.index().0
                )])
            })?;
        references.push(GltfBufferReference { uri, byte_length });
    }
    let buffers = resolve_gltf_buffers(
        &references,
        container.format,
        container.bin,
        resolver,
        limits,
    )?;
    Ok(Import {
        document,
        resources: ResourceStore { buffers },
        input_format: container.format,
        profile,
        extensions: extensions.clone(),
        #[cfg(feature = "resources")]
        provenance: Vec::new(),
    })
}
