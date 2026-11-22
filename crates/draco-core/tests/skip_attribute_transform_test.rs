use draco_core::attribute_octahedron_transform::AttributeOctahedronTransform;
use draco_core::attribute_quantization_transform::AttributeQuantizationTransform;
use draco_core::attribute_transform::{AttributeTransform, AttributeTransformType};
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;

fn float_attribute(
    attribute_type: GeometryAttributeType,
    components: u8,
    values: &[f32],
) -> PointAttribute {
    let count = values.len() / components as usize;
    let mut attribute = PointAttribute::new();
    attribute.init(attribute_type, components, DataType::Float32, false, count);
    for (index, value) in values.iter().enumerate() {
        attribute
            .buffer_mut()
            .write(index * 4, &value.to_le_bytes());
    }
    attribute
}

fn encode_point_cloud(cloud: PointCloud, method: i32, options: &EncoderOptions) -> Vec<u8> {
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud);
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(options, &mut buffer)
        .expect("encode point cloud");
    assert_eq!(method, options.get_global_int("encoding_method", 0));
    buffer.data().to_vec()
}

#[test]
fn sequential_point_cloud_skips_only_the_requested_attribute_type() {
    let mut cloud = PointCloud::new();
    cloud.set_num_points(4);
    cloud.add_attribute(float_attribute(
        GeometryAttributeType::Position,
        3,
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.5, 0.25, 0.75],
    ));
    cloud.add_attribute(float_attribute(
        GeometryAttributeType::Generic,
        1,
        &[0.0, 0.5, 0.75, 1.0],
    ));

    let mut options = EncoderOptions::new();
    options.set_encoding_method(0);
    options.set_attribute_int(0, "quantization_bits", 8);
    options.set_attribute_int(1, "quantization_bits", 8);
    let bytes = encode_point_cloud(cloud, 0, &options);

    let mut decoder = PointCloudDecoder::new();
    decoder.set_skip_attribute_transform(GeometryAttributeType::Position);
    let mut decoded = PointCloud::new();
    decoder
        .decode(&mut DecoderBuffer::new(&bytes), &mut decoded)
        .expect("decode point cloud");

    let position = decoded.attribute(0);
    assert_eq!(position.data_type(), DataType::Uint32);
    assert_eq!(position.num_components(), 3);
    assert_eq!(
        position
            .attribute_transform_data()
            .expect("quantization metadata")
            .transform_type(),
        AttributeTransformType::QuantizationTransform
    );
    let mut quantization = AttributeQuantizationTransform::new();
    quantization
        .init_from_attribute(position)
        .expect("read quantization metadata");
    let mut restored = PointAttribute::new();
    restored.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        position.size(),
    );
    quantization
        .inverse_transform_attribute(position, &mut restored)
        .expect("restore positions");
    assert!(restored
        .read_f32s(4, 3)
        .iter()
        .all(|value| value.is_finite()));

    assert_eq!(decoded.attribute(1).data_type(), DataType::Float32);
    assert_eq!(decoded.attribute(1).num_components(), 1);
}

#[test]
fn point_cloud_normals_keep_their_two_octahedral_coordinates() {
    let mut cloud = PointCloud::new();
    cloud.set_num_points(4);
    cloud.add_attribute(float_attribute(
        GeometryAttributeType::Normal,
        3,
        &[
            1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.57735, 0.57735, 0.57735,
        ],
    ));
    let mut options = EncoderOptions::new();
    options.set_encoding_method(0);
    options.set_attribute_int(0, "quantization_bits", 8);
    let bytes = encode_point_cloud(cloud, 0, &options);

    let mut decoder = PointCloudDecoder::new();
    decoder.set_skip_attribute_transform(GeometryAttributeType::Normal);
    let mut decoded = PointCloud::new();
    decoder
        .decode(&mut DecoderBuffer::new(&bytes), &mut decoded)
        .expect("decode point cloud");

    let portable = decoded.attribute(0);
    assert_eq!(portable.data_type(), DataType::Uint32);
    assert_eq!(portable.num_components(), 2);
    let mut octahedron = AttributeOctahedronTransform::new(-1);
    octahedron
        .init_from_attribute(portable)
        .expect("read octahedron metadata");
    assert_eq!(octahedron.quantization_bits(), 8);

    let mut restored = PointAttribute::new();
    restored.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        portable.size(),
    );
    octahedron
        .inverse_transform_attribute(portable, &mut restored)
        .expect("restore normals");
    assert!(restored
        .read_f32s(4, 3)
        .iter()
        .all(|value| value.is_finite()));
}

#[test]
fn mesh_and_kd_tree_decoders_keep_portable_position_values() {
    let mut mesh = Mesh::new();
    mesh.add_attribute(float_attribute(
        GeometryAttributeType::Position,
        3,
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
    ));
    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    let mut mesh_options = EncoderOptions::new();
    mesh_options.set_attribute_int(0, "quantization_bits", 8);
    let mut mesh_encoder = MeshEncoder::new();
    mesh_encoder.set_mesh(mesh);
    let mut mesh_buffer = EncoderBuffer::new();
    mesh_encoder
        .encode(&mesh_options, &mut mesh_buffer)
        .expect("encode mesh");

    let mut mesh_decoder = MeshDecoder::new();
    mesh_decoder.set_skip_attribute_transform(GeometryAttributeType::Position);
    let mut decoded_mesh = Mesh::new();
    mesh_decoder
        .decode(
            &mut DecoderBuffer::new(mesh_buffer.data()),
            &mut decoded_mesh,
        )
        .expect("decode mesh");
    let position_id = decoded_mesh.named_attribute_id(GeometryAttributeType::Position);
    let position = decoded_mesh.attribute(position_id);
    assert_eq!(position.data_type(), DataType::Uint32);
    assert_eq!(position.num_components(), 3);
    assert!(position.attribute_transform_data().is_some());

    for version in [None, Some((2, 2))] {
        let mut cloud = PointCloud::new();
        cloud.set_num_points(4);
        cloud.add_attribute(float_attribute(
            GeometryAttributeType::Position,
            3,
            &[
                -3.0, -6.0, -9.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 2.0, 4.0, 6.0,
            ],
        ));
        let mut kd_options = EncoderOptions::new();
        kd_options.set_encoding_method(1);
        kd_options.set_attribute_int(0, "quantization_bits", 8);
        if let Some((major, minor)) = version {
            kd_options.set_version(major, minor);
        }
        let bytes = encode_point_cloud(cloud, 1, &kd_options);

        let mut reference_decoder = PointCloudDecoder::new();
        let mut reference_cloud = PointCloud::new();
        reference_decoder
            .decode(&mut DecoderBuffer::new(&bytes), &mut reference_cloud)
            .expect("decode KD-tree point cloud with inverse transforms");
        let reference_values = reference_cloud.attribute(0).read_f32s(4, 3);
        assert!(reference_values.iter().any(|&value| value < 0.0));

        let mut kd_decoder = PointCloudDecoder::new();
        kd_decoder.set_skip_attribute_transform(GeometryAttributeType::Position);
        let mut decoded_cloud = PointCloud::new();
        kd_decoder
            .decode(&mut DecoderBuffer::new(&bytes), &mut decoded_cloud)
            .expect("decode KD-tree point cloud");
        let position = decoded_cloud.attribute(0);
        let expected_data_type = if version == Some((2, 2)) {
            DataType::Int32
        } else {
            DataType::Uint32
        };
        assert_eq!(position.data_type(), expected_data_type);
        assert!(position.attribute_transform_data().is_some());
        if version == Some((2, 2)) {
            assert!(position
                .buffer()
                .data()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|&bytes| i32::from_le_bytes(bytes) < 0));
        }

        let mut quantization = AttributeQuantizationTransform::new();
        quantization
            .init_from_attribute(position)
            .expect("read KD-tree quantization metadata");
        let mut restored = PointAttribute::new();
        restored.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            position.size(),
        );
        quantization
            .inverse_transform_attribute(position, &mut restored)
            .expect("restore KD-tree positions");
        assert_eq!(
            restored.read_f32s(4, 3),
            reference_values,
            "portable values with their transform must match the regular decode"
        );
    }
}
