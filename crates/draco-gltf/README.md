# draco-gltf

Load and save **full glTF scenes with Draco-compressed geometry**.

`draco-gltf` is a thin bridge between [`gltf`](https://crates.io/crates/gltf)
(gltf-rs), which models the whole glTF scene — materials, textures, nodes,
animations, skins, lights, and arbitrary extensions — and the Draco crates:

- **decode** uses [`draco-core`](https://crates.io/crates/draco-core) to
  decompress `KHR_draco_mesh_compression` geometry;
- **encode** reuses
  [`draco-io`](https://crates.io/crates/draco-io)'s document-preserving
  compressor core, so the compression logic lives in exactly one place — while
  reading the geometry to compress through gltf-rs, so it depends on `draco-io`
  with only its writer, never its glTF reader.

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
let bytes = draco_gltf::compress(&scene.document, &scene.buffers)?;
std::fs::write("model.draco.gltf", bytes)?;
# Ok::<(), draco_gltf::Error>(())
```

`compress` preserves everything the geometry change does not touch — materials,
textures, images, nodes, animations, skins, `extras`, and unknown extensions. It
runs `draco-io`'s shared document-preserving compressor core, decoding the
geometry to compress straight from the gltf-rs accessors through `draco-io`'s
reader-agnostic `decode_geometry`. So there is a single compression path, and
`draco-gltf` depends on `draco-io` with only its `gltf-writer` feature — not its
glTF reader (`gltf-rs` already parses the document).

## Validation

gltf-rs's own validator rejects Draco assets outright (it treats
`KHR_draco_mesh_compression` as an unsupported required extension). `import`
therefore runs **Draco-aware validation**: full gltf-rs validation with only the
expected Draco errors filtered out (the unsupported-extension error, and the
"missing bufferView" on accessors whose data comes from the Draco stream). A
structurally invalid asset — out-of-range indices, malformed accessors — is
still rejected. The validation is also panic-safe: gltf-rs 1.4's validator can
panic on a primitive that references an out-of-range accessor, so `validate`
pre-checks those references and returns a controlled `Error::Validation` before
the panic can happen (this holds even on wasm targets built with
`panic = "abort"`, where `catch_unwind` would not). Use
`draco_gltf::validate(&document)` to check a document you built yourself.

## WebAssembly

`draco-gltf` works on `wasm32` for native-Rust web apps that want the full glTF
scene model in the browser. Use the byte API — [`import_slice`], [`compress`],
and [`Import::decompress_in_place`] — since the filesystem [`import`] is
native-only. Fetch the glTF/GLB bytes (and any external resources) yourself and
pass them in. For plugging a Draco decoder into a JavaScript glTF loader instead
(three.js, babylon, …), use `draco-core` directly.

## Features

- **`image`** (default): decode embedded/external images into pixels via
  gltf-rs, exposing them as `Import::images`. This pulls the `image` crate
  (PNG/JPEG codecs), which is the largest part of the build. Disable it with
  `default-features = false` when you only need geometry and the scene model —
  e.g. on wasm, where the host usually decodes textures. Without it, images are
  not decoded, `Import::images` is absent, and a small built-in loader resolves
  buffers (data URIs and the GLB BIN chunk; external files only with a base
  path). Measured size-optimized wasm: ~384 KB → ~279 KB gzip without `image`.

## glTF 2.1

These crates target glTF 2.0. glTF 2.1 (announced 2026, backward-compatible) is
not implemented yet, but a 2.1 asset is handled safely — 2.0 content is supported,
new scene-level content is preserved, and attributes using new 2.1 component types
are kept verbatim rather than corrupted. See [GLTF_2_1.md](GLTF_2_1.md).

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
