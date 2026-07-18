use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;
use draco_io::{
    parse_compact_document, parse_glb_json_and_bin, GltfError, GltfReader, ResourceLimits,
};

fn fixture_resource() -> Vec<u8> {
    let mut bytes = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    bytes.extend([0u16, 1, 2].into_iter().flat_map(u16::to_le_bytes));
    bytes
}

fn fixture_json(uri: Option<&str>) -> String {
    let buffer = match uri {
        Some(uri) => format!(r#"{{"uri":"{uri}","byteLength":42}}"#),
        None => r#"{"byteLength":42}"#.to_string(),
    };
    format!(
        r#"{{
          "asset":{{"version":"2.0"}},
          "buffers":[{buffer}],
          "bufferViews":[
            {{"buffer":0,"byteOffset":0,"byteLength":36}},
            {{"buffer":0,"byteOffset":36,"byteLength":6}}
          ],
          "accessors":[
            {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}},
            {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}}
          ],
          "meshes":[
            {{"name":"First","primitives":[
              {{"attributes":{{"POSITION":0}},"indices":1}},
              {{"attributes":{{"POSITION":0}},"indices":1}}
            ]}},
            {{"name":"Second","primitives":[{{"attributes":{{"POSITION":0}},"indices":1}}]}}
          ],
          "nodes":[
            {{"name":"MatrixNode","mesh":1,"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,3,4,5,1],"children":[1]}},
            {{"name":"TrsNode","mesh":0,"translation":[1,2,3],"rotation":[0,0,0,1],"scale":[2,3,4]}}
          ],
          "scenes":[{{"name":"Main","nodes":[0]}}],
          "scene":0
        }}"#
    )
}

fn build_glb(json: &str, bin: &[u8]) -> Vec<u8> {
    const GLB_MAGIC: u32 = 0x4654_6c67;
    const GLB_VERSION: u32 = 2;
    const JSON_CHUNK: u32 = 0x4e4f_534a;
    const BIN_CHUNK: u32 = 0x004e_4942;

    let json_len = (json.len() + 3) & !3;
    let bin_len = (bin.len() + 3) & !3;
    let total_len = 12 + 8 + json_len + 8 + bin_len;
    let mut glb = Vec::with_capacity(total_len);
    glb.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    glb.extend_from_slice(&GLB_VERSION.to_le_bytes());
    glb.extend_from_slice(&(total_len as u32).to_le_bytes());
    glb.extend_from_slice(&(json_len as u32).to_le_bytes());
    glb.extend_from_slice(&JSON_CHUNK.to_le_bytes());
    glb.extend_from_slice(json.as_bytes());
    glb.resize(12 + 8 + json_len, b' ');
    glb.extend_from_slice(&(bin_len as u32).to_le_bytes());
    glb.extend_from_slice(&BIN_CHUNK.to_le_bytes());
    glb.extend_from_slice(bin);
    glb.resize(total_len, 0);
    glb
}

fn native_positions(mesh: &Mesh) -> Vec<f32> {
    mesh.named_attribute(GeometryAttributeType::Position)
        .expect("native reader must decode POSITION")
        .buffer()
        .data()
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
        .collect()
}

fn native_indices(mesh: &Mesh) -> Vec<u32> {
    (0..mesh.num_faces())
        .flat_map(|index| mesh.face(FaceIndex(index as u32)).map(|point| point.0))
        .collect()
}

fn assert_reader_parity(native: GltfReader, compact: draco_io::CompactDocument) {
    let metadata = native.document_metadata();
    let native_meshes = native.decode_all_meshes().unwrap();

    assert_eq!(native_meshes.len(), compact.meshes.len());
    for (native, compact) in native_meshes.iter().zip(&compact.meshes) {
        assert_eq!(native_positions(native), compact.positions);
        assert_eq!(native_indices(native), compact.indices);
    }
    assert_eq!(
        metadata.primitive_names,
        compact
            .meshes
            .iter()
            .map(|mesh| mesh.name.clone())
            .collect::<Vec<_>>(),
    );
    assert_eq!(native.num_meshes(), compact.mesh_primitive_ranges.len());
    assert_eq!(
        compact.mesh_primitive_ranges,
        vec![
            draco_io::CompactMeshRange {
                first_primitive: 0,
                primitive_count: 2,
            },
            draco_io::CompactMeshRange {
                first_primitive: 2,
                primitive_count: 1,
            },
        ]
    );
    assert_eq!(metadata.nodes.len(), compact.nodes.len());
    for (native, compact) in metadata.nodes.iter().zip(&compact.nodes) {
        assert_eq!(native.name, compact.name);
        assert_eq!(native.mesh, compact.mesh);
        assert_eq!(native.matrix, compact.matrix);
        assert_eq!(
            native.translation.map(|value| value.to_vec()),
            compact.translation
        );
        assert_eq!(
            native.rotation.map(|value| value.to_vec()),
            compact.rotation
        );
        assert_eq!(native.scale.map(|value| value.to_vec()), compact.scale);
        assert_eq!(native.children, compact.children);
    }
    assert_eq!(metadata.scenes.len(), compact.scenes.len());
    for (native, compact) in metadata.scenes.iter().zip(&compact.scenes) {
        assert_eq!(native.name, compact.name);
        assert_eq!(native.nodes, compact.nodes);
    }
    assert_eq!(metadata.default_scene, compact.default_scene);
    assert_eq!(metadata.uses_draco, compact.uses_draco);
}

#[test]
fn compact_matches_native_for_external_gltf() {
    let resource = fixture_resource();
    let json = fixture_json(Some("scene.bin"));
    let native_resource = resource.clone();
    let resolver = move |uri: &str| {
        if uri == "scene.bin" {
            Ok(native_resource.clone())
        } else {
            Err(GltfError::ExternalResourceDenied(uri.to_owned()))
        }
    };
    let native = GltfReader::from_bytes_with_resolver(
        json.as_bytes(),
        &resolver,
        &ResourceLimits::default(),
    )
    .unwrap();
    let compact =
        parse_compact_document(&json, None, &[("scene.bin".to_string(), resource)]).unwrap();

    assert_reader_parity(native, compact);
}

#[test]
fn compact_matches_native_for_glb() {
    let resource = fixture_resource();
    let glb = build_glb(&fixture_json(None), &resource);
    let native = GltfReader::from_bytes(&glb).unwrap();
    let (json, bin) = parse_glb_json_and_bin(&glb).unwrap();
    let compact = parse_compact_document(std::str::from_utf8(json).unwrap(), bin, &[]).unwrap();

    assert_reader_parity(native, compact);
}
