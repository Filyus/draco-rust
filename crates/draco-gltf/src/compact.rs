//! Allocation-conscious geometry-oriented facade over the native document.
//!
//! This module does not parse a second schema. It exposes compact index/range
//! views over [`Document`].

use crate::{
    Document, Error, MeshIndex, NativeAccessorSource, NativeImport, Result, ValidationProfile,
};
use draco_core::draco_types::DataType;

#[derive(Clone, Debug)]
pub struct CompactDocument {
    document: Document,
}

impl CompactDocument {
    pub fn parse(bytes: &[u8], profile: ValidationProfile) -> Result<Self> {
        let document = Document::from_json_bytes(bytes)?;
        document.validate(profile)?;
        Ok(Self { document })
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn mesh_primitive_ranges(&self) -> impl Iterator<Item = CompactMeshRange> + '_ {
        self.document
            .meshes()
            .into_iter()
            .map(|mesh| CompactMeshRange {
                mesh: mesh.index(),
                primitives: mesh
                    .value()
                    .get("primitives")
                    .and_then(|value| value.as_array())
                    .map_or(0, |values| values.len()),
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompactMeshRange {
    pub mesh: MeshIndex,
    pub primitives: usize,
}

/// One materialized, tightly packed geometry attribute suitable for direct typed-array use.
#[derive(Clone, Debug)]
pub struct PackedAttribute {
    pub semantic: String,
    pub components: u8,
    pub component_type: u32,
    pub normalized: bool,
    pub bytes: Vec<u8>,
}

/// Materialized primitive geometry with no scene or JavaScript-array expansion.
#[derive(Clone, Debug)]
pub struct PackedPrimitive {
    pub mode: u32,
    pub indices: Option<PackedAttribute>,
    pub attributes: Vec<PackedAttribute>,
}

impl NativeImport {
    /// Decodes ordinary accessors or `KHR_draco_mesh_compression` into packed buffers.
    pub fn decode_packed_primitive(
        &self,
        mesh: MeshIndex,
        primitive: usize,
    ) -> Result<PackedPrimitive> {
        let primitive_ref = self
            .document
            .primitive(mesh, primitive)
            .ok_or_else(|| Error::Extension("primitive out of range".into()))?;
        if primitive_ref
            .extension(crate::KHR_DRACO_MESH_COMPRESSION)
            .is_some()
        {
            let contract = crate::extensions::parse_draco_extension(
                primitive_ref.extension(crate::KHR_DRACO_MESH_COMPRESSION),
            )?
            .ok_or_else(|| Error::Extension("missing Draco extension".into()))?;
            let decoded = self.decode_primitive(primitive_ref)?;
            let attributes = contract
                .attributes
                .into_iter()
                .map(|(semantic, unique_id)| {
                    let attribute = decoded.attribute_by_unique_id(unique_id).ok_or_else(|| {
                        Error::Extension(format!("decoded Draco attribute {unique_id} is missing"))
                    })?;
                    Ok(PackedAttribute {
                        semantic,
                        components: attribute.num_components(),
                        component_type: component_type(attribute.data_type())?,
                        normalized: attribute.normalized(),
                        bytes: crate::native_import::decoded_attribute_bytes(&decoded, unique_id)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok(PackedPrimitive {
                mode: primitive_ref.mode(),
                indices: Some(PackedAttribute {
                    semantic: "INDICES".into(),
                    components: 1,
                    component_type: 5125,
                    normalized: false,
                    bytes: crate::native_import::decoded_index_bytes(&decoded)?,
                }),
                attributes,
            });
        }
        let source = NativeAccessorSource::new(&self.document, &self.resources);
        let attributes = primitive_ref
            .attribute_indices()
            .map(|(semantic, index)| {
                let data = source.read_accessor(index.0)?;
                Ok::<PackedAttribute, Error>(PackedAttribute {
                    semantic: semantic.into(),
                    components: data.components,
                    component_type: component_type(data.data_type)?,
                    normalized: data.normalized,
                    bytes: data.bytes,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let indices = primitive_ref
            .indices()
            .map(|index| {
                let data = source.read_accessor(index.0)?;
                Ok::<PackedAttribute, Error>(PackedAttribute {
                    semantic: "INDICES".into(),
                    components: data.components,
                    component_type: component_type(data.data_type)?,
                    normalized: data.normalized,
                    bytes: data.bytes,
                })
            })
            .transpose()?;
        Ok(PackedPrimitive {
            mode: primitive_ref.mode(),
            indices,
            attributes,
        })
    }
}

fn component_type(data_type: DataType) -> Result<u32> {
    match data_type {
        DataType::Int8 => Ok(5120),
        DataType::Uint8 => Ok(5121),
        DataType::Int16 => Ok(5122),
        DataType::Uint16 => Ok(5123),
        DataType::Uint32 => Ok(5125),
        DataType::Float32 => Ok(5126),
        DataType::Int32 => Ok(5127),
        DataType::Float64 => Ok(5132),
        DataType::Int64 => Ok(5133),
        DataType::Uint64 => Ok(5134),
        _ => Err(Error::Extension(
            "unsupported compact attribute data type".into(),
        )),
    }
}
