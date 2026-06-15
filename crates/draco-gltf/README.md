# draco-gltf

Load and save **full glTF scenes with Draco-compressed geometry**.

`draco-gltf` is a thin bridge between [`gltf`](https://crates.io/crates/gltf)
(gltf-rs), which models the whole glTF scene — materials, textures, nodes,
animations, skins, lights, and arbitrary extensions — and the Draco crates:

- **decode** uses [`draco-core`](https://crates.io/crates/draco-core) to
  decompress `KHR_draco_mesh_compression` geometry;
- **encode** delegates to
  [`draco-io`](https://crates.io/crates/draco-io)'s document-preserving
  compressor, so the compression logic lives in exactly one place.

It exists because neither side does the whole job alone: gltf-rs does not decode
Draco (its validator even *rejects* a Draco asset, since
`KHR_draco_mesh_compression` is a required extension it does not implement),
while the Draco crates intentionally do not model the rest of a glTF scene.

```toml
[dependencies]
draco-gltf = "0.1"
```

## Load a scene and decode geometry

```rust,no_run
let scene = draco_gltf::import("model.glb")?;

// The full scene is available through the gltf-rs API.
println!("{} materials, {} animations, {} skins",
    scene.document.materials().count(),
    scene.document.animations().count(),
    scene.document.skins().count());

// Decode the Draco-compressed geometry.
for (mesh, prim) in scene.draco_primitives() {
    let geometry = scene.decode_primitive(&prim)?; // draco_core::Mesh
    println!("mesh {:?}: {} faces, {} points",
        mesh.name(), geometry.num_faces(), geometry.num_points());
}
# Ok::<(), draco_gltf::Error>(())
```

### Transparent reading

If you would rather not deal with Draco at all, `decompress_in_place` replaces
every Draco primitive with plain geometry, after which the normal gltf-rs reader
works on the whole document:

```rust,no_run
let mut scene = draco_gltf::import("model.glb")?;
scene.decompress_in_place()?; // Draco primitives become ordinary geometry

for mesh in scene.document.meshes() {
    for prim in mesh.primitives() {
        let reader = prim.reader(|b| scene.buffers.get(b.index()).map(|d| &d.0[..]));
        let positions: Vec<_> = reader.read_positions().unwrap().collect();
        // ... use positions, just like any uncompressed glTF
    }
}
# Ok::<(), draco_gltf::Error>(())
```

## Compress a scene back to Draco

```rust,no_run
let scene = draco_gltf::import("model.gltf")?;
let bytes = draco_gltf::compress(&scene.document, &scene.buffers)?;
std::fs::write("model.draco.gltf", bytes)?;
# Ok::<(), draco_gltf::Error>(())
```

`compress` preserves everything the geometry change does not touch — materials,
textures, images, nodes, animations, skins, `extras`, and unknown extensions —
because it reuses `draco-io`'s document-preserving compressor.

## Validation

gltf-rs's own validator rejects Draco assets outright (it treats
`KHR_draco_mesh_compression` as an unsupported required extension). `import`
therefore runs **Draco-aware validation**: full gltf-rs validation with only the
expected Draco errors filtered out (the unsupported-extension error, and the
"missing bufferView" on accessors whose data comes from the Draco stream). A
structurally invalid asset — out-of-range indices, malformed accessors — is
still rejected. The validation is also panic-safe: gltf-rs's validator can panic
on some malformed documents, so it is isolated and any panic becomes a
controlled `Error::Validation`. Use `draco_gltf::validate(&document)` to check a
document you built yourself.

## Where it sits

| Crate | Role | depends on gltf-rs |
|---|---|---|
| `draco-core` | raw `.drc` bitstream | no |
| `draco-io` | container plumbing + document-preserving compressor | no |
| **`draco-gltf`** | full-scene load/save bridging gltf-rs ↔ Draco | yes (only here) |

If you only need raw `.drc` or geometry-level glTF I/O, use `draco-core` /
`draco-io` directly and skip the gltf-rs dependency.

## License

Apache-2.0.
