//! Turns decoded geometry into a Bevy [`Mesh`].
//!
//! The values are not converted here. Bevy's own
//! [`convert_attribute`] decides which `MeshVertexAttribute` a semantic maps
//! to, how normalized integers widen, and how coordinates rotate; doing that
//! again would be a second copy to keep in step with every Bevy release. So
//! the decoded attributes are described to it as what they are -- tightly
//! packed accessors -- in a one-buffer document built for the purpose, and it
//! reads them exactly as it reads an uncompressed file.

use bevy::asset::RenderAssetUsages;
use bevy::gltf::vertex_attributes::convert_attribute;
use bevy::gltf::{MorphTargetNames, PrimitiveMorphAttributesIter};
use bevy::log::warn;
use bevy::mesh::{Indices, Mesh, MeshVertexAttribute, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use draco_gltf::{ComponentType, PackedAttribute};
use gltf::json::validation::Checked;
use gltf::Semantic;
use serde_json::json;

use crate::decode::Decoded;

/// What the loader knows about the primitive beyond its geometry.
pub(crate) struct MeshContext<'a> {
    pub load_meshes: RenderAssetUsages,
    pub rotate_meshes: bool,
    pub custom_vertex_attributes: &'a HashMap<Box<str>, MeshVertexAttribute>,
    pub on_skinned_nodes: bool,
    pub on_non_skinned_nodes: bool,
    /// The asset label, for messages.
    pub label: &'a str,
}

/// Builds the mesh for a decoded primitive.
///
/// `primitive` and `buffers` are the original document's: attributes the
/// extension does not compress, and morph targets, are ordinary accessors
/// there.
pub(crate) fn build_mesh(
    decoded: &Decoded,
    primitive: &gltf::Primitive<'_>,
    gltf_mesh: &gltf::Mesh<'_>,
    buffers: &[Vec<u8>],
    context: &MeshContext<'_>,
) -> Mesh {
    let geometry = &decoded.geometry;
    let vertex_count = geometry.vertex_count();
    // A Draco stream is an indexed triangle list whatever `mode` says; see
    // `PackedGeometry`'s Draco constructor for why the declared mode is not
    // trusted.
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, context.load_meshes);

    let mut compressed = Vec::with_capacity(geometry.attributes().len());
    for attribute in geometry.attributes() {
        let Some(semantic) = parse_semantic(attribute.semantic()) else {
            warn!(
                "{}: ignoring Draco attribute with invalid semantic {:?}",
                context.label,
                attribute.semantic()
            );
            continue;
        };
        if !is_core_component_type(attribute.component_type()) {
            warn!(
                "{}: ignoring Draco attribute {semantic:?}: component type {:?} is not in glTF 2.0",
                context.label,
                attribute.component_type()
            );
            continue;
        }
        compressed.push((semantic, attribute));
    }
    let (document, packed) = packed_document(compressed.iter().map(|(_, attribute)| *attribute));
    for (accessor, (semantic, _)) in document.accessors().zip(compressed) {
        insert_attribute(&mut mesh, semantic, accessor, &packed, context);
    }

    // KHR_draco_mesh_compression: attributes the primitive lists and the
    // extension does not are read as usual. They index the decoded vertices,
    // so they are only usable when the stream kept the vertex count.
    let mut uncompressed = None;
    for (semantic, accessor) in primitive.attributes() {
        if decoded.extension.unique_id(&semantic.to_string()).is_some() {
            continue;
        }
        if accessor.count() != vertex_count {
            warn!(
                "{}: ignoring uncompressed attribute {semantic:?}: it has {} values and the Draco stream decoded {vertex_count} vertices",
                context.label,
                accessor.count()
            );
            continue;
        }
        // `convert_attribute` takes the buffers as a `Vec`; the hook lends a
        // slice. Only primitives that mix compressed and uncompressed
        // attributes -- rare in practice -- pay for the copy.
        let buffers = uncompressed.get_or_insert_with(|| buffers.to_vec());
        insert_attribute(&mut mesh, semantic, accessor, buffers, context);
    }

    if let Some(indices) = geometry.indices() {
        mesh.insert_indices(indices_for(indices.bytes(), vertex_count));
    }

    insert_morph_targets(
        &mut mesh,
        primitive,
        gltf_mesh,
        buffers,
        vertex_count,
        context,
    );
    mesh
}

/// The mesh to hand Bevy when a primitive could not be decoded.
///
/// Leaving the hook's output empty is worse than it looks: Bevy would then
/// read the primitive's own accessors, which for a Draco primitive are
/// placeholders without data, get no positions, and panic computing flat
/// normals for them. An empty mesh with empty normals renders nothing and
/// asks for nothing to be computed.
pub(crate) fn empty_mesh(load_meshes: RenderAssetUsages) -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, load_meshes)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new())
}

fn insert_attribute(
    mesh: &mut Mesh,
    semantic: Semantic,
    accessor: gltf::Accessor<'_>,
    buffers: &Vec<Vec<u8>>,
    context: &MeshContext<'_>,
) {
    // The same rule, and the same messages, as Bevy's own primitive loader.
    if [Semantic::Joints(0), Semantic::Weights(0)].contains(&semantic) {
        if !context.on_skinned_nodes {
            warn!(
                "Ignoring attribute {:?} for skinned mesh {} used on non skinned nodes (NODE_SKINNED_MESH_WITHOUT_SKIN)",
                semantic, context.label
            );
            return;
        } else if context.on_non_skinned_nodes {
            bevy::log::error!(
                "Skinned mesh {} used on both skinned and non skin nodes, this is likely to cause an error (NODE_SKINNED_MESH_WITHOUT_SKIN)",
                context.label
            );
        }
    }
    match convert_attribute(
        semantic,
        accessor,
        buffers,
        context.custom_vertex_attributes,
        context.rotate_meshes,
    ) {
        Ok((attribute, values)) => mesh.insert_attribute(attribute, values),
        Err(error) => warn!("{}: {error}", context.label),
    }
}

/// Describes `attributes` as accessors over one buffer, in the same order.
fn packed_document<'a>(
    attributes: impl ExactSizeIterator<Item = &'a PackedAttribute>,
) -> (gltf::Document, Vec<Vec<u8>>) {
    let mut buffer = Vec::new();
    let mut views = Vec::with_capacity(attributes.len());
    let mut accessors = Vec::with_capacity(attributes.len());
    for (index, attribute) in attributes.enumerate() {
        // Accessor offsets must be multiples of the component size.
        buffer.resize(buffer.len().next_multiple_of(4), 0);
        views.push(json!({
            "buffer": 0,
            "byteOffset": buffer.len(),
            "byteLength": attribute.bytes().len(),
        }));
        buffer.extend_from_slice(attribute.bytes());
        accessors.push(json!({
            "bufferView": index,
            "componentType": attribute.component_type() as u32,
            "count": attribute.count(),
            "type": accessor_type(attribute.components()),
            "normalized": attribute.normalized(),
        }));
    }
    let root = json!({
        "asset": { "version": "2.0" },
        "buffers": [{ "byteLength": buffer.len() }],
        "bufferViews": views,
        "accessors": accessors,
    });
    // Every value above is in range by construction -- component types are
    // filtered to glTF 2.0 core first -- so the document is valid and gltf-rs
    // can skip validating it.
    let root: gltf::json::Root =
        serde_json::from_value(root).expect("the packed document is well-formed glTF");
    (
        gltf::Document::from_json_without_validation(root),
        vec![buffer],
    )
}

/// gltf-rs unwraps the component type of every accessor it reads, so one
/// outside glTF 2.0 core -- which `draco-gltf` admits for the 2.1 draft --
/// must never reach it.
fn is_core_component_type(component_type: ComponentType) -> bool {
    matches!(
        component_type,
        ComponentType::I8
            | ComponentType::U8
            | ComponentType::I16
            | ComponentType::U16
            | ComponentType::U32
            | ComponentType::F32
    )
}

fn accessor_type(components: u8) -> &'static str {
    match components {
        1 => "SCALAR",
        2 => "VEC2",
        3 => "VEC3",
        _ => "VEC4",
    }
}

fn parse_semantic(semantic: &str) -> Option<Semantic> {
    match serde_json::from_value::<Checked<Semantic>>(serde_json::Value::from(semantic)) {
        Ok(Checked::Valid(semantic)) => Some(semantic),
        _ => None,
    }
}

/// Narrows `u32` indices to `u16` when every vertex fits, as Bevy's loader
/// ends up doing for the uncompressed meshes Draco streams usually start as.
fn indices_for(bytes: &[u8], vertex_count: usize) -> Indices {
    let values = bytes
        .chunks_exact(4)
        .map(|index| u32::from_le_bytes([index[0], index[1], index[2], index[3]]));
    if vertex_count <= usize::from(u16::MAX) + 1 {
        // `draco-gltf` has already checked every index against the vertex
        // count, so none of these truncate.
        Indices::U16(values.map(|index| index as u16).collect())
    } else {
        Indices::U32(values.collect())
    }
}

fn insert_morph_targets(
    mesh: &mut Mesh,
    primitive: &gltf::Primitive<'_>,
    gltf_mesh: &gltf::Mesh<'_>,
    buffers: &[Vec<u8>],
    vertex_count: usize,
    context: &MeshContext<'_>,
) {
    let targets = primitive.morph_targets();
    if targets.len() == 0 {
        return;
    }
    // Morph targets stay uncompressed, and index the vertices in the order the
    // encoder wrote them. A stream that decodes to a different vertex count
    // has provably not kept that order, and applying the targets anyway would
    // move the wrong vertices.
    let mismatch = targets
        .flat_map(|target| [target.positions(), target.normals(), target.tangents()])
        .flatten()
        .find(|accessor| accessor.count() != vertex_count);
    if let Some(accessor) = mismatch {
        warn!(
            "{}: dropping morph targets: accessor {} has {} values and the Draco stream decoded {vertex_count} vertices",
            context.label,
            accessor.index(),
            accessor.count()
        );
        return;
    }

    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
    mesh.set_morph_targets(
        reader
            .read_morph_targets()
            .flat_map(
                |(positions, normals, tangents)| PrimitiveMorphAttributesIter {
                    convert_coordinates: context.rotate_meshes,
                    positions,
                    normals,
                    tangents,
                },
            )
            .collect(),
    );
    if let Some(names) = gltf_mesh
        .extras()
        .as_ref()
        .and_then(|extras| serde_json::from_str::<MorphTargetNames>(extras.get()).ok())
    {
        mesh.set_morph_target_names(names.target_names);
    }
}
