use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{AttributeValueIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::version::{has_header_flags, uses_varint_encoding};
use std::path::Path;
use std::process::Command;

fn write_f32s(attribute: &mut PointAttribute, values: &[f32]) {
    for (i, value) in values.iter().enumerate() {
        attribute.buffer_mut().write(i * 4, &value.to_le_bytes());
    }
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
        &[
            0.0, 0.0, 0.0, // 0
            1.0, 0.0, 0.0, // 1
            0.0, 1.0, 0.0, // 2
            1.0, 1.0, 0.0, // 3
        ],
    );
    positions.set_explicit_mapping(6);
    for (point, entry) in [0, 1, 2, 1, 3, 2].iter().copied().enumerate() {
        positions.set_point_map_entry(PointIndex(point as u32), AttributeValueIndex(entry));
    }
    mesh.add_attribute(positions);

    let mut texcoords = PointAttribute::new();
    texcoords.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Float32,
        false,
        6,
    );
    write_f32s(
        &mut texcoords,
        &[
            0.0, 0.0, // p0
            1.0, 0.0, // p1
            0.0, 1.0, // p2
            0.2, 0.0, // p3: same position vertex as p1, different UV
            1.0, 1.0, // p4
            0.2, 1.0, // p5: same position vertex as p2, different UV
        ],
    );
    mesh.add_attribute(texcoords);

    mesh.add_face([PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.add_face([PointIndex(3), PointIndex(4), PointIndex(5)]);
    mesh
}

fn encoded_edgebreaker_num_attribute_data(bytes: &[u8]) -> u8 {
    assert_eq!(&bytes[0..5], b"DRACO");
    let major = bytes[5];
    let minor = bytes[6];
    assert_eq!(bytes[7], 1, "expected triangular mesh geometry type");
    assert_eq!(bytes[8], 1, "expected Edgebreaker encoding");

    let mut buffer = DecoderBuffer::new(&bytes[9..]);
    buffer.set_version(major, minor);
    if has_header_flags(major, minor) {
        let _ = buffer.decode_u16().expect("flags");
    }
    let _traversal_type = buffer.decode_u8().expect("traversal type");
    if uses_varint_encoding(major, minor) {
        let _ = buffer.decode_varint().expect("num vertices");
        let _ = buffer.decode_varint().expect("num faces");
    } else {
        let _ = buffer.decode_u32().expect("num vertices");
        let _ = buffer.decode_u32().expect("num faces");
    }
    buffer.decode_u8().expect("num attribute data")
}

fn find_cpp_decoder() -> Option<String> {
    std::env::var("DRACO_CPP_DECODER").ok().or_else(|| {
        [
            "../../build-original/src/draco/Release/draco_decoder.exe",
            "../../build/src/draco/Release/draco_decoder.exe",
            "../../build/src/draco/Debug/draco_decoder.exe",
        ]
        .iter()
        .find(|path| Path::new(path).exists())
        .map(|path| path.to_string())
    })
}

fn encode_uv_seam_mesh() -> Vec<u8> {
    let mesh = build_uv_seam_mesh();
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1);
    options.set_global_int("encoding_speed", 5);
    options.set_global_int("decoding_speed", 5);
    options.set_global_int("split_mesh_on_seams", 0);

    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("Edgebreaker encode should succeed");
    buffer.data().to_vec()
}

#[test]
fn edgebreaker_emits_attribute_seam_data_when_position_dedup_splits_uvs() {
    let bytes = encode_uv_seam_mesh();

    assert_eq!(encoded_edgebreaker_num_attribute_data(&bytes), 1);

    let mut decoder = MeshDecoder::new();
    let mut decoded = Mesh::new();
    let mut decode_buffer = DecoderBuffer::new(&bytes);
    decoder
        .decode(&mut decode_buffer, &mut decoded)
        .expect("Rust decoder should read Rust seam stream");

    assert!(decoded.num_faces() > 0);
    assert!(decoded.named_attribute_id(GeometryAttributeType::TexCoord) >= 0);
}

#[test]
fn cpp_decoder_accepts_rust_edgebreaker_attribute_seam_stream_when_available() {
    let Some(decoder_path) = find_cpp_decoder() else {
        eprintln!("C++ decoder not found, skipping seam interop test");
        return;
    };

    let bytes = encode_uv_seam_mesh();
    let tmp = std::env::temp_dir().join("draco_edgebreaker_attribute_seam_test");
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let drc_path = tmp.join("seam.drc");
    let obj_path = tmp.join("seam.obj");
    std::fs::write(&drc_path, bytes).expect("write drc");

    let output = Command::new(&decoder_path)
        .arg("-i")
        .arg(&drc_path)
        .arg("-o")
        .arg(&obj_path)
        .output()
        .expect("run C++ decoder");

    assert!(
        output.status.success(),
        "C++ decoder failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
