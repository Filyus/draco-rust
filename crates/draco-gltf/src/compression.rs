use crate::geometry::{sh_coefficient_indices, KHR_GAUSSIAN_SPLATTING_PREFIX};
use crate::{Error, Import, Result};
use draco_core::{
    encoder_buffer::EncoderBuffer,
    encoder_options::EncoderOptions,
    geometry_attribute::GeometryAttributeType,
    mesh_encoder::{EncodedAttributeInfo, EncodedMeshInfo, MeshEncoder},
    quantization_utils::{Dequantizer, Quantizer},
};

/// How the exported primitive exposes its Draco payload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompressionMode {
    /// Keep an ordinary-accessor copy for readers without Draco. The extension
    /// is listed in `extensionsUsed`, never `extensionsRequired`.
    ///
    /// The copy is the decoded stream, since both readers share the accessors.
    /// Source accessors stay, as other primitives may reference them.
    Fallback,
    /// Require Draco and remove ordinary geometry payloads owned solely by
    /// the transformed primitive.
    #[default]
    DracoOnly,
}

fn add_extension_name(root: &mut crate::JsonValue, field: &str, name: &str) -> Result<()> {
    if root.get(field).is_none() {
        root[field] = crate::JsonValue::Array(Vec::new());
    }
    let list = root[field]
        .as_array_mut()
        .ok_or_else(|| Error::Validation(vec![format!("{field} is not an array")]))?;
    if !list.iter().any(|value| value.as_str() == Some(name)) {
        list.push(crate::JsonValue::from(name));
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
    if mapping.len() != layout.attributes.len() {
        return Err(Error::Extension(
            "decoded Draco attributes do not match the extension".into(),
        ));
    }
    for ((semantic, _), attribute) in mapping.iter().zip(&layout.attributes) {
        let source = root["meshes"][mesh_index]["primitives"][primitive_index]["attributes"]
            [semantic.as_str()]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| Error::Extension(format!("Draco attribute {semantic} has no accessor")))?;
        let accessor = clone_accessor(root, source)?;
        root["meshes"][mesh_index]["primitives"][primitive_index]["attributes"]
            [semantic.as_str()] = crate::JsonValue::from(accessor as u64);
        set_draco_accessor_layout(
            root,
            accessor,
            layout.points,
            attribute.components,
            attribute.component_type,
            attribute.position_bounds.as_ref(),
        )?;
    }

    // A point cloud has no faces, so there is no index accessor to detach --
    // and one the source primitive did carry, whose points the encode
    // gathered, must go: it names a bufferView the compressed primitive has
    // no use for, and a count the stream's points may not match. The
    // fallback's own rewrite drops it the same way.
    if layout.point_cloud {
        let primitive = &mut root["meshes"][mesh_index]["primitives"][primitive_index];
        if let Some(entries) = primitive.as_object_mut() {
            entries.retain(|(name, _)| name != "indices");
        }
        return Ok(());
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
    set_draco_accessor_layout(
        root,
        accessor,
        layout.index_count,
        1,
        crate::ComponentType::U32,
        None,
    )?;
    Ok(())
}

/// What one encoded attribute's accessor has to declare.
#[derive(Clone)]
pub(crate) struct DracoAttributeLayout {
    pub(crate) components: u8,
    pub(crate) component_type: crate::ComponentType,
    pub(crate) position_bounds: Option<(Vec<f64>, Vec<f64>)>,
}

/// What the accessors of an encoded primitive declare, from the encoder's
/// report: the spec requires them to "match the decompressed data".
pub(crate) struct DracoGeometryLayout {
    /// Whether the payload is a point cloud, which has no indices to detach.
    pub(crate) point_cloud: bool,
    pub(crate) points: usize,
    pub(crate) index_count: usize,
    /// In the order of the extension's attribute map.
    pub(crate) attributes: Vec<DracoAttributeLayout>,
}

impl DracoGeometryLayout {
    pub(crate) fn from_encoded_info(
        info: &EncodedMeshInfo,
        mapping: &[(String, u32)],
    ) -> Result<Self> {
        let attributes = mapping
            .iter()
            .map(|(semantic, unique_id)| {
                let attribute = info
                    .attributes
                    .iter()
                    .find(|attribute| attribute.unique_id == *unique_id)
                    .ok_or_else(|| {
                        Error::Extension(format!("encoded Draco attribute {unique_id} is missing"))
                    })?;
                let position_bounds = if semantic == "POSITION" {
                    Some((
                        attribute.position_min.clone().ok_or_else(|| {
                            Error::Extension("encoded POSITION min bounds are missing".into())
                        })?,
                        attribute.position_max.clone().ok_or_else(|| {
                            Error::Extension("encoded POSITION max bounds are missing".into())
                        })?,
                    ))
                } else {
                    None
                };
                // glTF requires POSITION bounds and requires them finite, and
                // an unquantized stream carries whatever floats it was given.
                if position_bounds
                    .as_ref()
                    .is_some_and(|(min, max)| min.iter().chain(max).any(|bound| !bound.is_finite()))
                {
                    return Err(Error::Extension(
                        "POSITION holds values that are not finite, which glTF bounds cannot state"
                            .into(),
                    ));
                }
                Ok(DracoAttributeLayout {
                    components: attribute.num_components,
                    component_type: gltf20_component_type(
                        crate::packed::component_type_for_data_type(attribute.data_type)?,
                    )?,
                    position_bounds,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let index_count = info
            .num_encoded_faces
            .checked_mul(3)
            .ok_or_else(|| Error::ResourceLimit("encoded index count overflows".into()))?;
        Ok(Self {
            point_cloud: false,
            points: info.num_encoded_points,
            index_count,
            attributes,
        })
    }

    /// The layout of a point-cloud encode, from its own decoded stream.
    ///
    /// A point cloud's encoder reports no POSITION bounds -- the kd-tree coder
    /// reports no per-attribute anything -- so the accessors are declared from
    /// what the stream decodes to, which is what the spec asks them to match
    /// anyway. The decode behind this is of the stream this crate just wrote,
    /// under permissive limits, and the caller checks its point count against
    /// what the encoder reported.
    pub(crate) fn from_decoded_point_cloud(
        decoded: &draco_core::Mesh,
        mapping: &[(String, u32)],
    ) -> Result<Self> {
        let attributes = mapping
            .iter()
            .map(|(semantic, unique_id)| {
                let attribute = decoded.attribute_by_unique_id(*unique_id).ok_or_else(|| {
                    Error::Extension(format!("decoded Draco attribute {unique_id} is missing"))
                })?;
                let position_bounds = if semantic == "POSITION" {
                    Some(decoded_position_bounds(attribute, decoded.num_points())?)
                } else {
                    None
                };
                if position_bounds
                    .as_ref()
                    .is_some_and(|(min, max)| min.iter().chain(max).any(|bound| !bound.is_finite()))
                {
                    return Err(Error::Extension(
                        "POSITION holds values that are not finite, which glTF bounds cannot state"
                            .into(),
                    ));
                }
                Ok(DracoAttributeLayout {
                    components: attribute.num_components(),
                    component_type: gltf20_component_type(
                        crate::packed::component_type_for_data_type(attribute.data_type())?,
                    )?,
                    position_bounds,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            point_cloud: true,
            points: decoded.num_points(),
            index_count: 0,
            attributes,
        })
    }
}

/// POSITION's bounds as its accessor declares them, from a decoded attribute
/// read a component at a time.
fn decoded_position_bounds(
    attribute: &draco_core::PointAttribute,
    points: usize,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let components = usize::from(attribute.num_components());
    let stride = usize::try_from(attribute.byte_stride())
        .map_err(|_| Error::Extension("decoded POSITION stride is invalid".into()))?;
    let width = attribute.data_type().byte_length();
    let data = attribute.buffer().data();
    let mut min = vec![f64::INFINITY; components];
    let mut max = vec![f64::NEG_INFINITY; components];
    for point in 0..points {
        let value_index = attribute
            .mapped_index(draco_core::PointIndex(point as u32))
            .0 as usize;
        for component in 0..components {
            let start = value_index
                .checked_mul(stride)
                .and_then(|start| start.checked_add(component * width))
                .ok_or_else(|| Error::Extension("decoded POSITION is out of bounds".into()))?;
            let bytes = data
                .get(start..start + width)
                .ok_or_else(|| Error::Extension("decoded POSITION is truncated".into()))?;
            let value = scalar_as_f64(attribute.data_type(), bytes);
            min[component] = min[component].min(value);
            max[component] = max[component].max(value);
        }
    }
    Ok((min, max))
}

/// One component as a double, which every Draco type fits without loss.
fn scalar_as_f64(data_type: draco_core::DataType, bytes: &[u8]) -> f64 {
    use draco_core::DataType as Dt;
    match data_type {
        Dt::Int8 => bytes[0] as i8 as f64,
        Dt::Uint8 | Dt::Bool => bytes[0] as f64,
        Dt::Int16 => i16::from_le_bytes(bytes.try_into().unwrap()) as f64,
        Dt::Uint16 => u16::from_le_bytes(bytes.try_into().unwrap()) as f64,
        Dt::Int32 => i32::from_le_bytes(bytes.try_into().unwrap()) as f64,
        Dt::Uint32 => u32::from_le_bytes(bytes.try_into().unwrap()) as f64,
        Dt::Int64 => i64::from_le_bytes(bytes.try_into().unwrap()) as f64,
        Dt::Uint64 => u64::from_le_bytes(bytes.try_into().unwrap()) as f64,
        Dt::Float32 => f32::from_le_bytes(bytes.try_into().unwrap()) as f64,
        Dt::Float64 => f64::from_le_bytes(bytes.try_into().unwrap()),
        Dt::Invalid => 0.0,
    }
}

/// A primitive encoded, with what its accessors declare.
pub(crate) struct EncodedPrimitive {
    pub(crate) bytes: Vec<u8>,
    /// `(numeric Draco method, speed)` the encoder settled on, for the report.
    pub(crate) encoding: (i32, i32),
    /// The per-attribute report, for the prediction summary. Empty where the
    /// encoder has no per-attribute choice to report -- the kd-tree point-cloud
    /// coder encodes every attribute through one coder.
    pub(crate) attributes: Vec<EncodedAttributeInfo>,
    /// `(glTF semantic, Draco unique id)`, the extension's attribute map.
    pub(crate) mapping: Vec<(String, u32)>,
    pub(crate) layout: DracoGeometryLayout,
}

/// Encodes a mesh built by `decode_geometry`.
pub(crate) fn encode_primitive(
    mesh: draco_core::Mesh,
    mapping: Vec<(String, u32)>,
    options: &CompressionOptions,
) -> Result<EncodedPrimitive> {
    let (bytes, info) = encode_draco_mesh(mesh, &mapping, options)?;
    let layout = DracoGeometryLayout::from_encoded_info(&info, &mapping)?;
    Ok(EncodedPrimitive {
        encoding: (info.encoding_method, info.speed),
        attributes: info.attributes,
        bytes,
        mapping,
        layout,
    })
}

/// Encodes a `POINTS` primitive's geometry through Draco's point-cloud coder.
///
/// The mesh carries no faces -- `decode_geometry` built it from a mode-0
/// primitive -- and `mesh.into_point_cloud` drops what it does not have, so
/// the same builder feeds both coders. The layout is declared from what the
/// stream decodes back to, not from an encoder report: the kd-tree coder has
/// no per-attribute report and neither coder reports POSITION bounds, and the
/// spec asks the accessors to match the decoded data anyway.
pub(crate) fn encode_point_cloud_primitive(
    mesh: draco_core::Mesh,
    mapping: Vec<(String, u32)>,
    options: &CompressionOptions,
) -> Result<EncodedPrimitive> {
    let num_points = mesh.num_points();
    let (bytes, info) = encode_draco_point_cloud(mesh, &mapping, options)?;
    let decoded = crate::draco_primitive::decode_payload(
        &bytes,
        &crate::DracoDecodeOptions::default().with_limits(draco_core::DecodeLimits::permissive()),
    )?;
    if decoded.num_points() != num_points {
        return Err(Error::Extension(format!(
            "Draco point-cloud encoder reported {num_points} points, its stream decodes to {}",
            decoded.num_points()
        )));
    }
    let layout = DracoGeometryLayout::from_decoded_point_cloud(&decoded, &mapping)?;
    Ok(EncodedPrimitive {
        encoding: (info.encoding_method, info.speed),
        attributes: info.attributes,
        bytes,
        mapping,
        layout,
    })
}

/// Encodes the built geometry through Draco's point-cloud coder.
///
/// Both paths resolve quantization against the glTF attribute mapping. The
/// three point-cloud options are read from
/// [`CompressionOptions`], which leaves them unset for a mesh encode: they
/// name a point order and a prediction search the mesh coder does not take.
fn encode_draco_point_cloud(
    mesh: draco_core::Mesh,
    mapping: &[(String, u32)],
    options: &CompressionOptions,
) -> Result<(Vec<u8>, draco_core::EncodedPointCloudInfo)> {
    use draco_core::point_cloud_encoder::PointCloudEncoder;

    let mut settings = EncoderOptions::new();
    settings.set_global_int("encoding_speed", options.encoding_speed as i32);
    settings.set_global_int("decoding_speed", options.decoding_speed as i32);
    let is_gaussian_splat = mapping
        .iter()
        .any(|(semantic, _)| semantic.starts_with(KHR_GAUSSIAN_SPLATTING_PREFIX));
    // The caller-facing method convention is the mesh one -- 1 sequential,
    // 2 the connectivity coder -- and a point cloud has no connectivity, so 2
    // lands on the kd-tree coder, the other thing the choice is between.
    // Gaussian splats carry many attributes with differing bit budgets, where
    // the kd-tree coder's shared bit depth inflates the stream, so auto mode
    // defaults them to sequential.
    match options.encoding_method {
        1 => settings.set_encoding_method(0), // force sequential
        2 => settings.set_encoding_method(1), // force kd-tree
        0 if is_gaussian_splat => settings.set_encoding_method(0),
        _ => {}
    }
    set_quantization_options(&mut settings, &mesh, mapping, options)?;
    if options.encoding_method == 2
        && options.gaussian_splat_quantization.rotation_bits.is_none()
        && mapping.iter().any(|(semantic, id)| {
            semantic == "KHR_gaussian_splatting:ROTATION"
                && mesh
                    .attribute_by_unique_id(*id)
                    .is_some_and(|attribute| attribute.data_type() == draco_core::DataType::Float32)
        })
    {
        return Err(Error::Extension(
            "kd-tree requires quantized float attributes; Gaussian splat ROTATION must retain its precision unless rotation_bits is set. Use auto, sequential, or set rotation_bits".into(),
        ));
    }
    settings.set_prediction_search(options.prediction_search);
    settings.set_spatial_point_order(options.spatial_point_order);
    settings.set_point_order_search(options.point_order_search);

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(mesh.into_point_cloud());
    let mut output = EncoderBuffer::new();
    encoder
        .encode(&settings, &mut output)
        .map_err(|error| Error::Extension(error.to_string()))?;
    let bytes = output.data().to_vec();
    let info = encoder
        .encoded_point_cloud_info()
        .cloned()
        .ok_or_else(|| Error::Extension("the point-cloud encoder reported nothing".into()))?;
    Ok((bytes, info))
}

/// Decodes a stream just encoded, for callers that need the data (a fallback
/// copy, the morph-target order check), and holds the encoder's report to it.
pub(crate) fn decode_encoded(
    bytes: &[u8],
    mapping: &[(String, u32)],
    reported: (usize, usize),
    normalized: &[(String, bool)],
    mode: crate::PrimitiveMode,
) -> Result<crate::PackedGeometry> {
    // Our own stream: the limits for hostile input do not apply.
    let mesh = crate::draco_primitive::decode_payload(
        bytes,
        &crate::DracoDecodeOptions::default().with_limits(draco_core::DecodeLimits::permissive()),
    )?;
    let decoded = crate::PackedGeometry::from_draco_mesh_with_attributes(
        &mesh,
        mapping,
        &|semantic| {
            // The last entry for a semantic wins, as it would in a map built
            // from the same list.
            normalized
                .iter()
                .rev()
                .find(|(name, _)| name == semantic)
                .map(|(_, normalized)| *normalized)
        },
        Vec::new(),
        mode,
    )?;
    let index_count = decoded.indices().map_or(0, crate::PackedIndices::count);
    if (decoded.vertex_count(), index_count) != reported {
        return Err(Error::Extension(format!(
            "Draco encoder reported {} points and {} indices, its stream decodes to {} and {}",
            reported.0,
            reported.1,
            decoded.vertex_count(),
            index_count
        )));
    }
    Ok(decoded)
}

/// Whether every vertex of `source` decodes at its own index, within the
/// position quantization.
pub(crate) fn keeps_vertex_order(
    source: &crate::PackedGeometry,
    decoded: &crate::PackedGeometry,
    position_bits: Option<u8>,
) -> bool {
    let position = |geometry: &crate::PackedGeometry| {
        geometry
            .attributes()
            .iter()
            .find(|attribute| attribute.semantic() == "POSITION")
            .filter(|attribute| attribute.component_type() == crate::ComponentType::F32)
            .map(|attribute| {
                attribute
                    .bytes()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|v| f32::from_le_bytes(*v))
                    .collect::<Vec<_>>()
            })
    };
    let (Some(source), Some(decoded)) = (position(source), position(decoded)) else {
        return false;
    };
    if source.len() != decoded.len() {
        return false;
    }
    // One quantization step over the largest extent bounds every coordinate.
    let tolerance = match position_bits {
        None => 0.0,
        Some(bits) => {
            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            for vertex in source.as_chunks::<3>().0 {
                for axis in 0..3 {
                    min[axis] = min[axis].min(vertex[axis]);
                    max[axis] = max[axis].max(vertex[axis]);
                }
            }
            let extent = (0..3).map(|axis| max[axis] - min[axis]).fold(0.0, f32::max);
            let steps = ((1u64 << bits.min(31)) - 1).max(1) as f32;
            extent / steps * 1.01
        }
    };
    source
        .iter()
        .zip(&decoded)
        .all(|(source, decoded)| (source - decoded).abs() <= tolerance)
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
    component_type: crate::ComponentType,
    position_bounds: Option<&(Vec<f64>, Vec<f64>)>,
) -> Result<()> {
    let component_type = gltf20_component_type(component_type)?.to_gltf();
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
        object.retain(|(name, _)| {
            !matches!(
                name.as_str(),
                "bufferView" | "byteOffset" | "sparse" | "min" | "max"
            )
        });
    }
    if let Some((min, max)) = position_bounds {
        accessor["min"] = crate::JsonValue::Array(
            min.iter()
                .map(|value| crate::JsonValue::Number(crate::writer::finite_float_lexeme(*value)))
                .collect(),
        );
        accessor["max"] = crate::JsonValue::Array(
            max.iter()
                .map(|value| crate::JsonValue::Number(crate::writer::finite_float_lexeme(*value)))
                .collect(),
        );
    }
    Ok(())
}

/// Refuses a component type glTF 2.0 cannot declare.
pub(crate) fn gltf20_component_type(
    component_type: crate::ComponentType,
) -> Result<crate::ComponentType> {
    match component_type {
        crate::ComponentType::I8
        | crate::ComponentType::U8
        | crate::ComponentType::I16
        | crate::ComponentType::U16
        | crate::ComponentType::U32
        | crate::ComponentType::F32 => Ok(component_type),
        other => Err(Error::Extension(format!(
            "Draco attribute type {other:?} cannot be represented by glTF 2.0"
        ))),
    }
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

/// Controls document-preserving Draco compression.
///
/// [`CompressionMode::DracoOnly`] is the default: it requires the Draco
/// extension and removes raw geometry owned only by the compressed primitive.
/// Use [`CompressionMode::Fallback`] when non-Draco readers must retain the
/// original accessors.
///
/// ```
/// # use draco_gltf::{CompressionMode, CompressionOptions};
/// let mut options = CompressionOptions::default();
/// options.mode = CompressionMode::Fallback;
/// assert_eq!(options.mode, CompressionMode::Fallback);
/// ```
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
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
    /// Quantization applied per attribute type, or `None` to leave an attribute
    /// in its original floating-point form.
    ///
    /// Without quantization an attribute never reaches Draco's integer coder,
    /// so no prediction scheme runs on it and the entropy stage has nothing to
    /// work with -- a compressed primitive stays close to its uncompressed size
    /// and the encoding speed stops making any difference to it. `None` is the
    /// default so that existing callers keep the bytes they already produce;
    /// anything writing assets for delivery wants values here.
    pub quantization: QuantizationBits,
    /// Overrides the type-wide `generic` bit count for Gaussian splat rotation,
    /// scale, opacity and SH. By default, Float32 rotation and integer splat
    /// accessors retain their precision; Float32 rotation can be quantized
    /// explicitly via [`GaussianSplatQuantization::rotation_bits`]. These
    /// settings apply when [`Self::compress_splats`] or the legacy
    /// [`Self::allow_points`] option enables point-cloud compression.
    pub gaussian_splat_quantization: GaussianSplatQuantization,
    /// Explicit three-dimensional POSITION grid to use with
    /// [`QuantizationBits::position`]. When position quantization is enabled,
    /// `None` keeps Draco's data-derived grid.
    ///
    /// Reusing the same origin and range lets separately encoded primitives,
    /// such as adjacent tiles, share one quantization grid. The range must be
    /// finite and positive, and `quantization.position` must specify 1..=30
    /// bits.
    pub position_quantization_grid: Option<PositionQuantizationGrid>,
    /// Forces the mesh connectivity coder, or picks automatically.
    ///
    /// `0` (the default) leaves the choice to `draco-core`'s own encoder,
    /// which without a forced method falls back to the C++ `ExpertEncoder`
    /// default: EdgeBreaker unless speed is 10. `1` forces sequential, `2`
    /// forces EdgeBreaker. This is the caller-facing convention rather than
    /// `EncoderOptions::set_encoding_method`'s own (`-1`/`0`/`1`) so that the
    /// value an omitted argument coerces to at every FFI boundary above this
    /// -- 0 -- already means "leave it alone" and cannot be mistaken for a
    /// forced choice.
    ///
    /// On a `POINTS` primitive the same two values pick between the point
    /// cloud's own coders instead: `1` forces the sequential coder, `2` the
    /// kd-tree one. There is no EdgeBreaker for a cloud to force.
    /// `KHR_gaussian_splatting` point clouds default to sequential in auto
    /// mode. Forcing kd-tree on a cloud with an unquantized Float32 splat
    /// rotation returns an error unless
    /// [`GaussianSplatQuantization::rotation_bits`] is set.
    pub encoding_method: i32,
    /// Compress `POINTS` using the experimental Draco splat extension.
    ///
    /// Document compression writes `TEST_gaussian_splatting_compression_draco`
    /// for `POINTS` declaring `KHR_gaussian_splatting`. Indexed input is
    /// expanded into points, including repeated indices. This requires
    /// [`CompressionMode::DracoOnly`] and works independently of
    /// [`Self::allow_points`].
    pub compress_splats: bool,
    /// Enable legacy `POINTS` compression for existing callers.
    ///
    /// This mode writes a point cloud under `KHR_draco_mesh_compression`,
    /// whose specification covers triangle primitives. Readers enforcing
    /// that restriction reject these files. For Gaussian splat documents,
    /// use [`Self::compress_splats`].
    pub allow_points: bool,
    /// Orders a point cloud's points along a Hilbert curve before they are
    /// written. Applies to compressed `POINTS` primitives. It reorders the
    /// decoded points and can make a file
    /// larger, when an attribute varies along the order the points came in.
    pub spatial_point_order: bool,
    /// Runs the order search before the points are written and keeps it only
    /// when the coder's own plan prices it under the order they came in. Read
    /// only when compressing a `POINTS` primitive. It
    /// costs encode time, and it reorders the decoded points when it fires.
    pub point_order_search: bool,
    /// Lets the point-cloud coder choose each attribute's prediction scheme by
    /// the estimated cost of the candidates rather than by the fixed rule. Read
    /// only when compressing a `POINTS` primitive. It
    /// costs encode time and nothing else, and every stream it can produce is
    /// one an ordinary decoder reads.
    pub prediction_search: bool,
}
impl Default for CompressionOptions {
    fn default() -> Self {
        Self {
            encoding_speed: 5,
            decoding_speed: 5,
            mode: CompressionMode::DracoOnly,
            max_output_bytes: None,
            quantization: QuantizationBits::default(),
            gaussian_splat_quantization: GaussianSplatQuantization::default(),
            position_quantization_grid: None,
            encoding_method: 0,
            allow_points: false,
            compress_splats: false,
            spatial_point_order: false,
            point_order_search: false,
            prediction_search: false,
        }
    }
}

/// Origin and range for a shared three-dimensional POSITION quantization grid.
///
/// Pair this with the `position` bit count in [`CompressionOptions::quantization`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionQuantizationGrid {
    /// The minimum coordinate represented by the grid.
    pub origin: [f32; 3],
    /// The shared extent from `origin` to the highest quantized value.
    pub range: f32,
}

/// Quantization settings for `KHR_gaussian_splatting` attributes.
///
/// Unset `scale_bits`, `opacity_bits`, and SH rules inherit
/// [`QuantizationBits::generic`]. Integer accessors keep their original
/// precision, and Float32 rotation quaternions keep their original precision
/// unless [`Self::rotation_bits`] is set explicitly.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GaussianSplatQuantization {
    /// Quantization bits for Float32 `KHR_gaussian_splatting:ROTATION` (1..=30).
    /// `None` (the default) stores Float32 rotation without quantization.
    pub rotation_bits: Option<u8>,
    /// Quantization bits for Float32 `KHR_gaussian_splatting:SCALE` (1..=30).
    pub scale_bits: Option<u8>,
    /// Quantization bits for Float32 `KHR_gaussian_splatting:OPACITY` (1..=30).
    /// Uses the observed range when it can be quantized within [0, 1].
    pub opacity_bits: Option<u8>,
    /// Default rule for all SH coefficients in each degree, indexed 0 through 3.
    pub sh_degrees: [Option<ShQuantization>; 4],
    sh_coefficients: [Option<ShCoefficientQuantization>; 16],
}

impl GaussianSplatQuantization {
    /// Overrides one SH coefficient named by its full glTF semantic.
    /// Pass `None` to remove the override and inherit the degree rule again.
    pub fn set_sh_coefficient(
        &mut self,
        semantic: &str,
        rule: Option<ShCoefficientQuantization>,
    ) -> Result<()> {
        let (degree, index) = semantic
            .strip_prefix(KHR_GAUSSIAN_SPLATTING_PREFIX)
            .and_then(sh_coefficient_indices)
            .ok_or_else(|| {
                Error::Extension(format!("invalid Gaussian splat SH semantic {semantic}"))
            })?;
        self.sh_coefficients[degree * degree + index] = rule;
        Ok(())
    }

    fn sh_rule(&self, semantic: &str) -> Option<ShQuantization> {
        let (degree, index) = sh_coefficient_indices(semantic)?;
        self.sh_coefficients[degree * degree + index]
            .map(ShQuantization::from)
            .or(self.sh_degrees[degree])
    }
}

/// An override for one RGB spherical harmonics coefficient.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShCoefficientQuantization {
    /// Store the source Float32 values without quantization.
    Preserve,
    /// Quantize to 1..=30 bits using bounds derived from this coefficient.
    Bits(u8),
}

/// The default rule for RGB spherical harmonics coefficients in one degree.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ShQuantization {
    /// Store the source Float32 values without quantization.
    Preserve,
    /// Quantize to 1..=30 bits using bounds derived from each coefficient.
    Bits(u8),
    /// Use the same explicit grid for every coefficient this rule covers.
    /// Values outside the grid are clamped to its nearest endpoint.
    Grid {
        /// Quantization bits, in 1..=30.
        bits: u8,
        /// Grid origin for the R, G and B components.
        origin: [f32; 3],
        /// Shared extent for each component.
        range: f32,
    },
    /// Fit a grid covering `[-max_abs, max_abs]` that decodes zero exactly.
    /// Values beyond the grid are clamped to its nearest endpoint.
    CenteredOnZero {
        /// Quantization bits, in 2..=30.
        bits: u8,
        /// A positive finite bound on the absolute value in all components.
        max_abs: f32,
    },
}

impl From<ShCoefficientQuantization> for ShQuantization {
    fn from(rule: ShCoefficientQuantization) -> Self {
        match rule {
            ShCoefficientQuantization::Preserve => Self::Preserve,
            ShCoefficientQuantization::Bits(bits) => Self::Bits(bits),
        }
    }
}

fn round_up_to_f32(value: f64) -> f32 {
    let rounded = value as f32;
    if f64::from(rounded) < value {
        rounded.next_up()
    } else {
        rounded
    }
}

fn centered_sh_grid(bits: u8, max_abs: f32) -> std::result::Result<([f32; 3], f32), &'static str> {
    if !(2..=30).contains(&bits) || !max_abs.is_finite() || max_abs <= 0.0 {
        return Err("a grid centered on zero requires 2..=30 bits and a finite positive max_abs");
    }
    let max_code = (1 << bits) - 1;
    let zero_code = max_code / 2;
    // Round up to keep both sides of the requested interval. Draco decodes
    // using Float32 codes and step, so derive the origin from its decoded
    // offset at the middle code. Adding that same offset then cancels exactly.
    let span = f64::from(max_code as f32);
    let step = round_up_to_f32(f64::from(max_abs) / f64::from(zero_code as f32));
    let range = round_up_to_f32(f64::from(step) * span);
    if let Some(grid) = check_centered_sh_grid(max_code, zero_code, range, max_abs) {
        return Ok(grid);
    }

    // If Float32 rounding moves zero to a different decoded level, use a
    // power-of-two step. Multiplication and division by that step are exact
    // exponent shifts in the normal range. Choose the upper middle code (a
    // power of two), so Draco's Float32 addition of 0.5 also keeps that code.
    let zero_code = (max_code + 1) / 2;
    let zero = f64::from(zero_code as f32);
    let min_step = f64::from(max_abs) / zero.min(span - zero);
    let exponent = min_step.to_bits() & !((1u64 << 52) - 1);
    let step = if f64::from_bits(exponent) < min_step {
        f64::from_bits(exponent + (1u64 << 52))
    } else {
        f64::from_bits(exponent)
    };
    check_centered_sh_grid(max_code, zero_code, (step * span) as f32, max_abs)
        .ok_or("grid cannot cover the requested bounds and represent zero exactly in Float32")
}

fn check_centered_sh_grid(
    max_code: i32,
    zero_code: i32,
    range: f32,
    max_abs: f32,
) -> Option<([f32; 3], f32)> {
    if !range.is_finite() || !(max_code as f32 / range).is_finite() {
        return None;
    }
    let mut decoder = Dequantizer::new();
    decoder.init(range, max_code);
    let offset = decoder.dequantize_float(zero_code);
    let upper = decoder.dequantize_float(max_code) - offset;
    let mut encoder = Quantizer::new();
    encoder.init(range, max_code);
    let encoded_zero = encoder.quantize_float(offset).clamp(0, max_code);
    if offset < max_abs
        || upper < max_abs
        || !upper.is_finite()
        || decoder.dequantize_float(encoded_zero) != offset
    {
        return None;
    }
    Some(([-offset; 3], range))
}

fn set_quantization_options(
    settings: &mut EncoderOptions,
    point_cloud: &draco_core::PointCloud,
    mapping: &[(String, u32)],
    options: &CompressionOptions,
) -> Result<()> {
    let grid = options.position_quantization_grid;
    if let Some(grid) = grid {
        let Some(bits) = options.quantization.position else {
            return Err(Error::Extension(
                "position_quantization_grid requires quantization.position bits".into(),
            ));
        };
        if !(1..=30).contains(&bits) {
            return Err(Error::Extension(format!(
                "position quantization bits must be in 1..=30, got {bits}"
            )));
        }
        if !grid.range.is_finite() || grid.range <= 0.0 {
            return Err(Error::Extension(format!(
                "position quantization grid range must be finite and positive, got {}",
                grid.range
            )));
        }
        if let Some(component) = grid.origin.iter().position(|value| !value.is_finite()) {
            return Err(Error::Extension(format!(
                "position quantization grid origin component {component} is not finite"
            )));
        }

        let mut position_attribute_found = false;
        for attribute_id in 0..point_cloud.num_attributes() {
            let attribute = point_cloud.attribute(attribute_id);
            if attribute.attribute_type() == GeometryAttributeType::Position {
                position_attribute_found = true;
                if attribute.num_components() != 3
                    || attribute.data_type() != draco_core::DataType::Float32
                {
                    return Err(Error::Extension(
                        "position_quantization_grid requires a three-component Float32 POSITION attribute"
                            .into(),
                    ));
                }
            }
        }
        if !position_attribute_found {
            return Err(Error::Extension(
                "position_quantization_grid requires a POSITION attribute".into(),
            ));
        }
    }

    for attribute_id in 0..point_cloud.num_attributes() {
        let attribute_type = point_cloud.attribute(attribute_id).attribute_type();
        if let Some(bits) = options.quantization.for_attribute(attribute_type) {
            if attribute_type == GeometryAttributeType::Position {
                if let Some(grid) = grid {
                    settings.set_attribute_explicit_quantization(
                        attribute_id,
                        bits as i32,
                        3,
                        &grid.origin,
                        grid.range,
                    );
                    continue;
                }
            }
            settings.set_attribute_int(attribute_id, "quantization_bits", bits as i32);
        }
    }

    for (semantic, unique_id) in mapping {
        let Some(splat_semantic) = semantic.strip_prefix(KHR_GAUSSIAN_SPLATTING_PREFIX) else {
            continue;
        };
        let attribute_id = point_cloud.attribute_id_by_unique_id(*unique_id);
        if attribute_id < 0 {
            return Err(Error::Extension(format!(
                "Gaussian splat attribute {semantic} maps to missing Draco unique id {unique_id}"
            )));
        }
        let attribute = point_cloud.attribute(attribute_id);
        let splat = &options.gaussian_splat_quantization;
        if splat_semantic == "OPACITY" && attribute.data_type() == draco_core::DataType::Float32 {
            let bits = splat
                .opacity_bits
                .or(options.quantization.generic.filter(|&bits| bits != 0));
            if let Some(bits) = bits {
                if !(1..=30).contains(&bits) {
                    return Err(Error::Extension(format!(
                        "{semantic}: quantization bits must be in 1..=30"
                    )));
                }
                settings.set_attribute_int(attribute_id, "quantization_bits", bits.into());
                if opacity_needs_unit_grid(attribute, bits) {
                    settings.set_attribute_explicit_quantization(
                        attribute_id,
                        bits.into(),
                        1,
                        &[0.0],
                        1.0,
                    );
                }
            }
            continue;
        }
        let rule = match splat_semantic {
            _ if attribute.data_type() != draco_core::DataType::Float32 => {
                Some(ShQuantization::Preserve)
            }
            // Generic quantization cannot preserve a unit quaternion; only
            // quantize Float32 rotation when requested explicitly.
            "ROTATION" => Some(
                splat
                    .rotation_bits
                    .map_or(ShQuantization::Preserve, ShQuantization::Bits),
            ),
            "SCALE" => splat.scale_bits.map(ShQuantization::Bits),
            _ => splat.sh_rule(splat_semantic),
        };
        if let Some(rule) = rule {
            apply_splat_quantization(settings, attribute_id, rule)
                .map_err(|error| Error::Extension(format!("{semantic}: {error}")))?;
        }
    }
    Ok(())
}

fn opacity_needs_unit_grid(attribute: &draco_core::PointAttribute, bits: u8) -> bool {
    // The glTF geometry builder copies OPACITY's Float32 scalar values into a
    // tightly packed Draco attribute. Use the same Float32 bounds as Draco.
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for bytes in attribute.buffer().data().as_chunks::<4>().0 {
        let value = f32::from_le_bytes(*bytes);
        if !value.is_finite() {
            // Let the encoder report its normal quantization error.
            return false;
        }
        min = min.min(value);
        max = max.max(value);
    }
    if min > max {
        return false;
    }
    if min < 0.0 || max > 1.0 {
        return true;
    }
    let range = max - min;
    if range == 0.0 {
        return false;
    }
    let max_code = (1 << bits) - 1;
    if !(max_code as f32 / range).is_finite() {
        return true;
    }
    // Both transforms are monotone, so checking the largest input is enough.
    let mut encoder = Quantizer::new();
    encoder.init(range, max_code);
    let mut decoder = Dequantizer::new();
    decoder.init(range, max_code);
    decoder.dequantize_float(encoder.quantize_float(range)) + min > 1.0
}

fn apply_splat_quantization(
    settings: &mut EncoderOptions,
    attribute_id: i32,
    rule: ShQuantization,
) -> std::result::Result<(), &'static str> {
    let (bits, grid) = match rule {
        ShQuantization::Preserve => {
            settings.set_attribute_int(attribute_id, "quantization_bits", 0);
            return Ok(());
        }
        ShQuantization::Bits(bits) => (bits, None),
        ShQuantization::Grid {
            bits,
            origin,
            range,
        } => (bits, Some((origin, range))),
        ShQuantization::CenteredOnZero { bits, max_abs } => {
            (bits, Some(centered_sh_grid(bits, max_abs)?))
        }
    };
    if !(1..=30).contains(&bits) {
        return Err("quantization bits must be in 1..=30");
    }
    if let Some((origin, range)) = grid {
        let max_code = (1 << bits) - 1;
        let scale = max_code as f32 / range;
        let step = range / max_code as f32;
        let span = max_code as f32 * step;
        if !scale.is_finite()
            || scale <= 0.0
            || step <= 0.0
            || origin
                .iter()
                .any(|&value| !value.is_finite() || !(value + span).is_finite())
        {
            return Err(
                "grid must have finite Float32 endpoints and a usable positive scale and step",
            );
        }
        settings.set_attribute_explicit_quantization(attribute_id, bits.into(), 3, &origin, range);
    } else {
        settings.set_attribute_int(attribute_id, "quantization_bits", bits.into());
    }
    Ok(())
}

/// Quantization bit counts per attribute type, as the `draco_encoder` CLI's
/// `-qp`/`-qn`/`-qt`/`-qg` express them.
///
/// [`QuantizationBits::NONE`] is the default and quantizes nothing.
/// [`QuantizationBits::GLTF`] carries what Blender's glTF exporter uses, which
/// is the closest thing to a convention for delivered glTF assets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuantizationBits {
    /// Bits for `POSITION`.
    pub position: Option<u8>,
    /// Bits for `NORMAL`.
    pub normal: Option<u8>,
    /// Bits for `TEXCOORD_n`.
    pub tex_coord: Option<u8>,
    /// Bits for `COLOR_n`.
    pub color: Option<u8>,
    /// Bits for every other attribute, `_`-prefixed ones included.
    pub generic: Option<u8>,
}

impl QuantizationBits {
    /// Quantizes nothing, which is what this crate did before the field existed.
    pub const NONE: Self = Self {
        position: None,
        normal: None,
        tex_coord: None,
        color: None,
        generic: None,
    };

    /// What Blender's glTF exporter writes: 14/10/12/10/12.
    pub const GLTF: Self = Self {
        position: Some(14),
        normal: Some(10),
        tex_coord: Some(12),
        color: Some(10),
        generic: Some(12),
    };

    /// The bits for one attribute type, or `None` to leave it unquantized.
    fn for_attribute(self, attribute_type: GeometryAttributeType) -> Option<u8> {
        match attribute_type {
            GeometryAttributeType::Position => self.position,
            GeometryAttributeType::Normal => self.normal,
            GeometryAttributeType::TexCoord => self.tex_coord,
            GeometryAttributeType::Color => self.color,
            _ => self.generic,
        }
    }
}

#[cfg(test)]
mod quantization_tests {
    use super::*;
    use crate::{
        ComponentType, DracoPrimitiveEncoding, PackedAttribute, PackedGeometry, PrimitiveMode,
    };

    #[test]
    fn float_rotation_and_integer_splat_attributes_keep_their_precision() {
        let mut options = CompressionOptions {
            allow_points: true,
            quantization: QuantizationBits::GLTF,
            ..CompressionOptions::default()
        };
        options.gaussian_splat_quantization.scale_bits = Some(6);
        options.gaussian_splat_quantization.opacity_bits = Some(6);
        options.gaussian_splat_quantization.sh_degrees[0] = Some(ShQuantization::CenteredOnZero {
            bits: 3,
            max_abs: 0.25,
        });
        let prefix = crate::geometry::KHR_GAUSSIAN_SPLATTING_PREFIX;
        let attr = |name: &str, components, data: &[f32]| {
            PackedAttribute::new(
                name,
                3,
                components,
                ComponentType::F32,
                false,
                data.iter().flat_map(|value| value.to_le_bytes()).collect(),
            )
            .unwrap()
        };
        let rotation = format!("{prefix}ROTATION");
        let scale = format!("{prefix}SCALE");
        let opacity = format!("{prefix}OPACITY");
        let sh = format!("{prefix}SH_DEGREE_0_COEF_0");

        for (rotation_type, normalized, rotation_bytes, method) in [
            (
                ComponentType::F32,
                false,
                [
                    std::f32::consts::FRAC_1_SQRT_2,
                    0.0,
                    0.0,
                    std::f32::consts::FRAC_1_SQRT_2,
                    0.0,
                    0.0,
                    0.0,
                    1.0,
                    1.0,
                    0.0,
                    0.0,
                    0.0,
                ]
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>(),
                0,
            ),
            (ComponentType::I8, true, [0, 0, 0, 127].repeat(3), 2),
        ] {
            options.encoding_method = method;
            let geometry = PackedGeometry::new(
                PrimitiveMode::Points,
                vec![
                    attr(
                        "POSITION",
                        3,
                        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0],
                    ),
                    PackedAttribute::new(
                        &rotation,
                        3,
                        4,
                        rotation_type,
                        normalized,
                        rotation_bytes.clone(),
                    )
                    .unwrap(),
                    PackedAttribute::new(&scale, 3, 3, ComponentType::U8, false, vec![3; 9])
                        .unwrap(),
                    PackedAttribute::new(&opacity, 3, 1, ComponentType::U8, true, vec![127; 3])
                        .unwrap(),
                    attr(&sh, 3, &[0.0; 9]),
                ],
                None,
            )
            .unwrap();
            let encoded = DracoPrimitiveEncoding::encode(&geometry, &options).unwrap();
            let decoded = encoded.decoded().unwrap();
            for source in &geometry.attributes()[1..] {
                let attr = decoded
                    .attributes()
                    .iter()
                    .find(|attr| attr.semantic() == source.semantic())
                    .unwrap();
                assert_eq!(attr, source);
            }
            if rotation_type == ComponentType::F32 {
                options.encoding_method = 2;
                let error = DracoPrimitiveEncoding::encode(&geometry, &options).unwrap_err();
                assert!(error
                    .to_string()
                    .contains("ROTATION must retain its precision"));
            }
        }
    }

    #[test]
    fn sh_coefficient_rules_override_the_degree() {
        let mut splat = GaussianSplatQuantization::default();
        splat.sh_degrees[1] = Some(ShQuantization::Bits(8));
        let full_semantic = "KHR_gaussian_splatting:SH_DEGREE_1_COEF_0";
        splat
            .set_sh_coefficient(full_semantic, Some(ShCoefficientQuantization::Preserve))
            .unwrap();
        assert_eq!(
            splat.sh_rule("SH_DEGREE_1_COEF_0"),
            Some(ShQuantization::Preserve)
        );
        assert_eq!(
            splat.sh_rule("SH_DEGREE_1_COEF_1"),
            Some(ShQuantization::Bits(8))
        );
        assert_eq!(splat.sh_rule("SH_DEGREE_0_COEF_0"), None);
        splat.set_sh_coefficient(full_semantic, None).unwrap();
        assert_eq!(
            splat.sh_rule("SH_DEGREE_1_COEF_0"),
            Some(ShQuantization::Bits(8))
        );
        assert!(splat
            .set_sh_coefficient("KHR_gaussian_splatting:SH_DEGREE_1_COEF_3", None)
            .is_err());
        for suffix in [
            "SH_DEGREE_01_COEF_00",
            "SH_DEGREE_+1_COEF_0",
            "SH_DEGREE_1_COEF_00",
        ] {
            let semantic = format!("{KHR_GAUSSIAN_SPLATTING_PREFIX}{suffix}");
            assert!(splat
                .set_sh_coefficient(&semantic, Some(ShCoefficientQuantization::Preserve))
                .is_err());
            assert!(crate::geometry::supported_semantic_spec(&semantic).is_err());
        }
    }

    #[test]
    fn float_opacity_uses_the_observed_range_when_safe() {
        let semantic = "KHR_gaussian_splatting:OPACITY";
        let attribute = |name, components, values: &[f32]| {
            PackedAttribute::new(
                name,
                3,
                components,
                ComponentType::F32,
                false,
                values
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect(),
            )
            .unwrap()
        };
        for source in [[0.0005, 0.001, 0.0015], [0.0013, 0.5, 1.0]] {
            let geometry = PackedGeometry::new(
                PrimitiveMode::Points,
                vec![
                    attribute(
                        "POSITION",
                        3,
                        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0],
                    ),
                    attribute(semantic, 1, &source),
                ],
                None,
            )
            .unwrap();
            let mut options = CompressionOptions {
                allow_points: true,
                encoding_method: 1,
                ..CompressionOptions::default()
            };
            for (generic, opacity) in [(Some(8), None), (None, Some(8))] {
                options.quantization.generic = generic;
                options.gaussian_splat_quantization.opacity_bits = opacity;
                let encoded = DracoPrimitiveEncoding::encode(&geometry, &options).unwrap();
                let opacity = &encoded.decoded().unwrap().attributes()[1];
                let values = opacity
                    .bytes()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .copied()
                    .map(f32::from_le_bytes)
                    .collect::<Vec<_>>();
                assert!(values[0] < values[1] && values[1] < values[2]);
                assert_eq!(values[2], source[2]);
            }
        }
    }

    #[test]
    fn centered_sh_grid_checks_codec_arithmetic() {
        use draco_core::attribute_quantization_transform::AttributeQuantizationTransform;
        for bits in 2..=30 {
            for bound in [0.01, 0.25, 1.0, 170.3] {
                let (origin, range) = centered_sh_grid(bits, bound).unwrap();
                let mut transform = AttributeQuantizationTransform::new();
                transform
                    .set_parameters(bits.into(), &origin, range)
                    .unwrap();
                assert_eq!(transform.round_trip_component(0, 0.0).unwrap(), 0.0);
            }
        }
        assert!(centered_sh_grid(10, 1e-38).is_err());
        assert!(centered_sh_grid(1, 0.25).is_err());
        assert!(centered_sh_grid(10, f32::MAX).is_err());
    }

    #[test]
    fn gaussian_splat_points_default_to_sequential_in_auto_mode() {
        let prefix = crate::geometry::KHR_GAUSSIAN_SPLATTING_PREFIX;
        let positions = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        // 11 total components (below draco-core's 12-component KD-tree cap),
        // with integer ROTATION/SCALE/OPACITY so KD-tree is eligible.
        let geometry = PackedGeometry::new(
            PrimitiveMode::Points,
            vec![
                PackedAttribute::new("POSITION", 3, 3, ComponentType::F32, false, positions)
                    .unwrap(),
                PackedAttribute::new(
                    format!("{prefix}ROTATION"),
                    3,
                    4,
                    ComponentType::I8,
                    true,
                    [0, 0, 0, 127].repeat(3),
                )
                .unwrap(),
                PackedAttribute::new(
                    format!("{prefix}SCALE"),
                    3,
                    3,
                    ComponentType::U8,
                    false,
                    vec![3; 9],
                )
                .unwrap(),
                PackedAttribute::new(
                    format!("{prefix}OPACITY"),
                    3,
                    1,
                    ComponentType::U8,
                    true,
                    vec![127; 3],
                )
                .unwrap(),
            ],
            None,
        )
        .unwrap();

        let mut options = CompressionOptions {
            allow_points: true,
            quantization: QuantizationBits::GLTF,
            ..CompressionOptions::default()
        };
        let auto = DracoPrimitiveEncoding::encode(&geometry, &options).unwrap();
        assert_eq!(
            auto.bytes()[8],
            0,
            "KHR_gaussian_splatting POINTS must default to sequential in auto mode"
        );

        // Explicit KD-tree remains available when all attributes are compatible.
        options.encoding_method = 2;
        let forced = DracoPrimitiveEncoding::encode(&geometry, &options).unwrap();
        assert_eq!(forced.bytes()[8], 1);
    }

    #[test]
    fn float_rotation_quantizes_when_rotation_bits_is_set() {
        let prefix = crate::geometry::KHR_GAUSSIAN_SPLATTING_PREFIX;
        let rotation_semantic = format!("{prefix}ROTATION");
        let positions = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let source_rotations = [
            0.123_456_7_f32,
            -0.234_567_8,
            0.345_678_9,
            0.899_123_4,
            0.0,
            0.0,
            0.0,
            1.0,
            -0.5,
            0.5,
            -0.5,
            0.5,
        ];
        let geometry = PackedGeometry::new(
            PrimitiveMode::Points,
            vec![
                PackedAttribute::new("POSITION", 3, 3, ComponentType::F32, false, positions)
                    .unwrap(),
                PackedAttribute::new(
                    &rotation_semantic,
                    3,
                    4,
                    ComponentType::F32,
                    false,
                    source_rotations
                        .iter()
                        .flat_map(|value| value.to_le_bytes())
                        .collect(),
                )
                .unwrap(),
            ],
            None,
        )
        .unwrap();

        let mut options = CompressionOptions {
            allow_points: true,
            quantization: QuantizationBits::GLTF,
            ..CompressionOptions::default()
        };
        options.gaussian_splat_quantization.rotation_bits = Some(8);
        let encoded = DracoPrimitiveEncoding::encode(&geometry, &options).unwrap();
        assert_eq!(encoded.bytes()[8], 0);
        let decoded = encoded.decoded().unwrap();
        let decoded_rotations = decoded
            .attributes()
            .iter()
            .find(|attr| attr.semantic() == rotation_semantic)
            .unwrap()
            .bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(f32::from_le_bytes)
            .collect::<Vec<_>>();
        assert_ne!(
            decoded_rotations.as_slice(),
            source_rotations.as_slice(),
            "rotation_bits = Some(8) should quantize Float32 ROTATION"
        );
        for (decoded, expected) in decoded_rotations.iter().zip(source_rotations.iter()) {
            assert!(
                (decoded - expected).abs() < 0.02,
                "quantized rotation {decoded} drifted too far from {expected}"
            );
        }

        // With rotation_bits set, forcing KD-tree is allowed.
        options.encoding_method = 2;
        let forced = DracoPrimitiveEncoding::encode(&geometry, &options).unwrap();
        assert_eq!(forced.bytes()[8], 1);

        // Out-of-range rotation_bits is rejected.
        options.encoding_method = 0;
        for invalid in [0, 31] {
            options.gaussian_splat_quantization.rotation_bits = Some(invalid);
            assert!(DracoPrimitiveEncoding::encode(&geometry, &options).is_err());
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
    /// Numeric Draco method used in the payload. `0` is sequential. `1` is
    /// EdgeBreaker for a mesh or KD-tree for a point cloud.
    pub encoding_method: i32,
    /// Resolved Draco speed used by the encoder.
    pub encoding_speed: i32,
    /// The prediction schemes selected for the encoded attributes, formatted
    /// with their glTF semantic. Absent when no attribute reached the integer
    /// prediction path.
    pub prediction_scheme: Option<String>,
}

fn prediction_scheme_name(
    method: draco_core::prediction_scheme::PredictionSchemeMethod,
    transform: draco_core::prediction_scheme::PredictionSchemeTransformType,
) -> String {
    let method = match method {
        draco_core::prediction_scheme::PredictionSchemeMethod::None => "None",
        draco_core::prediction_scheme::PredictionSchemeMethod::Undefined => "Undefined",
        draco_core::prediction_scheme::PredictionSchemeMethod::Difference => "Difference",
        draco_core::prediction_scheme::PredictionSchemeMethod::MeshPredictionParallelogram => {
            "Parallelogram"
        }
        draco_core::prediction_scheme::PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
            "Multi-parallelogram"
        }
        draco_core::prediction_scheme::PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
            "TexCoords (legacy)"
        }
        draco_core::prediction_scheme::PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram => {
            "Constrained multi-parallelogram"
        }
        draco_core::prediction_scheme::PredictionSchemeMethod::MeshPredictionTexCoordsPortable => {
            "TexCoords"
        }
        draco_core::prediction_scheme::PredictionSchemeMethod::MeshPredictionGeometricNormal => {
            "Geometric normal"
        }
    };
    let transform = match transform {
        draco_core::prediction_scheme::PredictionSchemeTransformType::None => "None",
        draco_core::prediction_scheme::PredictionSchemeTransformType::Delta => "Delta",
        draco_core::prediction_scheme::PredictionSchemeTransformType::Wrap => "Wrap",
        draco_core::prediction_scheme::PredictionSchemeTransformType::NormalOctahedron => {
            "Normal octahedron"
        }
        draco_core::prediction_scheme::PredictionSchemeTransformType::NormalOctahedronCanonicalized => {
            "Canonicalized normal octahedron"
        }
        draco_core::prediction_scheme::PredictionSchemeTransformType::Parallelogram => {
            "Parallelogram"
        }
        draco_core::prediction_scheme::PredictionSchemeTransformType::TexCoordsPortable => {
            "TexCoords"
        }
        draco_core::prediction_scheme::PredictionSchemeTransformType::GeometricNormal => {
            "Geometric normal"
        }
        draco_core::prediction_scheme::PredictionSchemeTransformType::MultiParallelogram => {
            "Multi-parallelogram"
        }
        draco_core::prediction_scheme::PredictionSchemeTransformType::ConstrainedMultiParallelogram => {
            "Constrained multi-parallelogram"
        }
    };
    format!("{method} ({transform})")
}

fn prediction_summary(
    attributes: &[EncodedAttributeInfo],
    mapping: &[(String, u32)],
) -> Option<String> {
    let schemes: Vec<String> = mapping
        .iter()
        .filter_map(|(semantic, unique_id)| {
            let attribute = attributes
                .iter()
                .find(|attribute| attribute.unique_id == *unique_id)?;
            let (method, transform) = attribute.prediction?;
            Some(format!(
                "{semantic}: {}",
                prediction_scheme_name(method, transform)
            ))
        })
        .collect();
    (!schemes.is_empty()).then(|| schemes.join("; "))
}

/// Encodes an already decoded mesh to a raw Draco payload and its report.
pub(crate) fn encode_draco_mesh(
    mesh: draco_core::Mesh,
    mapping: &[(String, u32)],
    options: &CompressionOptions,
) -> Result<(Vec<u8>, EncodedMeshInfo)> {
    let mut settings = EncoderOptions::new();
    settings.set_global_int("encoding_speed", options.encoding_speed as i32);
    settings.set_global_int("decoding_speed", options.decoding_speed as i32);
    // Left untouched for "auto" (0): the encoder's own default, applied by
    // never calling this, is what every caller got before this field
    // existed and what `CompressionOptions::default()` still asks for.
    match options.encoding_method {
        1 => settings.set_encoding_method(0), // force sequential
        2 => settings.set_encoding_method(1), // force EdgeBreaker
        _ => {}
    }
    // Quantization is keyed by attribute id here, while the caller names
    // attribute types, so the mapping has to happen against the mesh that is
    // about to be encoded rather than in the options.
    set_quantization_options(&mut settings, &mesh, mapping, options)?;
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut output = EncoderBuffer::new();
    // The extension needs the description, so this is the entry point that
    // derives it; plain `encode` would leave it uncomputed.
    let info = encoder
        .encode_with_info(&settings, &mut output)
        .map_err(|error| Error::Extension(error.to_string()))?;
    Ok((output.data().to_vec(), info))
}

impl Import {
    /// Compresses one ordinary triangle, triangle-strip or triangle-fan
    /// primitive atomically.
    ///
    /// A strip or fan is unwound into a triangle list before Draco ever sees
    /// it -- Draco's connectivity has no notion of either -- and the output
    /// primitive's `mode` is rewritten to `TRIANGLES` to say so truthfully.
    ///
    /// Accessors declare what the stream decodes to, as the spec requires. A
    /// primitive with morph targets is encoded sequentially and refused if its
    /// vertices move, or if `encoding_method` forces EdgeBreaker.
    ///
    /// The document and resolved resources are updated only after encoding,
    /// validation, reference remapping, and output-limit checks all succeed.
    /// In [`CompressionMode::DracoOnly`] the operation rejects extensions
    /// whose binary references cannot be remapped safely, including animation
    /// pointers into collections that the operation compacts.
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
            .document()
            .primitive(mesh, primitive)
            .ok_or_else(|| Error::Extension("primitive out of range".into()))?;
        self.ensure_transform_safe(reference)?;
        if options.mode == CompressionMode::DracoOnly {
            self.ensure_document_binary_transform_safe()?;
        }
        let source_mode = reference.mode();
        let points = source_mode == 0;
        let splats = points
            && options.compress_splats
            && (reference.extension(crate::splat_draco::BASE).is_some()
                || reference
                    .attribute_indices()
                    .any(|(name, _)| name.starts_with(KHR_GAUSSIAN_SPLATTING_PREFIX)));
        if splats {
            if reference
                .extension(crate::splat_draco::BASE)
                .and_then(crate::JsonRef::as_object)
                .is_none()
            {
                return Err(Error::Extension(
                    "experimental Draco splat compression requires KHR_gaussian_splatting".into(),
                ));
            }
            if options.mode != CompressionMode::DracoOnly {
                return Err(Error::Extension(
                    "experimental Draco splat compression requires DracoOnly".into(),
                ));
            }
        }
        if points {
            // A point cloud sits outside what the extension's own text
            // covers, so it is strictly opt-in; see `CompressionOptions::
            // allow_points`.
            if !options.allow_points && !splats {
                return Err(Error::Extension(format!(
                    "KHR_draco_mesh_compression encoding supports only TRIANGLES (4), \
                     TRIANGLE_STRIP (5) and TRIANGLE_FAN (6), not mode {source_mode}; \
                     use compress_splats for KHR_gaussian_splatting or allow_points for legacy POINTS output"
                )));
            }
            if reference.morph_targets().next().is_some() {
                return Err(Error::Extension(
                    "a primitive with morph targets cannot be point-cloud compressed: \
                     the point cloud carries no connectivity for the targets to ride on"
                        .into(),
                ));
            }
        } else if !matches!(source_mode, 4..=6) {
            return Err(Error::Extension(format!(
                "KHR_draco_mesh_compression encoding supports only TRIANGLES (4), \
                 TRIANGLE_STRIP (5) and TRIANGLE_FAN (6), not mode {source_mode}"
            )));
        }
        // Morph targets stay uncompressed and index vertices in place, and
        // EdgeBreaker renumbers vertices: encode sequentially, then check.
        let has_targets = reference.morph_targets().next().is_some();
        let mut options = options;
        if has_targets {
            if options.encoding_method == 2 {
                return Err(Error::Extension(
                    "a primitive with morph targets cannot be EdgeBreaker-encoded: \
                     EdgeBreaker reorders vertices and the targets index them in place"
                        .into(),
                ));
            }
            options.encoding_method = 1;
        }
        let normalized: Vec<(String, bool)> = reference
            .attribute_indices()
            .map(|(semantic, index)| {
                let normalized = self
                    .document()
                    .accessor(index)
                    .is_some_and(crate::Accessor::normalized);
                (semantic.to_owned(), normalized)
            })
            .collect();
        let (geometry, mapping) = self.decode_geometry_primitive(reference)?;
        let encoded = if points {
            encode_point_cloud_primitive(geometry, mapping, &options)?
        } else {
            encode_primitive(geometry, mapping, &options)?
        };
        let mut decoded = if splats || has_targets || options.mode == CompressionMode::Fallback {
            Some(decode_encoded(
                &encoded.bytes,
                &encoded.mapping,
                (encoded.layout.points, encoded.layout.index_count),
                &normalized,
                if encoded.layout.point_cloud {
                    crate::PrimitiveMode::Points
                } else {
                    crate::PrimitiveMode::Triangles
                },
            )?)
        } else {
            None
        };
        if let (true, Some(decoded)) = (has_targets, &decoded) {
            let source = self.read_primitive(crate::PrimitiveIndex::new(mesh, primitive))?;
            if !keeps_vertex_order(&source, decoded, options.quantization.position) {
                return Err(Error::Extension(
                    "cannot compress a primitive with morph targets: Draco did not keep \
                     its vertices in order, and the targets would move the wrong ones"
                        .into(),
                ));
            }
        }
        let EncodedPrimitive {
            bytes,
            encoding,
            attributes: encoded_attributes,
            mapping,
            layout,
        } = encoded;
        let encoding_method = encoding.0;
        let encoding_speed = encoding.1;
        let prediction_scheme = prediction_summary(&encoded_attributes, &mapping);
        // The spec: fallback data should be "decompressed from the Draco
        // buffer, rather than the original source data".
        if let (CompressionMode::Fallback, Some(decoded)) = (options.mode, &decoded) {
            self.write_raw_primitive_inner(crate::PrimitiveIndex::new(mesh, primitive), decoded)?;
        }
        let buffer = self.resources.buffers.len();
        let view;
        {
            let root = self.document_mut().as_value_mut();
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
            let extension = crate::JsonValue::object([
                ("bufferView", crate::JsonValue::from(view)),
                ("attributes", attributes),
            ]);
            let extensions = &mut root["meshes"][mesh.0]["primitives"][primitive]["extensions"];
            let name = if splats {
                extensions[crate::splat_draco::BASE]["extensions"]
                    [crate::extensions::SPLAT_DRACO] = extension;
                add_extension_name(root, "extensionsUsed", crate::splat_draco::BASE)?;
                add_extension_name(root, "extensionsRequired", crate::splat_draco::BASE)?;
                crate::extensions::SPLAT_DRACO
            } else {
                extensions[crate::KHR_DRACO_MESH_COMPRESSION] = extension;
                crate::KHR_DRACO_MESH_COMPRESSION
            };
            // A strip or fan was unwound into an ordinary triangle list before
            // Draco ever saw it -- its connectivity has no notion of either --
            // so the primitive now truthfully is TRIANGLES, whatever mode it
            // arrived as. Left untouched when it already was: an explicit
            // `mode: 4` next to an unchanged geometry stream would be a diff
            // with nothing behind it. A compressed point cloud keeps `mode 0`,
            // which is what it is.
            if matches!(source_mode, 5..=6) {
                root["meshes"][mesh.0]["primitives"][primitive]["mode"] =
                    crate::JsonValue::from(4u64);
            }
            add_extension_name(root, "extensionsUsed", name)?;
            if options.mode == CompressionMode::DracoOnly {
                add_extension_name(root, "extensionsRequired", name)?;
            }
            if options.mode == CompressionMode::DracoOnly {
                detach_draco_only_accessors(root, mesh.0, primitive, &mapping, &layout)?;
            }
        }
        self.resources.buffers.push(bytes.clone());
        if options.mode == CompressionMode::DracoOnly {
            compact_draco_only_resources(
                self.checked.get_mut(),
                &mut self.resources.buffers,
                &self.extensions,
                options.max_output_bytes,
            )?;
        }
        if splats {
            let reference = self
                .document()
                .primitive(mesh, primitive)
                .ok_or_else(|| Error::Extension("compressed splat primitive is missing".into()))?;
            crate::splat_draco::contract(self.document(), reference)?;
            let geometry = decoded
                .as_mut()
                .expect("experimental splats are decoded for validation");
            crate::splat_draco::validate_decoded(self.document(), reference, geometry)?;
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
            encoding_method,
            encoding_speed,
            prediction_scheme,
        })
    }
}
