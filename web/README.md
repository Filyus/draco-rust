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

The release artifact enables `read` and `draco-decode`: it reads ordinary and
Draco primitives. `write` adds raw `writePrimitive`, `pushPrimitive`,
`fromGeometry`, and JSON bundle output. Add `draco-decode` to a custom write
build for atomic Draco decompression; `draco-encode` includes both and adds
explicit Draco storage.

```sh
cargo run --manifest-path web/build-tool/Cargo.toml -- \
  --module gltf-wasm --features write
cargo run --manifest-path web/build-tool/Cargo.toml -- \
  --module gltf-wasm --features write,draco-encode
```

To deliberately omit the decoder in a custom raw-only build, invoke the
underlying WASM build with `--no-default-features --features write`.

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

`build.ps1` defaults to the interactive converter profile: the format modules
use their normal features and `gltf-wasm` additionally enables
`draco-encode`. Use `-ReleaseProfile` when reproducing the lightweight glTF
release artifact and its size budget.

```powershell
./build.ps1 -Serve
./build.ps1 -ReleaseProfile
./build.ps1 -ReleaseProfile -Modules gltf-wasm -Features write,draco-encode
```

The build tool itself defaults to the lightweight release profile. Pass
`--app` to select the converter profile directly.

```sh
cargo run --manifest-path web/build-tool/Cargo.toml --
cargo run --manifest-path web/build-tool/Cargo.toml -- --app
cargo test --manifest-path web/Cargo.toml --workspace
npm install --prefix web
npm run --prefix web test:node
```

Optimized packages are written to `web/www/pkg/`.
