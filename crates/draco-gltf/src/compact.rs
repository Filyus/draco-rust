//! Allocation-conscious geometry-oriented facade over the native document.
//!
//! This module does not parse a second schema. It exposes compact index/range
//! views over [`Document`].

use crate::{Document, MeshIndex, Result, ValidationProfile};

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
