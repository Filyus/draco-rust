# draco-gltf

Lossless glTF 2.0 and pinned 2.1-draft scene handling with
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

`Import::document` is a lossless `Document`: use typed views such as
`document.meshes()`, `document.nodes()`, `document.files()`, and
`document.primitive(MeshIndex(0), 0)`. Unknown properties, `extras`, and
unregistered extension JSON survive parse and serialization.

The default profile targets the pinned glTF 2.1 draft. It supports GLB v2 and
v3, explicit external assets through `files`, shapes, UIDs, and non-sequential
attribute sets. See `GLTF_2_1_SNAPSHOT.md`, `GLTF_2_1.md`, and
`MIGRATING_0_2.md`.

`ExtensionRegistry` owns extension validation and geometry decoding. Its default
registry contains Draco and maps compressed primitives to `draco_core::Mesh`.
Call `Import::compress_primitive` with `CompressionMode::DracoOnly` (the
default) for a compact Draco-required primitive, or
`CompressionMode::Fallback` to retain ordinary geometry for non-Draco readers.
The two modes map directly to `extensionsRequired` and `extensionsUsed`:
fallback never requires Draco. `DracoOnly` detaches the primitive's accessors
from raw views, preserves shared accessors for other scene consumers, and
compacts retained binary resources. `Import::decompress_in_place` materializes
plain geometry; `to_bytes` selects JSON, GLB v2, or GLB v3 output. Enable the
`compact` feature for geometry-oriented views and `PackedPrimitive` buffers
over the same lossless `Document`. `compact` names the smaller API surface;
`PackedPrimitive` names the materialized contiguous geometry representation.

For a portable `.gltf` write, use `Import::to_gltf_output()`: it returns the
JSON bytes plus every non-data-URI buffer as a named companion resource. This
is required after transforms that append binary data; `to_bytes(GltfJson)` is
only the JSON representation. Draco transforms clone changed accessors before
materializing them, so accessors shared with animations, skins, morph targets,
or other primitives remain intact. A transform also rejects an extension on
the changed primitive unless its registered handler explicitly declares its
binary-reference semantics transform-safe and provides its accessor/buffer-view
reference collect/remap contract. `CompressionOptions::max_output_bytes` caps
resolved binary output atomically; `CompressionReport` records the mode,
encoded bytes, final bytes, and reclaimed bytes.

## Features

The feature graph separates document completeness from executable geometry
operations:

- `document-core` provides the lossless DOM, typed views, GLB and resource
  contracts without a Draco decoder.
- `geometry` adds accessor materialization and the `draco_io::PackedPrimitive`
  contract.
- `draco-decode` adds `KHR_draco_mesh_compression` decoding.
- `resources` and `scene-validation` add explicit `files` loading and strict
  scene validation; `transform` adds the Draco encoder and document mutations.
- `full` is the default release scene API. `compact` is the compact runtime
  path (`document-core + geometry + draco-decode`) and intentionally excludes
  `transform`.

For a decoder-only consumer, depend with `default-features = false, features =
["compact"]`. The resulting document can still preserve every scene field; the
excluded code is mutation and encoding behaviour, not a second document model.

## License

Apache-2.0.
