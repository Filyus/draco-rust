# Draco Web Converter

The WebAssembly workspace exposes independently loaded helpers for OBJ, PLY,
FBX and glTF assets.

## glTF modules

`gltf-document-wasm` exports `GltfDocument`, the full lossless scene API. It
parses JSON glTF and GLB, validates 2.0/2.1 profiles, returns source-preserving
or minified JSON, serializes GLB v2/v3, loads explicit resources, and performs
Draco compression and decompression.

`gltf-compact-wasm` exports `CompactDocument` and `PackedGeometry`. The default
artifact reads ordinary or Draco-compressed primitives into contiguous
`Uint8Array` payloads. `PackedGeometry` is also constructible from JavaScript.
Optional feature `write` adds raw `writePrimitive`, `pushPrimitive`,
`fromGeometry`, JSON bundle and GLB output; `draco-encode` adds explicit Draco
storage.

The released compact artifact remains reader-only. Build optional writers from
the same crate and API:

```sh
cargo run --manifest-path web/build-tool/Cargo.toml -- \
  --module gltf-compact-wasm --features write
cargo run --manifest-path web/build-tool/Cargo.toml -- \
  --module gltf-compact-wasm --features write,draco-encode
```

Reference optimized sizes from the 2026-07-18 Windows stable toolchain build:

| compact build | raw WASM | gzip |
| --- | ---: | ---: |
| reader + Draco decode | 250,419 B | 102,989 B |
| reader + raw write | 275,854 B | 113,322 B |
| reader + raw/Draco write | 414,613 B | 164,985 B |

The released reader is 100.6 KiB gzip, within the 112 KiB budget and 5.0% above
the previous 95.8 KiB reference. Writer sizes are informational because those
features are not included in the release asset. The full document artifact is
161.3 KiB gzip; its budget is 164 KiB after adding validated raw geometry output.

## Build and test

```sh
cargo run --manifest-path web/build-tool/Cargo.toml --
cargo test --manifest-path web/Cargo.toml --workspace
npm install --prefix web
npm run --prefix web test:node
```

Optimized packages are written to `web/www/pkg/`.
