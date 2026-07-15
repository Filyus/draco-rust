# draco-gltf

Load and save **full glTF scenes with Draco-compressed geometry**.

`draco-gltf` is a thin bridge between [`gltf`](https://crates.io/crates/gltf)
(gltf-rs), which models the whole glTF scene — materials, textures, nodes,
animations, skins, lights, and arbitrary extensions — and the Draco crates:

- **decode** uses [`draco-core`](https://crates.io/crates/draco-core) to
  decompress `KHR_draco_mesh_compression` geometry;
- **encode** reuses
  [`draco-io`](https://crates.io/crates/draco-io)'s document-preserving
  compressor and hardened container/resource layer, so the format logic lives
  in exactly one place while geometry is exposed through gltf-rs.

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
        let reader = prim.reader(|b| scene.buffers.get(b.index()).map(|d| &d[..]));
        let positions: Vec<_> = reader.read_positions().unwrap().collect();
        // ... use positions, just like any uncompressed glTF
    }
}
# Ok::<(), draco_gltf::Error>(())
```

## Compress a scene back to Draco

```rust,no_run
let scene = draco_gltf::import("model.gltf")?;
let compressed = scene.compress()?;
std::fs::write("model.draco.gltf", compressed.data)?;
println!("{:?}", compressed.report);
# Ok::<(), draco_gltf::Error>(())
```

`Import::compress` preserves everything the geometry change does not touch —
materials, textures, images, nodes, animations, skins, `extras`, arbitrary
extension JSON, and unknown object fields. `Import` retains the original JSON
alongside the typed gltf-rs document so fields outside gltf-rs's model are not
dropped. Its `CompressionReport` distinguishes compressed primitives from
valid but unsupported primitives that were preserved (for example morph
targets or a non-triangle layout). Malformed input is always an error, never a
preserve reason.

The default quantization is lossy. `GltfCompressionOptions` selects
quantization per attribute class (`None` disables it), encoding/decoding speed,
encoding method, and `SameAsInput`, embedded glTF, or GLB output. Out-of-range
values are rejected rather than clamped.

Unknown JSON is preserved, but repacking refuses an extension that contains
opaque `buffer`/`bufferView`/offset-like references. Such an extension needs
explicit reference semantics before its binary data can be moved safely.

## Validation

gltf-rs's own validator rejects Draco assets outright (it treats
`KHR_draco_mesh_compression` as an unsupported required extension). `import`
therefore runs **Draco-aware validation**: full gltf-rs validation with only the
expected Draco errors filtered out (the unsupported-extension error, and the
"missing bufferView" on accessors whose data comes from the Draco stream). A
shared strict KHR parser additionally checks the extension schema, u32 unique
IDs, primitive mode, semantic subset, declarations, and fallback accessors. A
structurally invalid asset is rejected. The validation is also panic-safe:
gltf-rs 1.4's validator can
panic on a primitive that references an out-of-range accessor, so `validate`
pre-checks those references and returns a controlled `Error::Validation` before
the panic can happen (this holds even on wasm targets built with
`panic = "abort"`, where `catch_unwind` would not). Use
`draco_gltf::validate(&document)` to check a document you built yourself.

## WebAssembly

`draco-gltf` works on `wasm32` for native-Rust web apps that want the full glTF
scene model in the browser. Use `import_slice_with_options`,
`Import::compress`, and `Import::decompress_in_place` because filesystem
`import` is native-only. `ImportOptions` accepts a synchronous
`ResourceResolver`, an `ExternalFilePolicy`, and optional per-resource,
total-buffer, and image-pixel quotas. For plugging a Draco decoder into a
JavaScript glTF loader instead (three.js, babylon, …), use `draco-core`
directly.

## Features

- **`image`** (default): decode embedded/external images into pixels, exposing
  them as `Import::images`. This pulls the `image` crate
  (PNG/JPEG codecs), which is the largest part of the build. Disable it with
  `default-features = false` when you only need geometry and the scene model —
  e.g. on wasm, where the host usually decodes textures. Without it, images are
  not decoded and `Import::images` is absent; buffers still use the shared
  strict resolver.

`GltfEmbeddedBuffers` embeds glTF buffers. It does not turn external image URIs
into data URIs; images remain external unless the caller performs that separate
document transform.

## glTF 2.1

These crates target glTF 2.0. glTF 2.1-specific component types and 64-bit GLB
are not implemented. Unsupported geometry is preserved only when its structure
can be understood safely; malformed containers and opaque binary references are
errors. See [GLTF_2_1.md](GLTF_2_1.md).

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
