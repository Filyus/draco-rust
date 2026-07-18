//! Allocation-conscious geometry facade over one lossless glTF import.
//!
//! The compact profile uses the same [`crate::Document`] and resource store as
//! the full scene API. It narrows the public workflow to primitive geometry.

use crate::{
    Document, Import, ImportOptions, MeshIndex, OutputFormat, PackedGeometry, PrimitiveIndex,
    Result, ValidationProfile,
};

/// Compact geometry facade backed by one lossless document and its resources.
#[derive(Clone)]
pub struct CompactDocument {
    import: Import,
}

impl CompactDocument {
    /// Parses JSON glTF or GLB and validates the selected profile.
    ///
    /// ```
    /// # use draco_gltf::{CompactDocument, ValidationProfile};
    /// let input = br#"{"asset":{"version":"2.0"},"meshes":[]}"#;
    /// let document = CompactDocument::parse(input, ValidationProfile::Gltf20)?;
    /// assert_eq!(document.mesh_primitive_ranges().count(), 0);
    /// # Ok::<(), draco_gltf::Error>(())
    /// ```
    pub fn parse(bytes: &[u8], profile: ValidationProfile) -> Result<Self> {
        crate::parse(bytes, profile).map(Self::from_import)
    }

    /// Parses a document with explicit resource and validation options.
    pub fn parse_with_options(bytes: &[u8], options: &ImportOptions<'_>) -> Result<Self> {
        crate::import_slice_with_options(bytes, options).map(Self::from_import)
    }

    /// Wraps an existing fully resolved import with the compact facade.
    pub fn from_import(import: Import) -> Self {
        Self { import }
    }

    /// Returns the underlying lossless document.
    pub fn document(&self) -> &Document {
        &self.import.document
    }

    /// Returns the underlying resolved import.
    pub fn as_import(&self) -> &Import {
        &self.import
    }

    /// Consumes the facade and returns the underlying import.
    pub fn into_import(self) -> Import {
        self.import
    }

    /// Lists each mesh and its primitive count without materializing geometry.
    pub fn mesh_primitive_ranges(&self) -> impl Iterator<Item = CompactMeshRange> + '_ {
        self.document()
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

    /// Reads one ordinary or Draco-compressed primitive into packed buffers.
    ///
    /// This delegates to [`Import::read_primitive`] and returns the same
    /// bidirectional [`PackedGeometry`] representation as the full API.
    pub fn read_primitive(&self, primitive: PrimitiveIndex) -> Result<PackedGeometry> {
        self.import.read_primitive(primitive)
    }

    /// Serializes this document to JSON glTF, GLB v2 or GLB v3.
    pub fn to_bytes(&self, output: OutputFormat) -> Result<Vec<u8>> {
        self.import.to_bytes(output)
    }

    /// Serializes this document and its companion `.gltf` resources.
    pub fn to_gltf_output(&self) -> Result<crate::GltfOutput> {
        self.import.to_gltf_output()
    }

    /// Replaces one primitive with packed geometry atomically.
    ///
    /// This delegates to [`Import::write_primitive`].
    #[cfg(feature = "write")]
    pub fn write_primitive(
        &mut self,
        primitive: PrimitiveIndex,
        geometry: &PackedGeometry,
        options: crate::GeometryWriteOptions,
    ) -> Result<crate::GeometryWriteReport> {
        self.import.write_primitive(primitive, geometry, options)
    }

    /// Appends packed geometry to an existing mesh atomically.
    ///
    /// This delegates to [`Import::push_primitive`].
    #[cfg(feature = "write")]
    pub fn push_primitive(
        &mut self,
        mesh: MeshIndex,
        geometry: &PackedGeometry,
        options: crate::GeometryWriteOptions,
    ) -> Result<PrimitiveIndex> {
        self.import.push_primitive(mesh, geometry, options)
    }

    /// Creates a minimal scene containing one packed primitive.
    #[cfg(feature = "write")]
    pub fn from_geometry(
        geometry: &PackedGeometry,
        profile: ValidationProfile,
        options: crate::GeometryWriteOptions,
    ) -> Result<Self> {
        Import::from_geometry(geometry, profile, options).map(Self::from_import)
    }
}

/// Mesh index and primitive count exposed by the compact facade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompactMeshRange {
    /// Typed mesh index.
    pub mesh: MeshIndex,
    /// Number of primitives in the mesh.
    pub primitives: usize,
}
