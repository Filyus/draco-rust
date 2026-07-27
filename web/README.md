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

### KTX2 textures

`KHR_texture_basisu` points a texture at a KTX2 file holding Basis Universal
data, which no GPU and no browser image decoder reads directly. `ktx2-wasm`
transcodes it, and the preview fetches that module the first time a file turns
out to need it — it carries baked conversion tables and is several times the
size of the others, so a page that never meets a KTX2 texture never loads it.

Both Basis codecs are decoded: ETC1S, which most glTF assets use, and UASTC
LDR, including Zstd supercompression. Each is a Rust port of Binomial's
transcoder and is gated byte for byte against Binomial's own build, every mip
level of every fixture.

What the texture is turned into depends on the machine, and the choice is made
per texture:

| source | target | needs |
|---|---|---|
| ETC1S, no alpha | BC1, 8 bytes per block | `WEBGL_compressed_texture_s3tc` |
| ETC1S with alpha | BC3, 16 bytes per block | `WEBGL_compressed_texture_s3tc` |
| ETC1S, no alpha | ETC1, 8 bytes per block | `WEBGL_compressed_texture_etc` |
| ETC1S with alpha | ETC2, 16 bytes per block | `WEBGL_compressed_texture_etc` |
| UASTC | BC7, 16 bytes per block | `EXT_texture_compression_bptc` |
| UASTC | ASTC 4×4, 16 bytes per block | `WEBGL_compressed_texture_astc` |

A texture that reaches one of those is uploaded compressed, at about an eighth
of the video memory the pixels would take, and its mip chain comes from the
file because a compressed texture cannot have mips generated for it. A context
offering none of them decodes to RGBA8 and uploads as an ordinary image.

The two families barely overlap, which is why this is a ranking rather than a
constant. Published survey figures:

|      | Windows | macOS | Android |  iOS |
|------|--------:|------:|--------:|-----:|
| s3tc |   99.9% | 88.1% |   28.6% | 39.8% |
| ETC2 |    2.1% | 88.0% |   99.9% |  100% |
| ASTC |    2.1% | 88.0% |   99.9% |  100% |

So a desktop takes the BC path and a phone the ETC or ASTC one, and the phone —
the machine that can least afford eight times the video memory — is the reason
the second half of that table is worth transcoding to at all.

Each of those three families is a Cargo feature, so a build can carry only the
one its machines have. The axis is deliberate: a target ages out when the
hardware taking it does, and hardware ages by family — BC1 and BC3 arrive and
leave together, so a flag per target would cut where nothing ever changes.
Measured, gzipped, built with `--no-default-features`:

| built for | module |
|---|--:|
| every family (what is served) | 129 KiB |
| `bc` — desktop | 123 KiB |
| `etc,astc` — phones | 55 KiB |
| `astc` alone | 51 KiB |

Almost all of the weight is the baked ETC1S-to-BC endpoint tables, which is why
dropping the desktop family more than halves the module and dropping the mobile
ones barely registers. Nothing about the container or either codec is optional:
a KTX2 file is read and decoded to pixels whatever the module was built for.

Two more flags sit over those and say not what a target is but why it is still
here:

- `modern` — what hardware sold today takes: `bc` and `astc` both, since being
  current is not the same as being one kind of machine. s3tc is on 99.9% of
  Windows, ASTC on essentially every phone.
- `legacy` — what is carried only for hardware with no current alternative.
  Today that is `etc` and nothing else: every machine with ASTC also has ETC,
  so the family serves precisely the devices that have one and not the other.
  On Android that difference is nil, as the table above shows; where it exists
  is Linux, at 93.9% ETC against 87.6% ASTC. `bc` is old by date and nowhere
  near legacy by use.

Cargo features add and never subtract, so `legacy` cannot be a flag the default
set turns off; it is *in* the default set, and retiring the family means
deleting that one word. CI builds both ways, so it stays a one-line edit rather
than a day's work when the figures say it is time. Doing it today would save
4 KiB — which is the point: the reason to drop it will be that it serves nobody,
not that it costs anything.

Two gaps are worth stating. Five of UASTC's nineteen block modes appear in none
of the fixtures, so the transcoder is written from the reference for those and
verified by nothing; the UASTC gate names them rather than leaving it unsaid.
And the ETC and ASTC uploads cannot be exercised on a desktop, which offers
neither — their transcoding is checked byte for byte against the reference in
Node, but the `compressedTexImage2D` call itself is only covered where the
browser has the extension.

Exported glTF and GLB carry the KTX2 bytes through unchanged either way; OBJ,
PLY and FBX carry them too, and no importer of those formats can read them,
which is what the extension report and the FBX export warning say.

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

Both FBX containers load through the same WASM reader: an ASCII document takes
the same path as a binary one and arrives with its transforms, materials, skins
and animation, where the previous regex fallback recovered geometry only.

Vertex colours travel the whole path, arriving as `COLOR_0` and written back as
`LayerElementColor`; every UV layer reaches `TEXCOORD_0`..`TEXCOORD_7`; and
tangents arrive as `TANGENT`, with the handedness sign in `w`, and are written
back split across `Tangents` and `TangentsW`.

Current limitations are explicit. Binormals, hard edges and crease weights have
no glTF equivalent, so they survive only on the FBX provenance path and are
dropped when a document is lowered through the portable form. Cameras, lights
and non-TRS FBX semantics are outside the portable subset entirely: `draco-io`
reads cameras and lights, but SceneDocument has no node payload for them.
Non-default `RotationOrder`/`InheritType` transform-stack behaviour remains
unvalidated beyond the verified fixtures.

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

The build tool itself — and therefore `build.sh`, which passes its arguments
straight through — defaults to the lightweight release profile, the opposite of
`build.ps1`. Pass `--app` to select the converter profile directly; `--serve`
implies it, because the release profile omits `readAccessor` and
`bufferViewBytes` and the front-end calls both on every skinned, animated or
morphed asset.

```sh
cargo run --manifest-path web/build-tool/Cargo.toml --
cargo run --manifest-path web/build-tool/Cargo.toml -- --app
cargo test --manifest-path web/Cargo.toml --workspace
npm install --prefix web
npm run --prefix web test:node
```

The gzip budget is enforced only on the release profile, since that is the
artifact it describes; a build carrying features reports its size instead. So
`bash ./build.sh` stays the budget gate, and anything that runs the front-end —
the Node scene gates included — needs `--app` built over it afterwards.

Optimized packages are written to `web/www/pkg/`.

The front-end is TypeScript in `web/src/`, compiled by `tsc` into `web/www/`
as plain ES modules — no bundler, and `index.html` still loads a single
`app.js`. `web/www/` therefore holds only build output and the two tracked
static files (`index.html`, `style.css`). Both `build.ps1` and `build.sh` run
that compile for you; the raw `cargo run` build-tool invocations above do not,
so run it yourself when using them:

```sh
npm run --prefix web build:ts     # once
npm run --prefix web watch:ts     # while working on the front-end
npm run --prefix web typecheck
```

Node 24 executes TypeScript directly, so `test:node` and the other Node suites
import `web/src/*.ts` and need no build step. Only the browser — and therefore
`test:browser` — consumes the compiled output.
