use std::path::Path;

use crate::json::Value;

use crate::{
    Document, Error, ExtensionRegistry, FileIndex, PrimitiveRef, ResourceStore, Result,
    ValidationProfile,
};
use draco_io::{
    parse_gltf_container, resolve_gltf_buffers, GltfBufferReference, GltfContainerFormat,
    ResourceLimits, ResourceResolver,
};
#[cfg(not(target_arch = "wasm32"))]
use draco_io::{ExternalFilePolicy, FileResourceResolver};

/// Native, lossless glTF import independent of `gltf-rs`.
pub struct NativeImport {
    pub document: Document,
    pub resources: ResourceStore,
    pub input_format: GltfContainerFormat,
    profile: ValidationProfile,
    extensions: ExtensionRegistry,
}

impl NativeImport {
    pub fn validate(&self, extensions: &ExtensionRegistry) -> Result<()> {
        self.document.validate(self.profile)?;
        extensions.validate(&self.document)?;
        Ok(())
    }

    /// Iterates primitives carrying the built-in Draco extension.
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

    /// Decodes a primitive through the supplied native extension registry.
    pub fn decode_primitive(&self, primitive: PrimitiveRef<'_>) -> Result<draco_core::Mesh> {
        self.validate(&self.extensions)?;
        self.extensions
            .decode_primitive(&self.document, &self.resources, primitive)
    }

    /// Decodes an ordinary (non-Draco) triangle or point primitive through the
    /// same geometry contract used by compact consumers.
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
        let source = crate::NativeAccessorSource::new(&self.document, &self.resources);
        Ok(draco_io::decode_geometry(
            &source,
            mode,
            &attributes,
            indices,
        )?)
    }

    pub fn to_bytes(&self, output: crate::OutputFormat) -> Result<Vec<u8>> {
        let json = self.document.to_json_bytes()?;
        let format = match output {
            crate::OutputFormat::GltfJson => return Ok(json),
            crate::OutputFormat::SameAsInput => self.input_format,
            crate::OutputFormat::GlbV2 => draco_io::GltfContainerFormat::GlbV2,
            crate::OutputFormat::GlbV3 => draco_io::GltfContainerFormat::GlbV3,
        };
        if format.is_glb() {
            let bin = self
                .resources
                .buffers
                .first()
                .map_or(&[][..], Vec::as_slice);
            return Ok(draco_io::gltf_container::build_glb_from_json(
                &json, bin, format,
            )?);
        }
        Ok(json)
    }

    /// Materializes all Draco primitives as ordinary indexed triangle geometry.
    pub fn decompress_in_place(&mut self) -> Result<()> {
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
                    let accessor = root["meshes"][*mesh_index]["primitives"][*primitive_index]
                        ["attributes"][semantic]
                        .as_u64()
                        .and_then(|value| usize::try_from(value).ok())
                        .ok_or_else(|| {
                            Error::Extension(format!("Draco attribute {semantic} has no accessor"))
                        })?;
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
                let accessor = root["meshes"][*mesh_index]["primitives"][*primitive_index]
                    .get("indices")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok());
                let accessor = match accessor {
                    Some(index) => index,
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
            for name in ["extensionsUsed", "extensionsRequired"] {
                if let Some(values) = root.get_mut(name).and_then(Value::as_array_mut) {
                    values
                        .retain(|value| value.as_str() != Some(crate::KHR_DRACO_MESH_COMPRESSION));
                }
            }
        }
        self.resources.buffers.push(bytes);
        Ok(())
    }

    /// Lists declared glTF 2.1 `files` entries without resolving them.
    pub fn external_assets(&self) -> impl Iterator<Item = FileIndex> + '_ {
        self.document.files().into_iter().map(|file| file.index())
    }

    /// Explicitly resolves and parses one nested glTF file.
    pub fn load_asset(
        &self,
        file: FileIndex,
        resolver: &dyn ResourceResolver,
        limits: &ResourceLimits,
        profile: ValidationProfile,
        extensions: &ExtensionRegistry,
    ) -> Result<Self> {
        let uri = self
            .document
            .files()
            .get(file)
            .and_then(|file| file.value().get("uri"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Extension(format!("file {} has no external URI", file.0)))?;
        let bytes = draco_io::resolve_resource_uri(uri, Some(resolver), limits.max_resource_bytes)?;
        parse_native_with_options(&bytes, None, Some(resolver), limits, profile, extensions)
    }
}

fn decoded_attribute_bytes(mesh: &draco_core::Mesh, unique_id: u32) -> Result<Vec<u8>> {
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

fn decoded_index_bytes(mesh: &draco_core::Mesh) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(mesh.num_faces() * 12);
    for face in 0..mesh.num_faces() {
        for point in mesh.face(draco_core::FaceIndex(face as u32)) {
            out.extend_from_slice(&point.0.to_le_bytes());
        }
    }
    Ok(out)
}

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

pub fn parse_native(bytes: &[u8], profile: ValidationProfile) -> Result<NativeImport> {
    parse_native_with_options(
        bytes,
        None,
        None,
        &ResourceLimits::default(),
        profile,
        &ExtensionRegistry::default(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub fn open_native(path: impl AsRef<Path>, profile: ValidationProfile) -> Result<NativeImport> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    let resolver = FileResourceResolver::new(
        path.parent().unwrap_or_else(|| Path::new(".")),
        ExternalFilePolicy::ConfineToBase,
    );
    parse_native_with_options(
        &bytes,
        path.parent(),
        Some(&resolver),
        &ResourceLimits::default(),
        profile,
        &ExtensionRegistry::default(),
    )
}

pub fn parse_native_with_options(
    bytes: &[u8],
    _base: Option<&Path>,
    resolver: Option<&dyn ResourceResolver>,
    limits: &ResourceLimits,
    profile: ValidationProfile,
    extensions: &ExtensionRegistry,
) -> Result<NativeImport> {
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
    Ok(NativeImport {
        document,
        resources: ResourceStore { buffers },
        input_format: container.format,
        profile,
        extensions: extensions.clone(),
    })
}
