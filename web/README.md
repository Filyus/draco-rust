# Draco Web Converter

The WebAssembly workspace exposes independently loaded helpers for OBJ, PLY,
FBX and glTF assets.

## glTF modules

`gltf-document-wasm` exports `GltfDocument`, the full lossless scene API. Its
default artifact parses JSON glTF and GLB, validates 2.0/2.1 profiles, returns
source-preserving or minified JSON, serializes GLB v2/v3, and loads explicit
resources. Feature `write` adds atomic Draco decompression; `draco-encode`
adds document-preserving Draco compression.

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

The document artifact follows the same codec split:

```sh
cargo run --manifest-path web/build-tool/Cargo.toml -- \
  --module gltf-document-wasm --features write
cargo run --manifest-path web/build-tool/Cargo.toml -- \
  --module gltf-document-wasm --features write,draco-encode
```

Reference optimized sizes from the 2026-07-18 Windows stable toolchain build:

| compact build | raw WASM | gzip |
| --- | ---: | ---: |
| reader + Draco decode | 250,419 B | 102,989 B |
| reader + raw write | 275,854 B | 113,322 B |
| reader + raw/Draco write | 414,613 B | 164,985 B |

The released reader is 100.6 KiB gzip, within the 112 KiB budget and 5.0% above
the previous 95.8 KiB reference. Writer sizes are informational because those
features are not included in the release asset.

| document build | raw WASM | gzip |
| --- | ---: | ---: |
| document read/save | 114,311 B | 48,946 B |
| document + `write` | 275,495 B | 113,804 B |
| document + `draco-encode` | 413,032 B | 165,133 B |

The released document artifact is 47.8 KiB gzip, within its 56 KiB budget.

## Build and test

```sh
cargo run --manifest-path web/build-tool/Cargo.toml --
cargo test --manifest-path web/Cargo.toml --workspace
npm install --prefix web
npm run --prefix web test:node
```

Optimized packages are written to `web/www/pkg/`.
