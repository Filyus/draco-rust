//! Core Draco geometry compression primitives.
//!
//! `draco-core` implements the raw Draco `.drc` bitstream layer for triangle
//! meshes and point clouds. It intentionally stops at the geometry compression
//! model: file containers such as glTF/GLB, OBJ, PLY, STL, and FBX live in
//! `draco-io`.
//!
//! # Main Entry Points
//!
//! - [`Mesh`] and [`PointCloud`] hold decoded geometry.
//! - [`PointAttribute`] stores typed attribute data such as positions, normals,
//!   colors, texture coordinates, and generic attributes.
//! - [`Metadata`], [`GeometryMetadata`], and [`AttributeMetadata`] expose raw
//!   Draco metadata plus C++-compatible typed helpers.
//! - With the `encoder` feature, use [`MeshEncoder`] or [`PointCloudEncoder`].
//! - With the `decoder` feature, use [`MeshDecoder`] or [`PointCloudDecoder`].
//!
//! # Features
//!
//! The default feature set enables both encoding and decoding, point-cloud
//! KD-tree decoding, EdgeBreaker valence traversal, and legacy bitstream
//! compatibility helpers. Disable default features when embedding only the
//! geometry data model or one codec direction is needed.
//!
//! # Metadata
//!
//! Draco metadata entries are stored as untyped byte blobs in the bitstream.
//! The typed helpers on [`Metadata`] write the same bytes used by C++ Draco
//! convenience APIs for `int32`, `double`, arrays, and strings.
//!
//! # Example
//!
//! ```no_run
//! use draco_core::{DecoderBuffer, Mesh, MeshDecoder};
//!
//! let bytes = std::fs::read("mesh.drc")?;
//! let mut buffer = DecoderBuffer::new(&bytes);
//! let mut mesh = Mesh::new();
//! MeshDecoder::new().decode(&mut buffer, &mut mesh)?;
//! println!("decoded {} faces", mesh.num_faces());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]
// Allow certain clippy lints that are intentional design decisions for C++ port compatibility
#![allow(clippy::needless_range_loop)] // Many loops follow C++ patterns for array indexing
#![allow(clippy::manual_memcpy)] // Manual copying matches C++ patterns for clarity

#[cfg(feature = "debug_logs")]
#[inline]
pub(crate) fn debug_env_enabled(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

/// Emit a diagnostic line to stderr, but only when the `debug_logs` feature is
/// enabled. `draco-core` is a library, so decode/encode paths must not print on
/// their own. Using `if cfg!(...)` keeps the formatting arguments type-checked
/// yet dead-code-eliminated in normal builds: nothing is evaluated and nothing
/// is printed, so there is no runtime cost and no unused-variable churn.
///
/// Defined at the crate root before the module declarations below so every
/// module can use it by bare name via textual macro scoping.
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if cfg!(feature = "debug_logs") {
            eprintln!($($arg)*);
        }
    };
}

// =============================================================================
// Core modules - always available
// =============================================================================

#[doc(hidden)]
pub mod ans;
#[doc(hidden)]
pub mod attribute_octahedron_transform;
#[doc(hidden)]
pub mod attribute_transform;
#[doc(hidden)]
pub mod attribute_transform_data;
#[doc(hidden)]
pub mod bit_utils;
#[doc(hidden)]
pub mod compression_config;
#[doc(hidden)]
pub mod corner_table;
/// Draco scalar data type identifiers.
pub mod draco_types;
/// Geometry attribute descriptors and point attribute storage.
pub mod geometry_attribute;
/// Strongly typed geometry index wrappers.
pub mod geometry_indices;
/// Keyframe animation container built on the point-cloud path.
pub mod keyframe_animation;
/// Triangle mesh geometry data.
pub mod mesh;
/// Draco metadata containers and bitstream serialization helpers.
pub mod metadata;

// Internal codec modules are kept public but hidden so existing parity tests can
// exercise ported Draco internals without presenting them as the public API.
#[doc(hidden)]
pub mod attribute_quantization_transform;
/// Depth-first corner-table traversal, shared by the two decode paths that
/// order attribute values by connectivity.
#[cfg(feature = "decoder")]
mod corner_traversal;
#[doc(hidden)]
pub mod data_buffer;
/// What a decode may allocate relative to the stream it is reading. Internal:
/// the ratio is an implementation detail, not a knob.
#[cfg(feature = "decoder")]
mod decode_budget;
/// Caller-set ceilings on what one decode may produce. Public: unlike the
/// budget above, where the ceiling sits is the caller's policy.
///
/// Not gated on `decoder`, and the difference is not cosmetic: the type is a
/// policy a caller states, so a crate that threads it through has to be able
/// to name it whether or not the decoder is compiled in. Only the checks the
/// decoder runs carry the gate.
pub mod decode_limits;
/// Coarse decode phase timing for the performance harness (`DECODE_PHASES=1`).
#[doc(hidden)]
#[cfg(feature = "decoder")]
pub mod decode_phase_probe;
#[doc(hidden)]
#[cfg(any(feature = "encoder", feature = "decoder"))]
pub mod dynamic_integer_points_kd_tree;
#[doc(hidden)]
pub mod edgebreaker_connectivity_decoder;
#[doc(hidden)]
pub mod folded_bit32_coder;
#[doc(hidden)]
pub mod math_utils;
#[doc(hidden)]
pub mod mesh_attribute_corner_table;
#[doc(hidden)]
pub mod mesh_edgebreaker_shared;
#[doc(hidden)]
pub mod mesh_prediction_scheme_data;
#[doc(hidden)]
pub mod normal_compression_utils;
/// Point cloud geometry data.
pub mod point_cloud;
/// The prediction-parent binding: how a scheme obtains portable parent values.
#[doc(hidden)]
pub mod portable_attribute;
#[doc(hidden)]
pub mod prediction_scheme;
#[doc(hidden)]
#[cfg(any(feature = "encoder", feature = "decoder"))]
pub mod prediction_scheme_constrained_multi_parallelogram;
#[doc(hidden)]
#[cfg(any(feature = "encoder", feature = "decoder"))]
pub mod prediction_scheme_delta;
#[doc(hidden)]
#[cfg(any(feature = "encoder", feature = "decoder"))]
pub mod prediction_scheme_geometric_normal;
#[cfg(any(
    all(feature = "encoder", feature = "legacy_bitstream_encode"),
    all(feature = "decoder", feature = "legacy_bitstream_decode")
))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(
        all(feature = "encoder", feature = "legacy_bitstream_encode"),
        all(feature = "decoder", feature = "legacy_bitstream_decode")
    )))
)]
#[doc(hidden)]
pub mod prediction_scheme_multi_parallelogram;
#[doc(hidden)]
pub mod prediction_scheme_normal_octahedron_canonicalized_transform_base;
#[doc(hidden)]
pub mod prediction_scheme_normal_octahedron_transform_base;
#[doc(hidden)]
#[cfg(any(feature = "encoder", feature = "decoder"))]
pub mod prediction_scheme_parallelogram;
#[doc(hidden)]
pub mod prediction_scheme_selection;
#[cfg(any(
    all(feature = "encoder", feature = "legacy_bitstream_encode"),
    all(feature = "decoder", feature = "legacy_bitstream_decode")
))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(
        all(feature = "encoder", feature = "legacy_bitstream_encode"),
        all(feature = "decoder", feature = "legacy_bitstream_decode")
    )))
)]
#[doc(hidden)]
pub mod prediction_scheme_tex_coords_deprecated;
#[doc(hidden)]
#[cfg(any(feature = "encoder", feature = "decoder"))]
pub mod prediction_scheme_tex_coords_portable;
#[doc(hidden)]
#[cfg(any(feature = "encoder", feature = "decoder"))]
pub mod prediction_scheme_wrap;
#[doc(hidden)]
pub mod quantization_utils;
#[doc(hidden)]
pub mod rans_symbol_coding;
/// Error and status types.
pub mod status;
#[doc(hidden)]
#[cfg(any(feature = "encoder", feature = "decoder"))]
pub mod symbol_encoding;
#[doc(hidden)]
pub mod test_event_log;
#[doc(hidden)]
pub mod version;

// =============================================================================
// Decoder-only modules
// =============================================================================

#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
/// Decoder input buffer.
pub mod decoder_buffer;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
#[doc(hidden)]
pub mod direct_bit_decoder;
#[cfg(all(feature = "decoder", feature = "point_cloud_decode"))]
#[cfg_attr(
    docsrs,
    doc(cfg(all(feature = "decoder", feature = "point_cloud_decode")))
)]
#[doc(hidden)]
pub mod kd_tree_attributes_decoder;
#[cfg(all(feature = "decoder", feature = "point_cloud_decode"))]
#[cfg_attr(
    docsrs,
    doc(cfg(all(feature = "decoder", feature = "point_cloud_decode")))
)]
/// Keyframe animation decoder entry point.
pub mod keyframe_animation_decoder;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
/// Mesh decoder entry point.
pub mod mesh_decoder;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
#[doc(hidden)]
pub mod mesh_edgebreaker_decoder;
#[cfg(all(feature = "decoder", feature = "legacy_bitstream_decode"))]
#[cfg_attr(
    docsrs,
    doc(cfg(all(feature = "decoder", feature = "legacy_bitstream_decode")))
)]
#[doc(hidden)]
pub mod mesh_edgebreaker_traversal_predictive_decoder;
#[cfg(all(feature = "decoder", feature = "edgebreaker_valence_decode"))]
#[cfg_attr(
    docsrs,
    doc(cfg(all(feature = "decoder", feature = "edgebreaker_valence_decode")))
)]
#[doc(hidden)]
pub mod mesh_edgebreaker_traversal_valence_decoder;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
/// Point cloud decoder entry point.
pub mod point_cloud_decoder;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
#[doc(hidden)]
pub mod prediction_scheme_normal_octahedron_canonicalized_decoding_transform;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
#[doc(hidden)]
pub mod rans_bit_decoder;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
#[doc(hidden)]
pub mod rans_symbol_decoder;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
#[doc(hidden)]
pub mod sequential_attribute_decoder;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
#[doc(hidden)]
pub mod sequential_generic_attribute_decoder;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
#[doc(hidden)]
pub mod sequential_integer_attribute_decoder;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
#[doc(hidden)]
pub mod sequential_normal_attribute_decoder;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
#[doc(hidden)]
pub mod sequential_quantization_attribute_decoder;

// =============================================================================
// Encoder-only modules
// =============================================================================

#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
#[doc(hidden)]
pub mod direct_bit_encoder;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
/// Encoder output buffer.
pub mod encoder_buffer;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
/// Encoder configuration options.
pub mod encoder_options;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
#[doc(hidden)]
pub mod kd_tree_attributes_encoder;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
/// Keyframe animation encoder entry point.
pub mod keyframe_animation_encoder;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
#[doc(hidden)]
pub mod mesh_edgebreaker_encoder;
#[cfg(all(feature = "encoder", feature = "legacy_bitstream_encode"))]
#[cfg_attr(
    docsrs,
    doc(cfg(all(feature = "encoder", feature = "legacy_bitstream_encode")))
)]
#[doc(hidden)]
pub mod mesh_edgebreaker_traversal_predictive_encoder;
#[cfg(all(feature = "encoder", feature = "edgebreaker_valence_encode"))]
#[cfg_attr(
    docsrs,
    doc(cfg(all(feature = "encoder", feature = "edgebreaker_valence_encode")))
)]
#[doc(hidden)]
pub mod mesh_edgebreaker_traversal_valence_encoder;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
/// Mesh encoder entry point.
pub mod mesh_encoder;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
/// Point cloud encoder entry point.
pub mod point_cloud_encoder;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
#[doc(hidden)]
pub mod prediction_scheme_normal_octahedron_canonicalized_encoding_transform;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
#[doc(hidden)]
pub mod rans_bit_encoder;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
#[doc(hidden)]
pub mod rans_symbol_encoder;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
#[doc(hidden)]
pub mod sequential_attribute_encoder;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
#[doc(hidden)]
pub mod sequential_integer_attribute_encoder;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
#[doc(hidden)]
pub mod sequential_normal_attribute_encoder;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
#[doc(hidden)]
pub mod shannon_entropy;

// =============================================================================
// Core re-exports - always available
// =============================================================================

pub use draco_types::DataType;
pub use geometry_attribute::{GeometryAttribute, GeometryAttributeType, PointAttribute};
pub use geometry_indices::{AttributeValueIndex, FaceIndex, PointIndex};
pub use keyframe_animation::KeyframeAnimation;
pub use mesh::Mesh;
pub use metadata::{AttributeMetadata, GeometryMetadata, Metadata};
pub use point_cloud::PointCloud;
pub use status::{DracoError, ErrorKind, Status};

// =============================================================================
// Decoder re-exports
// =============================================================================

pub use decode_limits::DecodeLimits;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
pub use decoder_buffer::DecoderBuffer;
#[cfg(all(feature = "decoder", feature = "point_cloud_decode"))]
#[cfg_attr(
    docsrs,
    doc(cfg(all(feature = "decoder", feature = "point_cloud_decode")))
)]
pub use keyframe_animation_decoder::KeyframeAnimationDecoder;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
pub use mesh_decoder::MeshDecoder;
#[cfg(feature = "decoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "decoder")))]
pub use point_cloud_decoder::PointCloudDecoder;

// =============================================================================
// Encoder re-exports
// =============================================================================

#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
pub use encoder_buffer::EncoderBuffer;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
pub use encoder_options::EncoderOptions;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
pub use keyframe_animation_encoder::KeyframeAnimationEncoder;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
pub use mesh_encoder::{EncodedAttributeInfo, EncodedMeshInfo, MeshEncoder};
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
pub use point_cloud_encoder::{EncodedPointCloudInfo, PointCloudEncoder};
