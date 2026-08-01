//! Prediction-method selection.
//!
//! [`select_prediction_method`] picks the prediction scheme for an attribute
//! from the encoder options, geometry type, and speed setting, mirroring C++
//! Draco's defaults (e.g. parallelogram for mesh positions, difference for
//! point clouds). Port of Draco's `prediction_scheme_encoder_factory` selection
//! logic.

#[cfg(feature = "encoder")]
use crate::compression_config::EncodedGeometryType;
#[cfg(feature = "encoder")]
use crate::encoder_options::EncoderOptions;
#[cfg(feature = "encoder")]
use crate::geometry_attribute::GeometryAttributeType;
#[cfg(feature = "encoder")]
use crate::point_cloud_encoder::GeometryEncoder;
#[cfg(feature = "encoder")]
use crate::prediction_scheme::PredictionSchemeMethod;

/// The bitstream version that first carried `method`.
///
/// A prediction scheme is part of the format rather than a private encoder
/// choice: its id travels in the stream, and a decoder released before the
/// scheme existed has no case for it. The versions are read off the upstream
/// trees rather than inferred. `MESH_PREDICTION_CONSTRAINED_MULTI_PARALLELOGRAM`
/// first appears in Draco 0.10.0, which writes bitstream 1.2;
/// `MESH_PREDICTION_TEX_COORDS_PORTABLE` and `MESH_PREDICTION_GEOMETRIC_NORMAL`
/// first in Draco 1.0.0, which writes 2.0 -- neither identifier occurs anywhere
/// in the 0.9.1 or 0.10.0 sources. The rest have been there since 0.9.1, which
/// is the oldest bitstream this crate writes.
#[cfg(feature = "encoder")]
fn introduced_at(method: PredictionSchemeMethod) -> (u8, u8) {
    match method {
        PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram => (1, 2),
        PredictionSchemeMethod::MeshPredictionTexCoordsPortable
        | PredictionSchemeMethod::MeshPredictionGeometricNormal => (2, 0),
        PredictionSchemeMethod::None
        | PredictionSchemeMethod::Undefined
        | PredictionSchemeMethod::Difference
        | PredictionSchemeMethod::MeshPredictionParallelogram
        | PredictionSchemeMethod::MeshPredictionMultiParallelogram
        | PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => (1, 1),
    }
}

/// The scheme an encoder of the target's era would have picked instead.
///
/// Every substitute predates what it replaces and predicts only from the
/// attribute's own previously coded values, so one substitution always lands on
/// something the target can express -- pinned by
/// `no_substitute_outlives_the_version_it_is_written_into`.
///
/// The tex-coord substitute is the parallelogram and not
/// `MeshPredictionTexCoordsDeprecated`, which is what a 0.9.1 encoder would
/// actually have chosen. That scheme predicts from position as well, and
/// upstream feeds it the dequantized floats described on
/// [`downgrade_to_bitstream_era`]; writing it would mean predicting through a
/// float domain no reference encoder produces. The parallelogram gives up some
/// ratio for a scheme both sides compute identically.
#[cfg(feature = "encoder")]
fn era_substitute(method: PredictionSchemeMethod) -> PredictionSchemeMethod {
    match method {
        PredictionSchemeMethod::MeshPredictionGeometricNormal => PredictionSchemeMethod::Difference,
        PredictionSchemeMethod::MeshPredictionTexCoordsPortable
        | PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram => {
            PredictionSchemeMethod::MeshPredictionParallelogram
        }
        other => other,
    }
}

/// Replaces a scheme the target bitstream predates with the one its era used.
///
/// Naming a scheme older decoders do not know is not a graceful degradation.
/// Draco 0.9.1's `CreateMeshPredictionSchemeInternal` knows ids 1, 2 and 3, and
/// anything else falls through to `CreatePredictionScheme`, which hands back a
/// `PredictionSchemeDifference` -- so the correction values are read as plain
/// deltas, and the file opens, reports the right vertex count and reconstructs
/// different geometry, with no error anywhere. Modern Draco refuses some of
/// these outright, which is how this was first noticed: speed 0 selects
/// `MeshPredictionConstrainedMultiParallelogram`, and a 1.1 stream written at
/// speed 0 was readable by no released decoder.
///
/// For the two schemes that predict one attribute from position, a modern
/// decoder does something worse than refusing -- it decodes, and the answer is
/// wrong. Below bitstream 2.0 upstream's `InitPredictionScheme`
/// (`sequential_attribute_decoder.cc:83`) gives the predictor the *final*
/// position attribute rather than the portable one, and at that version the
/// final attribute already holds dequantized floats, because
/// `SequentialIntegerAttributeDecoder::DecodeValues` calls `StoreValues` as
/// soon as position's own values are read. `GetPositionForDataId`
/// (`mesh_prediction_scheme_geometric_normal_predictor_base.h:63`) then
/// converts that into `VectorD<int64_t, 3>`, and `ConvertComponentValue`
/// casts float to integer by truncation: on a unit-scale mesh every coordinate
/// collapses to 0 or +-1 and every cross product is degenerate, whatever the
/// encoder predicted against. That branch is not an oversight -- it is there
/// for `MESH_PREDICTION_TEX_COORDS_DEPRECATED`, the one pre-2.0 scheme with a
/// parent attribute, whose predictor genuinely is float-domain (`Vector3f
/// GetPositionForEntryId`, `mesh_prediction_scheme_tex_coords_decoder.h:94`).
/// So no encoder-side domain makes a pre-2.0 geometric normal readable; the
/// combination is outside the format, which is why the encoder declines to
/// write it instead of trying to agree with it. Before this rule existed,
/// roughly 40% of a 289-point mesh's normals landed on the wrong point.
///
/// The selection is what moves rather than the writer, because the older scheme
/// is a complete substitute -- it is what upstream picked at the time.
#[cfg(feature = "encoder")]
fn downgrade_to_bitstream_era(
    method: PredictionSchemeMethod,
    options: &EncoderOptions,
) -> PredictionSchemeMethod {
    let (major, minor) = options.get_version();
    // (0, 0) means "encoder default", which is always the newest.
    if major == 0 {
        return method;
    }
    if crate::version::version_less_than(major, minor, introduced_at(method)) {
        return era_substitute(method);
    }
    method
}

#[cfg(feature = "encoder")]
pub fn select_prediction_method(
    att_id: i32,
    options: &EncoderOptions,
    encoder: &dyn GeometryEncoder,
) -> PredictionSchemeMethod {
    downgrade_to_bitstream_era(
        select_prediction_method_for_newest(att_id, options, encoder),
        options,
    )
}

#[cfg(feature = "encoder")]
fn select_prediction_method_for_newest(
    att_id: i32,
    options: &EncoderOptions,
    encoder: &dyn GeometryEncoder,
) -> PredictionSchemeMethod {
    // The larger of the two speeds, as upstream's SelectPredictionMethod reads
    // options.GetSpeed(). Asking for the encoding speed alone ignores a caller
    // who wants fast decoding of a slowly encoded mesh.
    let speed = options.get_speed();

    if speed >= 10 {
        return PredictionSchemeMethod::Difference;
    }

    if encoder.get_geometry_type() == EncodedGeometryType::TriangularMesh {
        let att_quant = options.get_attribute_int(att_id, "quantization_bits", -1);
        let pc = encoder.point_cloud().unwrap(); // Should be safe if called from encoder
        let att = pc.attribute(att_id);

        if att_quant != -1
            && att.attribute_type() == GeometryAttributeType::TexCoord
            && att.num_components() == 2
        {
            let pos_att = pc.named_attribute(GeometryAttributeType::Position);
            let mut is_pos_att_valid = false;

            if let Some(pos_att) = pos_att {
                if pos_att.data_type().is_integral() {
                    is_pos_att_valid = true;
                } else {
                    let pos_att_id = pc.named_attribute_id(GeometryAttributeType::Position);
                    let pos_quant = options.get_attribute_int(pos_att_id, "quantization_bits", -1);
                    if pos_quant > 0 && pos_quant <= 21 && 2 * pos_quant + att_quant < 64 {
                        is_pos_att_valid = true;
                    }
                }
            }

            if is_pos_att_valid && speed < 4 {
                return PredictionSchemeMethod::MeshPredictionTexCoordsPortable;
            }
        }

        if att.attribute_type() == GeometryAttributeType::Normal {
            if speed < 4 {
                let pos_att_id = pc.named_attribute_id(GeometryAttributeType::Position);
                let pos_att = pc.named_attribute(GeometryAttributeType::Position);
                if let Some(pos_att) = pos_att {
                    if pos_att.data_type().is_integral()
                        || options.get_attribute_int(pos_att_id, "quantization_bits", -1) > 0
                    {
                        return PredictionSchemeMethod::MeshPredictionGeometricNormal;
                    }
                }
            }
            return PredictionSchemeMethod::Difference;
        }

        if speed >= 8 {
            return PredictionSchemeMethod::Difference;
        }

        if speed >= 2 || pc.num_points() < 40 {
            return PredictionSchemeMethod::MeshPredictionParallelogram;
        }

        return PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram;
    }

    // Point Cloud prediction
    PredictionSchemeMethod::Difference
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compression_config::EncodedGeometryType;
    use crate::corner_table::CornerTable;
    use crate::draco_types::DataType;
    use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
    use crate::mesh::Mesh;
    use crate::point_cloud::PointCloud;

    struct MockGeometryEncoder {
        point_cloud: PointCloud,
        options: EncoderOptions,
        geometry_type: EncodedGeometryType,
        encoding_method: Option<i32>,
    }

    impl GeometryEncoder for MockGeometryEncoder {
        fn point_cloud(&self) -> Option<&PointCloud> {
            Some(&self.point_cloud)
        }

        fn mesh(&self) -> Option<&Mesh> {
            None
        }

        fn corner_table(&self) -> Option<&CornerTable> {
            None
        }

        fn options(&self) -> &EncoderOptions {
            &self.options
        }

        fn get_geometry_type(&self) -> EncodedGeometryType {
            self.geometry_type
        }

        fn get_encoding_method(&self) -> Option<i32> {
            self.encoding_method
        }
    }

    fn make_attribute(
        attribute_type: GeometryAttributeType,
        data_type: DataType,
    ) -> PointAttribute {
        let mut attribute = PointAttribute::new();
        attribute.init(attribute_type, 3, data_type, false, 1);
        attribute
    }

    #[test]
    fn sequential_mesh_still_selects_mesh_prediction_schemes() {
        let mut point_cloud = PointCloud::new();
        point_cloud.set_num_points(64);
        point_cloud.add_attribute(make_attribute(
            GeometryAttributeType::Position,
            DataType::Float32,
        ));
        let generic_att_id = point_cloud.add_attribute(make_attribute(
            GeometryAttributeType::Generic,
            DataType::Float32,
        ));

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", 5);

        let encoder = MockGeometryEncoder {
            point_cloud,
            options: options.clone(),
            geometry_type: EncodedGeometryType::TriangularMesh,
            encoding_method: Some(0),
        };

        assert_eq!(
            select_prediction_method(generic_att_id, &options, &encoder),
            PredictionSchemeMethod::MeshPredictionParallelogram
        );
    }

    #[test]
    fn normal_prediction_matches_cpp_when_positions_are_quantized() {
        let mut point_cloud = PointCloud::new();
        point_cloud.set_num_points(64);
        let pos_att_id = point_cloud.add_attribute(make_attribute(
            GeometryAttributeType::Position,
            DataType::Float32,
        ));
        let normal_att_id = point_cloud.add_attribute(make_attribute(
            GeometryAttributeType::Normal,
            DataType::Float32,
        ));

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", 1);
        options.set_attribute_int(pos_att_id, "quantization_bits", 14);

        let encoder = MockGeometryEncoder {
            point_cloud,
            options: options.clone(),
            geometry_type: EncodedGeometryType::TriangularMesh,
            encoding_method: Some(0),
        };

        assert_eq!(
            select_prediction_method(normal_att_id, &options, &encoder),
            PredictionSchemeMethod::MeshPredictionGeometricNormal
        );
    }

    /// The same mesh at a pre-2.0 target must not name the scheme, because
    /// upstream would predict it from dequantized floats truncated to integers.
    #[test]
    fn a_normal_is_not_predicted_from_position_below_2_0() {
        for version in [(1, 1), (1, 2), (1, 3)] {
            let mut point_cloud = PointCloud::new();
            point_cloud.set_num_points(64);
            let pos_att_id = point_cloud.add_attribute(make_attribute(
                GeometryAttributeType::Position,
                DataType::Float32,
            ));
            let normal_att_id = point_cloud.add_attribute(make_attribute(
                GeometryAttributeType::Normal,
                DataType::Float32,
            ));

            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_speed", 1);
            options.set_attribute_int(pos_att_id, "quantization_bits", 14);
            options.set_version(version.0, version.1);

            let encoder = MockGeometryEncoder {
                point_cloud,
                options: options.clone(),
                geometry_type: EncodedGeometryType::TriangularMesh,
                encoding_method: Some(0),
            };

            assert_eq!(
                select_prediction_method(normal_att_id, &options, &encoder),
                PredictionSchemeMethod::Difference,
                "bitstream {}.{} predates MESH_PREDICTION_GEOMETRIC_NORMAL",
                version.0,
                version.1
            );
        }
    }

    /// A substitute that is itself too new would move the defect rather than
    /// close it, and nothing else checks that the table stays consistent.
    #[test]
    fn no_substitute_outlives_the_version_it_is_written_into() {
        const EVERY_METHOD: [PredictionSchemeMethod; 9] = [
            PredictionSchemeMethod::None,
            PredictionSchemeMethod::Undefined,
            PredictionSchemeMethod::Difference,
            PredictionSchemeMethod::MeshPredictionParallelogram,
            PredictionSchemeMethod::MeshPredictionMultiParallelogram,
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated,
            PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram,
            PredictionSchemeMethod::MeshPredictionTexCoordsPortable,
            PredictionSchemeMethod::MeshPredictionGeometricNormal,
        ];

        for target in [
            crate::version::EncodeTarget::MeshEdgebreaker,
            crate::version::EncodeTarget::MeshSequential,
            crate::version::EncodeTarget::PointCloudSequential,
            crate::version::EncodeTarget::PointCloudKdTree,
        ] {
            for &(major, minor) in target.claimed_versions() {
                let mut options = EncoderOptions::new();
                options.set_version(major, minor);
                for method in EVERY_METHOD {
                    let picked = downgrade_to_bitstream_era(method, &options);
                    let (at_major, at_minor) = introduced_at(picked);
                    assert!(
                        !crate::version::version_less_than(major, minor, (at_major, at_minor)),
                        "{target:?} at {major}.{minor} turns {method:?} into {picked:?}, \
                         which arrived only at {at_major}.{at_minor}"
                    );
                }
            }
        }
    }
}
