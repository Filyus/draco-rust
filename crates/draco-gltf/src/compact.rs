//! Allocation-conscious geometry-oriented facade over the native document.
//!
//! This module does not parse a second schema. It exposes compact index/range
//! views over [`Document`].

use crate::{
    Document, Error, MeshIndex, NativeAccessorSource, NativeImport, Result, ValidationProfile,
};
use draco_io::{pack_draco_primitive, PackedAttribute, PackedPrimitive};

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
            return pack_draco_primitive(primitive_ref.mode(), &decoded, &contract.attributes)
                .map_err(Error::from);
        }
        let source = NativeAccessorSource::new(&self.document, &self.resources);
        let attributes = primitive_ref
            .attribute_indices()
            .map(|(semantic, index)| {
                let data = source.read_accessor(index.0)?;
                PackedAttribute::from_accessor(
                    semantic,
                    data.count,
                    data.components,
                    data.data_type,
                    data.normalized,
                    data.bytes,
                )
                .map_err(Error::from)
            })
            .collect::<Result<Vec<_>>>()?;
        let indices = primitive_ref
            .indices()
            .map(|index| {
                let data = source.read_accessor(index.0)?;
                PackedAttribute::from_accessor(
                    "INDICES",
                    data.count,
                    data.components,
                    data.data_type,
                    data.normalized,
                    data.bytes,
                )
                .map_err(Error::from)
            })
            .transpose()?;
        Ok(PackedPrimitive {
            mode: primitive_ref.mode(),
            indices,
            attributes,
        })
    }
}
