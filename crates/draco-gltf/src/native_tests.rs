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
    let document = Document::from_json_bytes(br#"{"asset":{"version":"2.1"},"files":[{"uri":"child.gltf"}],"shapes":[{}],"accessors":[{"componentType":5126,"type":"VEC3"},{"componentType":5126,"type":"VEC2"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_4":1}}]}]}"#).unwrap();
    document.validate(ValidationProfile::Gltf21Draft).unwrap();
}

#[test]
fn validation_rejects_dangling_core_references_and_draft_types_in_20() {
    let dangling = Document::from_json_bytes(
        br#"{"asset":{"version":"2.0"},"meshes":[{"primitives":[{"attributes":{"POSITION":1}}]}]}"#,
    )
    .unwrap();
    assert!(dangling.validate(ValidationProfile::Gltf20).is_err());

    let draft_component = Document::from_json_bytes(
        br#"{"asset":{"version":"2.0"},"accessors":[{"componentType":5132,"type":"SCALAR"}]}"#,
    )
    .unwrap();
    assert!(draft_component.validate(ValidationProfile::Gltf20).is_err());
    assert!(draft_component
        .validate(ValidationProfile::Gltf21Draft)
        .is_ok());
}

#[test]
fn typed_views_reference_the_lossless_document() {
    let document = Document::from_json_bytes(
        br#"{"asset":{"version":"2.1"},"buffers":[{"byteLength":12,"uri":"mesh.bin"}],"bufferViews":[{"buffer":0,"byteOffset":4,"byteLength":8}],"accessors":[{"bufferView":0,"componentType":5126,"count":2,"type":"VEC3"}],"images":[{"uri":"albedo.png"}],"textures":[{"source":0}],"meshes":[{"primitives":[]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"files":[{"uri":"child.gltf"}]}"#,
    )
    .unwrap();
    assert_eq!(
        document.buffer(crate::BufferIndex(0)).unwrap().uri(),
        Some("mesh.bin")
    );
    assert_eq!(
        document
            .buffer_view(crate::BufferViewIndex(0))
            .unwrap()
            .byte_offset(),
        4
    );
    assert_eq!(
        document
            .accessor(crate::AccessorIndex(0))
            .unwrap()
            .component_type(),
        Some(crate::ComponentType::F32)
    );
    assert_eq!(
        document.texture(crate::TextureIndex(0)).unwrap().source(),
        Some(crate::ImageIndex(0))
    );
    assert_eq!(
        document.node(crate::NodeIndex(0)).unwrap().mesh(),
        Some(crate::MeshIndex(0))
    );
    assert_eq!(
        document
            .scene(crate::SceneIndex(0))
            .unwrap()
            .nodes()
            .collect::<Vec<_>>(),
        [crate::NodeIndex(0)]
    );
    assert_eq!(
        document.file(crate::FileIndex(0)).unwrap().uri(),
        Some("child.gltf")
    );
    assert_eq!(
        document.to_json_bytes().unwrap().starts_with(b"{\"asset\""),
        true
    );
}

#[test]
fn validation_covers_scene_skin_and_animation_links() {
    let document = Document::from_json_bytes(
        br#"{"asset":{"version":"2.0"},"accessors":[{"componentType":5126,"type":"SCALAR"}],"nodes":[{}],"scenes":[{"nodes":[0]}],"scene":0,"skins":[{"joints":[0],"inverseBindMatrices":0}],"animations":[{"samplers":[{"input":0,"output":0}],"channels":[{"sampler":0,"target":{"node":0,"path":"translation"}}]}]}"#,
    )
    .unwrap();
    document.validate(ValidationProfile::Gltf20).unwrap();

    let mut invalid = document.clone();
    invalid.as_value_mut()["animations"][0]["channels"][0]["sampler"] = 1u64.into();
    assert!(invalid.validate(ValidationProfile::Gltf20).is_err());
}

#[test]
fn explicit_asset_loading_tracks_provenance_and_rejects_cycles() {
    let root = parse_native(
        br#"{"asset":{"version":"2.1"},"files":[{"uri":"child.gltf"}]}"#,
        ValidationProfile::Gltf21Draft,
    )
    .unwrap();
    let resolver = |uri: &str| match uri {
        "child.gltf" => {
            Ok(br#"{"asset":{"version":"2.1"},"files":[{"uri":"root.gltf"}]}"#.to_vec())
        }
        "root.gltf" => {
            Ok(br#"{"asset":{"version":"2.1"},"files":[{"uri":"child.gltf"}]}"#.to_vec())
        }
        _ => Err(draco_io::GltfError::ExternalResourceDenied(uri.into())),
    };
    let child = root
        .load_asset(
            crate::FileIndex(0),
            &resolver,
            &draco_io::ResourceLimits::default(),
            ValidationProfile::Gltf21Draft,
            &crate::ExtensionRegistry::default(),
        )
        .unwrap();
    assert_eq!(child.provenance(), ["child.gltf"]);
    let root_again = child
        .load_asset(
            crate::FileIndex(0),
            &resolver,
            &draco_io::ResourceLimits::default(),
            ValidationProfile::Gltf21Draft,
            &crate::ExtensionRegistry::default(),
        )
        .unwrap();
    assert!(root_again
        .load_asset(
            crate::FileIndex(0),
            &resolver,
            &draco_io::ResourceLimits::default(),
            ValidationProfile::Gltf21Draft,
            &crate::ExtensionRegistry::default(),
        )
        .is_err());
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

#[test]
fn native_compression_roundtrips_through_decompression() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse_native(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();
    import.decompress_in_place().unwrap();
    let primitive = import.document.primitive(crate::MeshIndex(0), 0).unwrap();
    assert!(primitive
        .extension(crate::KHR_DRACO_MESH_COMPRESSION)
        .is_none());
    assert_eq!(
        import
            .decode_geometry_primitive(primitive)
            .unwrap()
            .0
            .num_faces(),
        1
    );
}

#[test]
fn glb_serialization_consolidates_append_only_draco_buffers() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse_native(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();
    let bytes = import.to_bytes(crate::OutputFormat::GlbV2).unwrap();
    let reloaded = parse_native(&bytes, ValidationProfile::Gltf20).unwrap();
    assert_eq!(reloaded.resources.buffers.len(), 1);
    assert_eq!(reloaded.draco_primitives().count(), 1);
    assert_eq!(
        reloaded
            .decode_primitive(reloaded.draco_primitives().next().unwrap())
            .unwrap()
            .num_faces(),
        1
    );
}

#[test]
fn decompression_failure_is_atomic() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse_native(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();
    let before = import.document.to_json_bytes().unwrap();
    import.resources.buffers[1][0] ^= 0xff;
    assert!(import.decompress_in_place().is_err());
    assert_eq!(import.document.to_json_bytes().unwrap(), before);
    assert_eq!(import.draco_primitives().count(), 1);
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
