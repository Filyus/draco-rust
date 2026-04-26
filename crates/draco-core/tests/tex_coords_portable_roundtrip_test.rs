use draco_core::corner_table::CornerTable;
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{CornerIndex, PointIndex, VertexIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_prediction_scheme_data::MeshPredictionSchemeData;
use draco_core::prediction_scheme::{
    PredictionScheme, PredictionSchemeDecoder, PredictionSchemeEncoder,
};
use draco_core::prediction_scheme_tex_coords_portable::{
    MeshPredictionSchemeTexCoordsPortableDecoder, MeshPredictionSchemeTexCoordsPortableEncoder,
    PredictionSchemeTexCoordsPortableEncodingTransform,
};
use draco_core::prediction_scheme_wrap::PredictionSchemeWrapDecodingTransform;

#[test]
fn test_tex_coords_portable_roundtrip() {
    // 1. Create Mesh
    let mut mesh = Mesh::new();

    // Position Attribute (Parent)
    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Int32,
        false,
        4,
    );
    {
        let buffer = pos_att.buffer_mut();
        // 0: (0,0,0)
        buffer.write(0, &0i32.to_le_bytes());
        buffer.write(4, &0i32.to_le_bytes());
        buffer.write(8, &0i32.to_le_bytes());
        // 1: (10,0,0)
        buffer.write(12, &10i32.to_le_bytes());
        buffer.write(16, &0i32.to_le_bytes());
        buffer.write(20, &0i32.to_le_bytes());
        // 2: (0,10,0)
        buffer.write(24, &0i32.to_le_bytes());
        buffer.write(28, &10i32.to_le_bytes());
        buffer.write(32, &0i32.to_le_bytes());
        // 3: (10,10,0)
        buffer.write(36, &10i32.to_le_bytes());
        buffer.write(40, &10i32.to_le_bytes());
        buffer.write(44, &0i32.to_le_bytes());
    }
    pos_att.set_identity_mapping();
    let pos_att_id = mesh.add_attribute(pos_att);

    // Faces
    mesh.add_face([PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.add_face([PointIndex(2), PointIndex(1), PointIndex(3)]);

    // 2. Create CornerTable
    let mut corner_table = CornerTable::new(2);
    let faces = vec![
        [VertexIndex(0), VertexIndex(1), VertexIndex(2)],
        [VertexIndex(2), VertexIndex(1), VertexIndex(3)],
    ];
    corner_table.init(&faces);

    // Manually set opposites
    corner_table.set_opposite(CornerIndex(0), CornerIndex(5));
    corner_table.set_opposite(CornerIndex(5), CornerIndex(0));

    corner_table.compute_vertex_corners(4);

    // 3. Prepare Data for Prediction Scheme
    let vertex_to_data_map = vec![0, 1, 2, 3];
    let data_to_corner_map = vec![0, 1, 2, 5]; // c0, c1, c2, c5

    let mut mesh_data = MeshPredictionSchemeData::new();
    mesh_data.set(&corner_table, &data_to_corner_map, &vertex_to_data_map);

    // 4. Prepare Tex Coord Data (to be encoded)
    // Let's use simple mapping: u = x, v = y
    // 0: (0,0)
    // 1: (10,0)
    // 2: (0,10)
    // 3: (10,10)

    let in_data = vec![0, 0, 10, 0, 0, 10, 10, 10];

    // 5. Encode
    let transform = PredictionSchemeTexCoordsPortableEncodingTransform::new();
    let mut encoder = MeshPredictionSchemeTexCoordsPortableEncoder::new(transform);

    let pos_att_ref = mesh.attribute(pos_att_id);
    assert!(encoder.set_parent_attribute(pos_att_ref));
    assert!(encoder.init(&mesh_data));

    let mut out_corr = vec![0i32; 8];
    let entry_to_point_id_map = vec![0, 1, 2, 3];

    assert!(encoder.compute_correction_values(
        &in_data,
        &mut out_corr,
        4,
        2,
        Some(
            draco_core::prediction_scheme::EntryToPointIdMap::from_u32_slice(
                &entry_to_point_id_map
            )
        )
    ));

    let mut buffer = Vec::new();
    assert!(encoder.encode_prediction_data(&mut buffer));

    // 6. Decode
    let mut decoder_buffer = DecoderBuffer::new(&buffer);
    // Set version to 2.2+ so decoder reads size as varint
    // (encoder always writes v2.2+ format with varint sizes)
    decoder_buffer.set_version(2, 2);

    let transform_dec = PredictionSchemeWrapDecodingTransform::<i32>::new();
    let mut decoder = MeshPredictionSchemeTexCoordsPortableDecoder::new(transform_dec);

    assert!(decoder.set_parent_attribute(pos_att_ref));
    assert!(decoder.init(&mesh_data));

    assert!(decoder.decode_prediction_data(&mut decoder_buffer));

    let mut out_values = vec![0i32; 8];

    assert!(decoder.compute_original_values(
        &out_corr,
        &mut out_values,
        4,
        2,
        Some(
            draco_core::prediction_scheme::EntryToPointIdMap::from_u32_slice(
                &entry_to_point_id_map
            )
        )
    ));

    // 7. Verify
    assert_eq!(in_data, out_values);
}
