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

#[cfg(feature = "encoder")]
pub fn select_prediction_method(
    att_id: i32,
    options: &EncoderOptions,
    encoder: &dyn GeometryEncoder,
) -> PredictionSchemeMethod {
    let speed = options.get_encoding_speed();

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
}
