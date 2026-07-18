# draco-gltf

Lossless native glTF 2.0 and pinned 2.1-draft scene handling with
`KHR_draco_mesh_compression`.

```rust,no_run
let mut scene = draco_gltf::import("model.glb")?;
for primitive in scene.draco_primitives() {
    let mesh = scene.decode_primitive(primitive)?;
    println!("{} faces", mesh.num_faces());
}
scene.decompress_in_place()?;
# Ok::<(), draco_gltf::Error>(())
```

`Import::document` is a native lossless `Document`: use typed views such as
`document.meshes()`, `document.nodes()`, `document.files()`, and
`document.primitive(MeshIndex(0), 0)`. Unknown properties, `extras`, and
unregistered extension JSON survive parse and serialization.

The default profile targets the pinned glTF 2.1 draft. It supports GLB v2 and
v3, explicit external assets through `files`, shapes, UIDs, and non-sequential
attribute sets. See `GLTF_2_1_SNAPSHOT.md`, `GLTF_2_1.md`, and
`MIGRATING_0_2.md`.

`ExtensionRegistry` owns extension validation and geometry decoding. Its default
registry contains Draco and maps compressed primitives to `draco_core::Mesh`.
Call `Import::compress_primitive` for append-only document-preserving Draco
compression, `Import::decompress_in_place` to materialize plain geometry, and
`to_bytes` to select JSON, GLB v2, or GLB v3 output. Enable the `compact`
feature for geometry-oriented views over the same native `Document`.

## License

Apache-2.0.
