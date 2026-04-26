//! Draco Core Library
//!
//! Core compression and decompression functionality for 3D geometric meshes
//! and point clouds.

// Allow certain clippy lints that are intentional design decisions for C++ port compatibility
#![allow(clippy::needless_range_loop)] // Many loops follow C++ patterns for array indexing
#![allow(clippy::manual_memcpy)] // Manual copying matches C++ patterns for clarity

#[cfg(feature = "debug_logs")]
#[inline]
pub(crate) fn debug_env_enabled(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

// =============================================================================
// Core modules - always available
// =============================================================================

pub mod ans;
pub mod attribute_octahedron_transform;
pub mod attribute_quantization_transform;
pub mod attribute_transform;
pub mod attribute_transform_data;
pub mod bit_utils;
pub mod compression_config;
pub mod corner_table;
pub mod data_buffer;
pub mod draco_types;
pub mod dynamic_integer_points_kd_tree;
pub mod edgebreaker_connectivity_decoder;
pub mod folded_bit32_coder;
pub mod geometry_attribute;
pub mod geometry_indices;
pub mod math_utils;
pub mod mesh;
pub mod mesh_edgebreaker_shared;

// Test-only helpers (exposed so integration tests can access them)
pub mod mesh_prediction_scheme_data;
pub mod normal_compression_utils;
pub mod point_cloud;
pub mod prediction_scheme;
pub mod prediction_scheme_constrained_multi_parallelogram;
pub mod prediction_scheme_delta;
pub mod prediction_scheme_geometric_normal;
#[cfg(all(feature = "decoder", feature = "legacy_bitstream_decode"))]
pub mod prediction_scheme_multi_parallelogram;
pub mod prediction_scheme_normal_octahedron_canonicalized_transform_base;
pub mod prediction_scheme_normal_octahedron_transform_base;
pub mod prediction_scheme_parallelogram;
pub mod prediction_scheme_selection;
#[cfg(all(feature = "decoder", feature = "legacy_bitstream_decode"))]
pub mod prediction_scheme_tex_coords_deprecated;
pub mod prediction_scheme_tex_coords_portable;
pub mod prediction_scheme_wrap;
pub mod quantization_utils;
pub mod rans_symbol_coding;
pub mod status;
pub mod symbol_encoding;
pub mod test_event_log;
pub mod version;

// =============================================================================
// Decoder-only modules
// =============================================================================

#[cfg(feature = "decoder")]
pub mod decoder_buffer;
#[cfg(feature = "decoder")]
pub mod direct_bit_decoder;
#[cfg(all(feature = "decoder", feature = "point_cloud_decode"))]
pub mod kd_tree_attributes_decoder;
#[cfg(feature = "decoder")]
pub mod mesh_decoder;
#[cfg(feature = "decoder")]
pub mod mesh_edgebreaker_decoder;
#[cfg(all(feature = "decoder", feature = "edgebreaker_valence_decode"))]
pub mod mesh_edgebreaker_traversal_valence_decoder;
#[cfg(feature = "decoder")]
pub mod point_cloud_decoder;
#[cfg(feature = "decoder")]
pub mod prediction_scheme_normal_octahedron_canonicalized_decoding_transform;
#[cfg(feature = "decoder")]
pub mod rans_bit_decoder;
#[cfg(feature = "decoder")]
pub mod rans_symbol_decoder;
#[cfg(feature = "decoder")]
pub mod sequential_attribute_decoder;
#[cfg(feature = "decoder")]
pub mod sequential_generic_attribute_decoder;
#[cfg(feature = "decoder")]
pub mod sequential_integer_attribute_decoder;
#[cfg(feature = "decoder")]
pub mod sequential_normal_attribute_decoder;

// =============================================================================
// Encoder-only modules
// =============================================================================

#[cfg(feature = "encoder")]
pub mod direct_bit_encoder;
#[cfg(feature = "encoder")]
pub mod encoder_buffer;
#[cfg(feature = "encoder")]
pub mod encoder_options;
#[cfg(feature = "encoder")]
pub mod kd_tree_attributes_encoder;
#[cfg(feature = "encoder")]
pub mod mesh_edgebreaker_encoder;
#[cfg(all(feature = "encoder", feature = "edgebreaker_valence_encode"))]
pub mod mesh_edgebreaker_traversal_valence_encoder;
#[cfg(feature = "encoder")]
pub mod mesh_encoder;
#[cfg(feature = "encoder")]
pub mod point_cloud_encoder;
#[cfg(feature = "encoder")]
pub mod prediction_scheme_normal_octahedron_canonicalized_encoding_transform;
#[cfg(feature = "encoder")]
pub mod rans_bit_encoder;
#[cfg(feature = "encoder")]
pub mod rans_symbol_encoder;
#[cfg(feature = "encoder")]
pub mod sequential_attribute_encoder;
#[cfg(feature = "encoder")]
pub mod sequential_integer_attribute_encoder;
#[cfg(feature = "encoder")]
pub mod sequential_normal_attribute_encoder;
#[cfg(feature = "encoder")]
pub mod shannon_entropy;

// =============================================================================
// Core re-exports - always available
// =============================================================================

pub use ans::{AnsCoder, AnsDecoder};
pub use attribute_octahedron_transform::AttributeOctahedronTransform;
pub use attribute_quantization_transform::AttributeQuantizationTransform;
pub use attribute_transform::{AttributeTransform, AttributeTransformType};
pub use attribute_transform_data::AttributeTransformData;
pub use bit_utils::{BitDecoder, BitEncoder};
pub use corner_table::CornerTable;
pub use data_buffer::DataBuffer;
pub use draco_types::DataType;
pub use geometry_attribute::{GeometryAttribute, GeometryAttributeType, PointAttribute};
pub use geometry_indices::{AttributeValueIndex, FaceIndex, PointIndex};
pub use mesh::Mesh;
pub use normal_compression_utils::OctahedronToolBox;
pub use point_cloud::PointCloud;
pub use prediction_scheme::{
    PredictionScheme, PredictionSchemeMethod, PredictionSchemeTransformType,
};
pub use status::{DracoError, Status};

// =============================================================================
// Decoder re-exports
// =============================================================================

#[cfg(feature = "decoder")]
pub use decoder_buffer::DecoderBuffer;
#[cfg(feature = "decoder")]
pub use direct_bit_decoder::DirectBitDecoder;
#[cfg(feature = "decoder")]
pub use folded_bit32_coder::FoldedBit32Decoder;
#[cfg(feature = "decoder")]
pub use mesh_decoder::MeshDecoder;
#[cfg(feature = "decoder")]
pub use point_cloud_decoder::PointCloudDecoder;
#[cfg(feature = "decoder")]
pub use rans_bit_decoder::RAnsBitDecoder;

// =============================================================================
// Encoder re-exports
// =============================================================================

#[cfg(feature = "encoder")]
pub use direct_bit_encoder::DirectBitEncoder;
#[cfg(feature = "encoder")]
pub use encoder_buffer::EncoderBuffer;
#[cfg(feature = "encoder")]
pub use encoder_options::EncoderOptions;
#[cfg(feature = "encoder")]
pub use folded_bit32_coder::FoldedBit32Encoder;
#[cfg(feature = "encoder")]
pub use mesh_encoder::MeshEncoder;
#[cfg(feature = "encoder")]
pub use point_cloud_encoder::{GeometryEncoder, PointCloudEncoder};
#[cfg(feature = "encoder")]
pub use rans_bit_encoder::RAnsBitEncoder;
