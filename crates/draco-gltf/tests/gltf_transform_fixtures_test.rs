//! Draco glTF written by glTF-Transform, the tool most Draco files in the wild
//! come from with gltf-pipeline: each file imports, every primitive decodes to
//! the counts its accessors declare, and the EdgeBreaker and sequential
//! encodings of one source decode to the same triangles. See
//! `testdata/gltf_transform/README.md` for how the files were made.

#![cfg(feature = "draco-decode")]

use std::path::PathBuf;

use draco_gltf::{
    import_slice_with_options, ComponentType, Import, ImportOptions, MeshIndex, PackedGeometry,
    PrimitiveIndex, ValidationProfile,
};

fn import(name: &str) -> Import {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/gltf_transform")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let options = ImportOptions {
        profile: ValidationProfile::Gltf20,
        ..ImportOptions::default()
    };
    let import =
        import_slice_with_options(&bytes, &options).unwrap_or_else(|e| panic!("{name}: {e}"));
    let generator = import.document().as_value()["asset"]["generator"].as_str();
    assert!(
        generator.is_some_and(|g| g.starts_with("glTF-Transform")),
        "{name} was written by {generator:?}"
    );
    import
}

/// Every primitive of `name`, decoded, checked against the counts its
/// accessors declare.
fn primitives(name: &str) -> Vec<PackedGeometry> {
    let import = import(name);
    let mut decoded = Vec::new();
    for mesh in 0..import.document().meshes().len() {
        let count = import
            .document()
            .mesh(MeshIndex(mesh))
            .map_or(0, |mesh| mesh.primitive_count());
        for primitive in 0..count {
            let reference = import
                .document()
                .primitive(MeshIndex(mesh), primitive)
                .unwrap();
            let declared = |accessor: usize| {
                import
                    .document()
                    .accessor(accessor.into())
                    .and_then(|a| a.count())
            };
            let geometry = import
                .read_primitive(PrimitiveIndex::new(MeshIndex(mesh), primitive))
                .unwrap_or_else(|e| panic!("{name} {mesh}/{primitive}: {e}"));
            for (semantic, accessor) in reference.attribute_indices() {
                assert_eq!(
                    declared(accessor.into()),
                    Some(geometry.vertex_count() as u64),
                    "{name} {mesh}/{primitive}: {semantic}"
                );
            }
            assert_eq!(
                reference
                    .indices()
                    .and_then(|accessor| declared(accessor.into())),
                geometry.indices().map(|indices| indices.count() as u64),
                "{name} {mesh}/{primitive}: indices"
            );
            decoded.push(geometry);
        }
    }
    assert!(!decoded.is_empty(), "{name} has no primitive");
    decoded
}

/// The triangles of `geometry` independent of vertex order, which EdgeBreaker
/// changes and sequential keeps: each corner as every attribute's bytes in
/// semantic order, each triangle rotated to start at its smallest corner so
/// its winding is kept, and the triangles sorted.
fn triangles(geometry: &PackedGeometry) -> Vec<[Vec<u8>; 3]> {
    let mut attributes: Vec<_> = geometry.attributes().iter().collect();
    attributes.sort_by(|a, b| a.semantic().cmp(b.semantic()));
    let corner = |vertex: usize| -> Vec<u8> {
        attributes
            .iter()
            .flat_map(|attribute| {
                let width = attribute.bytes().len() / attribute.count();
                attribute.bytes()[vertex * width..(vertex + 1) * width].to_vec()
            })
            .collect()
    };
    let indices = geometry.indices().expect("an indexed primitive");
    assert_eq!(indices.component_type(), ComponentType::U32);
    let vertices: Vec<usize> = indices
        .bytes()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| u32::from_le_bytes(*b) as usize)
        .collect();
    let mut triangles: Vec<[Vec<u8>; 3]> = vertices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            let mut corners = [corner(t[0]), corner(t[1]), corner(t[2])];
            let first = (0..3).min_by(|&a, &b| corners[a].cmp(&corners[b])).unwrap();
            corners.rotate_left(first);
            corners
        })
        .collect();
    triangles.sort();
    triangles
}

fn same_triangles(edgebreaker: &str, sequential: &str) {
    let (a, b) = (primitives(edgebreaker), primitives(sequential));
    assert_eq!(a.len(), b.len());
    for (k, (a, b)) in a.iter().zip(&b).enumerate() {
        assert!(
            triangles(a) == triangles(b),
            "primitive {k}: {edgebreaker} and {sequential} decode to different triangles"
        );
    }
}

#[test]
fn two_primitives_decode_alike_through_either_coder() {
    same_triangles(
        "two_objects_edgebreaker_speed0.glb",
        "two_objects_sequential.glb",
    );
}

#[test]
fn a_valence_traversal_decodes_as_the_sequential_coder_does() {
    same_triangles("sphere_edgebreaker_speed0.glb", "sphere_sequential.glb");
}
