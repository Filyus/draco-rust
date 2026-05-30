use draco_core::{
    DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, FaceIndex, GeometryAttributeType, Mesh,
    MeshDecoder, MeshEncoder, Metadata, PointAttribute, PointCloud, PointCloudDecoder,
    PointCloudEncoder, PointIndex,
};

fn header_flags(bytes: &[u8]) -> u16 {
    assert!(bytes.len() >= 11);
    u16::from_le_bytes([bytes[9], bytes[10]])
}

fn position_attribute(num_points: usize) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );

    let values: [f32; 12] = [
        0.0, 0.0, 0.0, //
        1.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, //
        1.0, 1.0, 0.0,
    ];
    for i in 0..num_points {
        let bytes = [
            values[i * 3].to_le_bytes(),
            values[i * 3 + 1].to_le_bytes(),
            values[i * 3 + 2].to_le_bytes(),
        ]
        .concat();
        attribute.buffer_mut().write(i * 12, &bytes);
    }

    attribute
}

fn add_metadata(point_cloud: &mut PointCloud, att_id: i32, label: &'static [u8]) {
    point_cloud
        .attribute_mut(att_id)
        .set_unique_id(42 + att_id as u32);

    point_cloud
        .metadata_or_insert()
        .metadata_mut()
        .set_raw("geometry", label.to_vec())
        .unwrap();

    let mut attribute_metadata = Metadata::new();
    attribute_metadata
        .set_raw("semantic", b"POSITION".to_vec())
        .unwrap();
    point_cloud
        .set_attribute_metadata(att_id, attribute_metadata)
        .unwrap();
}

fn assert_metadata(point_cloud: &PointCloud, label: &'static [u8]) {
    let metadata = point_cloud.metadata().expect("geometry metadata");
    assert_eq!(metadata.metadata().get_raw("geometry"), Some(label));

    let unique_id = point_cloud.attribute(0).unique_id();
    let attribute_metadata = point_cloud
        .attribute_metadata_by_unique_id(unique_id)
        .expect("attribute metadata");
    assert_eq!(
        attribute_metadata.metadata().get_raw("semantic"),
        Some(b"POSITION".as_slice())
    );
}

fn add_typed_metadata(point_cloud: &mut PointCloud, att_id: i32) {
    point_cloud.attribute_mut(att_id).set_unique_id(77);

    let metadata = point_cloud.metadata_or_insert().metadata_mut();
    metadata.set_string("geometry", "typed").unwrap();
    metadata.set_i32("revision", 2).unwrap();
    metadata.set_f64_array("scale", &[1.0, 2.5]).unwrap();

    let mut attribute_metadata = Metadata::new();
    attribute_metadata.set_string("name", "position").unwrap();
    attribute_metadata.set_i32("components", 3).unwrap();
    point_cloud
        .set_attribute_metadata(att_id, attribute_metadata)
        .unwrap();
}

fn assert_typed_metadata(point_cloud: &PointCloud) {
    let metadata = point_cloud.metadata().expect("geometry metadata");
    assert_eq!(metadata.metadata().get_string("geometry"), Some("typed"));
    assert_eq!(metadata.metadata().get_i32("revision"), Some(2));
    assert_eq!(
        metadata.metadata().get_f64_array("scale"),
        Some(vec![1.0, 2.5])
    );

    let attribute_metadata = point_cloud
        .attribute_metadata_by_string_entry("name", "position")
        .expect("attribute metadata by string entry");
    assert_eq!(
        attribute_metadata.attribute_unique_id(),
        point_cloud.attribute(0).unique_id()
    );
    assert_eq!(attribute_metadata.metadata().get_i32("components"), Some(3));
}

fn make_point_cloud(label: &'static [u8]) -> PointCloud {
    let mut point_cloud = PointCloud::new();
    let att_id = point_cloud.add_attribute(position_attribute(4));
    add_metadata(&mut point_cloud, att_id, label);
    point_cloud
}

fn make_mesh(label: &'static [u8]) -> Mesh {
    let mut mesh = Mesh::new();
    let att_id = mesh.add_attribute(position_attribute(4));
    add_metadata(&mut mesh, att_id, label);

    mesh.set_num_faces(2);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(1), PointIndex(3), PointIndex(2)]);
    mesh
}

fn encode_decode_point_cloud(method: i32, label: &'static [u8]) -> (Vec<u8>, PointCloud) {
    let point_cloud = make_point_cloud(label);
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(point_cloud);

    let mut options = EncoderOptions::new();
    options.set_encoding_method(method);
    options.set_attribute_int(0, "quantization_bits", 10);

    let mut encoded = EncoderBuffer::new();
    encoder
        .encode(&options, &mut encoded)
        .expect("point cloud encode");

    let bytes = encoded.data().to_vec();
    let mut buffer = DecoderBuffer::new(&bytes);
    let mut decoded = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut buffer, &mut decoded)
        .expect("point cloud decode");

    (bytes, decoded)
}

fn encode_decode_mesh(method: i32, label: &'static [u8]) -> (Vec<u8>, Mesh) {
    let mesh = make_mesh(label);
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", method);
    options.set_attribute_int(0, "quantization_bits", 10);

    let mut encoded = EncoderBuffer::new();
    encoder.encode(&options, &mut encoded).expect("mesh encode");

    let bytes = encoded.data().to_vec();
    let mut buffer = DecoderBuffer::new(&bytes);
    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut buffer, &mut decoded)
        .expect("mesh decode");

    (bytes, decoded)
}

#[test]
fn point_cloud_sequential_metadata_roundtrips() {
    let (bytes, decoded) = encode_decode_point_cloud(0, b"point-cloud-sequential");

    assert_eq!(header_flags(&bytes), 0x8000);
    assert_metadata(&decoded, b"point-cloud-sequential");
}

#[test]
fn point_cloud_kdtree_metadata_roundtrips() {
    let (bytes, decoded) = encode_decode_point_cloud(1, b"point-cloud-kdtree");

    assert_eq!(header_flags(&bytes), 0x8000);
    assert_metadata(&decoded, b"point-cloud-kdtree");
}

#[test]
fn point_cloud_typed_metadata_roundtrips() {
    let mut point_cloud = PointCloud::new();
    let att_id = point_cloud.add_attribute(position_attribute(4));
    add_typed_metadata(&mut point_cloud, att_id);

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(point_cloud);

    let mut options = EncoderOptions::new();
    options.set_encoding_method(0);
    options.set_attribute_int(0, "quantization_bits", 10);

    let mut encoded = EncoderBuffer::new();
    encoder
        .encode(&options, &mut encoded)
        .expect("point cloud encode");

    let bytes = encoded.data().to_vec();
    let mut buffer = DecoderBuffer::new(&bytes);
    let mut decoded = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut buffer, &mut decoded)
        .expect("point cloud decode");

    assert_eq!(header_flags(&bytes), 0x8000);
    assert_typed_metadata(&decoded);
}

#[test]
fn mesh_sequential_metadata_roundtrips() {
    let (bytes, decoded) = encode_decode_mesh(0, b"mesh-sequential");

    assert_eq!(header_flags(&bytes), 0x8000);
    assert_metadata(&decoded, b"mesh-sequential");
    assert_eq!(decoded.num_faces(), 2);
}

#[test]
fn mesh_edgebreaker_metadata_roundtrips() {
    let (bytes, decoded) = encode_decode_mesh(1, b"mesh-edgebreaker");

    assert_eq!(header_flags(&bytes), 0x8000);
    assert_metadata(&decoded, b"mesh-edgebreaker");
    assert_eq!(decoded.num_faces(), 2);
}

#[test]
fn no_metadata_keeps_header_flags_clear() {
    let mut point_cloud = PointCloud::new();
    point_cloud.add_attribute(position_attribute(3));

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(point_cloud);

    let options = EncoderOptions::new();
    let mut encoded = EncoderBuffer::new();
    encoder.encode(&options, &mut encoded).unwrap();

    assert_eq!(header_flags(encoded.data()), 0);
}

#[test]
fn metadata_with_pre_flags_bitstream_version_is_rejected() {
    let point_cloud = make_point_cloud(b"old-version");
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(point_cloud);

    let mut options = EncoderOptions::new();
    options.set_version(1, 2);

    let mut encoded = EncoderBuffer::new();
    assert!(encoder.encode(&options, &mut encoded).is_err());
}
