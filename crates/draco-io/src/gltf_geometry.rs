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
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    /// Binary GLB structure is invalid.
    #[error("Invalid GLB: {0}")]
    InvalidGlb(String),

    /// glTF JSON or accessor/buffer structure is invalid.
    #[error("Invalid glTF: {0}")]
    InvalidGltf(String),

    /// Embedded Draco payload failed to decode.
    #[error("Draco decode error: {0}")]
    DracoDecode(String),

    /// The asset uses a glTF feature outside this crate's geometry scope.
    #[error("Unsupported feature: {0}")]
    Unsupported(String),
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
    ) -> Self {
        Self {
            count,
            num_components,
            data_type,
            normalized,
            bytes,
        }
    }

    fn gather(&self, indices: &[u32]) -> Result<Self> {
        let stride = self.num_components as usize * self.data_type.byte_length();
        let mut bytes = Vec::with_capacity(indices.len() * stride);

        for &index in indices {
            let index = index as usize;
            if index >= self.count {
                return Err(GltfError::InvalidGltf(format!(
                    "Accessor index {} out of bounds for {} values",
                    index, self.count
                )));
            }
            let offset = index * stride;
            bytes.extend_from_slice(&self.bytes[offset..offset + stride]);
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

pub(crate) struct SemanticSpec {
    pub(crate) attribute_type: GeometryAttributeType,
    pub(crate) expected_accessor_types: &'static [&'static str],
    pub(crate) allowed_component_types: &'static [u32],
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
    use draco_core::geometry_indices::PointIndex;

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
            for i in 0..num_faces {
                mesh.add_face([
                    PointIndex(indices[i * 3]),
                    PointIndex(indices[i * 3 + 1]),
                    PointIndex(indices[i * 3 + 2]),
                ]);
            }
        } else {
            // Non-indexed: generate sequential triangle faces.
            if mesh.num_points() % 3 != 0 {
                return Err(GltfError::InvalidGltf(
                    "Non-indexed primitive point count not divisible by 3".into(),
                ));
            }
            for i in 0..(mesh.num_points() / 3) {
                let base = (i * 3) as u32;
                mesh.add_face([PointIndex(base), PointIndex(base + 1), PointIndex(base + 2)]);
            }
        }
    }

    // Optionally read NORMAL.
    if let Some(normal_idx) = attributes
        .iter()
        .find(|(semantic, _)| semantic == "NORMAL")
        .map(|(_, accessor)| *accessor)
    {
        let normal_id = read_and_add_standard_attribute(
            &mut mesh,
            src,
            normal_idx,
            GeometryAttributeType::Normal,
            &["VEC3"],
            &[GLTF_COMPONENT_FLOAT],
            point_indices.as_deref(),
        )?;
        semantics.push(("NORMAL".to_string(), normal_id as u32));
    }

    // Read every remaining semantic in sorted order. Draco can carry multiple
    // attributes with the same semantic type (extra TEXCOORD_n/COLOR_n), plus
    // TANGENT, JOINTS_n, WEIGHTS_n, and custom `_*`.
    let mut sorted: Vec<&(String, usize)> = attributes.iter().collect();
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
            attribute_spec.attribute_type,
            attribute_spec.expected_accessor_types,
            attribute_spec.allowed_component_types,
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
    read_and_add_standard_attribute(
        mesh,
        src,
        accessor_idx,
        spec.attribute_type,
        spec.expected_accessor_types,
        spec.allowed_component_types,
        point_indices,
    )
}

fn read_and_add_standard_attribute<S: AccessorSource>(
    mesh: &mut Mesh,
    src: &S,
    accessor_idx: usize,
    attribute_type: GeometryAttributeType,
    expected_types: &[&str],
    allowed_component_types: &[u32],
    point_indices: Option<&[u32]>,
) -> Result<i32> {
    let decoded = src.read_attribute(accessor_idx, expected_types, allowed_component_types)?;
    let decoded = if let Some(indices) = point_indices {
        decoded.gather(indices)?
    } else {
        decoded
    };
    add_decoded_attribute(mesh, attribute_type, decoded)
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
    attribute.init(
        attribute_type,
        decoded.num_components,
        decoded.data_type,
        decoded.normalized,
        decoded.count,
    );
    attribute.buffer_mut().write(0, &decoded.bytes);
    Ok(mesh.add_attribute(attribute))
}

pub(crate) fn supported_semantic_spec(semantic: &str) -> Result<SemanticSpec> {
    let spec = if semantic == "POSITION" {
        SemanticSpec {
            attribute_type: GeometryAttributeType::Position,
            expected_accessor_types: &["VEC3"],
            allowed_component_types: FLOAT_ONLY,
        }
    } else if semantic == "NORMAL" {
        SemanticSpec {
            attribute_type: GeometryAttributeType::Normal,
            expected_accessor_types: &["VEC3"],
            allowed_component_types: FLOAT_ONLY,
        }
    } else if semantic.starts_with("TEXCOORD_") {
        SemanticSpec {
            attribute_type: GeometryAttributeType::TexCoord,
            expected_accessor_types: &["VEC2"],
            allowed_component_types: TEXCOORD_COMPONENT_TYPES,
        }
    } else if semantic.starts_with("COLOR_") {
        SemanticSpec {
            attribute_type: GeometryAttributeType::Color,
            expected_accessor_types: &["VEC3", "VEC4"],
            allowed_component_types: COLOR_COMPONENT_TYPES,
        }
    } else {
        // TANGENT, JOINTS_n, WEIGHTS_n, custom `_*`, and any other semantic are
        // carried as generic attributes. The glTF semantic name is not stored on
        // the Draco attribute itself; it is preserved by the caller through the
        // `KHR_draco_mesh_compression` attributes map and `primitive.attributes`.
        SemanticSpec {
            attribute_type: GeometryAttributeType::Generic,
            expected_accessor_types: &["SCALAR", "VEC2", "VEC3", "VEC4"],
            allowed_component_types: GENERIC_COMPONENT_TYPES,
        }
    };

    Ok(spec)
}
