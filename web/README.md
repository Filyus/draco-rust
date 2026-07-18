# Draco Web Converter

The WebAssembly workspace exposes independently loaded helpers for OBJ, PLY,
FBX and glTF assets.

## glTF module

`gltf-wasm` is the single browser entry point for glTF. Its default artifact
exports `GltfAsset`: it opens JSON glTF and GLB, resolves an explicit resource
map, validates 2.0/2.1 profiles, preserves or minifies JSON, writes GLB v2/v3,
and reads ordinary or Draco-compressed primitives as `PackedGeometry`.
`PackedGeometry` is constructible from JavaScript for applications that need to
prepare geometry before an optional write build.

The release artifact is read-only. Feature `write` adds raw
`writePrimitive`, `pushPrimitive`, `fromGeometry`, JSON bundle output, and
atomic Draco decompression. `draco-encode` adds explicit Draco storage.

```sh
cargo run --manifest-path web/build-tool/Cargo.toml -- \
  --module gltf-wasm --features write
cargo run --manifest-path web/build-tool/Cargo.toml -- \
  --module gltf-wasm --features write,draco-encode
```

For an application that only needs document inspection and lossless
serialization, build the same module with `--no-default-features`; this is a
custom profile rather than a separate public package.

Reference optimized sizes from the 2026-07-18 Windows stable toolchain build:

| glTF build | raw WASM | gzip |
| --- | ---: | ---: |
| reader + Draco decode | 267,030 B | 109,818 B |
| reader + raw write | 288,199 B | 118,367 B |
| reader + raw/Draco write | 426,358 B | 170,191 B |

The released reader is 107.2 KiB gzip, within the 112 KiB budget. Writer sizes
are informational because those features are not included in the release asset.

## Build and test

```sh
cargo run --manifest-path web/build-tool/Cargo.toml --
cargo test --manifest-path web/Cargo.toml --workspace
npm install --prefix web
npm run --prefix web test:node
```

Optimized packages are written to `web/www/pkg/`.
