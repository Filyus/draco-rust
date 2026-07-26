//! Materialized primitive geometry shared by glTF read and write operations.

use std::collections::BTreeSet;

#[cfg(feature = "draco-decode")]
use draco_core::draco_types::DataType;
#[cfg(feature = "draco-decode")]
use draco_core::mesh::Mesh;
use thiserror::Error as ThisError;

use crate::{ComponentType, ValidationProfile};
#[cfg(feature = "draco-decode")]
use crate::{Error, Result};

/// glTF primitive topology mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum PrimitiveMode {
    /// Independent points.
    Points = 0,
    /// Independent line segments.
    Lines = 1,
    /// Closed line loop.
    LineLoop = 2,
    /// Connected line strip.
    LineStrip = 3,
    /// Independent triangles.
    #[default]
    Triangles = 4,
    /// Connected triangle strip.
    TriangleStrip = 5,
    /// Connected triangle fan.
    TriangleFan = 6,
}

impl PrimitiveMode {
    /// Converts a glTF primitive mode code to its typed representation.
    pub fn from_gltf(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Points,
            1 => Self::Lines,
            2 => Self::LineLoop,
            3 => Self::LineStrip,
            4 => Self::Triangles,
            5 => Self::TriangleStrip,
            6 => Self::TriangleFan,
            _ => return None,
        })
    }

    /// Returns the glTF numeric mode code.
    pub const fn to_gltf(self) -> u32 {
        self as u32
    }
}

/// Validation errors for materialized primitive geometry.
#[derive(Clone, Debug, ThisError, PartialEq, Eq)]
pub enum GeometryError {
    /// A byte-size calculation overflowed the addressable range.
    #[error("packed geometry byte size overflow")]
    ByteSizeOverflow,
    /// A byte payload does not match its declared accessor layout.
    #[error("packed {kind} has {actual} bytes, expected {expected}")]
    ByteLength {
        /// Kind of payload being checked.
        kind: &'static str,
        /// Actual byte length.
        actual: usize,
        /// Expected byte length.
        expected: usize,
    },
    /// An attribute semantic occurs more than once.
    #[error("duplicate packed attribute semantic {0:?}")]
    DuplicateSemantic(String),
    /// Vertex attributes do not agree on their element count.
    #[error("attribute {semantic:?} has count {actual}, expected {expected}")]
    AttributeCount {
        /// Attribute semantic.
        semantic: String,
        /// Actual element count.
        actual: usize,
        /// Expected element count.
        expected: usize,
    },
    /// A primitive has no POSITION attribute.
    #[error("packed geometry is missing POSITION")]
    MissingPosition,
    /// A primitive has no vertices.
    #[error("packed geometry has no vertices")]
    EmptyGeometry,
    /// A well-known attribute has the wrong number of components.
    #[error("{semantic:?} has {actual} components; expected {expected}")]
    AttributeComponents {
        /// Attribute semantic.
        semantic: String,
        /// Actual component count.
        actual: u8,
        /// Required component count.
        expected: &'static str,
    },
    /// A well-known attribute uses storage forbidden by the selected profile.
    #[error("invalid {component_type:?}/normalized={normalized} for {semantic:?} in {profile:?}")]
    AttributeComponentType {
        /// Attribute semantic.
        semantic: String,
        /// Rejected scalar storage type.
        component_type: ComponentType,
        /// Whether normalized integer interpretation was requested.
        normalized: bool,
        /// Active validation profile.
        profile: ValidationProfile,
    },
    /// A floating-point POSITION value cannot be represented in JSON bounds.
    #[cfg(feature = "write")]
    #[error("POSITION contains a non-finite floating-point value")]
    NonFinitePosition,
    /// The number of vertices or indices is invalid for the primitive mode.
    #[error("invalid {mode:?} element count {count}")]
    InvalidElementCount {
        /// Primitive topology.
        mode: PrimitiveMode,
        /// Number of indexed or non-indexed elements.
        count: usize,
    },
    /// A primitive mode code is outside the glTF core range.
    #[error("primitive mode {0} is not supported")]
    InvalidPrimitiveMode(u32),
    /// A component count is not valid for primitive attributes.
    #[error("packed attribute component count {0} is not supported")]
    InvalidComponents(u8),
    /// An index accessor uses a component type forbidden by the profile.
    #[error("index component type {0:?} is not permitted")]
    InvalidIndexType(ComponentType),
    /// An index references a vertex outside the attribute range.
    #[error("index {index} is outside vertex count {vertex_count}")]
    IndexOutOfRange {
        /// Invalid vertex index.
        index: u64,
        /// Number of vertices in the primitive.
        vertex_count: usize,
    },
    /// Decoded Draco topology disagrees with its glTF accessor metadata.
    #[error("decoded Draco {semantic} count {decoded} does not match accessor count {declared}")]
    DracoAccessorCount {
        /// Attribute semantic or `indices`.
        semantic: String,
        /// Count materialized from the Draco stream.
        decoded: u64,
        /// Count declared by the glTF accessor.
        declared: u64,
    },
    /// A component type is outside the selected validation profile.
    #[error("component type {component_type:?} is not permitted by {profile:?}")]
    ComponentTypeProfile {
        /// Rejected component type.
        component_type: ComponentType,
        /// Active validation profile.
        profile: ValidationProfile,
    },
    /// Draco cannot represent the supplied geometry without conversion.
    #[error("Draco encoding does not support {0}")]
    UnsupportedDraco(String),
    /// Existing morph targets would become invalid after replacement.
    #[cfg(feature = "write")]
    #[error("replacement vertex count {actual} does not match morph target count {expected}")]
    MorphTargetCount {
        /// Existing morph-target element count.
        expected: usize,
        /// Replacement vertex count.
        actual: usize,
    },
}

/// One materialized, tightly packed vertex attribute.
///
/// ```
/// use draco_gltf::{ComponentType, PackedAttribute};
///
/// let position = PackedAttribute::new(
///     "POSITION",
///     1,
///     3,
///     ComponentType::F32,
///     false,
///     vec![0; 12],
/// )?;
/// assert_eq!(position.count(), 1);
/// # Ok::<(), draco_gltf::GeometryError>(())
/// ```
#[derive(Clone, Debug, Eq)]
pub struct PackedAttribute {
    semantic: String,
    count: usize,
    components: u8,
    component_type: ComponentType,
    normalized: bool,
    bytes: Vec<u8>,
    source_accessor: Option<usize>,
}

/// Equality is over the vertex data, not over where it was read from.
///
/// `source_accessor` records provenance: the same bytes materialized from a
/// different document, or re-materialized after a write, are the same
/// attribute, and a round-trip that renumbers accessors has lost nothing.
impl PartialEq for PackedAttribute {
    fn eq(&self, other: &Self) -> bool {
        self.semantic == other.semantic
            && self.count == other.count
            && self.components == other.components
            && self.component_type == other.component_type
            && self.normalized == other.normalized
            && self.bytes == other.bytes
    }
}

impl PackedAttribute {
    /// Creates and validates a tightly packed vertex attribute.
    pub fn new(
        semantic: impl Into<String>,
        count: usize,
        components: u8,
        component_type: ComponentType,
        normalized: bool,
        bytes: Vec<u8>,
    ) -> std::result::Result<Self, GeometryError> {
        if !(1..=4).contains(&components) {
            return Err(GeometryError::InvalidComponents(components));
        }
        validate_byte_len("attribute", count, components, component_type, bytes.len())?;
        Ok(Self {
            semantic: semantic.into(),
            count,
            components,
            component_type,
            normalized,
            bytes,
            source_accessor: None,
        })
    }

    /// Records which document accessor these bytes were materialized from.
    ///
    /// Primitives routinely share one accessor — a mesh split by material is
    /// the usual case — and a consumer that rebuilds its own buffers has no
    /// other way to notice, since the bytes arrive already materialized. Left
    /// unset for compressed geometry, whose bytes come from the codec stream
    /// rather than from the accessor the attribute names.
    #[must_use]
    pub fn with_source_accessor(mut self, accessor: usize) -> Self {
        self.source_accessor = Some(accessor);
        self
    }

    /// Returns the document accessor these bytes came from, when known.
    pub const fn source_accessor(&self) -> Option<usize> {
        self.source_accessor
    }

    /// Returns the glTF attribute semantic.
    pub fn semantic(&self) -> &str {
        &self.semantic
    }

    /// Returns the number of attribute elements.
    pub const fn count(&self) -> usize {
        self.count
    }

    /// Returns the number of scalar components in each element.
    pub const fn components(&self) -> u8 {
        self.components
    }

    /// Returns the scalar storage type.
    pub const fn component_type(&self) -> ComponentType {
        self.component_type
    }

    /// Returns whether integer values use normalized interpretation.
    pub const fn normalized(&self) -> bool {
        self.normalized
    }

    /// Borrows the tightly packed row-major bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// One materialized, tightly packed primitive index stream.
#[derive(Clone, Debug, Eq)]
pub struct PackedIndices {
    count: usize,
    component_type: ComponentType,
    bytes: Vec<u8>,
    source_accessor: Option<usize>,
}

/// Equality is over the index data; see [`PackedAttribute`]'s implementation.
impl PartialEq for PackedIndices {
    fn eq(&self, other: &Self) -> bool {
        self.count == other.count
            && self.component_type == other.component_type
            && self.bytes == other.bytes
    }
}

impl PackedIndices {
    /// Creates and validates a tightly packed scalar index stream.
    pub fn new(
        count: usize,
        component_type: ComponentType,
        bytes: Vec<u8>,
    ) -> std::result::Result<Self, GeometryError> {
        if !matches!(
            component_type,
            ComponentType::U8 | ComponentType::U16 | ComponentType::U32
        ) {
            return Err(GeometryError::InvalidIndexType(component_type));
        }
        validate_byte_len("indices", count, 1, component_type, bytes.len())?;
        Ok(Self {
            count,
            component_type,
            bytes,
            source_accessor: None,
        })
    }

    /// Records which document accessor these indices were materialized from.
    ///
    /// See [`PackedAttribute::with_source_accessor`]; the same sharing applies.
    #[must_use]
    pub fn with_source_accessor(mut self, accessor: usize) -> Self {
        self.source_accessor = Some(accessor);
        self
    }

    /// Returns the document accessor these indices came from, when known.
    pub const fn source_accessor(&self) -> Option<usize> {
        self.source_accessor
    }

    /// Returns the number of indices.
    pub const fn count(&self) -> usize {
        self.count
    }

    /// Returns the scalar index storage type.
    pub const fn component_type(&self) -> ComponentType {
        self.component_type
    }

    /// Borrows the tightly packed little-endian index bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Materialized primitive geometry with contiguous attribute and index buffers.
///
/// ```
/// use draco_gltf::{ComponentType, PackedAttribute, PackedGeometry, PrimitiveMode};
///
/// let position = PackedAttribute::new(
///     "POSITION", 1, 3, ComponentType::F32, false, vec![0; 12],
/// )?;
/// let geometry = PackedGeometry::new(PrimitiveMode::Points, vec![position], None)?;
/// assert_eq!(geometry.vertex_count(), 1);
/// # Ok::<(), draco_gltf::GeometryError>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedGeometry {
    mode: PrimitiveMode,
    indices: Option<PackedIndices>,
    attributes: Vec<PackedAttribute>,
}

impl PackedGeometry {
    /// Creates and validates one materialized primitive.
    pub fn new(
        mode: PrimitiveMode,
        attributes: Vec<PackedAttribute>,
        indices: Option<PackedIndices>,
    ) -> std::result::Result<Self, GeometryError> {
        let geometry = Self {
            mode,
            indices,
            attributes,
        };
        geometry.validate(ValidationProfile::Gltf21Draft)?;
        Ok(geometry)
    }

    /// Returns the primitive topology.
    pub const fn mode(&self) -> PrimitiveMode {
        self.mode
    }

    /// Returns the shared vertex count.
    pub fn vertex_count(&self) -> usize {
        self.attributes.first().map_or(0, PackedAttribute::count)
    }

    /// Borrows the packed vertex attributes in document order.
    pub fn attributes(&self) -> &[PackedAttribute] {
        &self.attributes
    }

    /// Borrows the optional packed index stream.
    pub fn indices(&self) -> Option<&PackedIndices> {
        self.indices.as_ref()
    }

    /// Validates the geometry against a glTF profile.
    pub fn validate(&self, profile: ValidationProfile) -> std::result::Result<(), GeometryError> {
        let mut semantics = BTreeSet::new();
        let mut vertex_count = None;
        for attribute in &self.attributes {
            validate_component_profile(attribute.component_type, profile)?;
            validate_attribute_components(attribute)?;
            validate_attribute_profile(attribute, profile)?;
            if !semantics.insert(attribute.semantic.as_str()) {
                return Err(GeometryError::DuplicateSemantic(attribute.semantic.clone()));
            }
            match vertex_count {
                None => vertex_count = Some(attribute.count),
                Some(expected) if expected != attribute.count => {
                    return Err(GeometryError::AttributeCount {
                        semantic: attribute.semantic.clone(),
                        actual: attribute.count,
                        expected,
                    })
                }
                _ => {}
            }
        }
        if !semantics.contains("POSITION") {
            return Err(GeometryError::MissingPosition);
        }
        let vertex_count = vertex_count.unwrap_or(0);
        if vertex_count == 0 {
            return Err(GeometryError::EmptyGeometry);
        }
        if let Some(indices) = &self.indices {
            validate_component_profile(indices.component_type, profile)?;
            for index in index_values(indices) {
                let index = index?;
                if index >= vertex_count as u64 {
                    return Err(GeometryError::IndexOutOfRange {
                        index,
                        vertex_count,
                    });
                }
            }
        }
        validate_element_count(
            self.mode,
            self.indices
                .as_ref()
                .map_or(vertex_count, PackedIndices::count),
        )?;
        Ok(())
    }

    #[cfg(feature = "draco-decode")]
    pub(crate) fn from_draco_mesh(
        mode: PrimitiveMode,
        mesh: &Mesh,
        attributes: &[(String, u32)],
        normalized: &std::collections::BTreeMap<String, bool>,
    ) -> Result<Self> {
        let attributes = attributes
            .iter()
            .map(|(semantic, unique_id)| {
                let attribute = mesh.attribute_by_unique_id(*unique_id).ok_or_else(|| {
                    Error::Geometry(GeometryError::UnsupportedDraco(format!(
                        "decoded attribute {unique_id} is missing"
                    )))
                })?;
                PackedAttribute::new(
                    semantic.clone(),
                    mesh.num_points(),
                    attribute.num_components(),
                    component_type_for_data_type(attribute.data_type())?,
                    // The glTF accessor is authoritative here; the decoded
                    // Draco attribute carries its own flag, which encoders
                    // leave unset even for normalized colours and weights.
                    normalized
                        .get(semantic.as_str())
                        .copied()
                        .unwrap_or_else(|| attribute.normalized()),
                    packed_draco_attribute_bytes(mesh, *unique_id)?,
                )
                .map_err(Error::Geometry)
            })
            .collect::<Result<Vec<_>>>()?;
        let count = mesh
            .num_faces()
            .checked_mul(3)
            .ok_or(Error::Geometry(GeometryError::ByteSizeOverflow))?;
        let indices =
            PackedIndices::new(count, ComponentType::U32, packed_draco_index_bytes(mesh)?)
                .map_err(Error::Geometry)?;
        Self::new(mode, attributes, Some(indices)).map_err(Error::Geometry)
    }
}

fn validate_element_count(
    mode: PrimitiveMode,
    count: usize,
) -> std::result::Result<(), GeometryError> {
    let valid = match mode {
        PrimitiveMode::Points => count >= 1,
        PrimitiveMode::Lines => count >= 2 && count.is_multiple_of(2),
        PrimitiveMode::LineLoop | PrimitiveMode::LineStrip => count >= 2,
        PrimitiveMode::Triangles => count >= 3 && count.is_multiple_of(3),
        PrimitiveMode::TriangleStrip | PrimitiveMode::TriangleFan => count >= 3,
    };
    if !valid {
        return Err(GeometryError::InvalidElementCount { mode, count });
    }
    Ok(())
}

fn validate_attribute_components(
    attribute: &PackedAttribute,
) -> std::result::Result<(), GeometryError> {
    let expected = if attribute.semantic == "POSITION" || attribute.semantic == "NORMAL" {
        Some("3")
    } else if attribute.semantic == "TANGENT"
        || attribute.semantic.starts_with("JOINTS_")
        || attribute.semantic.starts_with("WEIGHTS_")
    {
        Some("4")
    } else if attribute.semantic.starts_with("TEXCOORD_") {
        Some("2")
    } else if attribute.semantic.starts_with("COLOR_") && !matches!(attribute.components, 3 | 4) {
        Some("3 or 4")
    } else {
        None
    };
    if let Some(expected) = expected {
        let valid = match expected {
            "2" => attribute.components == 2,
            "3" => attribute.components == 3,
            "4" => attribute.components == 4,
            "3 or 4" => matches!(attribute.components, 3 | 4),
            _ => unreachable!("known component requirement"),
        };
        if !valid {
            return Err(GeometryError::AttributeComponents {
                semantic: attribute.semantic.clone(),
                actual: attribute.components,
                expected,
            });
        }
    }
    Ok(())
}

fn validate_attribute_profile(
    attribute: &PackedAttribute,
    profile: ValidationProfile,
) -> std::result::Result<(), GeometryError> {
    if profile != ValidationProfile::Gltf20 {
        return Ok(());
    }
    let float = attribute.component_type == ComponentType::F32 && !attribute.normalized;
    let normalized_unsigned = matches!(
        attribute.component_type,
        ComponentType::U8 | ComponentType::U16
    ) && attribute.normalized;
    let valid = if matches!(
        attribute.semantic.as_str(),
        "POSITION" | "NORMAL" | "TANGENT"
    ) {
        float
    } else if attribute.semantic.starts_with("TEXCOORD_")
        || attribute.semantic.starts_with("COLOR_")
        || attribute.semantic.starts_with("WEIGHTS_")
    {
        float || normalized_unsigned
    } else if attribute.semantic.starts_with("JOINTS_") {
        matches!(
            attribute.component_type,
            ComponentType::U8 | ComponentType::U16
        ) && !attribute.normalized
    } else {
        true
    };
    if !valid {
        return Err(GeometryError::AttributeComponentType {
            semantic: attribute.semantic.clone(),
            component_type: attribute.component_type,
            normalized: attribute.normalized,
            profile,
        });
    }
    Ok(())
}

fn validate_component_profile(
    component_type: ComponentType,
    profile: ValidationProfile,
) -> std::result::Result<(), GeometryError> {
    if profile == ValidationProfile::Gltf20
        && !matches!(
            component_type,
            ComponentType::I8
                | ComponentType::U8
                | ComponentType::I16
                | ComponentType::U16
                | ComponentType::U32
                | ComponentType::F32
        )
    {
        return Err(GeometryError::ComponentTypeProfile {
            component_type,
            profile,
        });
    }
    Ok(())
}

fn validate_byte_len(
    kind: &'static str,
    count: usize,
    components: u8,
    component_type: ComponentType,
    actual: usize,
) -> std::result::Result<(), GeometryError> {
    let expected = count
        .checked_mul(components as usize)
        .and_then(|value| value.checked_mul(component_type.byte_width()))
        .ok_or(GeometryError::ByteSizeOverflow)?;
    if actual != expected {
        return Err(GeometryError::ByteLength {
            kind,
            actual,
            expected,
        });
    }
    Ok(())
}

fn index_values(
    indices: &PackedIndices,
) -> impl Iterator<Item = std::result::Result<u64, GeometryError>> + '_ {
    let width = indices.component_type.byte_width();
    indices.bytes.chunks_exact(width).map(move |bytes| {
        Ok(match indices.component_type {
            ComponentType::U8 => bytes[0] as u64,
            ComponentType::U16 => u16::from_le_bytes(bytes.try_into().unwrap()) as u64,
            ComponentType::U32 => u32::from_le_bytes(bytes.try_into().unwrap()) as u64,
            _ => return Err(GeometryError::InvalidIndexType(indices.component_type)),
        })
    })
}

#[cfg(feature = "draco-decode")]
fn component_type_for_data_type(data_type: DataType) -> Result<ComponentType> {
    match data_type {
        DataType::Int8 => Ok(ComponentType::I8),
        DataType::Uint8 => Ok(ComponentType::U8),
        DataType::Int16 => Ok(ComponentType::I16),
        DataType::Uint16 => Ok(ComponentType::U16),
        DataType::Int32 => Ok(ComponentType::I32),
        DataType::Uint32 => Ok(ComponentType::U32),
        DataType::Float32 => Ok(ComponentType::F32),
        DataType::Int64 => Ok(ComponentType::I64),
        DataType::Uint64 => Ok(ComponentType::U64),
        DataType::Float64 => Ok(ComponentType::F64),
        other => Err(Error::Geometry(GeometryError::UnsupportedDraco(format!(
            "component type {other:?}"
        )))),
    }
}

#[cfg(feature = "draco-decode")]
fn packed_draco_attribute_bytes(mesh: &Mesh, unique_id: u32) -> Result<Vec<u8>> {
    let attribute = mesh.attribute_by_unique_id(unique_id).ok_or_else(|| {
        Error::Geometry(GeometryError::UnsupportedDraco(format!(
            "decoded attribute {unique_id} is missing"
        )))
    })?;
    let stride = usize::try_from(attribute.byte_stride()).map_err(|_| {
        Error::Geometry(GeometryError::UnsupportedDraco(
            "decoded attribute stride is invalid".into(),
        ))
    })?;
    let byte_len = mesh
        .num_points()
        .checked_mul(stride)
        .ok_or(Error::Geometry(GeometryError::ByteSizeOverflow))?;
    let mut out = vec![0; byte_len];
    let mut row = vec![0; stride];
    for point in 0..mesh.num_points() {
        let index = attribute.mapped_index(draco_core::PointIndex(point as u32));
        if !attribute
            .buffer()
            .try_read(index.0 as usize * stride, &mut row)
        {
            return Err(Error::Geometry(GeometryError::UnsupportedDraco(
                "decoded attribute is out of bounds".into(),
            )));
        }
        out[point * stride..(point + 1) * stride].copy_from_slice(&row);
    }
    Ok(out)
}

#[cfg(feature = "draco-decode")]
fn packed_draco_index_bytes(mesh: &Mesh) -> Result<Vec<u8>> {
    let byte_len = mesh
        .num_faces()
        .checked_mul(3)
        .and_then(|value| value.checked_mul(4))
        .ok_or(Error::Geometry(GeometryError::ByteSizeOverflow))?;
    let mut out = Vec::with_capacity(byte_len);
    for face in 0..mesh.num_faces() {
        for index in mesh.face(draco_core::FaceIndex(face as u32)) {
            out.extend_from_slice(&index.0.to_le_bytes());
        }
    }
    Ok(out)
}
