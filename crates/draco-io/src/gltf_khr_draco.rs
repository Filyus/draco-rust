//! Strict `KHR_draco_mesh_compression` metadata parsing.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::gltf_geometry::{GltfError, Result};

/// Official extension name.
pub const KHR_DRACO_MESH_COMPRESSION: &str = "KHR_draco_mesh_compression";
const MODE_TRIANGLES: u64 = 4;
const MODE_TRIANGLE_STRIP: u64 = 5;

/// Strictly parsed `KHR_draco_mesh_compression` metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KhrDracoMeshCompression {
    /// Buffer view containing the Draco bitstream.
    pub buffer_view: usize,
    /// glTF semantic to Draco attribute unique ID.
    pub attributes: BTreeMap<String, u32>,
    /// `TRIANGLES` (4) or `TRIANGLE_STRIP` (5).
    pub mode: u32,
    /// Whether the extension is listed in `extensionsRequired`.
    pub required: bool,
}

/// Schema-only extension payload, usable when the surrounding document is owned
/// by another glTF front end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KhrDracoExtension {
    pub buffer_view: usize,
    pub attributes: BTreeMap<String, u32>,
}

/// Borrowed KHR declaration consumed by the shared semantic validator.
pub(crate) struct KhrDracoExtensionContract<'a> {
    pub buffer_view: usize,
    pub attributes: &'a BTreeMap<String, u32>,
}

pub(crate) struct KhrDracoPrimitiveContract<'a, A> {
    pub extension: KhrDracoExtensionContract<'a>,
    pub primitive_attributes: A,
    pub indices: Option<usize>,
    pub mode: u32,
    pub extension_used: bool,
    pub extension_required: bool,
    pub buffer_view_count: usize,
}

/// Validate KHR semantics independently of the JSON front end.
///
/// Both the raw-`Value` adapter below and the typed serde reader call this
/// function, so schema parsing can differ without duplicating extension rules.
pub(crate) fn validate_khr_draco_contract<'a, A, F>(
    contract: KhrDracoPrimitiveContract<'_, A>,
    mut accessor_has_fallback: F,
) -> Result<()>
where
    A: Iterator<Item = (&'a str, usize)> + Clone,
    F: FnMut(usize) -> Option<bool>,
{
    let KhrDracoPrimitiveContract {
        extension,
        primitive_attributes,
        indices,
        mode,
        extension_used,
        extension_required,
        buffer_view_count,
    } = contract;
    if !extension_used {
        return Err(GltfError::InvalidGltf(format!(
            "primitive uses {KHR_DRACO_MESH_COMPRESSION} but extensionsUsed does not list it"
        )));
    }
    if extension.buffer_view >= buffer_view_count {
        return Err(GltfError::InvalidGltf(format!(
            "{KHR_DRACO_MESH_COMPRESSION}.bufferView {} is out of range",
            extension.buffer_view
        )));
    }
    if extension.attributes.is_empty() {
        return Err(GltfError::InvalidGltf(format!(
            "{KHR_DRACO_MESH_COMPRESSION}.attributes must be a non-empty object"
        )));
    }
    if mode != MODE_TRIANGLES as u32 && mode != MODE_TRIANGLE_STRIP as u32 {
        return Err(GltfError::InvalidGltf(format!(
            "{KHR_DRACO_MESH_COMPRESSION} permits only TRIANGLES=4 or TRIANGLE_STRIP=5, got {mode}"
        )));
    }

    let mut has_primitive_attribute = false;
    for (semantic, accessor) in primitive_attributes.clone() {
        has_primitive_attribute = true;
        if !extension_required || !extension.attributes.contains_key(semantic) {
            match accessor_has_fallback(accessor) {
                Some(true) => {}
                Some(false) => {
                    return Err(GltfError::InvalidGltf(format!(
                        "{semantic} accessor {accessor} has no fallback data"
                    )));
                }
                None => {
                    return Err(GltfError::InvalidGltf(format!(
                        "accessor {accessor} is out of range"
                    )));
                }
            }
        }
    }
    if !has_primitive_attribute {
        return Err(GltfError::InvalidGltf(
            "primitive.attributes must be a non-empty object".into(),
        ));
    }
    for semantic in extension.attributes.keys() {
        if !primitive_attributes
            .clone()
            .any(|(primitive_semantic, _)| primitive_semantic == semantic)
        {
            return Err(GltfError::InvalidGltf(format!(
                "{KHR_DRACO_MESH_COMPRESSION} semantic {semantic} is not present in primitive.attributes"
            )));
        }
    }
    if let Some(accessor) = indices {
        if !extension_required {
            match accessor_has_fallback(accessor) {
                Some(true) => {}
                Some(false) => {
                    return Err(GltfError::InvalidGltf(format!(
                        "indices accessor {accessor} has no fallback data"
                    )));
                }
                None => {
                    return Err(GltfError::InvalidGltf(format!(
                        "accessor {accessor} is out of range"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Parse the extension object itself, including `additionalProperties: false`.
pub fn parse_khr_draco_extension_value(extension: &Value) -> Result<KhrDracoExtension> {
    let extension = extension.as_object().ok_or_else(|| {
        GltfError::InvalidGltf(format!(
            "{KHR_DRACO_MESH_COMPRESSION} value is not an object"
        ))
    })?;
    for key in extension.keys() {
        if key != "bufferView" && key != "attributes" {
            return Err(GltfError::InvalidGltf(format!(
                "unexpected {KHR_DRACO_MESH_COMPRESSION} property {key}"
            )));
        }
    }
    let buffer_view = extension
        .get("bufferView")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .map(|value| value as usize)
        .ok_or_else(|| {
            GltfError::InvalidGltf(format!(
                "{KHR_DRACO_MESH_COMPRESSION}.bufferView is not a valid u32 index"
            ))
        })?;
    let extension_attributes = extension
        .get("attributes")
        .and_then(Value::as_object)
        .filter(|attributes| !attributes.is_empty())
        .ok_or_else(|| {
            GltfError::InvalidGltf(format!(
                "{KHR_DRACO_MESH_COMPRESSION}.attributes must be a non-empty object"
            ))
        })?;
    let mut attributes = BTreeMap::new();
    for (semantic, value) in extension_attributes {
        let value = value.as_u64().ok_or_else(|| {
            GltfError::InvalidGltf(format!("Draco unique ID for {semantic} is not an integer"))
        })?;
        let unique_id = u32::try_from(value).map_err(|_| {
            GltfError::InvalidGltf(format!("Draco unique ID for {semantic} exceeds u32"))
        })?;
        attributes.insert(semantic.clone(), unique_id);
    }
    Ok(KhrDracoExtension {
        buffer_view,
        attributes,
    })
}

/// Validate every Draco primitive in a JSON glTF document.
pub fn validate_khr_draco_document(document: &Value) -> Result<()> {
    let used = extension_list(document, "extensionsUsed")?;
    let required = extension_list(document, "extensionsRequired")?;
    if required.contains(&KHR_DRACO_MESH_COMPRESSION) && !used.contains(&KHR_DRACO_MESH_COMPRESSION)
    {
        return Err(GltfError::InvalidGltf(format!(
            "{KHR_DRACO_MESH_COMPRESSION} is required but is not listed in extensionsUsed"
        )));
    }
    let Some(meshes) = document.get("meshes") else {
        return Ok(());
    };
    let meshes = meshes
        .as_array()
        .ok_or_else(|| GltfError::InvalidGltf("meshes is not an array".into()))?;
    for mesh in meshes {
        let primitives = mesh
            .get("primitives")
            .and_then(Value::as_array)
            .ok_or_else(|| GltfError::InvalidGltf("mesh.primitives is not an array".into()))?;
        for primitive in primitives {
            parse_khr_draco_mesh_compression(document, primitive)?;
        }
    }
    Ok(())
}

/// Parse and validate one primitive's extension against its document.
pub fn parse_khr_draco_mesh_compression(
    document: &Value,
    primitive: &Value,
) -> Result<Option<KhrDracoMeshCompression>> {
    let Some(extensions_value) = primitive.get("extensions") else {
        return Ok(None);
    };
    let extensions = extensions_value
        .as_object()
        .ok_or_else(|| GltfError::InvalidGltf("primitive.extensions is not an object".into()))?;
    let Some(extension) = extensions.get(KHR_DRACO_MESH_COMPRESSION) else {
        return Ok(None);
    };

    let used = extension_list(document, "extensionsUsed")?;
    let required = extension_list(document, "extensionsRequired")?;
    if required.contains(&KHR_DRACO_MESH_COMPRESSION) && !used.contains(&KHR_DRACO_MESH_COMPRESSION)
    {
        return Err(GltfError::InvalidGltf(format!(
            "{KHR_DRACO_MESH_COMPRESSION} is required but is not listed in extensionsUsed"
        )));
    }
    if !used.contains(&KHR_DRACO_MESH_COMPRESSION) {
        return Err(GltfError::InvalidGltf(format!(
            "primitive uses {KHR_DRACO_MESH_COMPRESSION} but extensionsUsed does not list it"
        )));
    }
    let required = required.contains(&KHR_DRACO_MESH_COMPRESSION);
    let parsed_extension = parse_khr_draco_extension_value(extension)?;
    let buffer_view = parsed_extension.buffer_view;
    let buffer_views = document
        .get("bufferViews")
        .and_then(Value::as_array)
        .ok_or_else(|| GltfError::InvalidGltf("missing bufferViews array".into()))?;
    if buffer_view >= buffer_views.len() {
        return Err(GltfError::InvalidGltf(format!(
            "{KHR_DRACO_MESH_COMPRESSION}.bufferView {buffer_view} is out of range"
        )));
    }
    let primitive_attributes = primitive
        .get("attributes")
        .and_then(Value::as_object)
        .filter(|attributes| !attributes.is_empty())
        .ok_or_else(|| {
            GltfError::InvalidGltf("primitive.attributes must be a non-empty object".into())
        })?;

    let attributes = parsed_extension.attributes;
    for semantic in attributes.keys() {
        if !primitive_attributes.contains_key(semantic) {
            return Err(GltfError::InvalidGltf(format!(
                "{KHR_DRACO_MESH_COMPRESSION} semantic {semantic} is not present in primitive.attributes"
            )));
        }
    }

    let mode_u64 = primitive
        .get("mode")
        .map(|mode| {
            mode.as_u64()
                .ok_or_else(|| GltfError::InvalidGltf("primitive.mode is not an integer".into()))
        })
        .transpose()?
        .unwrap_or(MODE_TRIANGLES);
    if mode_u64 != MODE_TRIANGLES && mode_u64 != MODE_TRIANGLE_STRIP {
        return Err(GltfError::InvalidGltf(format!(
            "{KHR_DRACO_MESH_COMPRESSION} permits only TRIANGLES=4 or TRIANGLE_STRIP=5, got {mode_u64}"
        )));
    }
    let mode = u32::try_from(mode_u64)
        .map_err(|_| GltfError::InvalidGltf("primitive.mode exceeds u32".into()))?;

    let accessors = document
        .get("accessors")
        .and_then(Value::as_array)
        .ok_or_else(|| GltfError::InvalidGltf("missing accessors array".into()))?;
    for (semantic, accessor_value) in primitive_attributes {
        let accessor = json_index(accessor_value, "attribute accessor")?;
        let has_compressed_value = attributes.contains_key(semantic);
        if !required || !has_compressed_value {
            require_accessor_fallback(accessors, accessor, semantic)?;
        }
    }
    let indices = primitive
        .get("indices")
        .map(|indices| json_index(indices, "indices accessor"))
        .transpose()?;
    if let Some(accessor) = indices {
        if !required {
            require_accessor_fallback(accessors, accessor, "indices")?;
        }
    }

    let mut contract_attributes = Vec::new();
    contract_attributes
        .try_reserve_exact(primitive_attributes.len())
        .map_err(|_| {
            GltfError::ResourceLimitExceeded("KHR primitive attribute allocation failed".into())
        })?;
    for (semantic, accessor) in primitive_attributes {
        contract_attributes.push((
            semantic.as_str(),
            json_index(accessor, "attribute accessor")?,
        ));
    }
    validate_khr_draco_contract(
        KhrDracoPrimitiveContract {
            extension: KhrDracoExtensionContract {
                buffer_view,
                attributes: &attributes,
            },
            primitive_attributes: contract_attributes.iter().copied(),
            indices,
            mode,
            extension_used: true,
            extension_required: required,
            buffer_view_count: buffer_views.len(),
        },
        |accessor| {
            accessors
                .get(accessor)
                .and_then(Value::as_object)
                .map(|accessor| {
                    accessor.contains_key("bufferView") || accessor.contains_key("sparse")
                })
        },
    )?;

    Ok(Some(KhrDracoMeshCompression {
        buffer_view,
        attributes,
        mode,
        required,
    }))
}

fn extension_list<'a>(document: &'a Value, name: &str) -> Result<Vec<&'a str>> {
    let Some(value) = document.get(name) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| GltfError::InvalidGltf(format!("{name} is not an array")))?;
    let mut extensions = Vec::new();
    extensions
        .try_reserve_exact(values.len())
        .map_err(|_| GltfError::ResourceLimitExceeded(format!("{name} allocation failed")))?;
    for value in values {
        extensions.push(
            value
                .as_str()
                .ok_or_else(|| GltfError::InvalidGltf(format!("{name} contains a non-string")))?,
        );
    }
    Ok(extensions)
}

fn json_index(value: &Value, label: &str) -> Result<usize> {
    value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| GltfError::InvalidGltf(format!("{label} is not a valid index")))
}

fn require_accessor_fallback(accessors: &[Value], index: usize, label: &str) -> Result<()> {
    let accessor = accessors
        .get(index)
        .and_then(Value::as_object)
        .ok_or_else(|| GltfError::InvalidGltf(format!("accessor {index} is out of range")))?;
    if !accessor.contains_key("bufferView") && !accessor.contains_key("sparse") {
        return Err(GltfError::InvalidGltf(format!(
            "{label} accessor {index} has no fallback data"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Value {
        serde_json::json!({
            "asset": {"version": "2.0"},
            "extensionsUsed": [KHR_DRACO_MESH_COMPRESSION],
            "extensionsRequired": [KHR_DRACO_MESH_COMPRESSION],
            "buffers": [{"byteLength": 4}],
            "bufferViews": [{"buffer": 0, "byteLength": 4}],
            "accessors": [{"componentType": 5126, "count": 3, "type": "VEC3"}],
            "meshes": [{"primitives": [{
                "attributes": {"POSITION": 0},
                "extensions": {KHR_DRACO_MESH_COMPRESSION: {
                    "bufferView": 0,
                    "attributes": {"POSITION": 20}
                }}
            }]}]
        })
    }

    #[test]
    fn accepts_u32_unique_ids_and_rejects_larger_values() {
        let mut document = base();
        let primitive = &document["meshes"][0]["primitives"][0];
        let parsed = parse_khr_draco_mesh_compression(&document, primitive)
            .unwrap()
            .unwrap();
        assert_eq!(parsed.attributes["POSITION"], 20);

        document["meshes"][0]["primitives"][0]["extensions"][KHR_DRACO_MESH_COMPRESSION]
            ["attributes"]["POSITION"] = Value::from(u64::from(u32::MAX) + 1);
        let primitive = &document["meshes"][0]["primitives"][0];
        assert!(parse_khr_draco_mesh_compression(&document, primitive).is_err());
    }
}
