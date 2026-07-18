//! Reader-agnostic glTF geometry decoding.
//!
//! This module holds the parts of glTF geometry handling that do **not** depend
//! on `draco-io`'s own glTF reader: the shared error type, the
//! [`AccessorSource`] seam, and [`decode_geometry`], which builds a
//! [`draco_core::Mesh`] (faces, deduplication, attribute typing, multi-set
//! semantics) from whatever accessor data a source yields.
//!
//! It is compiled whenever the glTF reader **or** writer is enabled, so the
//! document-preserving compressor ([`crate::compress_gltf_value`]) and external
//! front ends (e.g. a `gltf-rs` document) can reuse the same decode logic with
//! only the encoder, never linking the reader.

use std::io;

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::mesh::Mesh;
use thiserror::Error;

/// Errors that can occur when reading or decoding glTF geometry.
#[derive(Error, Debug)]
pub enum GltfError {
    /// Filesystem or stream I/O failed.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// glTF JSON parsing failed.
    ///
    /// Binary GLB structure is invalid.
    #[error("Invalid GLB: {0}")]
    InvalidGlb(String),

    /// glTF JSON or accessor/buffer structure is invalid.
    #[error("Invalid glTF: {0}")]
    InvalidGltf(String),

    /// Embedded Draco payload failed to decode.
    #[error("Draco decode error: {0}")]
    DracoDecode(#[source] draco_core::DracoError),

    /// Draco encoding failed with a typed codec error.
    #[error("Draco encode error: {0}")]
    DracoEncode(#[source] draco_core::DracoError),

    /// The asset uses a glTF feature outside this crate's geometry scope.
    #[error("Unsupported feature: {0}")]
    Unsupported(String),

    /// An external resource was rejected by policy or confinement.
    #[error("External resource denied: {0}")]
    ExternalResourceDenied(String),

    /// A configured resource quota or a checked allocation was exceeded.
    #[error("Resource limit exceeded: {0}")]
    ResourceLimitExceeded(String),

    /// Unknown extension JSON contains binary references that cannot be remapped safely.
    #[error("Opaque binary reference: {0}")]
    OpaqueBinaryReference(String),

    /// Compression options are outside their supported range.
    #[error("Invalid compression options: {0}")]
    InvalidOptions(String),
}

/// Result type used by glTF readers and the geometry decoder.
pub type Result<T> = std::result::Result<T, GltfError>;

pub(crate) const GLTF_MODE_POINTS: u32 = 0;
pub(crate) const GLTF_MODE_TRIANGLES: u32 = 4;
pub(crate) const GLTF_COMPONENT_BYTE: u32 = 5120;
pub(crate) const GLTF_COMPONENT_UNSIGNED_BYTE: u32 = 5121;
pub(crate) const GLTF_COMPONENT_SHORT: u32 = 5122;
pub(crate) const GLTF_COMPONENT_UNSIGNED_SHORT: u32 = 5123;
// Index component type, used only by the reader's index decoding.
#[cfg(feature = "gltf-reader")]
pub(crate) const GLTF_COMPONENT_UNSIGNED_INT: u32 = 5125;
pub(crate) const GLTF_COMPONENT_FLOAT: u32 = 5126;

/// One glTF accessor decoded to a tight, row-major byte block plus its layout.
///
/// This is the reader-agnostic unit the geometry decoder consumes: an
/// [`AccessorSource`] produces these, and [`decode_geometry`] turns them into a
/// [`Mesh`]. It carries the *original* component type / normalized flag, so the
/// output accessors (which the compressor preserves) keep matching layout.
pub struct DecodedAccessor {
    count: usize,
    num_components: u8,
    data_type: DataType,
    normalized: bool,
    bytes: Vec<u8>,
}

impl DecodedAccessor {
    /// Builds a decoded accessor from already-extracted values. `bytes` must be
    /// `count * num_components * data_type.byte_length()` long, row-major.
    pub fn new(
        count: usize,
        num_components: u8,
        data_type: DataType,
        normalized: bool,
        bytes: Vec<u8>,
    ) -> Result<Self> {
        let expected = count
            .checked_mul(num_components as usize)
            .and_then(|count| count.checked_mul(data_type.byte_length()))
            .ok_or_else(|| GltfError::InvalidGltf("accessor byte size overflow".into()))?;
        if bytes.len() != expected {
            return Err(GltfError::InvalidGltf(format!(
                "accessor has {} bytes, expected {expected}",
                bytes.len()
            )));
        }
        Ok(Self {
            count,
            num_components,
            data_type,
            normalized,
            bytes,
        })
    }

    fn gather(&self, indices: &[u32]) -> Result<Self> {
        let stride = (self.num_components as usize)
            .checked_mul(self.data_type.byte_length())
            .ok_or_else(|| GltfError::InvalidGltf("accessor stride overflow".into()))?;
        let byte_len = indices
            .len()
            .checked_mul(stride)
            .ok_or_else(|| GltfError::InvalidGltf("gathered accessor size overflow".into()))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(byte_len).map_err(|_| {
            GltfError::ResourceLimitExceeded("gathered accessor allocation failed".into())
        })?;

        for &index in indices {
            let index = index as usize;
            if index >= self.count {
                return Err(GltfError::InvalidGltf(format!(
                    "Accessor index {} out of bounds for {} values",
                    index, self.count
                )));
            }
            let offset = index
                .checked_mul(stride)
                .ok_or_else(|| GltfError::InvalidGltf("accessor offset overflow".into()))?;
            let end = offset
                .checked_add(stride)
                .filter(|end| *end <= self.bytes.len())
                .ok_or_else(|| GltfError::InvalidGltf("accessor bytes are truncated".into()))?;
            bytes.extend_from_slice(&self.bytes[offset..end]);
        }

        Ok(Self {
            count: indices.len(),
            num_components: self.num_components,
            data_type: self.data_type,
            normalized: self.normalized,
            bytes,
        })
    }
}

/// Source of raw accessor data for [`decode_geometry`].
///
/// This is the seam that lets the geometry decoder run against different glTF
/// front ends: `draco-io`'s own accessor reader implements it over the parsed
/// glTF document, but a caller that already holds a parsed scene (e.g. a
/// `gltf-rs` document) can implement it over that instead and reuse the exact
/// same decode logic, without linking `draco-io`'s glTF reader.
///
/// Implementors only have to locate and copy out bytes; all of the geometry
/// model building (faces, deduplication, attribute typing, multi-set semantics)
/// lives once in [`decode_geometry`].
pub trait AccessorSource {
    /// Reads one attribute accessor, validating its glTF type against
    /// `expected_types` (e.g. `["VEC3"]`) and component type against
    /// `allowed_component_types` (glTF component-type constants).
    fn read_attribute(
        &self,
        accessor: usize,
        expected_types: &[&str],
        allowed_component_types: &[u32],
    ) -> Result<DecodedAccessor>;

    /// Reads a `SCALAR` index accessor as `u32` values.
    fn read_indices(&self, accessor: usize) -> Result<Vec<u32>>;
}

#[derive(Clone, Copy)]
pub(crate) struct SemanticSpec {
    pub(crate) attribute_type: GeometryAttributeType,
    pub(crate) expected_accessor_types: &'static [&'static str],
    pub(crate) allowed_component_types: &'static [u32],
    normalization: NormalizationPolicy,
}

#[derive(Clone, Copy)]
enum NormalizationPolicy {
    Forbidden,
    RequiredForInteger,
    Generic,
}

const FLOAT_ONLY: &[u32] = &[GLTF_COMPONENT_FLOAT];
const TEXCOORD_COMPONENT_TYPES: &[u32] = &[
    GLTF_COMPONENT_FLOAT,
    GLTF_COMPONENT_UNSIGNED_BYTE,
    GLTF_COMPONENT_UNSIGNED_SHORT,
];
const COLOR_COMPONENT_TYPES: &[u32] = &[
    GLTF_COMPONENT_FLOAT,
    GLTF_COMPONENT_UNSIGNED_BYTE,
    GLTF_COMPONENT_UNSIGNED_SHORT,
];
const JOINT_COMPONENT_TYPES: &[u32] =
    &[GLTF_COMPONENT_UNSIGNED_BYTE, GLTF_COMPONENT_UNSIGNED_SHORT];
const WEIGHT_COMPONENT_TYPES: &[u32] = &[
    GLTF_COMPONENT_FLOAT,
    GLTF_COMPONENT_UNSIGNED_BYTE,
    GLTF_COMPONENT_UNSIGNED_SHORT,
];
const GENERIC_COMPONENT_TYPES: &[u32] = &[
    GLTF_COMPONENT_BYTE,
    GLTF_COMPONENT_UNSIGNED_BYTE,
    GLTF_COMPONENT_SHORT,
    GLTF_COMPONENT_UNSIGNED_SHORT,
    GLTF_COMPONENT_FLOAT,
];

/// Decodes a non-Draco primitive's geometry into a [`Mesh`] plus its
/// `(glTF semantic, Draco unique id)` mapping, reading attribute and index data
/// through any [`AccessorSource`].
///
/// `mode` is the glTF primitive mode (only `POINTS` = 0 and `TRIANGLES` = 4 are
/// supported), `attributes` maps each glTF semantic to its accessor index, and
/// `indices` is the optional index accessor. This is the single place the
/// geometry model is built — faces, deduplication, attribute typing, multi-set
/// `TEXCOORD_n`/`COLOR_n`, `TANGENT`/`JOINTS_n`/`WEIGHTS_n`, and custom `_*`
/// attributes — so different glTF front ends share it by implementing only
/// [`AccessorSource`], never duplicating this logic.
///
/// The returned attribute ids equal the Draco unique ids referenced by the
/// `KHR_draco_mesh_compression` attributes map.
pub fn decode_geometry<S: AccessorSource>(
    src: &S,
    mode: u32,
    attributes: &[(String, usize)],
    indices: Option<usize>,
) -> Result<(Mesh, Vec<(String, u32)>)> {
    if mode != GLTF_MODE_TRIANGLES && mode != GLTF_MODE_POINTS {
        return Err(GltfError::Unsupported(format!(
            "Primitive mode {} not supported (only POINTS=0 and TRIANGLES=4)",
            mode
        )));
    }

    // POSITION is required.
    let pos_accessor_idx = attributes
        .iter()
        .find(|(semantic, _)| semantic == "POSITION")
        .map(|(_, accessor)| *accessor)
        .ok_or_else(|| GltfError::InvalidGltf("primitive has no POSITION attribute".into()))?;

    let positions = src.read_attribute(pos_accessor_idx, &["VEC3"], &[GLTF_COMPONENT_FLOAT])?;
    validate_decoded_semantic("POSITION", &positions)?;

    let mut mesh = Mesh::new();
    let point_indices = if mode == GLTF_MODE_POINTS {
        indices.map(|idx| src.read_indices(idx)).transpose()?
    } else {
        None
    };
    let positions = if let Some(idx) = &point_indices {
        positions.gather(idx)?
    } else {
        positions
    };
    mesh.set_num_points(positions.count);

    let mut semantics: Vec<(String, u32)> = Vec::new();
    semantics.try_reserve_exact(attributes.len()).map_err(|_| {
        GltfError::ResourceLimitExceeded("attribute semantic table allocation failed".into())
    })?;
    let pos_id = add_decoded_attribute(&mut mesh, GeometryAttributeType::Position, positions)?;
    semantics.push(("POSITION".to_string(), pos_id as u32));

    if mode == GLTF_MODE_TRIANGLES {
        if let Some(indices_accessor_idx) = indices {
            let indices = src.read_indices(indices_accessor_idx)?;
            if indices.len() % 3 != 0 {
                return Err(GltfError::InvalidGltf(
                    "Index count not divisible by 3 for triangles".into(),
                ));
            }
            for &index in &indices {
                if index as usize >= mesh.num_points() {
                    return Err(GltfError::InvalidGltf(format!(
                        "Triangle index {} out of bounds for {} points",
                        index,
                        mesh.num_points()
                    )));
                }
            }
            let num_faces = indices.len() / 3;
            mesh.try_set_num_faces(num_faces)
                .map_err(GltfError::DracoEncode)?;
            for (face_id, face) in indices.chunks_exact(3).enumerate() {
                mesh.set_face_from_indices(face_id, [face[0], face[1], face[2]]);
            }
        } else {
            // Non-indexed: generate sequential triangle faces.
            if !mesh.num_points().is_multiple_of(3) {
                return Err(GltfError::InvalidGltf(
                    "Non-indexed primitive point count not divisible by 3".into(),
                ));
            }
            let num_faces = mesh.num_points() / 3;
            mesh.try_set_num_faces(num_faces)
                .map_err(GltfError::DracoEncode)?;
            for face_id in 0..num_faces {
                let base = face_id
                    .checked_mul(3)
                    .and_then(|base| u32::try_from(base).ok())
                    .ok_or_else(|| {
                        GltfError::InvalidGltf(
                            "Non-indexed primitive exceeds Draco's u32 point-id limit".into(),
                        )
                    })?;
                let second = base
                    .checked_add(1)
                    .ok_or_else(|| GltfError::InvalidGltf("Triangle point-id overflow".into()))?;
                let third = base
                    .checked_add(2)
                    .ok_or_else(|| GltfError::InvalidGltf("Triangle point-id overflow".into()))?;
                mesh.set_face_from_indices(face_id, [base, second, third]);
            }
        }
    }

    // Optionally read NORMAL.
    if let Some(normal_idx) = attributes
        .iter()
        .find(|(semantic, _)| semantic == "NORMAL")
        .map(|(_, accessor)| *accessor)
    {
        let spec = supported_semantic_spec("NORMAL")?;
        let normal_id = read_and_add_standard_attribute(
            &mut mesh,
            src,
            normal_idx,
            "NORMAL",
            spec,
            point_indices.as_deref(),
        )?;
        semantics.push(("NORMAL".to_string(), normal_id as u32));
    }

    // Read every remaining semantic in sorted order. Draco can carry multiple
    // attributes with the same semantic type (extra TEXCOORD_n/COLOR_n), plus
    // TANGENT, JOINTS_n, WEIGHTS_n, and custom `_*`.
    let mut sorted: Vec<&(String, usize)> = Vec::new();
    sorted.try_reserve_exact(attributes.len()).map_err(|_| {
        GltfError::ResourceLimitExceeded("attribute sort table allocation failed".into())
    })?;
    sorted.extend(attributes.iter());
    sorted.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (semantic, accessor_idx) in sorted {
        if semantic == "POSITION" || semantic == "NORMAL" {
            continue;
        }
        let attribute_spec = supported_semantic_spec(semantic)?;
        let att_id = read_and_add_standard_attribute(
            &mut mesh,
            src,
            *accessor_idx,
            semantic,
            attribute_spec,
            point_indices.as_deref(),
        )?;
        semantics.push((semantic.clone(), att_id as u32));
    }

    // Match C++ Draco: deduplicate point IDs in face-traversal order for binary
    // compatibility. Remapping does not change attribute ids, so `semantics`
    // stays valid. (Draco-compressed meshes don't need this.)
    mesh.deduplicate_point_ids();

    Ok((mesh, semantics))
}

/// Reads the attribute for `semantic` from `src` and adds it to `mesh`,
/// returning its attribute id. Used by the reader for side attributes that
/// accompany a Draco stream but are not carried inside it.
#[cfg(feature = "gltf-reader")]
pub(crate) fn add_named_attribute<S: AccessorSource>(
    mesh: &mut Mesh,
    src: &S,
    semantic: &str,
    accessor_idx: usize,
    point_indices: Option<&[u32]>,
) -> Result<i32> {
    let spec = supported_semantic_spec(semantic)?;
    read_and_add_standard_attribute(mesh, src, accessor_idx, semantic, spec, point_indices)
}

fn read_and_add_standard_attribute<S: AccessorSource>(
    mesh: &mut Mesh,
    src: &S,
    accessor_idx: usize,
    semantic: &str,
    spec: SemanticSpec,
    point_indices: Option<&[u32]>,
) -> Result<i32> {
    let decoded = src.read_attribute(
        accessor_idx,
        spec.expected_accessor_types,
        spec.allowed_component_types,
    )?;
    validate_decoded_semantic(semantic, &decoded)?;
    let decoded = if let Some(indices) = point_indices {
        decoded.gather(indices)?
    } else {
        decoded
    };
    add_decoded_attribute(mesh, spec.attribute_type, decoded)
}

/// Adds a decoded attribute to the mesh, returning the new attribute id (which
/// equals its Draco unique id).
fn add_decoded_attribute(
    mesh: &mut Mesh,
    attribute_type: GeometryAttributeType,
    decoded: DecodedAccessor,
) -> Result<i32> {
    if decoded.count != mesh.num_points() {
        return Err(GltfError::InvalidGltf(format!(
            "Attribute {:?} has {} values but primitive has {} points",
            attribute_type,
            decoded.count,
            mesh.num_points()
        )));
    }

    let mut attribute = PointAttribute::new();
    attribute
        .try_init(
            attribute_type,
            decoded.num_components,
            decoded.data_type,
            decoded.normalized,
            decoded.count,
        )
        .map_err(GltfError::DracoEncode)?;
    if !attribute.buffer_mut().try_write(0, &decoded.bytes) {
        return Err(GltfError::DracoEncode(draco_core::DracoError::BufferError(
            "Decoded glTF attribute does not fit its Draco buffer".into(),
        )));
    }
    Ok(mesh.add_attribute(attribute))
}

pub(crate) fn supported_semantic_spec(semantic: &str) -> Result<SemanticSpec> {
    let spec = if semantic == "POSITION" {
        SemanticSpec {
            attribute_type: GeometryAttributeType::Position,
            expected_accessor_types: &["VEC3"],
            allowed_component_types: FLOAT_ONLY,
            normalization: NormalizationPolicy::Forbidden,
        }
    } else if semantic == "NORMAL" {
        SemanticSpec {
            attribute_type: GeometryAttributeType::Normal,
            expected_accessor_types: &["VEC3"],
            allowed_component_types: FLOAT_ONLY,
            normalization: NormalizationPolicy::Forbidden,
        }
    } else if semantic == "TANGENT" {
        SemanticSpec {
            attribute_type: GeometryAttributeType::Generic,
            expected_accessor_types: &["VEC4"],
            allowed_component_types: FLOAT_ONLY,
            normalization: NormalizationPolicy::Forbidden,
        }
    } else if indexed_semantic(semantic, "TEXCOORD_") {
        SemanticSpec {
            attribute_type: GeometryAttributeType::TexCoord,
            expected_accessor_types: &["VEC2"],
            allowed_component_types: TEXCOORD_COMPONENT_TYPES,
            normalization: NormalizationPolicy::RequiredForInteger,
        }
    } else if indexed_semantic(semantic, "COLOR_") {
        SemanticSpec {
            attribute_type: GeometryAttributeType::Color,
            expected_accessor_types: &["VEC3", "VEC4"],
            allowed_component_types: COLOR_COMPONENT_TYPES,
            normalization: NormalizationPolicy::RequiredForInteger,
        }
    } else if indexed_semantic(semantic, "JOINTS_") {
        SemanticSpec {
            attribute_type: GeometryAttributeType::Generic,
            expected_accessor_types: &["VEC4"],
            allowed_component_types: JOINT_COMPONENT_TYPES,
            normalization: NormalizationPolicy::Forbidden,
        }
    } else if indexed_semantic(semantic, "WEIGHTS_") {
        SemanticSpec {
            attribute_type: GeometryAttributeType::Generic,
            expected_accessor_types: &["VEC4"],
            allowed_component_types: WEIGHT_COMPONENT_TYPES,
            normalization: NormalizationPolicy::RequiredForInteger,
        }
    } else if semantic.starts_with('_') && semantic.len() > 1 {
        // Application-specific semantics are carried as generic Draco
        // attributes. The semantic name remains in primitive.attributes and in
        // the KHR_draco_mesh_compression attribute map.
        SemanticSpec {
            attribute_type: GeometryAttributeType::Generic,
            expected_accessor_types: &["SCALAR", "VEC2", "VEC3", "VEC4"],
            allowed_component_types: GENERIC_COMPONENT_TYPES,
            normalization: NormalizationPolicy::Generic,
        }
    } else {
        return Err(GltfError::InvalidGltf(format!(
            "invalid glTF attribute semantic {semantic}"
        )));
    };

    Ok(spec)
}

fn indexed_semantic(semantic: &str, prefix: &str) -> bool {
    semantic
        .strip_prefix(prefix)
        .is_some_and(|index| !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit()))
}

pub(crate) fn validate_semantic_accessor(
    semantic: &str,
    accessor_type: &str,
    component_type: u32,
    normalized: bool,
) -> Result<SemanticSpec> {
    let spec = supported_semantic_spec(semantic)?;
    if !spec.expected_accessor_types.contains(&accessor_type) {
        return Err(GltfError::InvalidGltf(format!(
            "{semantic} accessor type {accessor_type} is invalid"
        )));
    }
    if !spec.allowed_component_types.contains(&component_type) {
        return Err(GltfError::InvalidGltf(format!(
            "{semantic} accessor componentType {component_type} is invalid"
        )));
    }
    let integer = component_type != GLTF_COMPONENT_FLOAT;
    let valid_normalized = match spec.normalization {
        NormalizationPolicy::Forbidden => !normalized,
        NormalizationPolicy::RequiredForInteger => normalized == integer,
        NormalizationPolicy::Generic => !normalized || integer,
    };
    if !valid_normalized {
        return Err(GltfError::InvalidGltf(format!(
            "{semantic} accessor normalized={normalized} is invalid for componentType {component_type}"
        )));
    }
    Ok(spec)
}

fn validate_decoded_semantic(semantic: &str, accessor: &DecodedAccessor) -> Result<()> {
    validate_semantic_accessor(
        semantic,
        gltf_type_for_num_components(accessor.num_components)?,
        component_type_for_data_type(accessor.data_type)?,
        accessor.normalized,
    )?;
    Ok(())
}

pub(crate) fn gltf_type_for_num_components(num_components: u8) -> Result<&'static str> {
    match num_components {
        1 => Ok("SCALAR"),
        2 => Ok("VEC2"),
        3 => Ok("VEC3"),
        4 => Ok("VEC4"),
        _ => Err(GltfError::InvalidGltf(format!(
            "Invalid accessor component count: {num_components}"
        ))),
    }
}

pub(crate) fn component_type_for_data_type(data_type: DataType) -> Result<u32> {
    match data_type {
        DataType::Int8 => Ok(GLTF_COMPONENT_BYTE),
        DataType::Uint8 => Ok(GLTF_COMPONENT_UNSIGNED_BYTE),
        DataType::Int16 => Ok(GLTF_COMPONENT_SHORT),
        DataType::Uint16 => Ok(GLTF_COMPONENT_UNSIGNED_SHORT),
        #[cfg(feature = "gltf-reader")]
        DataType::Uint32 => Ok(GLTF_COMPONENT_UNSIGNED_INT),
        DataType::Float32 => Ok(GLTF_COMPONENT_FLOAT),
        _ => Err(GltfError::Unsupported(format!(
            "Unsupported Draco attribute data type for glTF: {data_type:?}"
        ))),
    }
}
