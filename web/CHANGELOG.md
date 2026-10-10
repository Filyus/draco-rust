# Web converter changelog

The converter and its WASM wrappers are not published to crates.io. The
converter deploys to GitHub Pages from `main`; the wrappers shipped as zipped
assets on crate releases until 2026-10-10 and now ship as `@draco-rust/*` npm
packages, all at the one version in `web/npm/VERSION`. This file records what
has changed in them; each package version has a section of its own.

## Unreleased

- **`@draco-rust/decoder` loads glTF's Draco meshes for three.js, on workers.**
  - `decode_draco` returns attributes by unique id or by type, in the typed
    array a loader asks for. Its conversions match upstream Draco 1.5.7's byte
    for byte, over every fixture and every array type.
  - `createDecoderPool` from `./pool` decodes on Web Workers or Node worker
    threads, from a bundle or from a CDN.
  - `createDracoLoader(THREE)` from `./three` stands in for three.js's
    `DRACOLoader` in `GLTFLoader` and for `.drc` files, without importing
    three. Against three 0.186's own, over the Draco glTF and `.drc`
    fixtures, it builds the same geometry byte for byte, padding included; an
    attribute that does not convert to the requested type is an error rather
    than whatever DRACOLoader's failed conversion left in memory.
  - Each entry has its own pool and loader, and the entries grow by about
    5 KiB of gzip each.
- **`@draco-rust/gltf` reads a value by JSON Pointer** with
  `asset.jsonAt("/nodes/0/translation")`.

- **KTX2 textures decode faster in the converter.** The transcoder in
  `ktx2-wasm` is now built for speed rather than size, as the Draco codec and
  the format readers already were: UASTC textures decode up to 2.4 times as
  fast and the fixtures 13% faster overall, for 9 kB of the module's gzip.

## [0.1.1] - 2026-10-10

- **Published from CI, with provenance.** The packages are built and published
  by `npm.yml` through npm trusted publishing, so each version on npmjs.com
  links to the commit and the workflow run that produced it.
- `@draco-rust/fbx` no longer lets a repeated connection multiply the scene:
  repeats of one edge tagged with stray strings each counted as a new edge,
  and a 3.8 kB file could decode to fourteen thousand morph targets. The other
  packages are unchanged from 0.1.0.

## [0.1.0] - 2026-10-10

- **The modules are packaged for npm as `@draco-rust/*`.**
  - `@draco-rust/decoder` decodes Draco: `.` reads meshes and point clouds
    (87 KiB gzip), `./mesh` meshes only (72 KiB, all glTF allows),
    `./point-cloud` point clouds only (58 KiB), and `./legacy` adds bitstreams
    older than 2.2 (97 KiB).
  - `@draco-rust/encoder` writes Draco (133 KiB).
  - `@draco-rust/gltf` reads glTF with any accessor (123 KiB), `./validate`
    adds strict validation (152 KiB), and `./writer` adds writing and Draco
    compression (261 KiB).
  - `@draco-rust/fbx` reads and writes FBX (234 KiB).
  - `@draco-rust/obj` (42 KiB), `@draco-rust/ply` (89 KiB) and
    `@draco-rust/stl` (48 KiB) read and write their formats. PLY keeps every
    property, so a Gaussian splat arrives whole.

  Every entry works from a bundler, from a CDN such as jsDelivr, and in Node.
  `drc-wasm` gained the `point-cloud-only` feature for the point-cloud entry.
  `create_ply` writes binary little-endian unless `format` says otherwise, as
  a `Uint8Array` in `binary_data`; `format: "ascii"` still returns text in
  `data`. The converter names its format and is unchanged.

- **The modules no longer ship as zips on crate releases.** The 148 zipped
  modules attached to 21 crate releases were removed, and releases attach none
  from now on; the modules are to be published as `@draco-rust/*` npm packages,
  split by task rather than by file format.

- **glTF scenes open faster.** gltf-wasm parses a scene's JSON into one flat
  table instead of a tree of separately allocated values: VirtualCity, whose
  JSON is 490 kB, opens in 1.1 ms instead of 1.86 (`JSON.parse` takes ~0.9 on
  the same text), and BrainStem in 0.67 instead of 0.95. Against upstream's
  glTF Draco decoder driven the way three.js drives it, VirtualCity now reads
  1.07x as long instead of 1.21x. The reader module is 0.8 kB of gzip smaller
  than before, and the converter's 3.0 kB larger.

- **The glTF reader module is 12% smaller: 142.4 -> 125.2 kB gzip.** It no
  longer decodes Draco point clouds, which `KHR_draco_mesh_compression` does
  not allow and upstream's glTF decoder leaves out too; the converter still
  reads them. The Draco decoder also stopped hashing one rare table. With its
  JavaScript glue the reader is 132.6 kB, against 92.1 kB for upstream's glTF
  decoder, its glue and three.js's two loaders, with every model read as fast
  as before.

- Every module is optimized with wasm-opt from Binaryen 133, pinned in the
  workflows, instead of whatever Ubuntu packages (108): 0.5% smaller on each
  module with code in it, 3.8 kB across all seven, with the same speed.

- The modules no longer embed the paths of the machine that built them: every
  panic location used to name its source file by absolute path, the builder's
  user directory included. Each module is a little smaller for it.

- **glTF scenes of many primitives load faster.** `GltfAsset.readPrimitives`
  reads several primitives in one call, and the converter reads a scene's
  primitives through it, 32 at a time. The document is validated once a call
  instead of once a primitive, which was most of the time on scenes of many
  small primitives: on one of 167, reading its geometry takes 6.4 ms instead
  of 58 ms in the browser. A scene of a few large primitives reads as before,
  since its time is the Draco decode itself. The geometry is the same, and a
  primitive that fails still fails the load with its own error.

- `create_drc` takes a `point_cloud` option, which writes the input through
  Draco's point-cloud coder instead of the mesh one. Off by default, since a
  caller passing no indices today gets a mesh with no faces and switching that
  silently would change the geometry type its decoder reports. It is what a
  file whose payload is per-point attributes actually is -- a scan, or a
  Gaussian splat read with the PLY reader's generic attributes turned on.

- `create_drc` takes the two point-cloud encoder options as well:
  `prediction_search` lets the encoder choose each attribute's prediction
  scheme by the estimated cost of the candidates instead of by upstream's
  fixed rule, and `spatial_point_order` emits the points along a Hilbert
  curve so the difference predictor has a spatial neighbour to predict from.
  Both act on the point-cloud coder only, so they need `point_cloud` on, and
  both are off by default because the module's output is otherwise
  byte-identical to C++ Draco's for the same input. Together they take a Gaussian splat scene
  from 53.02 to 45.43 bytes per point and a photogrammetry capture of eight
  million coloured points from 6.26 to 4.23. The order one reorders the
  decoded points, which matters to anything outside the file that indexes into
  it by point number, and it can make a file bigger when an attribute varies
  along the order it came in rather than through space.

- A `.drc` point cloud written by Draco 1.0 to 1.2 with the KD-tree opens.
  Those releases encoded any cloud of positions alone that way, in a layout
  bitstreams before 2.3 use, and the converter refused every one of them.

- `create_drc` given no indices writes a file C++ Draco can read. The mesh of
  points and no faces it makes lacked the byte that names how the faces are
  coded, which C++ Draco expects even when there are none, so C++ Draco --
  the decoder most viewers and engines use -- refused it, while this module
  read its own files back and hid that.

- `parse_ply_bytes` reports the vertex properties, face properties and whole
  elements the reader could not carry, through the `warnings` array it already
  returned. A Gaussian-splat PLY keeps everything but the position in
  properties this reader has no attribute for, so it parsed into a bare point
  cloud and said only that it had succeeded. Until now the array was filled
  only by the fallback path for a malformed file, which meant a broken file
  was explained and an intact one that lost most of its payload was not.
- The FBX preview draws every node's own geometry again. A geometry FBX
  carries without polygons — a curve, a lattice — was dropped from the mesh
  list but still counted by the walk that numbers them, so every node past the
  first such geometry drew its neighbour's mesh, with its material and morph
  targets shifted to match. One file of 501 geometries showed as a fragment of
  the model with the rest scattered.
- A file the selection did not hold is reported with the gesture that supplies
  it, not only by name: the folder for a single-file selection, and for FBX the
  reason a file that *was* supplied may still not have matched — its texture
  paths belong to the machine that authored the file, so only the name is
  compared. The glTF parse error no longer repeats that advice.
- `ktx2-wasm` names the single- and two-channel transcode targets the crate
  gained — `bc4`, `bc5`, `eac_r11`, `eac_rg11` — so a viewer can take a
  normal map or a channel mask in the format drawn for it.
- The viewer's format choice ranks a texture every material samples through
  `normalTexture` as a normal map and takes BC5 on the desktop family and
  EAC RG11 on the mobile one, with the color ranking as the fallback.
- `parse_ply_bytes` reads a large point cloud in a little over half the
  memory: the file goes to the reader without a second copy, and a property
  that is not `float` is held in its own width until it crosses, rather than
  every one widened to `f64` at once. A 29-million-point airborne scan peaks at
  2.2 GiB instead of 3.9, so a file of that size with a `double` timestamp now
  fits the 4 GiB a WebAssembly module can hold. What JavaScript receives is
  unchanged.
- A module that traps -- out of memory, or any panic in a release build -- is
  made again with an empty memory, so the next file is not refused for what the
  last one left behind, and running out of memory is reported as that instead
  of as `unreachable`.
- A "Z up" toggle on the viewport toolbar stands a lidar or survey file up in
  the Y-up preview, splats included. It turns the view only.
- Exporting a PLY, OBJ, STL or DRC to glTF or FBX offers "Source is Z-up:
  write it Y-up", which turns the geometry exactly, `(x, y, z)` to
  `(x, z, -y)`. It starts from the preview's toggle and is the export's own
  after that; the formats with no axes of their own are written as read.
- A point cloud exported to glTF or GLB -- a PLY, OBJ, STL or DRC with no
  faces -- is written as `POINTS`. It was written in glTF's default mode,
  triangles of consecutive vertices, which a vertex count not divisible by
  three made a file the converter itself refused to open. Its export no
  longer reports the document's warnings twice, nor that points need
  triangulating. Draco does not compress it: the extension's encoder here
  takes triangles only, and the export says so.

## 2026-09-05

- Added source-neutral SceneDocument capability reporting and FBX/glTF
  hierarchy UI; verified Mixamo, Samba Dancing, and Fox conversion controls.
- Preserved extra UV sets and up to eight skin influences through the
  SceneDocument GLB/typed-FBX paths, with explicit diagnostics for viewer and
  typed-FBX writer limitations. See [`web/README.md`](web/README.md).
- Added STL and standalone Draco (`.drc`) as import **and** export formats,
  through the new `stl-wasm` and `drc-wasm` modules. `stl-wasm` wraps
  `draco-io`'s reader and writer; `drc-wasm` wraps `draco-core` directly, the
  first web module to depend on neither `draco-io` nor `draco-gltf`.
- Carried a `.drc`'s attributes through a round trip instead of dropping the ones
  the flat mesh has no slot for. A second texture-coordinate or colour set is
  handed over with its type, component count, component type and unique id, and
  put back unchanged; a consumer's ids survive. Nothing in between reads their
  meaning, and the converter reports each one as carried but uninterpreted.
  Where another format has a name for one it gets it — `TEXCOORD_1`, `COLOR_1`
  into glTF — and a generic is reported as left behind rather than invented into
  an `_NAME`.
- Gave the flat formats (OBJ, PLY, STL, `.drc`) a route to glTF and GLB through
  the portable SceneDocument, and JSON glTF a route from every source. Both were
  previously unreachable.
- Baked node placement into flattened exports, so a multi-node scene exported to
  a flat format no longer collapses every object onto the origin. Vertex colours
  and PLY texture coordinates survive those exports too, and the reported Draco
  `Method` is the encoder's own rather than a guess.
- Read FBX `UnitScaleFactor` and the six axis fields instead of assuming
  centimetres and Y-up, made the export space a choice (`meters-y-up` by
  default, `meters-z-up` available), and turned V at every crossing rather than
  one. Both directions and both declarations now come from a single space, and
  Blender is the external oracle for it. See [`web/README.md`](web/README.md).
- Cleared compression statistics and scene fields on import, so a panel never
  describes the previous model.
