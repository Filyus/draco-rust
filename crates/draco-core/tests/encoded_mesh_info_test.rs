#![cfg(all(feature = "encoder", feature = "decoder"))]

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{AttributeValueIndex, FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::{EncodedMeshInfo, MeshEncoder};
use draco_core::version::has_header_flags;

fn write_f32s(attribute: &mut PointAttribute, values: &[f32]) {
    for (i, value) in values.iter().enumerate() {
        attribute.buffer_mut().write(i * 4, &value.to_le_bytes());
    }
}

fn add_f32_attribute(
    mesh: &mut Mesh,
    attribute_type: GeometryAttributeType,
    components: u8,
    values: &[f32],
) {
    let mut attribute = PointAttribute::new();
    attribute.init(
        attribute_type,
        components,
        DataType::Float32,
        false,
        values.len() / components as usize,
    );
    write_f32s(&mut attribute, values);
    mesh.add_attribute(attribute);
}

fn add_u8_attribute(
    mesh: &mut Mesh,
    attribute_type: GeometryAttributeType,
    components: u8,
    normalized: bool,
    values: &[u8],
) {
    let mut attribute = PointAttribute::new();
    attribute.init(
        attribute_type,
        components,
        DataType::Uint8,
        normalized,
        values.len() / components as usize,
    );
    attribute.buffer_mut().write(0, values);
    mesh.add_attribute(attribute);
}

fn build_triangle() -> Mesh {
    let mut mesh = Mesh::new();
    mesh.set_num_points(3);
    add_f32_attribute(
        &mut mesh,
        GeometryAttributeType::Position,
        3,
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0],
    );
    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh
}

fn build_multi_attribute_quad() -> Mesh {
    let mut mesh = Mesh::new();
    mesh.set_num_points(4);
    add_f32_attribute(
        &mut mesh,
        GeometryAttributeType::Position,
        3,
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0],
    );
    add_f32_attribute(
        &mut mesh,
        GeometryAttributeType::Normal,
        3,
        &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
    );
    add_u8_attribute(
        &mut mesh,
        GeometryAttributeType::Color,
        4,
        true,
        &[
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
    );
    add_f32_attribute(
        &mut mesh,
        GeometryAttributeType::TexCoord,
        2,
        &[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
    );
    mesh.set_num_faces(2);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);
    mesh
}

fn build_uv_seam_mesh() -> Mesh {
    let mut mesh = Mesh::new();
    mesh.set_num_points(6);

    let mut positions = PointAttribute::new();
    positions.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        4,
    );
    write_f32s(
        &mut positions,
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0],
    );
    positions.set_explicit_mapping(6);
    for (point, entry) in [0, 1, 2, 1, 3, 2].iter().copied().enumerate() {
        positions.set_point_map_entry(PointIndex(point as u32), AttributeValueIndex(entry));
    }
    mesh.add_attribute(positions);

    add_f32_attribute(
        &mut mesh,
        GeometryAttributeType::TexCoord,
        2,
        &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.2, 0.0, 1.0, 1.0, 0.2, 1.0],
    );

    mesh.add_face([PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.add_face([PointIndex(3), PointIndex(4), PointIndex(5)]);
    mesh
}

fn build_grid_mesh(cells_per_side: usize) -> Mesh {
    let vertices_per_side = cells_per_side + 1;
    let mut positions = Vec::with_capacity(vertices_per_side * vertices_per_side * 3);
    for y in 0..vertices_per_side {
        for x in 0..vertices_per_side {
            positions.push(x as f32);
            positions.push(y as f32);
            positions.push(((x * 17 + y * 31) % 11) as f32 * 0.01);
        }
    }

    let mut mesh = Mesh::new();
    mesh.set_num_points(vertices_per_side * vertices_per_side);
    add_f32_attribute(&mut mesh, GeometryAttributeType::Position, 3, &positions);

    for y in 0..cells_per_side {
        for x in 0..cells_per_side {
            let p0 = (y * vertices_per_side + x) as u32;
            let p1 = p0 + 1;
            let p2 = p0 + vertices_per_side as u32;
            let p3 = p2 + 1;
            mesh.add_face([PointIndex(p0), PointIndex(p1), PointIndex(p3)]);
            mesh.add_face([PointIndex(p0), PointIndex(p3), PointIndex(p2)]);
        }
    }

    mesh
}

fn encode_decode_bytes_with_info(
    mesh: Mesh,
    options: EncoderOptions,
) -> (EncodedMeshInfo, Mesh, Vec<u8>) {
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut bytes = EncoderBuffer::new();
    encoder
        .encode(&options, &mut bytes)
        .expect("encode should succeed");
    let info = encoder
        .encoded_mesh_info()
        .cloned()
        .expect("successful encode should expose mesh info");

    let mut decoder = MeshDecoder::new();
    let mut decoded = Mesh::new();
    let mut decode_buffer = DecoderBuffer::new(bytes.data());
    decoder
        .decode(&mut decode_buffer, &mut decoded)
        .expect("decode should succeed");
    (info, decoded, bytes.data().to_vec())
}

fn encode_decode_with_info(mesh: Mesh, options: EncoderOptions) -> (EncodedMeshInfo, Mesh) {
    let (info, decoded, _) = encode_decode_bytes_with_info(mesh, options);
    (info, decoded)
}

fn edgebreaker_traversal_type(bytes: &[u8]) -> u8 {
    assert_eq!(&bytes[0..5], b"DRACO");
    assert_eq!(bytes[7], 1, "expected triangular mesh geometry type");
    assert_eq!(bytes[8], 1, "expected Edgebreaker encoding");

    let major = bytes[5];
    let minor = bytes[6];
    let mut buffer = DecoderBuffer::new(&bytes[9..]);
    buffer.set_version(major, minor);
    if has_header_flags(major, minor) {
        let _ = buffer.decode_u16().expect("header flags");
    }
    buffer.decode_u8().expect("traversal type")
}

fn position_bounds(attribute: &PointAttribute) -> Option<(Vec<f64>, Vec<f64>)> {
    if attribute.attribute_type() != GeometryAttributeType::Position
        || attribute.data_type() != DataType::Float32
        || attribute.num_components() != 3
        || attribute.size() == 0
    {
        return None;
    }

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let bytes = attribute.buffer().data();
    for value_idx in 0..attribute.size() {
        for component in 0..3 {
            let offset = (value_idx * 3 + component) * DataType::Float32.byte_length();
            let value = f32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]);
            min[component] = min[component].min(value);
            max[component] = max[component].max(value);
        }
    }
    Some((
        min.into_iter().map(f64::from).collect(),
        max.into_iter().map(f64::from).collect(),
    ))
}

fn assert_vec_close(left: &[f64], right: &[f64]) {
    assert_eq!(left.len(), right.len());
    for (l, r) in left.iter().zip(right) {
        assert!((l - r).abs() <= 1e-6, "{left:?} != {right:?}");
    }
}

fn assert_info_matches_decoded(info: &EncodedMeshInfo, decoded: &Mesh) {
    assert_eq!(info.num_encoded_faces, decoded.num_faces());
    assert_eq!(info.num_encoded_points, decoded.num_points());
    assert_eq!(info.attributes.len(), decoded.num_attributes() as usize);

    for (att_id, actual) in info.attributes.iter().enumerate() {
        let decoded_att = decoded.attribute(att_id as i32);
        assert_eq!(actual.source_attribute_id, att_id as i32);
        assert_eq!(actual.attribute_type, decoded_att.attribute_type());
        assert_eq!(actual.data_type, decoded_att.data_type());
        assert_eq!(actual.num_components, decoded_att.num_components());
        assert_eq!(actual.normalized, decoded_att.normalized());
        assert_eq!(actual.unique_id, decoded_att.unique_id());
        assert_eq!(
            actual.num_encoded_values,
            decoded_att.size(),
            "attribute {att_id} value count"
        );

        if let Some((expected_min, expected_max)) = position_bounds(decoded_att) {
            assert_vec_close(actual.position_min.as_deref().unwrap(), &expected_min);
            assert_vec_close(actual.position_max.as_deref().unwrap(), &expected_max);
        } else {
            assert!(actual.position_min.is_none());
            assert!(actual.position_max.is_none());
        }
    }
}

#[test]
fn sequential_encoded_mesh_info_matches_decoded_mesh() {
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 0);
    options.set_attribute_int(0, "quantization_bits", 10);

    let (info, decoded) = encode_decode_with_info(build_triangle(), options);

    assert_eq!(info.encoding_method, 0);
    assert_info_matches_decoded(&info, &decoded);
}

#[test]
fn default_edgebreaker_encoded_mesh_info_matches_decoded_mesh() {
    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 10);

    let (info, decoded) = encode_decode_with_info(build_triangle(), options);

    assert_eq!(info.encoding_method, 1);
    assert_info_matches_decoded(&info, &decoded);
}

#[test]
fn edgebreaker_seam_encoded_mesh_info_matches_decoded_mesh() {
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1);
    options.set_global_int("encoding_speed", 5);
    options.set_global_int("decoding_speed", 5);
    options.set_global_int("split_mesh_on_seams", 0);

    let (info, decoded) = encode_decode_with_info(build_uv_seam_mesh(), options);

    assert_eq!(info.encoding_method, 1);
    assert_info_matches_decoded(&info, &decoded);
}

#[test]
fn quantized_edgebreaker_seam_encoded_mesh_info_matches_decoded_mesh() {
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1);
    options.set_global_int("encoding_speed", 5);
    options.set_global_int("decoding_speed", 5);
    options.set_global_int("split_mesh_on_seams", 0);
    options.set_attribute_int(0, "quantization_bits", 14);
    options.set_attribute_int(1, "quantization_bits", 12);

    let (info, decoded) = encode_decode_with_info(build_uv_seam_mesh(), options);

    assert_eq!(info.encoding_method, 1);
    assert_info_matches_decoded(&info, &decoded);
}

#[test]
fn valence_edgebreaker_encoded_mesh_info_matches_decoded_mesh() {
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1);
    options.set_global_int("encoding_speed", 0);
    options.set_global_int("decoding_speed", 0);
    options.set_attribute_int(0, "quantization_bits", 12);

    let (info, decoded, bytes) = encode_decode_bytes_with_info(build_grid_mesh(32), options);

    assert_eq!(edgebreaker_traversal_type(&bytes), 2);
    assert_eq!(info.encoding_method, 1);
    assert_info_matches_decoded(&info, &decoded);
}

#[test]
fn multi_attribute_encoded_mesh_info_matches_decoded_mesh() {
    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 12);
    options.set_attribute_int(1, "quantization_bits", 8);
    options.set_attribute_int(3, "quantization_bits", 10);

    let (info, decoded) = encode_decode_with_info(build_multi_attribute_quad(), options);

    assert_eq!(info.encoding_method, 1);
    assert_info_matches_decoded(&info, &decoded);
}
