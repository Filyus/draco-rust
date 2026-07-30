# Draco Web Converter

The WebAssembly workspace exposes independently loaded helpers for OBJ, PLY,
STL, standalone Draco (`.drc`), FBX and glTF assets.

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

## STL and standalone Draco

`stl-wasm` and `drc-wasm` are the two smallest modules and the same shape as
`obj-wasm` and `ply-wasm`: `parse_*_bytes` in, `create_*` out, flat triangle
meshes on both sides. Each splits into `read` and `write` features, so a build
that only opens files carries no writer.

STL is read from both containers and written as binary. Which container a file
is in follows from its length, not from the leading keyword — exporters write
`solid` into the 80 free header bytes of binary files. Its meshes keep STL's own
shape, three unshared vertices per triangle with the facet normal on each
corner, so a cube arrives as 36 vertices rather than 24.

`.drc` is the standalone Draco container, the one `draco_encoder` writes, and it
is a different route from Draco inside glTF: there the payload is a
`KHR_draco_mesh_compression` extension and `gltf-wasm` owns it. The export
panel's quantization controls reach both, because both end at the same encoder.

A source that arrives as a bare mesh list — OBJ, PLY, STL and `.drc` — reaches
glTF through the same portable document FBX uses: `buildSceneDocumentFromMeshes`
turns the list into a `SceneDocument`, which the GLB writer already knows how to
serialize, Draco pass included. OBJ's material library comes with it, textures
embedded, so the GLB is self-contained. JSON glTF is that GLB rewritten with its
binary as a `data:` buffer URI, which is what the embedded profile is for; a
`.gltf` source that is not being compressed is passed through as itself instead,
since nothing rebuilt could be closer to what was opened.

The other direction — a document down to a mesh list — is `flattenSceneDocument`,
and every flat target takes it. Node transforms are baked into the coordinates
there, because OBJ, PLY, STL and `.drc` have nowhere to put a hierarchy and a
scene that keeps its local coordinates collapses onto the origin.

`browser-smoke.spec.ts` walks every source against every target and validates
the glTF outputs with Khronos's validator. The gaps it was written for were not
visible one conversion at a time.

## FBX units

`UnitScaleFactor` is the number of centimetres in one FBX unit, and FBX's base
unit is the centimetre: a file saying 1 is in centimetres, one saying 100 is in
metres. The importer reads the field rather than assuming, which it did not
before -- it applied a flat 0.01, right for the first case and wrong for the
second, and this workspace's own writer emits 100. So a scene written here and
read back arrived a hundred times too small, and so did the GLB it became.

The four real-world fixtures on hand -- Mixamo, Samba Dancing, `morph_test` and
the Stanford bunny -- all state 1, which is why the constant went unnoticed and
why honouring the field moves none of them.

## FBX space

Which space an FBX export is written in is a choice, because FBX declares its
own rather than fixing one. `meters-y-up` is the default: glTF's own axes and
metres, which makes the conversion the identity, the round trip exact, and the
file look like every FBX on hand from another tool. `meters-z-up` writes the
Z-up convention a great deal of existing FBX uses.

Either way the file states the space it is in, and the importer reads that
statement rather than assuming — one `FbxSpace` drives the conversion and the
declaration together, so they cannot disagree. A re-export follows its source's
declared space instead of the option, which is what makes it a round trip rather
than a conversion.

## FBX texture coordinates

FBX puts V's origin at the opposite end of the image from glTF, so every
crossing turns it: the writer on the way out, the document importer on the way
in, and the preview on its own way in — it reads the FBX reader directly rather
than through the document. All three used to be one: only the writer turned it,
so a glTF exported to FBX and read back came home upside down, and a textured FBX
previewed mirrored against the GLB exported from the same file.

A re-export is the exception, and not one: it puts back the coordinates it read,
which have not been turned.

## FBX axes

The other six `GlobalSettings` fields say which axis is up, which points front
and which points right, each with a sign. glTF fixes all three — `+Y` up, `+Z`
front, `+X` right — so reading an FBX means mapping one system onto the other,
and the importer used to skip that entirely. Correct for exactly the files that
are already Y-up, which is all four fixtures, and wrong for anything else.

The change of basis reaches everything a basis touches: positions and normals
rotate, tangents rotate while their handedness stays, node matrices and skin
binds are conjugated (`B · M · B⁻¹`, because a transform has to receive and
return glTF-space points), animated rotations are conjugated as quaternions, and
per-axis scale permutes rather than rotates. Conjugation is linear, which is what
lets a cubic sampler's tangents go through the same call as its keys.

A Y-up file takes none of that: the basis reports itself as the identity and each
site returns its input untouched, rather than passing it through arithmetic that
ought to cancel out.

The writer's side of the same round trip was also wrong, and in the opposite
direction: its axis change writes glTF `(x, y, z)` as `(x, z, -y)`, which puts up
along FBX `-Z`, while its default `UpAxisSign` and `FrontAxisSign` claimed the
reverse. The file described an orientation it did not contain, so believing it —
which the importer now does — turned the scene over. Fixed in `draco-io`; the
geometry bytes are unchanged.

Draco puts no limit on how many attributes of a type a payload holds -- a second
texture-coordinate set is ordinary, and glTF's own extension keeps joints and
weights as generics -- while the flat mesh the shell works in has one slot per
named type and none for generics. The rest are carried rather than dropped: the
reader hands them over whole, with their type, component count, component type
and the id a consumer addresses them by, and the writer puts them back
unchanged. Nothing in between reads their meaning, which is what makes it safe,
and the file says so -- each one is reported as carried but uninterpreted.

Where another format has a name for one, it gets it: a second texture-coordinate
or colour set becomes `TEXCOORD_1` or `COLOR_1` on the way into glTF. A generic
does not. glTF's only home for one is an application-specific `_NAME`, and the
name would be this converter's invention rather than anything the payload
stated, so it is reported as left behind instead.

## Converter preview

The bundled converter includes a dependency-free WebGL2 preview. It loads raw
and Draco primitives through `gltf-wasm`, shades metallic-roughness materials
with image-based lighting and punctual lights, and plays glTF node and skin
animations with timeline controls. It also previews the flat meshes their own
WASM modules return, for OBJ, PLY, STL, `.drc` and FBX. Whatever it cannot
honour is reported as a warning rather than changed in the exported asset: the
preview never writes back to the document it is showing.

Shading happens in linear light throughout. The scene is drawn into a
half-float frame with multisampling, and one output pass does the whole display
transform — glare as a threshold-free pyramid, exposure, hue-preserving tone
mapping, sRGB encode. Everything that averages pixels (resolve, mip chains,
blur) therefore happens on radiance rather than on encoded values, which is the
difference between a light that blooms white and one that blooms yellow.

### Extensions the preview honours

Eleven material layers, read from one table that also defines how each is
shaded, so the list cannot claim a layer the renderer ignores
(`src/material-extensions.ts`):

| extension | what it adds |
|---|---|
| `KHR_materials_ior` | the dielectric reflectance, feeding specular and refraction |
| `KHR_materials_specular` | that reflectance tinted and scaled |
| `KHR_materials_transmission` | what passes through the surface, from a capture of the frame |
| `KHR_materials_volume` | thickness, attenuation colour and distance, over the ray inside |
| `KHR_materials_dispersion` | one index per wavelength instead of one for all |
| `KHR_materials_anisotropy` | a specular lobe stretched along a tangent |
| `KHR_materials_iridescence` | a thin film over the specular lobe |
| `KHR_materials_sheen` | a retroreflective lobe for cloth, over the base layer |
| `KHR_materials_clearcoat` | a second specular lobe over the whole material |
| `KHR_materials_emissive_strength` | emission past the 0..1 the factor allows |
| `KHR_materials_unlit` | flat shading, skipping all of the above |

Four more are read where they live rather than on the material:
`KHR_texture_transform` on every texture binding, not only base colour;
`KHR_lights_punctual`; `KHR_materials_variants`; `EXT_mesh_gpu_instancing`.

Four arrive already resolved by the Rust reader, so nothing is lost and none is
reported as ignored: `KHR_draco_mesh_compression`, `EXT_meshopt_compression`
(and the pre-ratification spelling `KHR_meshopt_compression`), and
`KHR_mesh_quantization`.

Three name an alternate image source and are honoured conditionally —
`KHR_texture_basisu`, `EXT_texture_webp` and `EXT_texture_avif`, described in
their own sections below. The document always carries their bytes; the preview
claims one only when the browser actually decoded every image that came through
it.

Anything outside those lists reaches the extension report as ignored. The
notable absences today are `KHR_materials_diffuse_transmission`,
`KHR_animation_pointer`, `KHR_node_visibility`, `KHR_xmp_json_ld`, and
`EXT_structural_metadata` — the last survives an export as an opaque block
without being interpreted.

### What refraction can and cannot see

Transmission reads a capture of the frame, so it shows what is genuinely behind
the surface on screen and nothing else. Beyond that the honest statement is a
list of absences: geometry that is off-screen or occluded, refraction at the
exit boundary, per-pixel volume depth, total internal reflection at the far
side, more than one internal bounce, and caustics. A ray leaving the frame
clamps at the border texel, the way the WebGL references do.

The frame is drawn wider than it is shown to push that boundary outwards — the
extra texels hold more scene, and the output pass shows the middle. The width
is one constant, `GUARD_BAND` at the top of `src/viewer/scene-target.ts`; a
fifth of the frame per side costs about ninety per cent more fill and covers
refraction through anything of ordinary thickness. Zero draws only what is
shown.

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
| ETC1S | ASTC 4×4, 16 bytes per block | `WEBGL_compressed_texture_astc` |
| UASTC | ETC1, 8 bytes per block | `WEBGL_compressed_texture_etc` |
| UASTC | ETC2, 16 bytes per block | `WEBGL_compressed_texture_etc` |
| UASTC | BC7, 16 bytes per block | `EXT_texture_compression_bptc` |
| UASTC | ASTC 4×4, 16 bytes per block | `WEBGL_compressed_texture_astc` |

Every pair either codec can reach is there, so what a machine takes is decided
by its extensions alone. Where a codec reaches two targets the ranking prefers
the one that loses less, and that is not the same answer for both: ASTC is what
UASTC is a restricted profile of, so it wins there, while for ETC1S it has to
solve four colours into two endpoints and lands below ETC1, which is nearly
lossless and half the size.

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
| every family (what is served) | 167 KiB |
| `bc` — desktop | 123 KiB |
| `etc,astc` — phones | 93 KiB |
| `astc` alone | 86 KiB |
| `etc` alone | 56 KiB |

Almost all of the weight is baked tables of solved endpoints, one set for BC and
one for ASTC, at about 60 KiB each; ETC needs none, which is why it is the
cheapest family by a wide margin. Nothing about the container or either codec is
optional: a KTX2 file is read and decoded to pixels whatever the module was
built for.

`etc` is now the one that could be retired, which it was not before this table
was full: every machine with ASTC also has ETC, and both codecs now reach ASTC,
so dropping the family costs the difference between the two — nil on Android,
about six points on Linux — rather than every ETC1S texture on every phone.
What it would cost in quality is the ETC1S half: ASTC lands below ETC1 there and
takes twice the space. That is the trade, and it is a decision rather than a
formality, which is why there is no flag pretending otherwise.

Five of UASTC's nineteen block modes appear in none of the fixtures — an
encoder picks modes by what compresses the image in front of it, and a natural
photograph never reaches for a three-subset block. Rather than wait for an asset
that happens to use them, the gate builds them: a UASTC block has no checksum
and no redundancy, so any 128 bits whose leading code names a mode and whose
pattern index exists is a legal block of that mode, and random bits with those
two fields fixed are a fair sample. They go into a real container, by rebuilding
a fixture's levels around them, and the reference is still the oracle.

One gap is left, and it is about the upload rather than the transcode: ETC and
ASTC cannot be exercised on a desktop, which offers neither. Their transcoding
is checked byte for byte against the reference in Node, but the
`compressedTexImage2D` call itself is only covered where the browser has the
extension.

### WebP and AVIF textures

`EXT_texture_webp` and `EXT_texture_avif` need no transcoder: the browser
decodes both, so what this has to get right is naming the source, carrying the
bytes, and saying whether the decode happened. The decode path never learns
either codec's name — it hands a MIME type to `createImageBitmap` and that is
the whole of it, which is why the second codec cost a list entry rather than an
implementation.

WebCodecs' `ImageDecoder` would be the wrong instrument here despite looking
like the specialized one: it reached Firefox in 130 and Safari in 26, where
AVIF in `createImageBitmap` has been available since Firefox 93 and Safari
16.4. The general API has the wider reach.

No JPEG or PNG fallback is written beside either source, and neither extension
is marked optional, because a reader that skips it finds a texture with no
source at all. glTF 2.1 promotes WebP to guaranteed support, which removes the
expectation of a fallback that was never emitted here anyway. AVIF is a draft
extension rather than a ratified one, and is honoured on the same terms: what
decides is the browser in front of it, which is what `honoredTextureSources`
reports.

Both save transfer and not video memory — an AVIF is RGBA8 by the time it
reaches the GPU, exactly like a PNG. `KHR_texture_basisu` is the one that
changes what the texture costs once uploaded, so these are alternatives to
JPEG, not to KTX2.

The round-trip gate carries a real file of each — 66 bytes of WebP and 354 of
AVIF, both the same four flat quadrants — and checks that content sniffing
reaches the same extension the declared type does, which is the half a writer
and a reader can disagree about over a file nobody looked inside. AVIF is
sniffed by its brand rather than by `ftyp` alone: HEIC is the same container.

The decode is checked too, and it needs a browser, so it lives in the Playwright
gate: each fixture is loaded, the decoded bitmap is drawn to a canvas, and every
quadrant is read back and compared against the colours the fixture was written
with — exactly, because both fixtures are lossless. That is the whole of either
path — declared, carried, sniffed, decoded, uploaded — with nothing left
standing on reasoning alone.

Exported glTF and GLB carry the KTX2 bytes through unchanged either way; OBJ,
PLY and FBX carry them too, and no importer of those formats can read them,
which is what the extension report and the FBX export warning say.

### Opening a folder

A model whose companions sit in a sibling directory cannot be selected file by
file — three.js's GPU-instanced Damaged Helmet lives in `glTF-instancing/` and
names `../glTF/DamagedHelmet.bin`, which no multi-file picker can reach. So a
folder can be dropped, and the drop decides: a file opens a file, a folder
becomes the selection.

Nothing in a folder is read for being in it. The chosen model is opened first,
it says which URIs it needs, and only those are fetched — a `File` is a handle,
so a folder of ten thousand costs ten thousand names and the size of the model.
OBJ takes two rounds rather than one, because the model names material
libraries and the libraries name the textures.

Companions are keyed by the URI exactly as the document wrote it, which is what
the resolver looks up. Keyed by bare filename, as they were, a document naming
`textures/wood.png` never found its own image however it was supplied, and two
files of the same name in different folders collided.

When a selection holds more than one model — the usual case for a folder, since
Khronos ships each asset as `glTF/`, `glTF-Binary/`, `glTF-Draco/` and
`glTF-KTX-BasisU/` — the import panel grows a picker listing them by the part of
the path that differs, shortest first. One model and it stays hidden, so
opening a single file looks exactly as it did. The picker also switches models
after the fact, which is the operation worth having: comparing a Draco variant
against the plain one is what a converter is for.

The button remains file-only: `webkitdirectory` would make it folder-only, and
a second button beside it buys less than the drop already gives.

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

## Deployment

`Pages: deploy converter` (`.github/workflows/pages.yml`) builds `--app` and
publishes `web/www` to GitHub Pages on every push to `main` that touches
`crates/**` or `web/**`, and on demand through `workflow_dispatch`. It requires
Pages to be enabled with **GitHub Actions** as the source; the workflow cannot
turn it on itself, because creating the site is a repository setting that
`GITHUB_TOKEN` may not change. There is
nothing to configure for a subdirectory URL: `index.html` references `app.js`
and `style.css` relatively, and [`src/app/modules.ts`](src/app/modules.ts)
resolves every WASM package against `document.baseURI`, so the site works
unchanged at a repository path.

The deploy is deliberately not tied to a crate tag. It shows what `main` does,
while [`Release: WASM assets`](../.github/workflows/release.yml) is what carries
a version: it builds the release profile from each `draco-io` and `draco-gltf`
tag and attaches the per-module zips to that GitHub Release.

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
