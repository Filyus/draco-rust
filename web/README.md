# Draco Web Converter

The WebAssembly workspace exposes browser-facing helpers for OBJ, PLY, FBX,
and glTF assets. Every module is independently loaded by the static demo in
`www/`.

## glTF modules

- `gltf-document-wasm` exports `GltfDocument`, a stateful lossless document
  handle. It parses JSON glTF or GLB, exposes lossless JSON and root-object
  access, validates 2.0/2.1 profiles, serializes GLB v2/v3, and runs
  Draco compression/decompression. `inspect_gltf(bytes)` remains a small
  summary convenience function.
- `gltf-canonicalize-wasm` exports `canonicalize_gltf(bytes)`. It validates a JSON
  glTF document and returns its canonical serialization. Unchanged documents keep
  their original JSON bytes.

Both functions use the same lossless document model as the Rust API.
They do not resolve companion files or turn scene documents into a flattened
mesh representation.

`gltf-compact-wasm` is the small geometry runtime. `CompactGeometry` opens JSON
glTF or GLB v2/v3, accepts an explicit URI-to-`Uint8Array` resource map, and
decodes ordinary or `KHR_draco_mesh_compression` primitives. Decoding returns a
`PackedGeometry` handle: metadata is queried per attribute and payloads cross
the boundary as `Uint8Array`, never as expanded JavaScript number arrays.

The compact name describes the constrained runtime surface and build footprint;
`PackedGeometry` describes the materialized contiguous geometry buffers.

## Build and test

```sh
cargo run --manifest-path web/build-tool/Cargo.toml --
cargo test --manifest-path web/Cargo.toml --workspace
npm install --prefix web
npm run --prefix web test:node
```

Optimized release builds use `wasm-opt` when it is available. The generated
packages are written to `web/www/pkg/`; serve `web/www/` with any static file
server to use the demo.
