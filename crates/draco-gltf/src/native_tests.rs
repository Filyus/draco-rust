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
