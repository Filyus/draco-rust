# Draco Web Converter

The WebAssembly workspace exposes browser-facing helpers for OBJ, PLY, FBX,
and glTF assets. Every module is independently loaded by the static demo in
`www/`.

## glTF modules

- `gltf-inspect-wasm` exports `inspect_gltf(bytes)`. It parses a JSON glTF or
  GLB container with `draco-gltf` and returns document-level counts plus a
  `usesDraco` flag.
- `gltf-canonicalize-wasm` exports `canonicalize_gltf(bytes)`. It validates a JSON
  glTF document and returns its native serialization. Unchanged documents keep
  their original JSON bytes.

Both functions use the same native lossless document model as the Rust API.
They do not resolve companion files or turn scene documents into a flattened
mesh representation.

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
