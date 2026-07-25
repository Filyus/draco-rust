# Draco Web Converter

The WebAssembly workspace exposes independently loaded helpers for OBJ, PLY,
FBX and glTF assets.

## glTF module

`gltf-wasm` is the single browser entry point for glTF. Its default artifact
exports `GltfAsset`: it opens JSON glTF and GLB, resolves an explicit resource
map, applies basic 2.0/2.1 profile checks, preserves or minifies JSON, writes GLB v2/v3,
and reads ordinary or Draco-compressed primitives as `PackedGeometry`.
`PackedGeometry` is constructible from JavaScript for applications that need to
prepare geometry before an optional write build.

The release artifact enables `read` and `draco-decode`: it reads ordinary and
Draco primitives while animations and the rest of the scene remain available
through the lossless document JSON. Feature `accessors` adds generic
`readAccessor` materialization as `PackedAccessor`; use it for animation
channels, skin inverse-bind matrices, morph targets, and other non-primitive
accessors. `write` adds raw `writePrimitive`, `pushPrimitive`,
`fromGeometry`, and JSON bundle output. Add `draco-decode` to a custom write
build for atomic Draco decompression; `draco-encode` includes both and adds
explicit Draco storage.

Feature `strict-validation` adds full cross-reference, node-tree, and
`POSITION`-bounds validation. It is enabled by the converter app and all write
profiles, but excluded from the release reader to keep the common read path
small. Enable it when an application needs to reject malformed scene graphs at
load time.

Feature `raw-resources` adds `bufferCount`, `bufferBytes`, and
`bufferViewBytes` for callers that need resolved binary payloads. These methods
return owned copies and are kept out of the lightweight release artifact. The
converter app profile includes both `accessors` and `raw-resources`.

```sh
cargo run --manifest-path web/build-tool/Cargo.toml -- \
  --module gltf-wasm --features write
cargo run --manifest-path web/build-tool/Cargo.toml -- \
  --module gltf-wasm --features write,draco-encode
cargo run --manifest-path web/build-tool/Cargo.toml -- \
  --module gltf-wasm --features accessors
cargo run --manifest-path web/build-tool/Cargo.toml -- \
  --module gltf-wasm --features strict-validation
```

To deliberately omit the decoder in a custom raw-only build, invoke the
underlying WASM build with `--no-default-features --features write`.

For an application that only needs document inspection and lossless
serialization, build the same module with `--no-default-features`; this is a
custom profile rather than a separate public package.

Reference optimized sizes from the 2026-07-19 Windows stable toolchain build:

| glTF build | raw WASM | gzip |
| --- | ---: | ---: |
| reader + Draco decode | 251,460 B | 105,075 B |
| reader + Draco decode + accessors | 255,669 B | 106,631 B |
| reader + Draco decode + strict validation | 295,370 B | 129,515 B |
| converter app (`accessors`, `strict-validation`, `draco-encode`, `raw-resources`) | 464,791 B | 194,719 B |

The released reader is 102.7 KiB gzip, within the 112 KiB budget. Optional and
converter sizes are informational because those features are not included in
the release asset.

## Converter preview

The bundled converter includes a dependency-free WebGL2 preview. It loads raw
and Draco primitives through `gltf-wasm`, renders base-color materials and
textures, and plays glTF node and skin animations with timeline controls. It
also previews the flat OBJ, PLY, and FBX meshes returned by their WASM modules.
The preview is intentionally a diagnostic renderer, not a replacement for a
full PBR glTF runtime: unsupported material and texture extensions are reported
as warnings instead of changing exported assets.

### Source-neutral scene conversion

Loaded glTF/GLB and semantic FBX inputs are normalized into a serializable
`SceneDocument` for cross-format export and diagnostics. The document carries
resources and MIME types, hierarchy/TRS, mesh primitives and material slots,
PBR-compatible materials/textures, skins/inverse bind matrices, morph targets,
and seconds-based TRS/weight clips. Extra UV sets, vertex colors, tangents, and
two four-influence sets (up to eight influences per vertex) survive the
SceneDocument → GLB path and the typed FBX writer where the target format can
represent them. The viewer keeps its hardware limits local: it renders the
first four influences and reports that limitation while export data remains
intact.

FBX and glTF parsing/adaptation remain separate at their format boundaries,
but converge on this shared document for the viewer's scene summary and GLB
export. Direct glTF/GLB document-preserving import remains the lossless
same-format path; FBX export uses the typed semantic writer and optional FBX
transform-stack provenance. The UI exposes a collapsible hierarchy tree after
load, clip selection, and capability/warning reports beside import/export
controls. Verified controls are `mixamo.fbx`, `Samba Dancing.fbx`, and the
Fox glTF fixture, including FBX → GLB → reload and typed-FBX round trips.

Current limitations are explicit: FBX vertex colors/tangents are retained in
SceneDocument/glTF but are warned as unsupported by the current typed FBX
writer; arbitrary FBX camera/light/non-TRS semantics are outside the portable
subset; and non-default `RotationOrder`/`InheritType` transform-stack behavior
remains unvalidated beyond the verified fixtures.

## Build and test

`build.ps1` defaults to the interactive converter profile: the format modules
use their normal features and `gltf-wasm` additionally enables
`accessors`, `strict-validation`, `draco-encode`, and `raw-resources`. Use `-ReleaseProfile` when
reproducing the lightweight glTF release artifact and its size budget.

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
