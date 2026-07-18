use crate::{Error, Import, Result};
use draco_core::{
    draco_types::DataType, encoder_buffer::EncoderBuffer, encoder_options::EncoderOptions,
    mesh_encoder::MeshEncoder,
};

/// How the exported primitive exposes its Draco payload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompressionMode {
    /// Preserve ordinary accessors as a non-Draco fallback. The extension is
    /// listed in `extensionsUsed`, never `extensionsRequired`.
    Fallback,
    /// Require Draco and remove ordinary geometry payloads owned solely by
    /// the transformed primitive.
    #[default]
    DracoOnly,
}

fn add_extension_name(root: &mut crate::JsonValue, field: &str) -> Result<()> {
    if root.get(field).is_none() {
        root[field] = crate::JsonValue::Array(Vec::new());
    }
    let list = root[field]
        .as_array_mut()
        .ok_or_else(|| Error::Validation(vec![format!("{field} is not an array")]))?;
    if !list
        .iter()
        .any(|value| value.as_str() == Some(crate::KHR_DRACO_MESH_COMPRESSION))
    {
        list.push(crate::JsonValue::from(crate::KHR_DRACO_MESH_COMPRESSION));
    }
    Ok(())
}

fn detach_draco_only_accessors(
    root: &mut crate::JsonValue,
    mesh_index: usize,
    primitive_index: usize,
    mapping: &[(String, u32)],
    layout: &DracoGeometryLayout,
) -> Result<()> {
    for (semantic, unique_id) in mapping {
        let source = root["meshes"][mesh_index]["primitives"][primitive_index]["attributes"]
            [semantic.as_str()]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| Error::Extension(format!("Draco attribute {semantic} has no accessor")))?;
        let accessor = clone_accessor(root, source)?;
        root["meshes"][mesh_index]["primitives"][primitive_index]["attributes"]
            [semantic.as_str()] = crate::JsonValue::from(accessor as u64);
        let attribute = layout
            .attributes
            .iter()
            .find(|attribute| attribute.unique_id == *unique_id)
            .ok_or_else(|| {
                Error::Extension(format!("encoded Draco attribute {unique_id} is missing"))
            })?;
        set_draco_accessor_layout(
            root,
            accessor,
            layout.points,
            attribute.components,
            attribute.data_type,
        )?;
    }

    let source = root["meshes"][mesh_index]["primitives"][primitive_index]
        .get("indices")
        .and_then(crate::JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok());
    let accessor = match source {
        Some(source) => clone_accessor(root, source)?,
        None => {
            let accessors = root["accessors"]
                .as_array_mut()
                .ok_or_else(|| Error::Validation(vec!["accessors is not an array".into()]))?;
            let index = accessors.len();
            accessors.push(crate::JsonValue::Object(Vec::new()));
            index
        }
    };
    root["meshes"][mesh_index]["primitives"][primitive_index]["indices"] =
        crate::JsonValue::from(accessor as u64);
    set_draco_accessor_layout(root, accessor, layout.faces * 3, 1, DataType::Uint32)?;
    Ok(())
}

#[derive(Clone, Copy)]
struct DracoAttributeLayout {
    unique_id: u32,
    components: u8,
    data_type: DataType,
}

struct DracoGeometryLayout {
    points: usize,
    faces: usize,
    attributes: Vec<DracoAttributeLayout>,
}

impl DracoGeometryLayout {
    fn from_mesh(mesh: &draco_core::Mesh, mapping: &[(String, u32)]) -> Result<Self> {
        let attributes = mapping
            .iter()
            .map(|(_, unique_id)| {
                let attribute = mesh.attribute_by_unique_id(*unique_id).ok_or_else(|| {
                    Error::Extension(format!("encoded Draco attribute {unique_id} is missing"))
                })?;
                Ok(DracoAttributeLayout {
                    unique_id: *unique_id,
                    components: attribute.num_components(),
                    data_type: attribute.data_type(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            points: mesh.num_points(),
            faces: mesh.num_faces(),
            attributes,
        })
    }
}

fn clone_accessor(root: &mut crate::JsonValue, source: usize) -> Result<usize> {
    let accessors = root["accessors"]
        .as_array_mut()
        .ok_or_else(|| Error::Validation(vec!["accessors is not an array".into()]))?;
    let source = accessors
        .get(source)
        .cloned()
        .ok_or_else(|| Error::Extension("Draco accessor out of range".into()))?;
    let index = accessors.len();
    accessors.push(source);
    Ok(index)
}

fn set_draco_accessor_layout(
    root: &mut crate::JsonValue,
    index: usize,
    count: usize,
    components: u8,
    data_type: DataType,
) -> Result<()> {
    let component_type = match data_type {
        DataType::Int8 => 5120,
        DataType::Uint8 => 5121,
        DataType::Int16 => 5122,
        DataType::Uint16 => 5123,
        DataType::Uint32 => 5125,
        DataType::Float32 => 5126,
        _ => {
            return Err(Error::Extension(format!(
                "Draco attribute data type {data_type:?} cannot be represented by glTF 2.0"
            )))
        }
    };
    let accessor_type = match components {
        1 => "SCALAR",
        2 => "VEC2",
        3 => "VEC3",
        4 => "VEC4",
        _ => {
            return Err(Error::Extension(format!(
                "Draco attribute component count {components} cannot be represented by glTF"
            )))
        }
    };
    let accessor = root["accessors"]
        .as_array_mut()
        .and_then(|values| values.get_mut(index))
        .ok_or_else(|| Error::Extension("Draco accessor out of range".into()))?;
    accessor["count"] = crate::JsonValue::from(count);
    accessor["componentType"] = crate::JsonValue::from(component_type as u64);
    accessor["type"] = crate::JsonValue::from(accessor_type);
    if let Some(object) = accessor.as_object_mut() {
        object.retain(|(name, _)| name != "bufferView" && name != "byteOffset" && name != "sparse");
    }
    Ok(())
}

fn compact_draco_only_resources(
    document: &mut crate::Document,
    buffers: &mut Vec<Vec<u8>>,
    extensions: &crate::ExtensionRegistry,
    max_output_bytes: Option<usize>,
) -> Result<()> {
    let used_accessors = collect_used_accessors(document.as_value(), extensions, document)?;
    let accessor_map = {
        let root = document.as_value_mut();
        let accessor_map = prune_accessors(root, &used_accessors)?;
        remap_accessor_references(root, &accessor_map)?;
        accessor_map
    };
    let view_identity = (0..document.buffer_views().len())
        .map(Some)
        .collect::<Vec<_>>();
    extensions.remap_binary_references(document, &accessor_map, &view_identity)?;

    let used_views = collect_used_views(document.as_value(), extensions, document)?;
    let old_views = document.as_value_mut()["bufferViews"]
        .as_array_mut()
        .ok_or_else(|| Error::Validation(vec!["bufferViews is not an array".into()]))
        .map(std::mem::take)?;
    if old_views.len() != used_views.len() {
        return Err(Error::Validation(vec![
            "bufferViews changed while compacting Draco resources".into(),
        ]));
    }
    #[derive(Clone, Copy)]
    struct Range {
        buffer: usize,
        start: usize,
        end: usize,
        output_offset: usize,
    }

    let mut ranges = Vec::new();
    for (index, view) in old_views.iter().enumerate() {
        if !used_views[index] {
            continue;
        }
        let buffer_index = view
            .get("buffer")
            .and_then(crate::JsonValue::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| Error::Validation(vec!["bufferView buffer is invalid".into()]))?;
        let buffer = buffers
            .get(buffer_index)
            .ok_or_else(|| Error::Validation(vec!["bufferView buffer is invalid".into()]))?;
        let start = view
            .get("byteOffset")
            .and_then(crate::JsonValue::as_u64)
            .unwrap_or(0);
        let start = usize::try_from(start)
            .map_err(|_| Error::ResourceLimit("bufferView offset exceeds this platform".into()))?;
        let length = view
            .get("byteLength")
            .and_then(crate::JsonValue::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| Error::Validation(vec!["bufferView byteLength is invalid".into()]))?;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= buffer.len())
            .ok_or_else(|| Error::Validation(vec!["bufferView range is invalid".into()]))?;
        ranges.push((
            index,
            Range {
                buffer: buffer_index,
                start,
                end,
                output_offset: 0,
            },
        ));
    }
    ranges.sort_unstable_by_key(|(_, range)| (range.buffer, range.start, range.end));
    let mut blocks = Vec::<Range>::new();
    let mut block_for_view = vec![usize::MAX; old_views.len()];
    for (view, range) in ranges {
        let last_index = blocks.len().checked_sub(1);
        let coalesces = last_index.is_some_and(|index| {
            let last = blocks[index];
            last.buffer == range.buffer && range.start <= last.end
        });
        if coalesces {
            let index = last_index.expect("coalescing has a preceding range");
            blocks[index].end = blocks[index].end.max(range.end);
            block_for_view[view] = index;
        } else {
            block_for_view[view] = blocks.len();
            blocks.push(range);
        }
    }
    let mut compacted = Vec::new();
    for block in &mut blocks {
        let padding = (4 - compacted.len() % 4) % 4;
        reserve_output(&mut compacted, padding, max_output_bytes)?;
        compacted.resize(compacted.len() + padding, 0);
        block.output_offset = compacted.len();
        let length = block.end - block.start;
        reserve_output(&mut compacted, length, max_output_bytes)?;
        compacted.extend_from_slice(&buffers[block.buffer][block.start..block.end]);
    }
    let mut view_map = vec![None; old_views.len()];
    let mut new_views = Vec::new();
    for (index, mut view) in old_views.into_iter().enumerate() {
        if !used_views[index] {
            continue;
        }
        let start = view
            .get("byteOffset")
            .and_then(crate::JsonValue::as_u64)
            .map(usize::try_from)
            .transpose()
            .map_err(|_| Error::ResourceLimit("bufferView offset exceeds this platform".into()))?
            .unwrap_or(0);
        let block = blocks
            .get(block_for_view[index])
            .ok_or_else(|| Error::Validation(vec!["bufferView range was not planned".into()]))?;
        view["buffer"] = crate::JsonValue::from(0usize);
        view["byteOffset"] = crate::JsonValue::from(block.output_offset + start - block.start);
        view_map[index] = Some(new_views.len());
        new_views.push(view);
    }
    {
        let root = document.as_value_mut();
        root["bufferViews"] = crate::JsonValue::Array(new_views);
        remap_buffer_view_references(root, &view_map)?;
    }
    let accessor_identity = (0..document.accessors().len())
        .map(Some)
        .collect::<Vec<_>>();
    extensions.remap_binary_references(document, &accessor_identity, &view_map)?;
    document.as_value_mut()["buffers"] =
        crate::JsonValue::Array(vec![crate::JsonValue::object([(
            "byteLength",
            crate::JsonValue::from(compacted.len()),
        )])]);
    *buffers = vec![compacted];
    Ok(())
}

fn reserve_output(
    output: &mut Vec<u8>,
    additional: usize,
    max_output_bytes: Option<usize>,
) -> Result<()> {
    let total = output
        .len()
        .checked_add(additional)
        .ok_or_else(|| Error::ResourceLimit("compressed output size overflow".into()))?;
    if max_output_bytes.is_some_and(|limit| total > limit) {
        return Err(Error::ResourceLimit(format!(
            "compressed output size {total} exceeds configured limit"
        )));
    }
    output
        .try_reserve(additional)
        .map_err(|_| Error::ResourceLimit("unable to reserve compressed output".into()))?;
    Ok(())
}

fn collect_used_accessors(
    root: &crate::JsonValue,
    extensions: &crate::ExtensionRegistry,
    document: &crate::Document,
) -> Result<Vec<bool>> {
    let mut used = vec![
        false;
        root["accessors"]
            .as_array()
            .map_or(0, <[crate::JsonValue]>::len)
    ];
    visit_core_accessor_refs(root, &mut used)?;
    let mut buffer_views = vec![
        false;
        root["bufferViews"]
            .as_array()
            .map_or(0, <[crate::JsonValue]>::len)
    ];
    extensions.collect_binary_references(document, &mut used, &mut buffer_views)?;
    Ok(used)
}

fn prune_accessors(root: &mut crate::JsonValue, used: &[bool]) -> Result<Vec<Option<usize>>> {
    let accessors = root["accessors"]
        .as_array_mut()
        .ok_or_else(|| Error::Validation(vec!["accessors is not an array".into()]))?;
    if accessors.len() != used.len() {
        return Err(Error::Validation(vec![
            "accessor count changed while compacting".into(),
        ]));
    }
    let old = std::mem::take(accessors);
    let mut map = vec![None; old.len()];
    for (index, accessor) in old.into_iter().enumerate() {
        if used[index] {
            map[index] = Some(accessors.len());
            accessors.push(accessor);
        }
    }
    Ok(map)
}

fn remap_accessor_references(root: &mut crate::JsonValue, map: &[Option<usize>]) -> Result<()> {
    remap_core_accessor_refs(root, map)
}

fn collect_used_views(
    root: &crate::JsonValue,
    extensions: &crate::ExtensionRegistry,
    document: &crate::Document,
) -> Result<Vec<bool>> {
    let mut used = vec![
        false;
        root["bufferViews"]
            .as_array()
            .map_or(0, <[crate::JsonValue]>::len)
    ];
    visit_core_buffer_view_refs(root, &mut used)?;
    let mut accessors = vec![
        false;
        root["accessors"]
            .as_array()
            .map_or(0, <[crate::JsonValue]>::len)
    ];
    extensions.collect_binary_references(document, &mut accessors, &mut used)?;
    Ok(used)
}

fn remap_buffer_view_references(root: &mut crate::JsonValue, map: &[Option<usize>]) -> Result<()> {
    remap_core_buffer_view_refs(root, map)
}

fn visit_core_accessor_refs(root: &crate::JsonValue, used: &mut [bool]) -> Result<()> {
    for mesh in root
        .get("meshes")
        .and_then(crate::JsonValue::as_array)
        .unwrap_or(&[])
    {
        for primitive in mesh
            .get("primitives")
            .and_then(crate::JsonValue::as_array)
            .unwrap_or(&[])
        {
            for (_, value) in primitive
                .get("attributes")
                .and_then(crate::JsonValue::as_object)
                .unwrap_or(&[])
            {
                mark_used(value, used, "accessor")?;
            }
            if let Some(value) = primitive.get("indices") {
                mark_used(value, used, "accessor")?;
            }
            for target in primitive
                .get("targets")
                .and_then(crate::JsonValue::as_array)
                .unwrap_or(&[])
            {
                for (_, value) in target.as_object().unwrap_or(&[]) {
                    mark_used(value, used, "accessor")?;
                }
            }
        }
    }
    for skin in root
        .get("skins")
        .and_then(crate::JsonValue::as_array)
        .unwrap_or(&[])
    {
        if let Some(value) = skin.get("inverseBindMatrices") {
            mark_used(value, used, "accessor")?;
        }
    }
    for animation in root
        .get("animations")
        .and_then(crate::JsonValue::as_array)
        .unwrap_or(&[])
    {
        for sampler in animation
            .get("samplers")
            .and_then(crate::JsonValue::as_array)
            .unwrap_or(&[])
        {
            mark_used(&sampler["input"], used, "accessor")?;
            mark_used(&sampler["output"], used, "accessor")?;
        }
    }
    Ok(())
}

fn visit_core_buffer_view_refs(root: &crate::JsonValue, used: &mut [bool]) -> Result<()> {
    for accessor in root
        .get("accessors")
        .and_then(crate::JsonValue::as_array)
        .unwrap_or(&[])
    {
        if let Some(value) = accessor.get("bufferView") {
            mark_used(value, used, "bufferView")?;
        }
        if let Some(sparse) = accessor.get("sparse") {
            mark_used(&sparse["indices"]["bufferView"], used, "bufferView")?;
            mark_used(&sparse["values"]["bufferView"], used, "bufferView")?;
        }
    }
    for name in ["images", "files"] {
        for object in root
            .get(name)
            .and_then(crate::JsonValue::as_array)
            .unwrap_or(&[])
        {
            if let Some(value) = object.get("bufferView") {
                mark_used(value, used, "bufferView")?;
            }
        }
    }
    Ok(())
}

fn remap_core_accessor_refs(root: &mut crate::JsonValue, map: &[Option<usize>]) -> Result<()> {
    if let Some(meshes) = root
        .get_mut("meshes")
        .and_then(crate::JsonValue::as_array_mut)
    {
        for mesh in meshes {
            if let Some(primitives) = mesh
                .get_mut("primitives")
                .and_then(crate::JsonValue::as_array_mut)
            {
                for primitive in primitives {
                    if let Some(values) = primitive
                        .get_mut("attributes")
                        .and_then(crate::JsonValue::as_object_mut)
                    {
                        for (_, value) in values {
                            remap_index(value, map, "accessor")?;
                        }
                    }
                    if let Some(value) = primitive.get_mut("indices") {
                        remap_index(value, map, "accessor")?;
                    }
                    if let Some(targets) = primitive
                        .get_mut("targets")
                        .and_then(crate::JsonValue::as_array_mut)
                    {
                        for target in targets {
                            if let Some(values) = target.as_object_mut() {
                                for (_, value) in values {
                                    remap_index(value, map, "accessor")?;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(skins) = root
        .get_mut("skins")
        .and_then(crate::JsonValue::as_array_mut)
    {
        for skin in skins {
            if let Some(value) = skin.get_mut("inverseBindMatrices") {
                remap_index(value, map, "accessor")?;
            }
        }
    }
    if let Some(animations) = root
        .get_mut("animations")
        .and_then(crate::JsonValue::as_array_mut)
    {
        for animation in animations {
            if let Some(samplers) = animation
                .get_mut("samplers")
                .and_then(crate::JsonValue::as_array_mut)
            {
                for sampler in samplers {
                    remap_index(&mut sampler["input"], map, "accessor")?;
                    remap_index(&mut sampler["output"], map, "accessor")?;
                }
            }
        }
    }
    Ok(())
}

fn remap_core_buffer_view_refs(root: &mut crate::JsonValue, map: &[Option<usize>]) -> Result<()> {
    if let Some(accessors) = root
        .get_mut("accessors")
        .and_then(crate::JsonValue::as_array_mut)
    {
        for accessor in accessors {
            if let Some(value) = accessor.get_mut("bufferView") {
                remap_index(value, map, "bufferView")?;
            }
            if let Some(sparse) = accessor.get_mut("sparse") {
                remap_index(&mut sparse["indices"]["bufferView"], map, "bufferView")?;
                remap_index(&mut sparse["values"]["bufferView"], map, "bufferView")?;
            }
        }
    }
    for name in ["images", "files"] {
        if let Some(values) = root.get_mut(name).and_then(crate::JsonValue::as_array_mut) {
            for value in values {
                if let Some(view) = value.get_mut("bufferView") {
                    remap_index(view, map, "bufferView")?;
                }
            }
        }
    }
    Ok(())
}

fn mark_used(value: &crate::JsonValue, used: &mut [bool], kind: &str) -> Result<()> {
    let index = value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|index| *index < used.len())
        .ok_or_else(|| Error::Validation(vec![format!("{kind} reference is invalid")]))?;
    used[index] = true;
    Ok(())
}

fn remap_index(value: &mut crate::JsonValue, map: &[Option<usize>], kind: &str) -> Result<()> {
    let old = value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| Error::Validation(vec![format!("{kind} reference is invalid")]))?;
    let new = map
        .get(old)
        .and_then(|value| *value)
        .ok_or_else(|| Error::Validation(vec![format!("{kind} reference was removed")]))?;
    *value = crate::JsonValue::from(new);
    Ok(())
}

#[derive(Clone, Copy, Debug)]
/// Controls document-preserving Draco compression.
///
/// [`CompressionMode::DracoOnly`] is the default: it requires the Draco
/// extension and removes raw geometry owned only by the compressed primitive.
/// Use [`CompressionMode::Fallback`] when non-Draco readers must retain the
/// original accessors.
pub struct CompressionOptions {
    /// Draco encoder speed in the range accepted by `draco-core`.
    pub encoding_speed: u8,
    /// Draco decoder speed hint in the range accepted by `draco-core`.
    pub decoding_speed: u8,
    /// Whether output requires Draco or retains ordinary geometry as fallback.
    pub mode: CompressionMode,
    /// Maximum number of resolved binary bytes permitted after compression.
    /// The limit includes retained fallback data and four-byte padding.
    pub max_output_bytes: Option<usize>,
}
impl Default for CompressionOptions {
    fn default() -> Self {
        Self {
            encoding_speed: 5,
            decoding_speed: 5,
            mode: CompressionMode::DracoOnly,
            max_output_bytes: None,
        }
    }
}
#[derive(Clone, Debug, Default)]
/// Measured result of one [`Import::compress_primitive`] operation.
pub struct CompressionReport {
    /// Export policy used for the transformed primitive.
    pub mode: CompressionMode,
    /// Number of primitives encoded by the operation.
    pub compressed_primitives: usize,
    /// Bytes in the newly encoded Draco payload.
    pub encoded_bytes: usize,
    /// Total resolved binary bytes before the transform.
    pub source_bytes: usize,
    /// Total resolved binary bytes after the transform.
    pub output_bytes: usize,
    /// Bytes removed from the resolved binary store; zero for a fallback that
    /// retains all ordinary geometry.
    pub reclaimed_bytes: usize,
}

impl Import {
    /// Encodes an already decoded mesh to a raw Draco payload.
    pub fn encode_draco_mesh(
        &self,
        mesh: draco_core::Mesh,
        options: CompressionOptions,
    ) -> Result<Vec<u8>> {
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh);
        let mut settings = EncoderOptions::new();
        settings.set_global_int("encoding_speed", options.encoding_speed as i32);
        settings.set_global_int("decoding_speed", options.decoding_speed as i32);
        let mut output = EncoderBuffer::new();
        encoder
            .encode(&settings, &mut output)
            .map_err(|error| Error::Extension(error.to_string()))?;
        Ok(output.data().to_vec())
    }

    /// Compresses one ordinary triangle primitive atomically.
    ///
    /// The document and resolved resources are updated only after encoding,
    /// validation, reference remapping, and output-limit checks all succeed.
    /// In [`CompressionMode::DracoOnly`] the operation rejects unregistered
    /// extensions whose binary references cannot be remapped safely.
    pub fn compress_primitive(
        &mut self,
        mesh: crate::MeshIndex,
        primitive: usize,
        options: CompressionOptions,
    ) -> Result<CompressionReport> {
        let mut candidate = self.clone();
        let report = candidate.compress_primitive_inner(mesh, primitive, options)?;
        *self = candidate;
        Ok(report)
    }

    fn compress_primitive_inner(
        &mut self,
        mesh: crate::MeshIndex,
        primitive: usize,
        options: CompressionOptions,
    ) -> Result<CompressionReport> {
        let source_bytes = self
            .resources
            .buffers
            .iter()
            .try_fold(0usize, |total, buffer| {
                total
                    .checked_add(buffer.len())
                    .ok_or_else(|| Error::ResourceLimit("total source buffer size overflow".into()))
            })?;
        let reference = self
            .document
            .primitive(mesh, primitive)
            .ok_or_else(|| Error::Extension("primitive out of range".into()))?;
        self.ensure_transform_safe(reference)?;
        if options.mode == CompressionMode::DracoOnly {
            self.ensure_document_binary_transform_safe()?;
        }
        if reference.mode() != 4 {
            return Err(Error::Extension(
                "KHR_draco_mesh_compression encoding currently supports only TRIANGLES (mode 4)"
                    .into(),
            ));
        }
        let (geometry, mapping) = self.decode_geometry_primitive(reference)?;
        let layout = DracoGeometryLayout::from_mesh(&geometry, &mapping)?;
        let bytes = self.encode_draco_mesh(geometry, options)?;
        let buffer = self.resources.buffers.len();
        let view;
        {
            let root = self.document.as_value_mut();
            let buffers = root["buffers"]
                .as_array_mut()
                .ok_or_else(|| Error::Extension("buffers is not an array".into()))?;
            buffers.push(crate::JsonValue::object([(
                "byteLength",
                crate::JsonValue::from(bytes.len()),
            )]));
            let views = root["bufferViews"]
                .as_array_mut()
                .ok_or_else(|| Error::Extension("bufferViews is not an array".into()))?;
            view = views.len();
            views.push(crate::JsonValue::object([
                ("buffer", crate::JsonValue::from(buffer)),
                ("byteLength", crate::JsonValue::from(bytes.len())),
            ]));
            let attributes = crate::JsonValue::Object(
                mapping
                    .iter()
                    .map(|(name, id)| (name.clone(), crate::JsonValue::from(*id as u64)))
                    .collect(),
            );
            root["meshes"][mesh.0]["primitives"][primitive]["extensions"]
                [crate::KHR_DRACO_MESH_COMPRESSION] = crate::JsonValue::object([
                ("bufferView", crate::JsonValue::from(view)),
                ("attributes", attributes),
            ]);
            add_extension_name(root, "extensionsUsed")?;
            if options.mode == CompressionMode::DracoOnly {
                add_extension_name(root, "extensionsRequired")?;
            }
            if options.mode == CompressionMode::DracoOnly {
                detach_draco_only_accessors(root, mesh.0, primitive, &mapping, &layout)?;
            }
        }
        self.resources.buffers.push(bytes.clone());
        if options.mode == CompressionMode::DracoOnly {
            compact_draco_only_resources(
                &mut self.document,
                &mut self.resources.buffers,
                &self.extensions,
                options.max_output_bytes,
            )?;
        }
        let output_bytes = self
            .resources
            .buffers
            .iter()
            .try_fold(0usize, |total, buffer| {
                total
                    .checked_add(buffer.len())
                    .ok_or_else(|| Error::ResourceLimit("total output buffer size overflow".into()))
            })?;
        if let Some(limit) = options.max_output_bytes {
            if output_bytes > limit {
                return Err(Error::ResourceLimit(format!(
                    "compressed output size {output_bytes} exceeds limit {limit}"
                )));
            }
        }
        Ok(CompressionReport {
            mode: options.mode,
            compressed_primitives: 1,
            encoded_bytes: bytes.len(),
            source_bytes,
            output_bytes,
            reclaimed_bytes: source_bytes.saturating_sub(output_bytes),
        })
    }
}
