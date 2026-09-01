use crate::{parse, Document, ValidationProfile};
#[cfg(feature = "draco-encode")]
use crate::{ExtensionHandler, ExtensionRegistry};

#[cfg(feature = "draco-encode")]
struct VendorBinaryLayout;

#[cfg(feature = "draco-encode")]
impl ExtensionHandler for VendorBinaryLayout {
    fn name(&self) -> &'static str {
        "VENDOR_binary_layout"
    }
    fn allows_binary_transform(&self) -> bool {
        true
    }
    fn collect_binary_references(
        &self,
        document: &Document,
        _accessors: &mut [bool],
        buffer_views: &mut [bool],
    ) -> crate::Result<()> {
        let index = document.as_value()["extensions"][self.name()]["bufferView"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|index| *index < buffer_views.len())
            .ok_or_else(|| crate::Error::Extension("vendor bufferView is invalid".into()))?;
        buffer_views[index] = true;
        Ok(())
    }
    fn remap_binary_references(
        &self,
        document: &mut Document,
        _accessors: &[Option<usize>],
        buffer_views: &[Option<usize>],
    ) -> crate::Result<()> {
        let value = &mut document.as_value_mut()["extensions"][self.name()]["bufferView"];
        let index = value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .and_then(|index| buffer_views.get(index).and_then(|value| *value))
            .ok_or_else(|| crate::Error::Extension("vendor bufferView was removed".into()))?;
        *value = crate::JsonValue::from(index);
        Ok(())
    }
}

#[test]
fn document_preserves_untouched_json() {
    let bytes = br#"{"asset":{"version":"2.1"},"unknown":{"a":[1,2,3]}}"#;
    let document = Document::from_json_bytes(bytes).unwrap();
    document.validate(ValidationProfile::Gltf21Draft).unwrap();
    assert_eq!(document.to_json_bytes().unwrap(), bytes);
}

#[cfg(not(feature = "strict-validation"))]
#[test]
fn basic_validation_defers_scene_graph_checks() {
    let invalid = Document::from_json_bytes(
        br#"{"asset":{"version":"2.0"},"meshes":[{"primitives":[{"attributes":{"POSITION":1}}]}]}"#,
    )
    .unwrap();
    invalid.validate(ValidationProfile::Gltf20).unwrap();
    parse(
        invalid.to_json_bytes().unwrap().as_slice(),
        ValidationProfile::Gltf20,
    )
    .unwrap();
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
    let document = Document::from_json_bytes(br#"{"asset":{"version":"2.1"},"files":[{"uri":"child.gltf","mimeType":"model/gltf+json"}],"shapes":[{"type":"box","box":{"size":[2,3,4]}}],"accessors":[{"componentType":5126,"count":0,"type":"VEC3","min":[0,0,0],"max":[0,0,0]},{"componentType":5126,"count":0,"type":"VEC2"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_4":1}}]}]}"#).unwrap();
    document.validate(ValidationProfile::Gltf21Draft).unwrap();
}

#[cfg(feature = "strict-validation")]
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
    let child = br#"{"asset":{"version":"2.1"},"buffers":[{"byteLength":36,"uri":"mesh.bin"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
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
        &draco_core::DecodeLimits::default(),
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

#[cfg(feature = "strict-validation")]
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

#[cfg(feature = "strict-validation")]
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
        br#"{"asset":{"version":"2.1"},"buffers":[{"byteLength":12,"uri":"mesh.bin"}],"bufferViews":[{"buffer":0,"byteOffset":4,"byteLength":8}],"accessors":[{"bufferView":0,"componentType":5126,"count":2,"type":"VEC3","min":[0,0,0],"max":[0,0,0]}],"images":[{"uri":"albedo.png"}],"textures":[{"source":0}],"materials":[{}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":0,"material":0,"targets":[{"POSITION":0}]}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0,"files":[{"uri":"child.gltf","mimeType":"model/gltf+json"}]}"#,
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
    assert!(document.to_json_bytes().unwrap().starts_with(b"{\"asset\""));
}

#[cfg(feature = "strict-validation")]
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

/// `KHR_animation_pointer` targets a JSON pointer instead of a node, through a
/// channel path core glTF does not define. Rejecting the document over it would
/// throw away the geometry for the sake of an animation the reader was free to
/// skip - and it did, for every file in the corpus that uses the extension.
#[cfg(feature = "strict-validation")]
#[test]
fn validation_accepts_the_animation_pointer_channel_path() {
    let document = Document::from_json_bytes(
        br#"{"asset":{"version":"2.0"},"extensionsUsed":["KHR_animation_pointer"],"accessors":[{"componentType":5126,"count":0,"type":"SCALAR"}],"nodes":[{}],"animations":[{"samplers":[{"input":0,"output":0}],"channels":[{"sampler":0,"target":{"path":"pointer","extensions":{"KHR_animation_pointer":{"pointer":"/materials/0/pbrMetallicRoughness/baseColorFactor"}}}}]}]}"#,
    )
    .unwrap();
    document.validate(ValidationProfile::Gltf20).unwrap();

    // Only with the extension on the target, though: `pointer` on its own says
    // where to write nothing.
    let mut bare = document.clone();
    bare.as_value_mut()["animations"][0]["channels"][0]["target"]
        .as_object_mut()
        .unwrap()
        .retain(|(key, _)| key != "extensions");
    assert!(bare.validate(ValidationProfile::Gltf20).is_err());
}

#[cfg(feature = "strict-validation")]
#[test]
fn validation_requires_finite_ordered_position_bounds() {
    for accessor in [
        r#"{"componentType":5126,"count":1,"type":"VEC3"}"#,
        r#"{"componentType":5126,"count":1,"type":"VEC3","min":[0,0],"max":[1,1,1]}"#,
        r#"{"componentType":5126,"count":1,"type":"VEC3","min":[0,0,0],"max":[1,1,1e9999]}"#,
        r#"{"componentType":5126,"count":1,"type":"VEC3","min":[2,0,0],"max":[1,1,1]}"#,
    ] {
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"accessors":[{accessor}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}]}}"#
        );
        let document = Document::from_json_bytes(json.as_bytes()).unwrap();
        assert!(document.validate(ValidationProfile::Gltf20).is_err());
    }

    let valid = Document::from_json_bytes(
        br#"{"asset":{"version":"2.0"},"accessors":[{"componentType":5126,"count":1,"type":"VEC3","min":[-1,0,0],"max":[1,2,3]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#,
    )
    .unwrap();
    valid.validate(ValidationProfile::Gltf20).unwrap();
}

#[cfg(feature = "strict-validation")]
#[test]
fn validation_requires_node_hierarchy_to_be_disjoint_trees() {
    for json in [
        br#"{"asset":{"version":"2.0"},"nodes":[{"children":[2]},{"children":[2]},{}]}"#.as_slice(),
        br#"{"asset":{"version":"2.0"},"nodes":[{"children":[1]},{"children":[0]}]}"#.as_slice(),
        br#"{"asset":{"version":"2.0"},"nodes":[{"children":[1]},{}],"scenes":[{"nodes":[1]}]}"#
            .as_slice(),
    ] {
        let document = Document::from_json_bytes(json).unwrap();
        assert!(document.validate(ValidationProfile::Gltf20).is_err());
    }

    let valid = Document::from_json_bytes(
        br#"{"asset":{"version":"2.0"},"nodes":[{"children":[1,2]},{"mesh":0},{"mesh":0}],"meshes":[{"primitives":[]}],"scenes":[{"nodes":[0]}]}"#,
    )
    .unwrap();
    valid.validate(ValidationProfile::Gltf20).unwrap();
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

#[cfg(feature = "strict-validation")]
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

#[cfg(feature = "draco-encode")]
mod compression_tests {
    use super::*;

    /// The accessor decides how decoded Draco integers are read.
    ///
    /// KHR_draco_mesh_compression makes the glTF accessor authoritative. A
    /// Draco attribute carries a normalization flag of its own, and third-party
    /// encoders leave it unset even for a normalized COLOR_0 — reading that
    /// flag handed consumers raw 0..65535 values, which saturate to white.
    ///
    /// Built here rather than round-tripped: this crate's own encoder writes
    /// the flag into the payload, so an encode/decode cycle cannot produce the
    /// disagreement that the files in the wild have.
    #[cfg(feature = "draco-decode")]
    #[test]
    fn draco_reading_takes_normalization_from_the_accessor() {
        use draco_core::{DataType, GeometryAttributeType, Mesh, PointAttribute};

        let mut mesh = Mesh::new();
        mesh.set_num_points(3);
        mesh.set_num_faces(1);
        mesh.set_face(
            draco_core::FaceIndex(0),
            [
                draco_core::PointIndex(0),
                draco_core::PointIndex(1),
                draco_core::PointIndex(2),
            ],
        );

        let mut position = PointAttribute::new();
        position.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            3,
        );
        let position_id = mesh.add_attribute(position);

        // As a foreign encoder leaves it: the payload says not normalized.
        let mut color = PointAttribute::new();
        color.init(GeometryAttributeType::Color, 4, DataType::Uint16, false, 3);
        let color_id = mesh.add_attribute(color);

        let unique_id = |id: i32| mesh.attribute(id).unique_id();
        let contract = vec![
            ("POSITION".to_owned(), unique_id(position_id)),
            ("COLOR_0".to_owned(), unique_id(color_id)),
        ];
        let normalized = [("COLOR_0".to_owned(), true)].into_iter().collect();

        let geometry =
            crate::PackedGeometry::from_draco_mesh(&mesh, &contract, &normalized).unwrap();

        let flag = |semantic: &str| {
            geometry
                .attributes()
                .iter()
                .find(|attribute| attribute.semantic() == semantic)
                .map(crate::PackedAttribute::normalized)
        };
        assert_eq!(flag("COLOR_0"), Some(true), "the accessor flag must win");
        assert_eq!(flag("POSITION"), Some(false), "and must not invent one");
    }

    #[test]
    fn compression_appends_a_decodable_draco_payload() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
        let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
        let report = import
            .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
            .unwrap();
        assert_eq!(report.mode, crate::CompressionMode::DracoOnly);
        assert_eq!(report.compressed_primitives, 1);
        assert!(report.encoded_bytes > 0);
        assert!(report.output_bytes >= report.encoded_bytes);
        assert_eq!(
            report.reclaimed_bytes,
            36usize.saturating_sub(report.output_bytes)
        );
        let primitive = import.draco_primitives().next().unwrap();
        assert_eq!(
            import
                .decode_draco_primitive(primitive)
                .unwrap()
                .num_faces(),
            1
        );
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
        let accessor = import
            .document
            .accessor(crate::AccessorIndex(accessor))
            .unwrap();
        assert!(accessor.buffer_view().is_none());
        assert_eq!(accessor.count(), Some(3));
        assert_eq!(accessor.value()["min"].as_array().unwrap().len(), 3);
        assert_eq!(accessor.value()["max"].as_array().unwrap().len(), 3);
    }

    /// Four points forming a unit-square strip: (0,0,0)-(1,0,0)-(0,1,0)-(1,1,0),
    /// mode 5, no index accessor. Draco has no notion of a strip, so
    /// compression has to unwind it into the two triangles the strip actually
    /// draws -- (0,1,2) and (2,1,3) by glTF's own strip convention -- before
    /// encoding, and the output primitive has to stop claiming `mode: 5`
    /// afterward: nothing about the Draco stream is a strip any more.
    #[test]
    fn compression_unwinds_a_triangle_strip_before_encoding() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":48,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAACAPwAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":48}],"accessors":[{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"mode":5,"attributes":{"POSITION":0}}]}]}"#;
        let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
        import
            .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
            .unwrap();

        let primitive = import.document.primitive(crate::MeshIndex(0), 0).unwrap();
        assert_eq!(
            primitive.mode(),
            4,
            "a strip has nothing left to mean once Draco has flattened it"
        );

        let triangles = decode_triangle_vertex_sets(
            &import
                .read_primitive(crate::PrimitiveIndex::new(crate::MeshIndex(0), 0))
                .unwrap(),
        );
        assert_triangle_sets_match(
            &triangles,
            &[
                [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            ],
        );
    }

    /// The same four points, mode 6: a fan roots every triangle at vertex 0,
    /// so it draws (0,1,2) and (0,2,3) instead of the strip's pairing --
    /// different triangles from the same vertices, which is exactly what
    /// distinguishes a fan from a strip and worth pinning separately.
    #[test]
    fn compression_unwinds_a_triangle_fan_before_encoding() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":48,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAACAPwAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":48}],"accessors":[{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"mode":6,"attributes":{"POSITION":0}}]}]}"#;
        let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
        import
            .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
            .unwrap();

        let primitive = import.document.primitive(crate::MeshIndex(0), 0).unwrap();
        assert_eq!(primitive.mode(), 4);

        let triangles = decode_triangle_vertex_sets(
            &import
                .read_primitive(crate::PrimitiveIndex::new(crate::MeshIndex(0), 0))
                .unwrap(),
        );
        assert_triangle_sets_match(
            &triangles,
            &[
                [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
            ],
        );
    }

    /// Decodes a compressed primitive's positions and index buffer into one
    /// `[position; 3]` per triangle, with each coordinate snapped to the
    /// nearest of 0.0/1.0 -- the two extremes every fixture in this file
    /// uses -- so a comparison doesn't have to guess how close Draco's
    /// default quantization keeps them.
    fn decode_triangle_vertex_sets(geometry: &crate::PackedGeometry) -> Vec<[[f32; 3]; 3]> {
        let snap = |value: f32| if value > 0.5 { 1.0 } else { 0.0 };
        let positions: Vec<[f32; 3]> = geometry.attributes()[0]
            .bytes()
            .as_chunks::<12>()
            .0
            .iter()
            .map(|point| {
                std::array::from_fn(|component| {
                    let bytes = point[component * 4..component * 4 + 4].try_into().unwrap();
                    snap(f32::from_le_bytes(bytes))
                })
            })
            .collect();
        let indices: Vec<u32> = geometry
            .indices()
            .unwrap()
            .bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| u32::from_le_bytes(*chunk))
            .collect();
        indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|face| face.map(|index| positions[index as usize]))
            .collect()
    }

    /// Compares two lists of triangles up to face order and up to each
    /// triangle's own corner order -- both of which Draco's connectivity
    /// encoding is free to change without changing the geometry.
    fn assert_triangle_sets_match(actual: &[[[f32; 3]; 3]], expected: &[[[f32; 3]; 3]]) {
        let normalize = |triangle: &[[f32; 3]; 3]| {
            let mut corners = *triangle;
            corners.sort_by(|a, b| a.partial_cmp(b).unwrap());
            corners
        };
        let mut actual: Vec<_> = actual.iter().map(normalize).collect();
        let mut expected: Vec<_> = expected.iter().map(normalize).collect();
        actual.sort_by(|a, b| a.partial_cmp(b).unwrap());
        expected.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(actual, expected);
    }

    /// The caller's Draco ceilings travel from `ImportOptions` through the
    /// import into every decode, and the failure reaches the caller with its
    /// kind intact: `LimitExceeded` is the caller's own policy refusing a
    /// large file, and a generic error would make it indistinguishable from
    /// the decoder refusing a malformed one -- the exact collapse
    /// `draco-core` once had across twelve re-wrapped errors.
    #[cfg(all(feature = "draco-encode", feature = "draco-decode"))]
    #[test]
    fn draco_decode_limits_refuse_a_primitive_over_the_ceiling_and_keep_the_kind() {
        use draco_core::status::ErrorKind;
        use draco_core::DecodeLimits;

        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
        let mut import = crate::parse(input, ValidationProfile::Gltf20).unwrap();
        import
            .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
            .unwrap();
        // A self-contained GLB, so the re-import reads the very buffer the
        // encoder wrote.
        let glb = import.to_bytes(crate::OutputFormat::GlbV2).unwrap();

        // A ceiling of zero faces refuses the one-face primitive outright. The
        // points ceiling is the wrong knob here: a mesh decode checks faces
        // and decoded bytes -- its point count comes out of the connectivity,
        // not the header -- and this fixture is the smallest mesh there is.
        let options = crate::ImportOptions {
            draco_decode_limits: DecodeLimits::default().with_max_faces(0),
            ..crate::ImportOptions::default()
        };
        let import = crate::import_slice_with_options(&glb, &options).unwrap();
        let primitive = import.draco_primitives().next().unwrap();
        let error = import
            .decode_draco_primitive(primitive)
            .expect_err("a decoded face over a ceiling of zero");
        assert!(
            error.is_decode_limit_exceeded(),
            "a caller should be able to ask without matching two levels deep: {error:?}",
        );
        match error {
            crate::Error::Decode(error) => {
                assert_eq!(error.kind(), ErrorKind::LimitExceeded);
            }
            other => panic!("the kind collapsed on the way out: {other:?}"),
        }

        // And the question separates the two refusals rather than answering
        // yes to any decode failure: a truncated payload is the file being
        // wrong, which no ceiling of the caller's produced.
        let mut broken = glb.clone();
        let tail = broken.len() - 8;
        broken.truncate(tail);
        if let Ok(import) = crate::import_slice(&broken, None) {
            if let Some(primitive) = import.draco_primitives().next() {
                if let Err(error) = import.decode_draco_primitive(primitive) {
                    assert!(
                        !error.is_decode_limit_exceeded(),
                        "a malformed stream is not the caller's ceiling: {error:?}",
                    );
                }
            }
        }

        // The same document under the defaults decodes: the refusal is the
        // caller's ceiling, not the file.
        let import = crate::import_slice(&glb, None).unwrap();
        let primitive = import.draco_primitives().next().unwrap();
        let decoded = import.decode_draco_primitive(primitive).unwrap();
        assert_eq!(decoded.num_points(), 3);
        assert_eq!(decoded.num_faces(), 1);
    }

    /// One document, two extensions, two answers.
    ///
    /// The safety check is whole-document, so an extension on a *material*
    /// decides whether the *geometry* may be compressed. That is right when
    /// nobody has read the extension's specification and wrong when someone
    /// has, and this pins both halves of the distinction: a registered
    /// binary-free layer compresses, an unknown vendor name still refuses.
    /// Without the second assertion the first one would be satisfied by simply
    /// deleting the check.
    #[cfg(feature = "draco-encode")]
    #[test]
    fn compression_allows_binary_free_extensions_and_still_refuses_unknown_ones() {
        let document = |extension: &str| {
            format!(
                r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}}],"bufferViews":[{{"buffer":0,"byteLength":36}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}}],"materials":[{{"extensions":{{"{extension}":{{}}}}}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"material":0}}]}}],"extensionsUsed":["{extension}"]}}"#
            )
        };
        let compress = |extension: &str| {
            parse(document(extension).as_bytes(), ValidationProfile::Gltf20)
                .unwrap()
                .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        };

        for extension in crate::BINARY_FREE_EXTENSIONS {
            let report = compress(extension)
                .unwrap_or_else(|error| panic!("{extension} must not block compression: {error}"));
            assert_eq!(report.compressed_primitives, 1);
        }

        let error = compress("VENDOR_unheard_of").unwrap_err().to_string();
        assert!(
            error.contains("VENDOR_unheard_of") && error.contains("transform-safe"),
            "an unregistered extension must still refuse by name, got {error}"
        );
    }

    #[cfg(feature = "draco-encode")]
    #[test]
    fn compression_uses_the_encoded_topology_for_accessor_metadata() {
        let positions = [
            0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 9.0, 9.0, 9.0,
        ]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
        let geometry = crate::PackedGeometry::new(
            crate::PrimitiveMode::Triangles,
            vec![crate::PackedAttribute::new(
                "POSITION",
                4,
                3,
                crate::ComponentType::F32,
                false,
                positions,
            )
            .unwrap()],
            Some(
                crate::PackedIndices::new(
                    3,
                    crate::ComponentType::U16,
                    [0u16, 1, 2]
                        .into_iter()
                        .flat_map(u16::to_le_bytes)
                        .collect(),
                )
                .unwrap(),
            ),
        )
        .unwrap();
        let mut import = crate::Import::from_geometry(
            &geometry,
            ValidationProfile::Gltf20,
            crate::GeometryWriteOptions::default(),
        )
        .unwrap();

        import
            .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
            .unwrap();

        let primitive = import.document.primitive(crate::MeshIndex(0), 0).unwrap();
        let position = primitive
            .attribute_indices()
            .find_map(|(semantic, index)| (semantic == "POSITION").then_some(index))
            .unwrap();
        assert_eq!(import.document.accessor(position).unwrap().count(), Some(3));
        let decoded = import.decode_draco_primitive(primitive).unwrap();
        assert_eq!(decoded.num_points(), 3);
        assert_eq!(decoded.num_faces(), 1);

        import.document.as_value_mut()["accessors"][position.0]["count"] = 4u64.into();
        let primitive = import.document.primitive(crate::MeshIndex(0), 0).unwrap();
        assert!(matches!(
            import.decode_draco_primitive(primitive),
            Err(crate::Error::Geometry(
                crate::GeometryError::DracoAccessorCount {
                    decoded: 3,
                    declared: 4,
                    ..
                }
            ))
        ));

        // An accessor that undercounts is what real encoders emit for a mesh
        // with attribute seams, and the decoded stream is self-consistent, so
        // the read must go through with the decoded count.
        import.document.as_value_mut()["accessors"][position.0]["count"] = 2u64.into();
        let primitive = import.document.primitive(crate::MeshIndex(0), 0).unwrap();
        let decoded = import.decode_draco_primitive(primitive).unwrap();
        assert_eq!(decoded.num_points(), 3);
        let geometry = import
            .read_primitive(crate::PrimitiveIndex {
                mesh: crate::MeshIndex(0),
                primitive: 0,
            })
            .unwrap();
        assert_eq!(geometry.vertex_count(), 3);
    }

    #[test]
    fn fallback_compression_keeps_raw_geometry_without_requiring_draco() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
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
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]},{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
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
        let input = br#"{"asset":{"version":"2.0"},"custom":{"bufferView":987,"attributes":{"POSITION":123}},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
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
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"mode":0,"attributes":{"POSITION":0}}]}]}"#;
        let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
        assert!(import
            .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
            .is_err());
    }

    /// A Draco-compressed primitive may legally declare `mode: TRIANGLE_STRIP`
    /// per the extension's own spec text ("must be either TRIANGLES or
    /// TRIANGLE_STRIP"), but decoding one never produces strip-shaped
    /// indices -- Draco's connectivity is always an explicit triangle list --
    /// and no real decoder (three.js's `DRACOLoader` among them) treats the
    /// declared mode as anything but TRIANGLES for a Draco primitive.
    /// `from_draco_mesh` must not repeat the primitive's own claim.
    #[test]
    fn draco_decode_ignores_a_strip_mode_the_source_primitive_declared() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
        let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
        import
            .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
            .unwrap();
        // Overwritten by hand: no real encoder emits this combination (see
        // compression_unwinds_a_triangle_strip_before_encoding), but the
        // extension's own spec text permits it on a file this crate merely
        // reads, and decode must not be fooled by it either.
        import.document.as_value_mut()["meshes"][0]["primitives"][0]["mode"] = 5u64.into();

        let geometry = import
            .read_primitive(crate::PrimitiveIndex::new(crate::MeshIndex(0), 0))
            .unwrap();
        assert_eq!(geometry.mode(), crate::PrimitiveMode::Triangles);
    }

    #[test]
    fn strict_validation_rejects_invalid_draco_attribute_ids() {
        let document = Document::from_json_bytes(br#"{"asset":{"version":"2.0"},"extensionsUsed":["KHR_draco_mesh_compression"],"buffers":[{"byteLength":0}],"bufferViews":[{"buffer":0,"byteLength":0}],"accessors":[{"componentType":5126,"count":0,"type":"VEC3","min":[0,0,0],"max":[0,0,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"extensions":{"KHR_draco_mesh_compression":{"bufferView":0,"attributes":{"POSITION":"bad"}}}}]}]}"#).unwrap();
        assert!(document.validate(ValidationProfile::Gltf20).is_err());
    }

    #[test]
    fn transform_rejects_unregistered_primitive_extensions() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"extensions":{"VENDOR_binary_layout":{}}}]}]}"#;
        let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
        let error = import
            .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
            .unwrap_err();
        assert!(error.to_string().contains("transform-safe"));
    }

    #[test]
    fn registered_extension_remaps_declared_binary_references() {
        let input = br#"{"asset":{"version":"2.0"},"extensions":{"VENDOR_binary_layout":{"bufferView":0}},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
        let mut registry = ExtensionRegistry::default();
        registry.register(VendorBinaryLayout).unwrap();
        let mut import = crate::parse_with_options(
            input,
            None,
            None,
            &draco_io::ResourceLimits::default(),
            &draco_core::DecodeLimits::default(),
            ValidationProfile::Gltf20,
            &registry,
        )
        .unwrap();
        import
            .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
            .unwrap();
        let view = import.document.as_value()["extensions"]["VENDOR_binary_layout"]["bufferView"]
            .as_u64()
            .unwrap() as usize;
        assert!(view < import.document.buffer_views().len());
        assert_eq!(
            import
                .document
                .buffer_view(crate::BufferViewIndex(view))
                .unwrap()
                .buffer(),
            Some(crate::BufferIndex(0))
        );
    }

    #[test]
    fn compression_roundtrips_through_decompression() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
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
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]},{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
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
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
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
                .decode_draco_primitive(reloaded.draco_primitives().next().unwrap())
                .unwrap()
                .num_faces(),
            1
        );
    }

    #[test]
    fn gltf_output_includes_appended_draco_buffer() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
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
            &draco_core::DecodeLimits::default(),
            ValidationProfile::Gltf20,
            &crate::ExtensionRegistry::default(),
        )
        .unwrap();
        assert_eq!(reloaded.draco_primitives().count(), 1);
    }

    #[test]
    fn json_only_output_rejects_materialized_companion_buffers() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
        let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
        import
            .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
            .unwrap();
        let error = import.to_bytes(crate::OutputFormat::GltfJson).unwrap_err();
        assert!(error.to_string().contains("to_gltf_output"));
    }

    #[test]
    fn compression_output_limit_is_atomic() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
        let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
        let before = import.document.to_json_bytes().unwrap();
        let error = import
            .compress_primitive(
                crate::MeshIndex(0),
                0,
                crate::CompressionOptions {
                    max_output_bytes: Some(0),
                    ..crate::CompressionOptions::default()
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("limit"));
        assert_eq!(import.document.to_json_bytes().unwrap(), before);
    }

    /// Forcing overrides what speed alone would have picked, in both
    /// directions: EdgeBreaker at speed 10, where the encoder's own default
    /// always chooses sequential, and sequential well below 10, where it
    /// always chooses EdgeBreaker. `encoding_method` on the report is
    /// `draco-core`'s own 0/1 (sequential/EdgeBreaker), not this option's
    /// 0/1/2 (auto/sequential/EdgeBreaker) convention.
    #[test]
    fn encoding_method_overrides_the_speed_default() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;

        let mut forced_edgebreaker = parse(input, ValidationProfile::Gltf20).unwrap();
        let report = forced_edgebreaker
            .compress_primitive(
                crate::MeshIndex(0),
                0,
                crate::CompressionOptions {
                    encoding_speed: 10,
                    encoding_method: 2,
                    ..crate::CompressionOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            report.encoding_method, 1,
            "speed 10 should not force sequential when EdgeBreaker is requested"
        );

        let mut forced_sequential = parse(input, ValidationProfile::Gltf20).unwrap();
        let report = forced_sequential
            .compress_primitive(
                crate::MeshIndex(0),
                0,
                crate::CompressionOptions {
                    encoding_speed: 4,
                    encoding_method: 1,
                    ..crate::CompressionOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            report.encoding_method, 0,
            "speed 4 should not force EdgeBreaker when sequential is requested"
        );

        // 0 (and the default) still means "auto": unforced, speed 4 is EdgeBreaker.
        let mut auto = parse(input, ValidationProfile::Gltf20).unwrap();
        let report = auto
            .compress_primitive(
                crate::MeshIndex(0),
                0,
                crate::CompressionOptions {
                    encoding_speed: 4,
                    encoding_method: 0,
                    ..crate::CompressionOptions::default()
                },
            )
            .unwrap();
        assert_eq!(report.encoding_method, 1);
    }

    #[test]
    fn draco_only_deduplicates_overlapping_retained_ranges() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"images":[{"bufferView":1,"mimeType":"application/octet-stream"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]},{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
        let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
        let report = import
            .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
            .unwrap();
        assert!(report.output_bytes <= report.encoded_bytes + 39);
        assert_eq!(import.resources.buffers.len(), 1);
        assert_eq!(import.document.buffer_views().len(), 3);
    }

    #[test]
    fn decompression_failure_is_atomic() {
        let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
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
}

#[cfg(feature = "geometry")]
#[test]
fn import_packs_accessor_geometry() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let import = parse(input, ValidationProfile::Gltf20).unwrap();

    let primitive = import
        .read_primitive(crate::PrimitiveIndex::new(crate::MeshIndex(0), 0))
        .unwrap();
    assert_eq!(primitive.mode(), crate::PrimitiveMode::Triangles);
    assert!(primitive.indices().is_none());
    assert_eq!(primitive.attributes().len(), 1);
    assert_eq!(primitive.attributes()[0].semantic(), "POSITION");
    assert_eq!(
        primitive.attributes()[0].component_type(),
        crate::ComponentType::F32
    );
    assert_eq!(primitive.attributes()[0].components(), 3);
    assert_eq!(primitive.attributes()[0].bytes().len(), 36);

    #[cfg(feature = "accessors")]
    {
        let source = crate::DocumentAccessorSource::new(&import.document, &import.resources);
        let accessor = source.read_accessor(0).unwrap();
        assert_eq!(accessor.count, 3);
        assert_eq!(accessor.accessor_type, "VEC3");
        assert_eq!(accessor.components, 3);
        assert_eq!(accessor.component_type, 5126);
        assert!(!accessor.normalized);
        assert_eq!(accessor.bytes.len(), 36);
        assert_eq!(source.read_buffer_view(0).unwrap(), accessor.bytes);
    }
}

#[cfg(feature = "accessors")]
#[test]
fn accessor_materialization_removes_matrix_column_padding() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":12,"uri":"data:application/octet-stream;base64,AQIDAAQFBgAHCAkA"}],"bufferViews":[{"buffer":0,"byteLength":12}],"accessors":[{"bufferView":0,"componentType":5121,"count":1,"type":"MAT3"}]}"#;
    let import = parse(input, ValidationProfile::Gltf20).unwrap();
    let source = crate::DocumentAccessorSource::new(&import.document, &import.resources);

    let accessor = source.read_accessor(0).unwrap();
    assert_eq!(accessor.accessor_type, "MAT3");
    assert_eq!(accessor.components, 9);
    assert_eq!(accessor.bytes, [1, 2, 3, 4, 5, 6, 7, 8, 9]);
    assert_eq!(
        source.read_buffer_view(0).unwrap(),
        [1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9, 0]
    );
}

#[cfg(feature = "geometry")]
#[test]
fn import_preserves_draft_half_float_accessors() {
    let input = br#"{"asset":{"version":"2.1"},"buffers":[{"byteLength":12,"uri":"mesh.bin"}],"bufferViews":[{"buffer":0,"byteLength":12}],"accessors":[{"bufferView":0,"componentType":5131,"count":2,"type":"VEC3","min":[0,0,0],"max":[1,1,1]}],"meshes":[{"primitives":[{"mode":0,"attributes":{"POSITION":0}}]}]}"#;
    let resolver = |uri: &str| match uri {
        "mesh.bin" => Ok(vec![0, 60, 0, 64, 0, 66, 0, 68, 0, 69, 0, 70]),
        _ => Err(draco_io::GltfError::ExternalResourceDenied(uri.into())),
    };
    let import = crate::parse_with_options(
        input,
        None,
        Some(&resolver),
        &draco_io::ResourceLimits::default(),
        &draco_core::DecodeLimits::default(),
        ValidationProfile::Gltf21Draft,
        &crate::ExtensionRegistry::default(),
    )
    .unwrap();

    let primitive = import
        .read_primitive(crate::PrimitiveIndex::new(crate::MeshIndex(0), 0))
        .unwrap();
    let half = &primitive.attributes()[0];
    assert_eq!(half.component_type(), crate::ComponentType::F16);
    assert_eq!(half.bytes(), [0, 60, 0, 64, 0, 66, 0, 68, 0, 69, 0, 70]);
}

#[cfg(feature = "draco-encode")]
#[test]
fn import_materializes_sparse_accessors() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":26,"uri":"mesh.bin"}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":2},{"buffer":0,"byteOffset":2,"byteLength":24}],"accessors":[{"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[3,4,5],"sparse":{"count":2,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":1}}}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
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
        &draco_core::DecodeLimits::default(),
        ValidationProfile::Gltf20,
        &crate::ExtensionRegistry::default(),
    )
    .unwrap();

    let primitive = import
        .read_primitive(crate::PrimitiveIndex::new(crate::MeshIndex(0), 0))
        .unwrap();
    let position = &primitive.attributes()[0];
    assert_eq!(position.bytes().len(), 36);
    assert_eq!(
        &position.bytes()[..12],
        &[0, 0, 128, 63, 0, 0, 0, 64, 0, 0, 64, 64]
    );
    assert_eq!(&position.bytes()[12..24], &[0; 12]);
    assert_eq!(
        &position.bytes()[24..],
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

#[cfg(feature = "draco-encode")]
#[test]
fn import_packs_draco_geometry() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
    let mut import = parse(input, ValidationProfile::Gltf20).unwrap();
    import
        .compress_primitive(crate::MeshIndex(0), 0, crate::CompressionOptions::default())
        .unwrap();

    let primitive = import
        .read_primitive(crate::PrimitiveIndex::new(crate::MeshIndex(0), 0))
        .unwrap();
    assert_eq!(primitive.mode(), crate::PrimitiveMode::Triangles);
    assert_eq!(primitive.indices().unwrap().bytes().len(), 12);
    assert_eq!(primitive.attributes().len(), 1);
    assert_eq!(primitive.attributes()[0].semantic(), "POSITION");
    assert_eq!(primitive.attributes()[0].components(), 3);
    assert_eq!(primitive.attributes()[0].bytes().len(), 36);
}

#[cfg(feature = "write")]
fn packed_triangle(component_type: crate::ComponentType) -> crate::PackedGeometry {
    let bytes = match component_type {
        crate::ComponentType::F32 => [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect(),
        crate::ComponentType::F64 => [0.0f64, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .into_iter()
            .flat_map(f64::to_le_bytes)
            .collect(),
        _ => unreachable!(),
    };
    crate::PackedGeometry::new(
        crate::PrimitiveMode::Triangles,
        vec![crate::PackedAttribute::new("POSITION", 3, 3, component_type, false, bytes).unwrap()],
        Some(
            crate::PackedIndices::new(
                3,
                crate::ComponentType::U16,
                [0u16, 1, 2]
                    .into_iter()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            )
            .unwrap(),
        ),
    )
    .unwrap()
}

#[cfg(feature = "write")]
#[test]
fn packed_geometry_rejects_duplicate_semantics_and_bad_indices() {
    let position = crate::PackedAttribute::new(
        "POSITION",
        1,
        3,
        crate::ComponentType::F32,
        false,
        vec![0; 12],
    )
    .unwrap();
    let duplicate = crate::PackedGeometry::new(
        crate::PrimitiveMode::Points,
        vec![position.clone(), position],
        None,
    )
    .unwrap_err();
    assert!(matches!(
        duplicate,
        crate::GeometryError::DuplicateSemantic(_)
    ));

    let indices =
        crate::PackedIndices::new(1, crate::ComponentType::U16, 4u16.to_le_bytes().to_vec())
            .unwrap();
    let error = crate::PackedGeometry::new(
        crate::PrimitiveMode::Points,
        vec![crate::PackedAttribute::new(
            "POSITION",
            1,
            3,
            crate::ComponentType::F32,
            false,
            vec![0; 12],
        )
        .unwrap()],
        Some(indices),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        crate::GeometryError::IndexOutOfRange { .. }
    ));

    let empty = crate::PackedGeometry::new(
        crate::PrimitiveMode::Points,
        vec![crate::PackedAttribute::new(
            "POSITION",
            0,
            3,
            crate::ComponentType::F32,
            false,
            Vec::new(),
        )
        .unwrap()],
        None,
    )
    .unwrap_err();
    assert_eq!(empty, crate::GeometryError::EmptyGeometry);

    let integer_position = crate::PackedGeometry::new(
        crate::PrimitiveMode::Points,
        vec![crate::PackedAttribute::new(
            "POSITION",
            1,
            3,
            crate::ComponentType::U16,
            false,
            vec![0; 6],
        )
        .unwrap()],
        None,
    )
    .unwrap();
    assert!(matches!(
        integer_position.validate(ValidationProfile::Gltf20),
        Err(crate::GeometryError::AttributeComponentType { .. })
    ));
}

#[cfg(feature = "write")]
#[test]
fn raw_writer_roundtrips_every_primitive_mode_and_append() {
    let modes = [
        (crate::PrimitiveMode::Points, vec![0u16, 1, 2]),
        (crate::PrimitiveMode::Lines, vec![0, 1]),
        (crate::PrimitiveMode::LineLoop, vec![0, 1, 2]),
        (crate::PrimitiveMode::LineStrip, vec![0, 1, 2]),
        (crate::PrimitiveMode::Triangles, vec![0, 1, 2]),
        (crate::PrimitiveMode::TriangleStrip, vec![0, 1, 2]),
        (crate::PrimitiveMode::TriangleFan, vec![0, 1, 2]),
    ];
    let positions = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    let mut import: Option<crate::Import> = None;
    for (mode, values) in modes {
        let indices = crate::PackedIndices::new(
            values.len(),
            crate::ComponentType::U16,
            values.into_iter().flat_map(u16::to_le_bytes).collect(),
        )
        .unwrap();
        let geometry = crate::PackedGeometry::new(
            mode,
            vec![crate::PackedAttribute::new(
                "POSITION",
                3,
                3,
                crate::ComponentType::F32,
                false,
                positions.clone(),
            )
            .unwrap()],
            Some(indices),
        )
        .unwrap();
        if let Some(scene) = import.as_mut() {
            let primitive = scene
                .push_primitive(
                    crate::MeshIndex(0),
                    &geometry,
                    crate::GeometryWriteOptions::default(),
                )
                .unwrap();
            assert_eq!(scene.read_primitive(primitive).unwrap(), geometry);
        } else {
            import = Some(
                crate::Import::from_geometry(
                    &geometry,
                    ValidationProfile::Gltf20,
                    crate::GeometryWriteOptions::default(),
                )
                .unwrap(),
            );
        }
    }
    assert_eq!(
        import
            .unwrap()
            .document
            .mesh(crate::MeshIndex(0))
            .unwrap()
            .primitive_count(),
        7
    );
}

#[cfg(feature = "write")]
#[test]
fn write_rejects_incompatible_morph_targets_atomically() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":24,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}],"bufferViews":[{"buffer":0,"byteLength":12},{"buffer":0,"byteOffset":12,"byteLength":12}],"accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3","min":[0,0,0],"max":[0,0,0]},{"bufferView":1,"componentType":5126,"count":1,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"targets":[{"POSITION":1}],"extras":{"keep":true}}]}]}"#;
    let mut import = crate::parse(input, ValidationProfile::Gltf20).unwrap();
    let before = import.document.to_json_bytes().unwrap();
    let error = import
        .write_primitive(
            crate::PrimitiveIndex::new(crate::MeshIndex(0), 0),
            &packed_triangle(crate::ComponentType::F32),
            crate::GeometryWriteOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        crate::Error::Geometry(crate::GeometryError::MorphTargetCount { .. })
    ));
    assert_eq!(import.document.to_json_bytes().unwrap(), before);
}

#[cfg(feature = "write")]
#[test]
fn standalone_raw_geometry_roundtrips_json_and_glb() {
    let geometry = packed_triangle(crate::ComponentType::F32);
    let import = crate::Import::from_geometry(
        &geometry,
        ValidationProfile::Gltf20,
        crate::GeometryWriteOptions::default(),
    )
    .unwrap();
    assert_eq!(import.document.nodes().len(), 1);
    assert_eq!(import.document.scenes().len(), 1);
    assert_eq!(
        import.document.as_value()["accessors"][0]["min"],
        crate::JsonValue::Array(vec![0u64.into(), 0u64.into(), 0u64.into()])
    );
    assert_eq!(
        import.document.as_value()["accessors"][0]["max"],
        crate::JsonValue::Array(vec![1u64.into(), 1u64.into(), 0u64.into()])
    );
    assert_eq!(
        import
            .read_primitive(crate::PrimitiveIndex::new(crate::MeshIndex(0), 0))
            .unwrap(),
        geometry
    );
    let output = import.to_gltf_output().unwrap();
    assert_eq!(output.resources.len(), 1);
    let resolver = |uri: &str| {
        output
            .resources
            .iter()
            .find(|resource| resource.uri == uri)
            .map(|resource| resource.bytes.clone())
            .ok_or_else(|| draco_io::GltfError::ExternalResourceDenied(uri.into()))
    };
    let reloaded = crate::parse_with_options(
        &output.json,
        None,
        Some(&resolver),
        &draco_io::ResourceLimits::default(),
        &draco_core::DecodeLimits::default(),
        ValidationProfile::Gltf20,
        &crate::ExtensionRegistry::default(),
    )
    .unwrap();
    assert_eq!(
        reloaded
            .read_primitive(crate::PrimitiveIndex::new(crate::MeshIndex(0), 0))
            .unwrap(),
        geometry
    );
    for format in [crate::OutputFormat::GlbV2, crate::OutputFormat::GlbV3] {
        let bytes = import.to_bytes(format).unwrap();
        let reloaded = crate::parse(&bytes, ValidationProfile::Gltf20).unwrap();
        assert_eq!(
            reloaded
                .read_primitive(crate::PrimitiveIndex::new(crate::MeshIndex(0), 0))
                .unwrap(),
            geometry
        );
    }
}

#[cfg(feature = "write")]
#[test]
fn write_primitive_preserves_non_geometry_fields_and_is_atomic() {
    let input = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":12,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAA"}],"bufferViews":[{"buffer":0,"byteLength":12}],"accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3","min":[0,0,0],"max":[0,0,0]}],"materials":[{}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"material":0,"extras":{"tag":7},"extensions":{"VENDOR_keep":{"value":9}}}]}]}"#;
    let mut import = crate::parse(input, ValidationProfile::Gltf20).unwrap();
    let before = import.document.to_json_bytes().unwrap();
    let invalid = crate::PackedAttribute::new(
        "POSITION",
        3,
        3,
        crate::ComponentType::F32,
        false,
        vec![0; 8],
    );
    assert!(invalid.is_err());
    assert_eq!(import.document.to_json_bytes().unwrap(), before);

    let geometry = packed_triangle(crate::ComponentType::F32);
    let report = import
        .write_primitive(
            crate::PrimitiveIndex::new(crate::MeshIndex(0), 0),
            &geometry,
            crate::GeometryWriteOptions::default(),
        )
        .unwrap();
    assert_eq!(
        report.preserve_reasons,
        vec![crate::PreserveReason::ExistingReferences]
    );
    let primitive = import.document.primitive(crate::MeshIndex(0), 0).unwrap();
    assert_eq!(primitive.value()["material"].as_u64(), Some(0));
    assert_eq!(primitive.value()["extras"]["tag"].as_u64(), Some(7));
    assert_eq!(
        primitive.value()["extensions"]["VENDOR_keep"]["value"].as_u64(),
        Some(9)
    );
}

#[cfg(feature = "write")]
#[test]
fn raw_writer_preserves_draft_64_bit_components() {
    let geometry = packed_triangle(crate::ComponentType::F64);
    let import = crate::Import::from_geometry(
        &geometry,
        ValidationProfile::Gltf21Draft,
        crate::GeometryWriteOptions::default(),
    )
    .unwrap();
    let read = import
        .read_primitive(crate::PrimitiveIndex::new(crate::MeshIndex(0), 0))
        .unwrap();
    assert_eq!(read, geometry);
    assert_eq!(
        import.document.as_value()["accessors"][0]["componentType"].as_u64(),
        Some(5130)
    );
}

#[cfg(feature = "draco-encode")]
#[test]
fn draco_write_roundtrips_and_rejects_f64_without_conversion() {
    let geometry = packed_triangle(crate::ComponentType::F32);
    let options = crate::GeometryWriteOptions {
        encoding: crate::GeometryEncoding::Draco(crate::CompressionOptions::default()),
    };
    let import =
        crate::Import::from_geometry(&geometry, ValidationProfile::Gltf20, options).unwrap();
    assert_eq!(import.draco_primitives().count(), 1);
    let decoded = import
        .read_primitive(crate::PrimitiveIndex::new(crate::MeshIndex(0), 0))
        .unwrap();
    assert_eq!(decoded.mode(), geometry.mode());
    assert_eq!(decoded.attributes(), geometry.attributes());
    assert_eq!(decoded.indices().unwrap().count(), 3);
    assert_eq!(
        decoded.indices().unwrap().bytes(),
        [0u32, 1, 2]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
    );

    let f64_geometry = packed_triangle(crate::ComponentType::F64);
    let error =
        crate::Import::from_geometry(&f64_geometry, ValidationProfile::Gltf21Draft, options)
            .err()
            .expect("f64 Draco geometry must be rejected");
    assert!(error.to_string().contains("not permitted") || error.to_string().contains("support"));
}

#[test]
fn minified_json_forces_serialization_and_preserves_order() {
    let document = crate::Document::from_json_bytes(
        br#"{ "asset": { "version": "2.0" }, "extras": { "number": 1.00 } }"#,
    )
    .unwrap();
    assert_eq!(
        document.to_minified_json_bytes(),
        br#"{"asset":{"version":"2.0"},"extras":{"number":1.00}}"#
    );
}

/// The Draco-only safety check walks the whole document, so it has to accept
/// nesting as deep as the parser does.
#[cfg(feature = "draco-encode")]
#[test]
fn deep_documents_survive_the_encode_safety_walk() {
    let depth = 200_000;
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"extras":{}1{}}}"#,
        "[".repeat(depth),
        "]".repeat(depth)
    );
    let import = crate::import_slice(json.as_bytes(), None).expect("deep extras import");
    import
        .ensure_document_binary_transform_safe()
        .expect("no extensions, so the check passes");
}
