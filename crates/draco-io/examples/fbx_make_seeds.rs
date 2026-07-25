//! Generate the tiny FBX seed inputs committed under `fuzz/seeds/`.
//!
//! Seeds give libFuzzer valid container structure to mutate instead of making
//! it rediscover the 27-byte header by chance. They are regenerated rather than
//! hand-edited:
//!
//! ```text
//! cargo run --example fbx_make_seeds -- fuzz/seeds
//! ```
//!
//! The malformed seeds encode one hazard each, so a crash minimises to an
//! obvious cause.

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_io::traits::Writer;
use draco_io::FbxWriter;
use std::path::Path;

const MAGIC: &[u8; 21] = b"Kaydara FBX Binary  \0";

fn header(version: u32) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(MAGIC);
    data.push(0x1A);
    data.push(0x00); // little-endian marker
    data.extend_from_slice(&version.to_le_bytes());
    data
}

/// A 32-bit node record: `end_offset`, `num_properties`, `property_list_len`,
/// `name_len`, then the name.
fn node_record_32(end_offset: u32, num_properties: u32, list_len: u32, name: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&end_offset.to_le_bytes());
    data.extend_from_slice(&num_properties.to_le_bytes());
    data.extend_from_slice(&list_len.to_le_bytes());
    data.push(name.len() as u8);
    data.extend_from_slice(name);
    data
}

fn triangle() -> Mesh {
    let mut mesh = Mesh::new();
    mesh.set_num_points(3);
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        3,
    );
    for (index, point) in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        .iter()
        .enumerate()
    {
        let bytes: Vec<u8> = point.iter().flat_map(|v| v.to_le_bytes()).collect();
        position.buffer_mut().write(index * 12, &bytes);
    }
    mesh.add_attribute(position);
    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: fbx_make_seeds <fuzz/seeds dir>")?;
    let root = Path::new(&root);

    let mut seeds: Vec<(&str, Vec<u8>)> = Vec::new();

    // A real, canonical file straight from our writer.
    let mut writer = FbxWriter::new();
    writer.add_mesh(&triangle(), Some("seed"))?;
    seeds.push(("valid_triangle.fbx", writer.write_to_vec()?));

    // Header plus an immediate root terminator: the smallest accepted file.
    let mut empty = header(7400);
    empty.extend_from_slice(&[0u8; 13]);
    seeds.push(("empty_7400.fbx", empty));

    // Same, in the 64-bit record layout.
    let mut empty75 = header(7500);
    empty75.extend_from_slice(&[0u8; 25]);
    seeds.push(("empty_7500.fbx", empty75));

    // Big-endian header, so the endian path is reachable from the corpus.
    let mut big = Vec::new();
    big.extend_from_slice(MAGIC);
    big.push(0x1A);
    big.push(0x01);
    big.extend_from_slice(&7500u32.to_be_bytes());
    big.extend_from_slice(&[0u8; 25]);
    seeds.push(("empty_big_endian_7500.fbx", big));

    // `end_offset` pointing back into the header: used to loop forever.
    let mut backward = header(7400);
    backward.extend_from_slice(&node_record_32(1, 0, 0, b"N"));
    seeds.push(("bad_end_offset_backward.fbx", backward));

    // `end_offset` past the end of the file.
    let mut past_eof = header(7400);
    past_eof.extend_from_slice(&node_record_32(u32::MAX, 0, 0, b"N"));
    seeds.push(("bad_end_offset_past_eof.fbx", past_eof));

    // A 32-bit record is 13 bytes; a one-byte name makes the property list
    // start at 27 + 14. An array property is its type code plus a 12-byte
    // array header.
    const PROPERTIES_START: u32 = 27 + 14;
    const ARRAY_PROPERTY_LEN: u32 = 1 + 12;

    // An `f64` array claiming ~4G elements from a 12-byte header.
    let mut huge_array = header(7400);
    let mut body = node_record_32(
        PROPERTIES_START + ARRAY_PROPERTY_LEN,
        1,
        ARRAY_PROPERTY_LEN,
        b"A",
    );
    body.push(b'd');
    body.extend_from_slice(&u32::MAX.to_le_bytes()); // element count
    body.extend_from_slice(&0u32.to_le_bytes()); // encoding: raw
    body.extend_from_slice(&0u32.to_le_bytes()); // stored length
    huge_array.extend_from_slice(&body);
    huge_array.extend_from_slice(&[0u8; 13]);
    seeds.push(("bad_array_element_count.fbx", huge_array));

    // A `zlib` array whose declared output dwarfs its input. The element count
    // stays under the size ceiling on purpose, so the read reaches inflation
    // and fails on the exact-output-size check rather than the limit.
    let mut bomb = header(7400);
    let payload: Vec<u8> = vec![
        0x78, 0x9c, 0x63, 0x60, 0x18, 0x05, 0xa3, 0x60, 0x14, 0x8c, 0x00,
    ];
    let list_len = ARRAY_PROPERTY_LEN + payload.len() as u32;
    let mut bomb_body = node_record_32(PROPERTIES_START + list_len, 1, list_len, b"Z");
    bomb_body.push(b'd');
    bomb_body.extend_from_slice(&100_000u32.to_le_bytes()); // claimed elements
    bomb_body.extend_from_slice(&1u32.to_le_bytes()); // encoding: zlib
    bomb_body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bomb_body.extend_from_slice(&payload);
    bomb.extend_from_slice(&bomb_body);
    bomb.extend_from_slice(&[0u8; 13]);
    seeds.push(("bad_zlib_bomb.fbx", bomb));

    // A property list declaring more bytes than its single `I` property holds,
    // while still fitting inside the node. The reader reports the mismatch and
    // resynchronizes on the declared end instead of eating the next record.
    let mut mismatch = header(7400);
    let declared_list_len = 9u32; // an `I` property is 1 + 4
    let mut mismatch_body = node_record_32(
        PROPERTIES_START + declared_list_len,
        1,
        declared_list_len,
        b"P",
    );
    mismatch_body.push(b'I');
    mismatch_body.extend_from_slice(&7i32.to_le_bytes());
    mismatch_body.extend_from_slice(&[0u8; 4]); // slack the header accounts for
    mismatch.extend_from_slice(&mismatch_body);
    mismatch.extend_from_slice(&[0u8; 13]);
    seeds.push(("bad_property_list_len.fbx", mismatch));

    for (target, names) in [
        (
            "fbx_read_scene",
            vec![
                "valid_triangle.fbx",
                "empty_7400.fbx",
                "empty_7500.fbx",
                "empty_big_endian_7500.fbx",
                "bad_end_offset_backward.fbx",
                "bad_end_offset_past_eof.fbx",
                "bad_array_element_count.fbx",
                "bad_zlib_bomb.fbx",
                "bad_property_list_len.fbx",
            ],
        ),
        (
            "fbx_roundtrip",
            vec![
                "valid_triangle.fbx",
                "empty_7400.fbx",
                "empty_7500.fbx",
                "empty_big_endian_7500.fbx",
            ],
        ),
    ] {
        let dir = root.join(target);
        std::fs::create_dir_all(&dir)?;
        for name in names {
            let bytes = &seeds
                .iter()
                .find(|(seed, _)| *seed == name)
                .expect("seed defined above")
                .1;
            std::fs::write(dir.join(name), bytes)?;
            println!("{}/{name}: {} bytes", target, bytes.len());
        }
    }
    Ok(())
}
