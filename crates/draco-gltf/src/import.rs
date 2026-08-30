use std::path::Path;

use crate::json::Value;

use crate::extensions::{meshopt_extension, meshopt_extension_mut};
#[cfg(feature = "draco-decode")]
use crate::PrimitiveRef;
use crate::{Document, Error, ExtensionRegistry, ResourceStore, Result, ValidationProfile};
#[cfg(feature = "resources")]
use crate::{ExternalAssetIndex, FileIndex};
use draco_io::{
    meshopt, parse_gltf_container, resolve_gltf_buffers, GltfBufferReference, GltfContainerFormat,
    MeshoptFilter, MeshoptMode, ResourceLimits, ResourceResolver,
};
#[cfg(not(target_arch = "wasm32"))]
use draco_io::{ExternalFilePolicy, FileResourceResolver};

/// Lossless glTF document plus its resolved resources.
#[derive(Clone)]
pub struct Import {
    /// Lossless parsed glTF document.
    pub document: Document,
    /// Resolved buffer resources indexed by document buffer index.
    pub resources: ResourceStore,
    /// Container format from which this import was read.
    pub input_format: GltfContainerFormat,
    profile: ValidationProfile,
    #[cfg(any(feature = "draco-decode", feature = "draco-encode"))]
    pub(crate) extensions: ExtensionRegistry,
    /// Caller's ceilings on what one Draco decode may produce, applied to
    /// every primitive this import decodes -- directly, through
    /// `decompress_in_place`, and in nested assets parsed from this one.
    ///
    /// Gated where the *storage* is: without the decoder nothing reads it.
    /// The parameter that carries it is not gated -- an options struct and a
    /// function signature should not change shape with a feature.
    #[cfg(feature = "draco-decode")]
    pub(crate) draco_decode_limits: draco_core::DecodeLimits,
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
    /// Serialized JSON document bytes.
    pub json: Vec<u8>,
    /// Companion resources to write relative to the JSON document.
    pub resources: Vec<GltfResource>,
}

/// One companion resource produced by [`Import::to_gltf_output`].
#[derive(Clone, Debug)]
pub struct GltfResource {
    /// Relative URI assigned to the companion resource.
    pub uri: String,
    /// Resource bytes.
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
    #[cfg(feature = "write")]
    pub(crate) const fn validation_profile(&self) -> ValidationProfile {
        self.profile
    }

    #[cfg(feature = "write")]
    pub(crate) fn validate_after_write(&self) -> Result<()> {
        self.document.validate(self.profile)?;
        #[cfg(feature = "draco-decode")]
        self.extensions.validate(&self.document)?;
        Ok(())
    }

    /// Validates the document and all registered extension handlers.
    ///
    /// With `strict-validation`, this also checks the complete scene graph.
    pub fn validate(&self, extensions: &ExtensionRegistry) -> Result<()> {
        self.document.validate(self.profile)?;
        extensions.validate(&self.document)?;
        Ok(())
    }

    #[cfg(all(feature = "write", feature = "draco-decode"))]
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

    #[cfg(feature = "draco-encode")]
    pub(crate) fn ensure_document_binary_transform_safe(&self) -> Result<()> {
        fn visit(value: &Value, registry: &ExtensionRegistry) -> Result<()> {
            match value {
                Value::Array(values) => {
                    for value in values {
                        visit(value, registry)?;
                    }
                }
                Value::Object(values) => {
                    for (name, value) in values {
                        if name == "extensions" {
                            let extensions = value.as_object().ok_or_else(|| {
                                Error::Extension("extensions is not an object".into())
                            })?;
                            for (extension, _) in extensions {
                                if !registry.allows_binary_transform(extension) {
                                    return Err(Error::Extension(format!(
                                        "cannot produce Draco-only output with extension {extension:?}: its binary-reference semantics are not registered as transform-safe"
                                    )));
                                }
                            }
                        }
                        visit(value, registry)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        visit(self.document.as_value(), &self.extensions)
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
    pub fn decode_draco_primitive(&self, primitive: PrimitiveRef<'_>) -> Result<draco_core::Mesh> {
        self.validate(&self.extensions)?;
        let mesh = self.extensions.decode_primitive(
            &self.document,
            &self.resources,
            &self.draco_decode_limits,
            primitive,
        )?;
        self.validate_decoded_draco_counts(primitive, &mesh)?;
        Ok(mesh)
    }

    #[cfg(feature = "draco-decode")]
    fn validate_decoded_draco_counts(
        &self,
        primitive: PrimitiveRef<'_>,
        mesh: &draco_core::Mesh,
    ) -> Result<()> {
        let decoded_points = u64::try_from(mesh.num_points())
            .map_err(|_| Error::ResourceLimit("decoded Draco point count exceeds u64".into()))?;
        for (semantic, index) in primitive.attribute_indices() {
            let declared = self
                .document
                .accessor(index)
                .and_then(|accessor| accessor.count())
                .ok_or_else(|| {
                    Error::Validation(vec![format!(
                        "Draco attribute {semantic:?} accessor count is missing"
                    )])
                })?;
            // Only an accessor that promises more vertices than the stream can
            // supply is fatal: the missing ones have nowhere to come from.
            //
            // The other direction is what real encoders emit. Draco stores
            // connectivity per position vertex and re-splits it at attribute
            // seams while decoding, so a mesh whose normals or texture
            // coordinates break along an edge decodes to more points than the
            // accessor written before compression declares. glTF-Pipeline,
            // Blender and the Draco encoder itself all produce such files —
            // Three.js's ferrari.glb among them — and every browser viewer
            // reads them, because the decoded geometry is self-consistent:
            // indices, positions and attributes all come out of the same
            // stream. Refusing them would reject working files over metadata
            // the extension has already superseded.
            if declared > decoded_points {
                return Err(crate::GeometryError::DracoAccessorCount {
                    semantic: semantic.into(),
                    decoded: decoded_points,
                    declared,
                }
                .into());
            }
        }

        if primitive.mode() == crate::PrimitiveMode::Triangles.to_gltf() {
            if let Some(index) = primitive.indices() {
                let declared = self
                    .document
                    .accessor(index)
                    .and_then(|accessor| accessor.count())
                    .ok_or_else(|| {
                        Error::Validation(vec!["Draco index accessor count is missing".into()])
                    })?;
                let decoded = mesh
                    .num_faces()
                    .checked_mul(3)
                    .and_then(|count| u64::try_from(count).ok())
                    .ok_or_else(|| {
                        Error::ResourceLimit("decoded Draco index count exceeds u64".into())
                    })?;
                if declared != decoded {
                    return Err(crate::GeometryError::DracoAccessorCount {
                        semantic: "indices".into(),
                        decoded,
                        declared,
                    }
                    .into());
                }
            }
        }
        Ok(())
    }

    /// Reads one ordinary or Draco-compressed primitive into packed buffers.
    ///
    /// Sparse overlays and byte strides are materialized without changing
    /// component types or normalization flags. Draco is decoded only when the
    /// `draco-decode` feature is enabled.
    #[cfg(feature = "geometry")]
    pub fn read_primitive(
        &self,
        primitive: crate::PrimitiveIndex,
    ) -> Result<crate::PackedGeometry> {
        let reference = self
            .document
            .primitive(primitive.mesh, primitive.primitive)
            .ok_or_else(|| Error::Extension("primitive out of range".into()))?;
        let mode = crate::PrimitiveMode::from_gltf(reference.mode()).ok_or_else(|| {
            Error::Geometry(crate::GeometryError::InvalidPrimitiveMode(reference.mode()))
        })?;
        if reference
            .extension(crate::KHR_DRACO_MESH_COMPRESSION)
            .is_some()
        {
            #[cfg(feature = "draco-decode")]
            {
                let contract = crate::extensions::parse_draco_extension(
                    reference.extension(crate::KHR_DRACO_MESH_COMPRESSION),
                )?
                .ok_or_else(|| Error::Extension("missing Draco extension".into()))?;
                let decoded = self.decode_draco_primitive(reference)?;
                // The accessor, not the Draco attribute, defines how the
                // decoded integers are read. KHR_draco_mesh_compression makes
                // the accessor authoritative, and encoders leave the Draco
                // flag unset, so a normalized COLOR_0 would otherwise reach the
                // consumer as raw 0..65535 values.
                let normalized: std::collections::BTreeMap<String, bool> = reference
                    .attribute_indices()
                    .map(|(semantic, index)| {
                        let normalized = self
                            .document
                            .accessor(index)
                            .is_some_and(crate::Accessor::normalized);
                        (semantic.to_owned(), normalized)
                    })
                    .collect();
                let geometry = crate::PackedGeometry::from_draco_mesh(
                    mode,
                    &decoded,
                    &contract.attributes,
                    &normalized,
                )?;
                geometry.validate(self.profile)?;
                return Ok(geometry);
            }
            #[cfg(not(feature = "draco-decode"))]
            return Err(Error::Extension(
                "Draco primitive reading requires feature draco-decode".into(),
            ));
        }

        let source = crate::DocumentAccessorSource::new(&self.document, &self.resources);
        let attributes = reference
            .attribute_indices()
            .map(|(semantic, index)| {
                let data = source.read_geometry_accessor(index.0)?;
                let component_type = crate::ComponentType::from_gltf(data.component_type as u64)
                    .ok_or_else(|| {
                        Error::Extension(format!(
                            "unsupported accessor componentType {}",
                            data.component_type
                        ))
                    })?;
                crate::PackedAttribute::new(
                    semantic,
                    data.count,
                    data.components,
                    component_type,
                    data.normalized,
                    data.bytes,
                )
                // Only the uncompressed path can name a source accessor. A
                // Draco primitive's bytes come from its own codec stream, so
                // two primitives naming one accessor say nothing about whether
                // their vertex data is the same.
                .map(|attribute| attribute.with_source_accessor(index.0))
                .map_err(Error::Geometry)
            })
            .collect::<Result<Vec<_>>>()?;
        let indices = reference
            .indices()
            .map(|index| {
                let data = source.read_geometry_accessor(index.0)?;
                let component_type = crate::ComponentType::from_gltf(data.component_type as u64)
                    .ok_or_else(|| {
                        Error::Extension(format!(
                            "unsupported index componentType {}",
                            data.component_type
                        ))
                    })?;
                crate::PackedIndices::new(data.count, component_type, data.bytes)
                    .map(|indices| indices.with_source_accessor(index.0))
                    .map_err(Error::Geometry)
            })
            .transpose()?;
        let geometry = crate::PackedGeometry::new(mode, attributes, indices)?;
        geometry.validate(self.profile)?;
        Ok(geometry)
    }

    /// Decodes an ordinary (non-Draco) triangle or point primitive through the
    /// same packed geometry contract used by readers and writers.
    #[cfg(feature = "draco-encode")]
    pub(crate) fn decode_geometry_primitive(
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

    /// Serializes this import into the requested container format.
    ///
    /// [`crate::OutputFormat::GltfJson`] is valid only when every materialized
    /// buffer already has an embedded or external URI. For transformed scenes
    /// that need newly generated companion buffers, use
    /// [`Import::to_gltf_output`] instead. GLB output embeds all resolved
    /// buffers in one binary chunk.
    ///
    /// ```
    /// # use draco_gltf::{import_slice, OutputFormat};
    /// # let input = br#"{"asset":{"version":"2.0"},"buffers":[],"meshes":[]}"#;
    /// let scene = import_slice(input, None)?;
    /// let glb = scene.to_bytes(OutputFormat::GlbV2)?;
    /// assert_eq!(&glb[0..4], b"glTF");
    /// # Ok::<(), draco_gltf::Error>(())
    /// ```
    pub fn to_bytes(&self, output: crate::OutputFormat) -> Result<Vec<u8>> {
        let format = match output {
            crate::OutputFormat::GltfJson => {
                if self.document.buffers().into_iter().any(|buffer| {
                    buffer.value().get("uri").and_then(Value::as_str).is_none()
                        && self
                            .resources
                            .buffers
                            .get(buffer.index().0)
                            .is_some_and(|bytes| !bytes.is_empty())
                }) {
                    return Err(Error::Extension(
                        "GltfJson cannot carry materialized companion buffers; use to_gltf_output()"
                            .into(),
                    ));
                }
                return self.document.to_json_bytes();
            }
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
    ///
    /// ```
    /// # use draco_gltf::import_slice;
    /// # let input = br#"{"asset":{"version":"2.0"},"buffers":[],"meshes":[]}"#;
    /// let scene = import_slice(input, None)?;
    /// let output = scene.to_gltf_output()?;
    /// assert!(!output.json.is_empty());
    /// assert!(output.resources.is_empty());
    /// # Ok::<(), draco_gltf::Error>(())
    /// ```
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
                // The compressed range of a meshopt view names its own buffer,
                // so it has to follow the view onto the consolidated buffer.
                if let Some((name, extension)) = meshopt_extension_mut(view.get_mut("extensions")) {
                    rebase_buffer_reference(
                        extension,
                        &offsets,
                        &format!("bufferViews[{index}].extensions.{name}"),
                    )?;
                }
                rebase_buffer_reference(view, &offsets, &format!("bufferViews[{index}]"))?;
            }
        }
        root["buffers"] = Value::Array(vec![Value::object([(
            "byteLength",
            Value::from(bin.len()),
        )])]);
        Ok((document.to_json_bytes()?, bin))
    }

    /// Materializes all Draco primitives as ordinary indexed triangle geometry.
    #[cfg(all(feature = "write", feature = "draco-decode"))]
    pub fn decompress_in_place(&mut self) -> Result<()> {
        let mut candidate = self.clone();
        candidate.decompress_in_place_inner()?;
        *self = candidate;
        Ok(())
    }

    #[cfg(all(feature = "write", feature = "draco-decode"))]
    fn decompress_in_place_inner(&mut self) -> Result<()> {
        let mut primitives = Vec::new();
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
                if primitive
                    .extension(crate::KHR_DRACO_MESH_COMPRESSION)
                    .is_none()
                {
                    continue;
                }
                self.ensure_transform_safe(primitive)?;
                primitives.push(crate::PrimitiveIndex::new(mesh.index(), primitive_index));
            }
        }
        for primitive in primitives {
            let geometry = self.read_primitive(primitive)?;
            self.write_raw_primitive_inner(primitive, &geometry)?;
        }
        self.document.validate(self.profile)?;
        self.extensions.validate(&self.document)?;
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
                #[cfg(feature = "draco-decode")]
                &self.draco_decode_limits,
                #[cfg(not(feature = "draco-decode"))]
                &draco_core::DecodeLimits::default(),
                profile,
                extensions,
            )?
        } else {
            parse_with_options(
                &bytes,
                None,
                Some(resolver),
                limits,
                #[cfg(feature = "draco-decode")]
                &self.draco_decode_limits,
                #[cfg(not(feature = "draco-decode"))]
                &draco_core::DecodeLimits::default(),
                profile,
                extensions,
            )?
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

/// Rebases one `{buffer, byteOffset}` pair onto the consolidated GLB buffer.
///
/// `offsets` holds where each declared buffer starts in the merged binary
/// chunk, indexed the way the document declared them.
fn rebase_buffer_reference(value: &mut Value, offsets: &[usize], label: &str) -> Result<()> {
    let buffer = value
        .get("buffer")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| Error::Validation(vec![format!("{label}.buffer is not a valid index")]))?;
    let prefix = *offsets.get(buffer).ok_or_else(|| {
        Error::Validation(vec![format!(
            "{label}.buffer references missing buffer {buffer}"
        )])
    })?;
    let offset = value.get("byteOffset").and_then(Value::as_u64).unwrap_or(0);
    let offset = usize::try_from(offset)
        .ok()
        .and_then(|offset| prefix.checked_add(offset))
        .ok_or_else(|| Error::ResourceLimit(format!("{label} byteOffset overflow")))?;
    value["buffer"] = Value::from(0usize);
    value["byteOffset"] = Value::from(offset);
    Ok(())
}

/// Expands every `EXT_meshopt_compression` buffer view into its target buffer.
///
/// The extension stores compressed views in a real buffer and points the plain
/// glTF view at a zero-filled fallback buffer, so decoding here keeps every
/// downstream accessor read unaware of the compression.
fn decode_meshopt_buffer_views(document: &Document, buffers: &mut [Vec<u8>]) -> Result<()> {
    let Some(views) = document
        .as_value()
        .get("bufferViews")
        .and_then(Value::as_array)
    else {
        return Ok(());
    };
    for (index, view) in views.iter().enumerate() {
        let Some((_, extension)) = meshopt_extension(view.get("extensions")) else {
            continue;
        };
        let fail = |message: &str| Error::Extension(format!("bufferViews[{index}]: {message}"));
        let number = |value: Option<&Value>| {
            value
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
        };

        let source_buffer = number(extension.get("buffer"))
            .filter(|buffer| *buffer < buffers.len())
            .ok_or_else(|| fail("meshopt buffer is invalid"))?;
        let source_offset = number(extension.get("byteOffset")).unwrap_or(0);
        let source_length = number(extension.get("byteLength"))
            .ok_or_else(|| fail("meshopt byteLength is invalid"))?;
        let source = buffers[source_buffer]
            .get(source_offset..)
            .and_then(|bytes| bytes.get(..source_length))
            .ok_or_else(|| fail("meshopt range is outside its buffer"))?
            .to_vec();

        let count =
            number(extension.get("count")).ok_or_else(|| fail("meshopt count is invalid"))?;
        let stride = number(extension.get("byteStride"))
            .ok_or_else(|| fail("meshopt byteStride is invalid"))?;
        let mode = MeshoptMode::from_name(
            extension
                .get("mode")
                .and_then(Value::as_str)
                .ok_or_else(|| fail("meshopt mode is missing"))?,
        )?;
        let filter = match extension.get("filter").and_then(Value::as_str) {
            Some(name) => MeshoptFilter::from_name(name)?,
            None => MeshoptFilter::None,
        };

        let target_buffer = number(view.get("buffer"))
            .filter(|buffer| *buffer < buffers.len())
            .ok_or_else(|| fail("buffer is invalid"))?;
        let target_offset = number(view.get("byteOffset")).unwrap_or(0);
        let target_length =
            number(view.get("byteLength")).ok_or_else(|| fail("byteLength is invalid"))?;
        let target = buffers[target_buffer]
            .get_mut(target_offset..)
            .and_then(|bytes| bytes.get_mut(..target_length))
            .ok_or_else(|| fail("buffer view is outside its buffer"))?;

        meshopt::decode_buffer_view(target, &source, mode, filter, count, stride)?;
    }
    Ok(())
}

/// Parses glTF or GLB bytes and applies the selected profile's basic checks.
///
/// Enable `strict-validation` to validate all cross-references before loading.
pub fn parse(bytes: &[u8], profile: ValidationProfile) -> Result<Import> {
    parse_with_options(
        bytes,
        None,
        None,
        &ResourceLimits::default(),
        &draco_core::DecodeLimits::default(),
        profile,
        &ExtensionRegistry::default(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
/// Opens a glTF or GLB file and applies the selected profile's basic checks.
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
        &draco_core::DecodeLimits::default(),
        profile,
        &ExtensionRegistry::default(),
    )
}

/// Parses a container with explicit resource, quota, profile and extension options.
///
/// `draco_limits` travels with the import rather than being set on it
/// afterwards. The difference matters only if parsing ever decodes something:
/// it does not today, and an import built with the defaults and corrected a
/// line later would be right by that fact alone, which is not a thing to rely
/// on.
#[cfg_attr(not(feature = "draco-decode"), allow(unused_variables))]
pub fn parse_with_options(
    bytes: &[u8],
    _base: Option<&Path>,
    resolver: Option<&dyn ResourceResolver>,
    limits: &ResourceLimits,
    draco_limits: &draco_core::DecodeLimits,
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
        let meshopt_fallback = meshopt_extension(buffer.value().get("extensions"))
            .and_then(|(_, value)| value.get("fallback"))
            .is_some_and(|value| matches!(value, Value::Bool(true)));
        references.push(GltfBufferReference {
            uri,
            byte_length,
            meshopt_fallback,
        });
    }
    let mut buffers = resolve_gltf_buffers(
        &references,
        container.format,
        container.bin,
        resolver,
        limits,
    )?;
    decode_meshopt_buffer_views(&document, &mut buffers)?;
    Ok(Import {
        document,
        resources: ResourceStore { buffers },
        input_format: container.format,
        profile,
        #[cfg(any(feature = "draco-decode", feature = "draco-encode"))]
        extensions: extensions.clone(),
        #[cfg(feature = "draco-decode")]
        draco_decode_limits: *draco_limits,
        #[cfg(feature = "resources")]
        provenance: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use draco_io::gltf_container::build_glb_from_json;

    /// One vertex of four zero deltas, so the decoded value is the tail
    /// baseline `[1, 2, 3, 4]`. Every byte group uses the literal encoding.
    fn meshopt_vertex_stream() -> Vec<u8> {
        let mut stream = vec![0xa0u8];
        for _ in 0..4 {
            stream.push(0x03);
            stream.resize(stream.len() + 16, 0);
        }
        stream.resize(stream.len() + 28, 0);
        stream.extend_from_slice(&[1, 2, 3, 4]);
        stream
    }

    #[test]
    fn meshopt_buffer_views_decode_into_the_fallback_buffer() {
        let bin = meshopt_vertex_stream();
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},
            "extensionsUsed":["EXT_meshopt_compression"],
            "extensionsRequired":["EXT_meshopt_compression"],
            "buffers":[{{"byteLength":{}}},
                       {{"byteLength":4,"extensions":{{"EXT_meshopt_compression":{{"fallback":true}}}}}}],
            "bufferViews":[{{"buffer":1,"byteOffset":0,"byteLength":4,"byteStride":4,
                "extensions":{{"EXT_meshopt_compression":{{"buffer":0,"byteOffset":0,"byteLength":{},
                "byteStride":4,"mode":"ATTRIBUTES","count":1}}}}}}]}}"#,
            bin.len(),
            bin.len()
        );
        let glb = build_glb_from_json(json.as_bytes(), &bin, GltfContainerFormat::GlbV2).unwrap();

        let import = parse(&glb, ValidationProfile::Gltf20).unwrap();

        assert_eq!(import.resources.buffers[1], vec![1, 2, 3, 4]);
    }

    /// gltfpack wrote `KHR_meshopt_compression` before the extension was
    /// ratified under the `EXT_` prefix, and assets carrying that spelling are
    /// still in circulation. The extension object, the bitstream and the
    /// fallback convention are identical, so refusing them refuses a file over
    /// its name: the fallback buffer has no URI, and a reader that does not
    /// recognise the extension sees a buffer it has no reason to accept.
    #[test]
    fn the_pre_ratification_extension_name_decodes_the_same_way() {
        let bin = meshopt_vertex_stream();
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},
            "extensionsUsed":["KHR_meshopt_compression"],
            "extensionsRequired":["KHR_meshopt_compression"],
            "buffers":[{{"byteLength":{}}},
                       {{"byteLength":4,"extensions":{{"KHR_meshopt_compression":{{"fallback":true}}}}}}],
            "bufferViews":[{{"buffer":1,"byteOffset":0,"byteLength":4,"byteStride":4,
                "extensions":{{"KHR_meshopt_compression":{{"buffer":0,"byteOffset":0,"byteLength":{},
                "byteStride":4,"mode":"ATTRIBUTES","count":1}}}}}}]}}"#,
            bin.len(),
            bin.len()
        );
        let glb = build_glb_from_json(json.as_bytes(), &bin, GltfContainerFormat::GlbV2).unwrap();

        let import = parse(&glb, ValidationProfile::Gltf20).unwrap();

        assert_eq!(import.resources.buffers[1], vec![1, 2, 3, 4]);
    }

    /// GLB output merges every declared buffer into one binary chunk, so a
    /// compressed range that does not start the chunk only survives when the
    /// extension's own offsets are rebased along with the buffer view's.
    #[test]
    fn glb_output_rebases_a_meshopt_source_buffer_that_is_not_first() {
        let stream = meshopt_vertex_stream();
        let uri: String = stream
            .iter()
            .map(|byte| format!("%{byte:02X}"))
            .collect::<Vec<_>>()
            .concat();
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},
            "extensionsUsed":["EXT_meshopt_compression"],
            "extensionsRequired":["EXT_meshopt_compression"],
            "buffers":[{{"byteLength":4,"extensions":{{"EXT_meshopt_compression":{{"fallback":true}}}}}},
                       {{"byteLength":{},"uri":"data:application/octet-stream,{uri}"}}],
            "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":4,"byteStride":4,
                "extensions":{{"EXT_meshopt_compression":{{"buffer":1,"byteOffset":0,"byteLength":{},
                "byteStride":4,"mode":"ATTRIBUTES","count":1}}}}}}]}}"#,
            stream.len(),
            stream.len()
        );

        let import = parse(json.as_bytes(), ValidationProfile::Gltf20).unwrap();
        assert_eq!(import.resources.buffers[0], vec![1, 2, 3, 4]);

        let glb = import.to_bytes(crate::OutputFormat::GlbV2).unwrap();
        let reimported = parse(&glb, ValidationProfile::Gltf20).unwrap();

        assert_eq!(
            &reimported.resources.buffers[0][..4],
            &[1, 2, 3, 4],
            "the consolidated buffer must still decode to the same vertex"
        );
    }

    #[test]
    fn an_optional_extension_keeps_the_stored_fallback_data() {
        // Without `extensionsRequired` the fallback buffer holds real data, so
        // it must still be resolved from its URI rather than zeroed.
        let json = r#"{"asset":{"version":"2.0"},
            "extensionsUsed":["EXT_meshopt_compression"],
            "buffers":[{"byteLength":4,"uri":"data:application/octet-stream;base64,AQIDBA==",
                "extensions":{"EXT_meshopt_compression":{"fallback":true}}}]}"#;

        let import = parse(json.as_bytes(), ValidationProfile::Gltf20).unwrap();

        assert_eq!(import.resources.buffers[0], vec![1, 2, 3, 4]);
    }
}
