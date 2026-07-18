use crate::{parse, Document, ValidationProfile};

#[test]
fn document_preserves_untouched_json() {
    let bytes = br#"{"asset":{"version":"2.1"},"unknown":{"a":[1,2,3]}}"#;
    let document = Document::from_json_bytes(bytes).unwrap();
    document.validate(ValidationProfile::Gltf21Draft).unwrap();
    assert_eq!(document.to_json_bytes().unwrap(), bytes);
}

#[test]
fn document_serializes_after_mutation() {
    let mut document = Document::from_json_bytes(br#"{"asset":{"version":"2.0"}}"#).unwrap();
    document.as_value_mut()["asset"]["generator"] = "draco-gltf".into();
    assert_eq!(
        document.to_json_bytes().unwrap(),
        br#"{"asset":{"version":"2.0","generator":"draco-gltf"}}"#
    );
}

#[test]
fn draft_profile_accepts_files_shapes_and_nonsequential_semantics() {
    let document = Document::from_json_bytes(br#"{"asset":{"version":"2.1"},"files":[{"uri":"child.gltf","mimeType":"model/gltf+json"}],"shapes":[{"type":"box","box":{"size":[2,3,4]}}],"accessors":[{"componentType":5126,"count":0,"type":"VEC3"},{"componentType":5126,"count":0,"type":"VEC2"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_4":1}}]}]}"#).unwrap();
    document.validate(ValidationProfile::Gltf21Draft).unwrap();
}

#[test]
fn draft_validation_covers_published_scene_links() {
    let document = Document::from_json_bytes(
        br#"{"asset":{"version":"2.1","thumbnail":0},"images":[{"uri":"thumbnail.png","mimeType":"image/png"}],"files":[{"uri":"part.glb","mimeType":"model/gltf-binary"}],"externalAssets":[{"name":"part","file":0}],"shapes":[{"type":"box","box":{"size":[2,3,4]}}],"nodes":[{"externalAsset":0,"boundingVolume":{"shape":0}}]}"#,
    )
    .unwrap();
    document.validate(ValidationProfile::Gltf21Draft).unwrap();
    assert_eq!(document.thumbnail(), Some(crate::ImageIndex(0)));
    assert_eq!(
        document
            .external_asset(crate::ExternalAssetIndex(0))
            .unwrap()
            .file(),
        Some(crate::FileIndex(0))
    );
    assert_eq!(
        document.node(crate::NodeIndex(0)).unwrap().external_asset(),
        Some(crate::ExternalAssetIndex(0))
    );
    assert_eq!(
        document
            .node(crate::NodeIndex(0))
            .unwrap()
            .bounding_volume()
            .unwrap()
            .shape(),
        Some(crate::ShapeIndex(0))
    );
    assert_eq!(
        document.shape(crate::ShapeIndex(0)).unwrap().shape_type(),
        Some("box")
    );

    let invalid = Document::from_json_bytes(
        br#"{"asset":{"version":"2.1"},"shapes":[{"type":"box"}],"nodes":[{"boundingVolume":{"shape":0}}]}"#,
    )
    .unwrap();
    assert!(invalid.validate(ValidationProfile::Gltf21Draft).is_err());
}

#[cfg(feature = "resources")]
#[test]
fn external_asset_models_load_explicitly() {
    let root = parse(
        br#"{"asset":{"version":"2.1"},"files":[{"uri":"part.gltf","mimeType":"model/gltf+json"}],"externalAssets":[{"file":0}]}"#,
        ValidationProfile::Gltf21Draft,
    )
    .unwrap();
    let resolver = |uri: &str| match uri {
        "part.gltf" => Ok(br#"{"asset":{"version":"2.1"}}"#.to_vec()),
        _ => Err(draco_io::GltfError::ExternalResourceDenied(uri.into())),
    };
    let loaded = root
        .load_external_asset(
            crate::ExternalAssetIndex(0),
            &resolver,
            &draco_io::ResourceLimits::default(),
            ValidationProfile::Gltf21Draft,
            &crate::ExtensionRegistry::default(),
        )
        .unwrap();
    assert_eq!(loaded.provenance(), ["part.gltf"]);
}

#[cfg(feature = "resources")]
#[test]
fn embedded_external_assets_resolve_packaged_file_names() {
    let child = br#"{"asset":{"version":"2.1"},"buffers":[{"byteLength":36,"uri":"mesh.bin"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut package = child.to_vec();
    package.extend_from_slice(&[0; 36]);
    let root = format!(
        r#"{{"asset":{{"version":"2.1"}},"buffers":[{{"byteLength":{},"uri":"package.bin"}}],"bufferViews":[{{"buffer":0,"byteLength":{}}},{{"buffer":0,"byteOffset":{},"byteLength":36}}],"files":[{{"name":"part.gltf","bufferView":0,"mimeType":"model/gltf+json"}},{{"name":"mesh.bin","bufferView":1,"mimeType":"application/octet-stream"}}],"externalAssets":[{{"file":0}}]}}"#,
        package.len(),
        child.len(),
        child.len(),
    );
    let resolver = |uri: &str| match uri {
        "package.bin" => Ok(package.clone()),
        _ => Err(draco_io::GltfError::ExternalResourceDenied(uri.into())),
    };
    let root = crate::parse_with_options(
        root.as_bytes(),
        None,
        Some(&resolver),
        &draco_io::ResourceLimits::default(),
        ValidationProfile::Gltf21Draft,
        &crate::ExtensionRegistry::default(),
    )
    .unwrap();
    let child = root
        .load_external_asset(
            crate::ExternalAssetIndex(0),
            &resolver,
            &draco_io::ResourceLimits::default(),
            ValidationProfile::Gltf21Draft,
            &crate::ExtensionRegistry::default(),
        )
        .unwrap();
    assert_eq!(child.resources.buffers, vec![vec![0; 36]]);
}

#[test]
fn validation_rejects_dangling_core_references_and_draft_types_in_20() {
    let dangling = Document::from_json_bytes(
        br#"{"asset":{"version":"2.0"},"meshes":[{"primitives":[{"attributes":{"POSITION":1}}]}]}"#,
    )
    .unwrap();
    assert!(dangling.validate(ValidationProfile::Gltf20).is_err());

    let draft_component = Document::from_json_bytes(
        br#"{"asset":{"version":"2.0"},"accessors":[{"componentType":5130,"count":0,"type":"SCALAR"}]}"#,
    )
    .unwrap();
    assert!(draft_component.validate(ValidationProfile::Gltf20).is_err());
    assert!(draft_component
        .validate(ValidationProfile::Gltf21Draft)
        .is_ok());

    let draft_components = Document::from_json_bytes(
        br#"{"asset":{"version":"2.1"},"accessors":[{"componentType":5124,"count":0,"type":"SCALAR"},{"componentType":5130,"count":0,"type":"SCALAR"},{"componentType":5131,"count":0,"type":"SCALAR"},{"componentType":5134,"count":0,"type":"SCALAR"},{"componentType":5135,"count":0,"type":"SCALAR"}]}"#,
    )
    .unwrap();
    assert!(draft_components
        .validate(ValidationProfile::Gltf21Draft)
        .is_ok());
}

#[test]
fn draft_validation_enforces_file_wide_uids() {
    let valid = Document::from_json_bytes(
        br#"{"asset":{"version":"2.1"},"nodes":[{"name":"visible","uid":"node-a"},{"uid":"node-b"}]}"#,
    )
    .unwrap();
    valid.validate(ValidationProfile::Gltf21Draft).unwrap();

    let duplicate = Document::from_json_bytes(
        br#"{"asset":{"version":"2.1"},"nodes":[{"uid":"node-a"}],"meshes":[{"uid":"node-a"}]}"#,
    )
    .unwrap();
    assert!(duplicate.validate(ValidationProfile::Gltf21Draft).is_err());

    let name_conflict = Document::from_json_bytes(
        br#"{"asset":{"version":"2.1"},"nodes":[{"name":"part-a"}],"meshes":[{"uid":"part-a"}]}"#,
    )
    .unwrap();
    assert!(name_conflict
        .validate(ValidationProfile::Gltf21Draft)
        .is_err());
}

#[test]
fn typed_views_reference_the_lossless_document() {
    let document = Document::from_json_bytes(
        br#"{"asset":{"version":"2.1"},"buffers":[{"byteLength":12,"uri":"mesh.bin"}],"bufferViews":[{"buffer":0,"byteOffset":4,"byteLength":8}],"accessors":[{"bufferView":0,"componentType":5126,"count":2,"type":"VEC3"}],"images":[{"uri":"albedo.png"}],"textures":[{"source":0}],"materials":[{}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":0,"material":0,"targets":[{"POSITION":0}]}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0,"files":[{"uri":"child.gltf","mimeType":"model/gltf+json"}]}"#,
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
    let primitive = document.primitive(crate::MeshIndex(0), 0).unwrap();
    assert_eq!(primitive.indices(), Some(crate::AccessorIndex(0)));
    assert_eq!(primitive.material(), Some(crate::MaterialIndex(0)));
    assert_eq!(
        primitive.attribute_indices().collect::<Vec<_>>(),
        [("POSITION", crate::AccessorIndex(0))]
    );
    assert_eq!(primitive.morph_targets().count(), 1);
    assert_eq!(document.default_scene(), Some(crate::SceneIndex(0)));
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
        br#"{"asset":{"version":"2.0"},"accessors":[{"componentType":5126,"count":0,"type":"SCALAR"}],"nodes":[{}],"scenes":[{"nodes":[0]}],"scene":0,"skins":[{"joints":[0],"inverseBindMatrices":0}],"animations":[{"samplers":[{"input":0,"output":0}],"channels":[{"sampler":0,"target":{"node":0,"path":"translation"}}]}]}"#,
    )
    .unwrap();
    document.validate(ValidationProfile::Gltf20).unwrap();

    let mut invalid = document.clone();
    invalid.as_value_mut()["animations"][0]["channels"][0]["sampler"] = 1u64.into();
    assert!(invalid.validate(ValidationProfile::Gltf20).is_err());
}

#[test]
fn explicit_asset_loading_tracks_provenance_and_rejects_cycles() {
    let root = parse(
        br#"{"asset":{"version":"2.1"},"files":[{"uri":"child.gltf","mimeType":"model/gltf+json"}]}"#,
        ValidationProfile::Gltf21Draft,
    )
    .unwrap();
    let resolver = |uri: &str| {
        match uri {
        "child.gltf" => {
            Ok(br#"{"asset":{"version":"2.1"},"files":[{"uri":"root.gltf","mimeType":"model/gltf+json"}]}"#.to_vec())
        }
        "root.gltf" => {
            Ok(br#"{"asset":{"version":"2.1"},"files":[{"uri":"child.gltf","mimeType":"model/gltf+json"}]}"#.to_vec())
        }
        _ => Err(draco_io::GltfError::ExternalResourceDenied(uri.into())),
    }
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
fn explicit_asset_loading_accepts_embedded_file_buffer_view() {
    let root = parse(
        br#"{"asset":{"version":"2.1"},"buffers":[{"byteLength":27,"uri":"data:application/octet-stream;base64,eyJhc3NldCI6eyJ2ZXJzaW9uIjoiMi4xIn19"}],"bufferViews":[{"buffer":0,"byteLength":27}],"files":[{"bufferView":0,"mimeType":"model/gltf+json"}]}"#,
        ValidationProfile::Gltf21Draft,
    )
    .unwrap();
    let resolver = |_uri: &str| -> Result<Vec<u8>, draco_io::GltfError> {
        Err(draco_io::GltfError::ExternalResourceDenied("unused".into()))
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
    assert_eq!(
        child.document.as_value()["asset"]["version"].as_str(),
        Some("2.1")
    );
}

#[cfg(feature = "resources")]
#[test]
fn explicit_asset_loading_honors_chain_depth_limit() {
    let root = parse(
        br#"{"asset":{"version":"2.1"},"files":[{"uri":"child.gltf","mimeType":"model/gltf+json"}]}"#,
        ValidationProfile::Gltf21Draft,
    )
    .unwrap();
    let resolver = |uri: &str| {
        match uri {
        "child.gltf" => {
            Ok(br#"{"asset":{"version":"2.1"},"files":[{"uri":"leaf.gltf","mimeType":"model/gltf+json"}]}"#.to_vec())
        }
        "leaf.gltf" => Ok(br#"{"asset":{"version":"2.1"}}"#.to_vec()),
        _ => Err(draco_io::GltfError::ExternalResourceDenied(uri.into())),
    }
    };
    let limits = draco_io::ResourceLimits {
        max_external_asset_depth: Some(1),
        ..draco_io::ResourceLimits::default()
    };
    let child = root
        .load_asset(
            crate::FileIndex(0),
            &resolver,
            &limits,
            ValidationProfile::Gltf21Draft,
            &crate::ExtensionRegistry::default(),
        )
        .unwrap();
    assert!(child
        .load_asset(
            crate::FileIndex(0),
            &resolver,
            &limits,
            ValidationProfile::Gltf21Draft,
            &crate::ExtensionRegistry::default(),
        )
        .is_err());
}

#[test]
fn draft_validation_checks_file_reference_form() {
    let both = Document::from_json_bytes(
        br#"{"asset":{"version":"2.1"},"files":[{"uri":"child.gltf","bufferView":0,"mimeType":"model/gltf+json"}],"bufferViews":[{"buffer":0}],"buffers":[{"byteLength":0}]}"#,
    )
    .unwrap();
    assert!(both.validate(ValidationProfile::Gltf21Draft).is_err());
    let neither = Document::from_json_bytes(
        br#"{"asset":{"version":"2.1"},"files":[{"mimeType":"model/gltf+json"}]}"#,
    )
    .unwrap();
    assert!(neither.validate(ValidationProfile::Gltf21Draft).is_err());
    let missing_mime =
        Document::from_json_bytes(br#"{"asset":{"version":"2.1"},"files":[{"uri":"child.gltf"}]}"#)
            .unwrap();
    assert!(missing_mime
        .validate(ValidationProfile::Gltf21Draft)
        .is_err());
}

#[test]
fn import_reads_json() {
    let import = parse(
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
fn compression_appends_a_decodable_draco_payload() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();
    let primitive = import.draco_primitives().next().unwrap();
    assert_eq!(import.decode_primitive(primitive).unwrap().num_faces(), 1);
    import.document.validate(ValidationProfile::Gltf20).unwrap();
    assert!(import.document.as_value()["extensionsRequired"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value.as_str() == Some(crate::KHR_DRACO_MESH_COMPRESSION)));
    let accessor = import
        .document
        .primitive(crate::MeshIndex(0), 0)
        .unwrap()
        .attributes()
        .unwrap()[0]
        .1
        .as_u64()
        .unwrap() as usize;
    assert!(import
        .document
        .accessor(crate::AccessorIndex(accessor))
        .unwrap()
        .buffer_view()
        .is_none());
}

#[test]
fn fallback_compression_keeps_raw_geometry_without_requiring_draco() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(
            crate::MeshIndex(0),
            0,
            crate::CompressionOptions {
                mode: crate::CompressionMode::Fallback,
                ..crate::CompressionOptions::default()
            },
        )
        .unwrap();
    import.document.validate(ValidationProfile::Gltf20).unwrap();
    assert!(import
        .document
        .as_value()
        .get("extensionsRequired")
        .is_none());
    assert_eq!(
        import
            .document
            .accessor(crate::AccessorIndex(0))
            .unwrap()
            .buffer_view(),
        Some(crate::BufferViewIndex(0))
    );
}

#[test]
fn draco_only_preserves_shared_raw_accessor_for_another_primitive() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]},{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();
    import.document.validate(ValidationProfile::Gltf20).unwrap();
    assert_eq!(
        import
            .document
            .primitive(crate::MeshIndex(1), 0)
            .unwrap()
            .attributes()
            .unwrap()[0]
            .1
            .as_u64(),
        Some(0)
    );
    assert!(import
        .document
        .accessor(crate::AccessorIndex(0))
        .unwrap()
        .buffer_view()
        .is_some());
    assert_eq!(import.resources.buffers.len(), 1);
    assert_eq!(import.document.buffer_views().len(), 2);
}

#[test]
fn draco_only_preserves_unknown_json_index_like_fields() {
    let input = br#"{"asset":{"version":"2.0"},"custom":{"bufferView":987,"attributes":{"POSITION":123}},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();
    assert_eq!(
        import.document.as_value()["custom"]["bufferView"].as_u64(),
        Some(987)
    );
    assert_eq!(
        import.document.as_value()["custom"]["attributes"]["POSITION"].as_u64(),
        Some(123)
    );
}

#[test]
fn compression_rejects_non_triangle_khr_mode() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"mode":0,"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
    assert!(import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .is_err());
}

#[test]
fn strict_validation_rejects_invalid_draco_attribute_ids() {
    let document = Document::from_json_bytes(br#"{"asset":{"version":"2.0"},"extensionsUsed":["KHR_draco_mesh_compression"],"buffers":[{"byteLength":0}],"bufferViews":[{"buffer":0,"byteLength":0}],"accessors":[{"componentType":5126,"count":0,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"extensions":{"KHR_draco_mesh_compression":{"bufferView":0,"attributes":{"POSITION":"bad"}}}}]}]}"#).unwrap();
    assert!(document.validate(ValidationProfile::Gltf20).is_err());
}

#[test]
fn transform_rejects_unregistered_primitive_extensions() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"extensions":{"VENDOR_binary_layout":{}}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
    let error = import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap_err();
    assert!(error.to_string().contains("transform-safe"));
}

#[test]
fn compression_roundtrips_through_decompression() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
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
fn decompression_detaches_shared_accessors() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]},{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();
    import.decompress_in_place().unwrap();

    let decoded_accessor = import
        .document
        .primitive(crate::MeshIndex(0), 0)
        .unwrap()
        .attributes()
        .unwrap()
        .iter()
        .find(|(name, _)| name == "POSITION")
        .unwrap()
        .1
        .as_u64()
        .unwrap();
    assert_ne!(decoded_accessor, 0);
    assert_eq!(
        import
            .document
            .primitive(crate::MeshIndex(1), 0)
            .unwrap()
            .attributes()
            .unwrap()[0]
            .1
            .as_u64(),
        Some(0)
    );
    assert_eq!(
        import
            .document
            .accessor(crate::AccessorIndex(0))
            .unwrap()
            .buffer_view(),
        Some(crate::BufferViewIndex(0))
    );
}

#[test]
fn glb_serialization_consolidates_append_only_draco_buffers() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();
    let bytes = import.to_bytes(crate::OutputFormat::GlbV2).unwrap();
    let reloaded = parse(&bytes, ValidationProfile::Gltf20).unwrap();
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
fn gltf_output_includes_appended_draco_buffer() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();

    let output = import.to_gltf_output().unwrap();
    assert_eq!(output.resources.len(), 1);
    assert_eq!(output.resources[0].uri, "buffer-0.bin");
    assert!(!output.resources[0].bytes.is_empty());
    let reloaded = crate::parse_with_options(
        &output.json,
        None,
        Some(&|uri: &str| {
            output
                .resources
                .iter()
                .find(|resource| resource.uri == uri)
                .map(|resource| resource.bytes.clone())
                .ok_or_else(|| draco_io::GltfError::ExternalResourceDenied(uri.into()))
        }),
        &draco_io::ResourceLimits::default(),
        ValidationProfile::Gltf20,
        &crate::ExtensionRegistry::default(),
    )
    .unwrap();
    assert_eq!(reloaded.draco_primitives().count(), 1);
}

#[test]
fn decompression_failure_is_atomic() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();
    let before = import.document.to_json_bytes().unwrap();
    import.resources.buffers[0][0] ^= 0xff;
    assert!(import.decompress_in_place().is_err());
    assert_eq!(import.document.to_json_bytes().unwrap(), before);
    assert_eq!(import.draco_primitives().count(), 1);
}

#[cfg(feature = "compact")]
#[test]
fn compact_facade_uses_lossless_document() {
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

#[cfg(feature = "compact")]
#[test]
fn compact_runtime_packs_accessor_geometry() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let import = parse(input, ValidationProfile::Gltf20).unwrap();

    let primitive = import
        .decode_packed_primitive(crate::MeshIndex(0), 0)
        .unwrap();
    assert_eq!(primitive.mode, 4);
    assert!(primitive.indices.is_none());
    assert_eq!(primitive.attributes.len(), 1);
    assert_eq!(primitive.attributes[0].semantic, "POSITION");
    assert_eq!(primitive.attributes[0].component_type, 5126);
    assert_eq!(primitive.attributes[0].components, 3);
    assert_eq!(primitive.attributes[0].bytes.len(), 36);
}

#[cfg(feature = "compact")]
#[test]
fn compact_runtime_preserves_draft_half_float_accessors() {
    let input = br#"{"asset":{"version":"2.1"},"buffers":[{"byteLength":4,"uri":"mesh.bin"}],"bufferViews":[{"buffer":0,"byteLength":4}],"accessors":[{"bufferView":0,"componentType":5131,"count":2,"type":"SCALAR"}],"meshes":[{"primitives":[{"attributes":{"_HALF":0}}]}]}"#;
    let resolver = |uri: &str| match uri {
        "mesh.bin" => Ok(vec![0, 60, 0, 64]),
        _ => Err(draco_io::GltfError::ExternalResourceDenied(uri.into())),
    };
    let import = crate::parse_with_options(
        input,
        None,
        Some(&resolver),
        &draco_io::ResourceLimits::default(),
        ValidationProfile::Gltf21Draft,
        &crate::ExtensionRegistry::default(),
    )
    .unwrap();

    let primitive = import
        .decode_packed_primitive(crate::MeshIndex(0), 0)
        .unwrap();
    let half = &primitive.attributes[0];
    assert_eq!(half.component_type, 5131);
    assert_eq!(half.bytes, [0, 60, 0, 64]);
}

#[cfg(feature = "compact")]
#[test]
fn compact_runtime_materializes_sparse_accessors() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":26,"uri":"mesh.bin"}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":2},{"buffer":0,"byteOffset":2,"byteLength":24}],"accessors":[{"componentType":5126,"count":3,"type":"VEC3","sparse":{"count":2,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":1}}}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut buffer = vec![0, 2];
    for value in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0] {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
    let resolver = move |uri: &str| {
        if uri == "mesh.bin" {
            Ok(buffer.clone())
        } else {
            Err(draco_io::GltfError::ExternalResourceDenied(uri.into()))
        }
    };
    let mut import = crate::parse_with_options(
        input,
        None,
        Some(&resolver),
        &draco_io::ResourceLimits::default(),
        ValidationProfile::Gltf20,
        &crate::ExtensionRegistry::default(),
    )
    .unwrap();

    let primitive = import
        .decode_packed_primitive(crate::MeshIndex(0), 0)
        .unwrap();
    let position = &primitive.attributes[0];
    assert_eq!(position.bytes.len(), 36);
    assert_eq!(
        &position.bytes[..12],
        &[0, 0, 128, 63, 0, 0, 0, 64, 0, 0, 64, 64]
    );
    assert_eq!(&position.bytes[12..24], &[0; 12]);
    assert_eq!(
        &position.bytes[24..],
        &[0, 0, 128, 64, 0, 0, 160, 64, 0, 0, 192, 64]
    );
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();
    import.decompress_in_place().unwrap();
    assert_eq!(
        import
            .decode_geometry_primitive(import.document.primitive(crate::MeshIndex(0), 0).unwrap())
            .unwrap()
            .0
            .num_points(),
        3
    );
}

#[cfg(feature = "compact")]
#[test]
fn compact_runtime_packs_draco_geometry() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();

    let primitive = import
        .decode_packed_primitive(crate::MeshIndex(0), 0)
        .unwrap();
    assert_eq!(primitive.mode, 4);
    assert_eq!(primitive.indices.unwrap().bytes.len(), 12);
    assert_eq!(primitive.attributes.len(), 1);
    assert_eq!(primitive.attributes[0].semantic, "POSITION");
    assert_eq!(primitive.attributes[0].components, 3);
    assert_eq!(primitive.attributes[0].bytes.len(), 36);
}
