use crate::{parse_native, Document, ValidationProfile};

#[test]
fn native_document_preserves_untouched_json() {
    let bytes = br#"{"asset":{"version":"2.1"},"unknown":{"a":[1,2,3]}}"#;
    let document = Document::from_json_bytes(bytes).unwrap();
    document.validate(ValidationProfile::Gltf21Draft).unwrap();
    assert_eq!(document.to_json_bytes().unwrap(), bytes);
}

#[test]
fn native_document_serializes_after_mutation() {
    let mut document = Document::from_json_bytes(br#"{"asset":{"version":"2.0"}}"#).unwrap();
    document.as_value_mut()["asset"]["generator"] = "draco-gltf".into();
    assert_eq!(
        document.to_json_bytes().unwrap(),
        br#"{"asset":{"version":"2.0","generator":"draco-gltf"}}"#
    );
}

#[test]
fn draft_profile_accepts_files_shapes_and_nonsequential_semantics() {
    let document = Document::from_json_bytes(br#"{"asset":{"version":"2.1"},"files":[{"uri":"child.gltf"}],"shapes":[{}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_4":1}}]}]}"#).unwrap();
    document.validate(ValidationProfile::Gltf21Draft).unwrap();
}

#[test]
fn native_import_reads_json() {
    let import = parse_native(
        br#"{"asset":{"version":"2.1"},"buffers":[]}"#,
        ValidationProfile::Gltf21Draft,
    )
    .unwrap();
    assert_eq!(
        import.document.to_json_bytes().unwrap(),
        br#"{"asset":{"version":"2.1"},"buffers":[]}"#
    );
}

#[test]
fn native_compression_appends_a_decodable_draco_payload() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse_native(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();
    let primitive = import.draco_primitives().next().unwrap();
    assert_eq!(import.decode_primitive(primitive).unwrap().num_faces(), 1);
}

#[cfg(feature = "compact")]
#[test]
fn compact_facade_uses_native_document() {
    let compact = crate::CompactDocument::parse(
        br#"{"asset":{"version":"2.1"},"meshes":[{"primitives":[{},{}]}]}"#,
        ValidationProfile::Gltf21Draft,
    )
    .unwrap();
    assert_eq!(
        compact.mesh_primitive_ranges().next().unwrap().primitives,
        2
    );
}
