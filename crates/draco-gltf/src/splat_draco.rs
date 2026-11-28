//! Contract for the experimental Gaussian splat compression profile.

use std::collections::BTreeSet;

use crate::extensions::{
    meshopt_extension, parse_draco_extension, splat_draco_extension, DracoContract,
    ExtensionHandler, ExtensionValidationContext, SPLAT_DRACO as NAME,
};
use crate::{ComponentType, Document, Error, JsonRef, PackedGeometry, PrimitiveRef, Result};

pub(crate) const BASE: &str = "KHR_gaussian_splatting";

fn error(reason: &str) -> Error {
    Error::Extension(format!("{NAME}: {reason}"))
}

pub(crate) fn extension(primitive: PrimitiveRef<'_>) -> Option<JsonRef<'_>> {
    splat_draco_extension(primitive.value())
}

pub(crate) struct SplatDracoExtension;

impl ExtensionHandler for SplatDracoExtension {
    fn name(&self) -> &'static str {
        NAME
    }

    #[cfg(feature = "write")]
    fn allows_binary_transform(&self) -> bool {
        true
    }

    #[cfg(feature = "write")]
    fn collect_binary_references(
        &self,
        document: &Document,
        _accessors: &mut [bool],
        buffer_views: &mut [bool],
    ) -> Result<()> {
        crate::extensions::collect_draco_views(document, buffer_views, splat_draco_extension)
    }

    #[cfg(feature = "write")]
    fn remap_binary_references(
        &self,
        document: &mut Document,
        _accessors: &[Option<usize>],
        buffer_views: &[Option<usize>],
    ) -> Result<()> {
        crate::extensions::remap_draco_views(document, buffer_views, |primitive| {
            primitive
                .get_mut("extensions")?
                .get_mut(BASE)?
                .get_mut("extensions")?
                .get_mut(NAME)
        })
    }

    fn validate(
        &self,
        document: &Document,
        context: &mut ExtensionValidationContext,
    ) -> Result<()> {
        for mesh in document.meshes() {
            let count = mesh
                .value()
                .get("primitives")
                .and_then(JsonRef::as_array)
                .map_or(0, |items| items.len());
            for index in 0..count {
                if let Some(primitive) = document.primitive(mesh.index(), index) {
                    if extension(primitive).is_some() {
                        contract(document, primitive)?;
                        for (_, index) in primitive.attribute_indices() {
                            context.allow_accessor_without_buffer_view(index.0);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn decode_primitive(
        &self,
        document: &Document,
        resources: &crate::ResourceStore,
        options: &crate::DracoDecodeOptions,
        primitive: PrimitiveRef<'_>,
    ) -> Option<Result<draco_core::Mesh>> {
        extension(primitive)?;
        Some((|| {
            let parsed = contract(document, primitive)?;
            let payload =
                crate::accessor::buffer_view_bytes(document, resources, parsed.buffer_view)?;
            check_header(payload)?;
            crate::draco_primitive::decode_payload(payload, options)
        })())
    }
}

pub(crate) fn contract(document: &Document, primitive: PrimitiveRef<'_>) -> Result<DracoContract> {
    let root = document.as_json();
    for field in ["extensionsUsed", "extensionsRequired"] {
        let names = root
            .get(field)
            .and_then(JsonRef::as_array)
            .unwrap_or_default();
        if [BASE, NAME]
            .iter()
            .any(|name| !names.iter().any(|value| value.as_str() == Some(name)))
        {
            return Err(error("both extensions must be declared"));
        }
    }
    if primitive.mode() != 0 || primitive.value().get("indices").is_some() {
        return Err(error("expected non-indexed POINTS"));
    }
    if [
        crate::KHR_DRACO_MESH_COMPRESSION,
        crate::EXT_MESHOPT_COMPRESSION,
        crate::KHR_MESHOPT_COMPRESSION,
    ]
    .iter()
    .any(|name| primitive.extension(name).is_some())
    {
        return Err(error("conflicting attribute source"));
    }

    let parsed =
        parse_draco_extension(extension(primitive))?.ok_or_else(|| error("missing extension"))?;
    let view = root
        .get("bufferViews")
        .and_then(JsonRef::as_array)
        .and_then(|views| views.get(parsed.buffer_view))
        .ok_or_else(|| error("invalid bufferView"))?;
    if view.get("byteStride").is_some() {
        return Err(error("strided payload"));
    }
    if meshopt_extension(view.get("extensions")).is_some() {
        return Err(error("conflicting payload source"));
    }

    let mut ids = BTreeSet::new();
    let mut semantics = BTreeSet::new();
    for (semantic, id) in &parsed.attributes {
        if !ids.insert(*id) {
            return Err(error("duplicate ID"));
        }
        semantics.insert(semantic.as_str());
    }
    let attributes = primitive
        .attributes()
        .ok_or_else(|| error("missing primitive attributes"))?;
    if attributes.len() != semantics.len()
        || attributes.iter().any(|(name, _)| !semantics.contains(name))
    {
        return Err(error("invalid semantic map"));
    }
    for required in [
        "POSITION",
        "KHR_gaussian_splatting:ROTATION",
        "KHR_gaussian_splatting:SCALE",
        "KHR_gaussian_splatting:OPACITY",
        "KHR_gaussian_splatting:SH_DEGREE_0_COEF_0",
    ] {
        if !semantics.contains(required) {
            return Err(error("missing required splat attribute"));
        }
    }
    let mut highest_degree = 0;
    for (semantic, index) in primitive.attribute_indices() {
        let accessor = document
            .accessor(index)
            .ok_or_else(|| error("invalid accessor"))?;
        if ["bufferView", "byteOffset", "sparse"]
            .iter()
            .any(|field| accessor.value().get(field).is_some())
        {
            return Err(error("conflicting accessor source"));
        }
        use ComponentType::{F32, I16, I8, U16, U8};
        let ty = accessor.component_type();
        let normalized = accessor.normalized();
        let shape = accessor.accessor_type();
        let float = ty == Some(F32) && !normalized;
        let unsigned = matches!(ty, Some(U8 | U16));
        let keyword = semantic.strip_prefix("KHR_gaussian_splatting:");
        let valid = match keyword {
            Some("ROTATION") => {
                shape == Some("VEC4") && (float || (normalized && matches!(ty, Some(I8 | I16))))
            }
            Some("SCALE") => shape == Some("VEC3") && (float || unsigned),
            Some("OPACITY") => shape == Some("SCALAR") && (float || (normalized && unsigned)),
            Some(name) => {
                let Some((degree, coefficient)) = name
                    .strip_prefix("SH_DEGREE_")
                    .and_then(|name| name.split_once("_COEF_"))
                else {
                    return Err(error("unknown splat semantic"));
                };
                let (Ok(degree), Ok(coefficient)) =
                    (degree.parse::<u8>(), coefficient.parse::<u8>())
                else {
                    return Err(error("invalid SH semantic"));
                };
                if degree > 3 || coefficient > 2 * degree {
                    return Err(error("invalid SH semantic"));
                }
                highest_degree = highest_degree.max(degree);
                shape == Some("VEC3") && float
            }
            None => true,
        };
        if !valid {
            return Err(error(&format!("invalid representation for {semantic}")));
        }
    }
    for degree in 0..=highest_degree {
        for coefficient in 0..=2 * degree {
            if !semantics.contains(
                format!("KHR_gaussian_splatting:SH_DEGREE_{degree}_COEF_{coefficient}").as_str(),
            ) {
                return Err(error("incomplete SH degree"));
            }
        }
    }
    Ok(parsed)
}

pub(crate) fn check_header(payload: &[u8]) -> Result<()> {
    if payload.get(..7) != Some(b"DRACO\x02\x03") {
        return Err(error("expected Draco stream version 2.3"));
    }
    if !matches!(payload.get(7..9), Some([0, 0 | 1])) {
        return Err(error("expected sequential or KD-tree point cloud type"));
    }
    Ok(())
}

pub(crate) fn validate_decoded(
    document: &Document,
    primitive: PrimitiveRef<'_>,
    geometry: &mut PackedGeometry,
) -> Result<()> {
    for attribute in &mut geometry.attributes {
        let semantic = attribute.semantic();
        let index = primitive
            .attribute_indices()
            .find(|(name, _)| *name == semantic)
            .ok_or_else(|| error("unmapped decoded attribute"))?
            .1;
        let accessor = document
            .accessor(index)
            .ok_or_else(|| error("invalid accessor"))?;
        let shape = match attribute.components() {
            1 => "SCALAR",
            2 => "VEC2",
            3 => "VEC3",
            4 => "VEC4",
            _ => return Err(error("invalid decoded shape")),
        };
        if accessor.count() != Some(attribute.count() as u64)
            || accessor.component_type() != Some(attribute.component_type())
            || accessor.accessor_type() != Some(shape)
        {
            return Err(error(&format!("decoded accessor mismatch for {semantic}")));
        }
        let rotation = semantic == "KHR_gaussian_splatting:ROTATION";
        if semantic == "POSITION" {
            validate_position_bounds(accessor.value(), attribute)?;
            continue;
        }
        if attribute.component_type() != ComponentType::F32 {
            if rotation
                && attribute
                    .bytes()
                    .chunks_exact(4 * attribute.component_type().byte_width())
                    .any(|row| row.iter().all(|byte| *byte == 0))
            {
                return Err(error("rotation must be nonzero"));
            }
            continue;
        }
        let opacity = semantic == "KHR_gaussian_splatting:OPACITY";
        let scale = semantic == "KHR_gaussian_splatting:SCALE";
        let width = 4 * usize::from(attribute.components());
        for row in attribute.bytes.chunks_exact_mut(width) {
            let mut nonzero = false;
            for scalar in row.as_chunks_mut::<4>().0 {
                let value = f32::from_le_bytes(*scalar);
                if !value.is_finite() {
                    return Err(error("decoded values must be finite"));
                }
                nonzero |= value != 0.0;
                if (opacity || scale) && value < 0.0 {
                    return Err(error("negative scale or opacity"));
                }
                if opacity && value > 1.0 {
                    if value != 1.0f32.next_up() {
                        return Err(error("opacity exceeds 1"));
                    }
                    scalar.copy_from_slice(&1.0f32.to_le_bytes());
                }
            }
            if rotation && !nonzero {
                return Err(error("rotation must be nonzero"));
            }
        }
    }
    Ok(())
}

fn validate_position_bounds(
    accessor: JsonRef<'_>,
    attribute: &crate::PackedAttribute,
) -> Result<()> {
    let component = attribute.component_type();
    let width = component.byte_width();
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for row in attribute.bytes().chunks_exact(3 * width) {
        for (axis, bytes) in row.chunks_exact(width).enumerate() {
            let value = match component {
                ComponentType::F32 => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
                ComponentType::I8 => f64::from(bytes[0] as i8),
                ComponentType::U8 => f64::from(bytes[0]),
                ComponentType::I16 => f64::from(i16::from_le_bytes(bytes.try_into().unwrap())),
                ComponentType::U16 => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())),
                _ => return Err(error("invalid POSITION component type")),
            };
            if !value.is_finite() {
                return Err(error("decoded POSITION must be finite"));
            }
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    for (key, expected) in [("min", min), ("max", max)] {
        let bounds = accessor
            .get(key)
            .and_then(JsonRef::as_array)
            .ok_or_else(|| error("missing POSITION bounds"))?;
        if bounds.len() != 3
            || bounds.iter().zip(expected).any(|(value, expected)| {
                value.as_f64().is_none_or(|value| {
                    // JSON numbers for Float32 accessors are interpreted as Float32.
                    let declared = if component == ComponentType::F32 {
                        f64::from(value as f32)
                    } else {
                        value
                    };
                    declared != expected
                })
            })
        {
            return Err(error("POSITION bounds disagree with decoded positions"));
        }
    }
    Ok(())
}
