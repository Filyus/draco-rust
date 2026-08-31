# Changelog — draco-io

Notable changes to the `draco-io` crate. This crate is versioned and released
independently; its release tags are `draco-io-vX.Y.Z`. It depends on a published
`draco-core`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Changed

- An FBX `Model` hierarchy deeper than the 256 levels the reader descends is
  still cut there -- the walk is recursive and a document chooses its own depth
  -- but it now says so, with a `model-depth-limit-reached` warning raised once.
  The cut was silent, and what a caller saw instead was its consequence: a
  300-bone chain came back missing its tail, and every skin cluster bound into
  the missing part reported a missing joint of its own.

- Each axis of an animated FBX property is sampled at its own key times. The
  three curves of a `Lcl Rotation` are separate objects and need not share a key
  grid; they were combined by shared index against the X curve's times, so Y was
  read wherever X happened to have a key, and past the shorter curve's last key
  it was read as zero. One take had 254 component curves end early, a root
  bone's vertical translation among them.

- The OBJ, PLY and glTF readers finish a mesh the way upstream finishes one:
  merge bit-identical attribute values, then merge the points those values made
  identical. They shared a face-order renumbering before, which merges nothing,
  so two vertices carrying the same position stayed two points. That cost
  connectivity rather than tidiness -- triangles that should share an edge
  shared only a corner, and the encoder wrote a larger stream that decoded to
  more points than it was given. Measured against C++ Draco 1.5.7 on a
  two-triangle case: 77 bytes and six decoded points before, 75 bytes and four
  after, byte-identical to what upstream writes.

  They then drop the points no face names, which upstream does not. The three
  readers disagreed with each other about those before -- OBJ and FBX dropped
  them by interning corners, PLY and glTF kept them -- so the same file
  re-saved through a different format changed its point count. Keeping them
  buys nothing: no encoder writes an unreferenced vertex, and all it reaches is
  the quantization range, where a stray far-away vertex costs every surviving
  coordinate precision. See `COMPATIBILITY.md`.

  Both halves reach real files. The Khronos `Fox` sample carries one vertex per
  corner -- 1,294 of its 1,728 duplicate another exactly -- and now encodes 434
  points for the same 576 triangles. The Stanford Bunny carries 1,113 vertices
  no face names among its 35,947, all of them setting the quantization range
  until now.

### Added

- `gltf_geometry::decode_geometry` accepts `TRIANGLE_STRIP` (mode 5) and
  `TRIANGLE_FAN` (mode 6) primitives, not only `TRIANGLES`. Draco's
  connectivity has no notion of either, so both are unwound into an ordinary
  triangle list before the mesh is built -- the same algorithm the web
  preview already used to render these two modes (`triangleIndices` in
  `web/src/gltf-loader.ts`), so preview and Draco-compressed output now agree
  on what geometry a strip or fan means. Previously any primitive mode other
  than `TRIANGLES`/`POINTS` was refused outright.
- FBX 6100 documents are read in either container: the name-keyed object model
  (`"Name\0\x01Class"` keys, `Connect` records, geometry on the `Model`,
  `Properties60`, repeated-scalar arrays) is normalized into the same scene the
  7.x layout produces, including `Takes` animation with its heterogeneous `Key`
  payloads. Previously such a document decoded to a structurally valid but
  empty scene with a `name-keyed-object-model` warning; that warning code is
  removed now that the layout is read.
- Writing the 6100 object model: `FbxWriter::with_legacy_object_model()`, or
  `FbxScene::to_legacy_bytes` / `to_legacy_ascii_bytes`, spell the document as
  FBX 6100 in either container so a pre-7000 source round-trips inside its own
  version. The 6100 writer carries meshes, transforms, materials, textures and
  Takes animation; a skin or blend shape is refused with an error rather than
  silently dropped.
- A binary `C`-typed property whose byte is neither 0 nor 1 reads as `U8`
  rather than `Bool(true)`: `C` is a raw byte, and the pre-7000 animation
  format packs its mode letters into them.

### Changed

- The ASCII container accepts version 6100 (the name-keyed object model);
  pre-6100 text is refused as before.
- ASCII FBX and ASCII STL spell floats through a Schubfach formatter instead of
  the core `Display`. Every value still reads back identically, but the bytes
  differ in two ways: a whole value keeps a decimal point (an STL facet normal
  now reads `0.0 0.0 1.0` where it read `0 0 1`), and very large or very small
  magnitudes come out in exponent notation (`1e+16`) where `Display` spelled
  them positionally. Any `strtod`-family reader takes both; a hand-rolled STL
  parser that only scans positional decimals may not.

### Fixed

- A `Model` that reaches neither the document root nor a parent `Model` by
  object connection is no longer rooted by absence. Such a Model is not part
  of the scene graph the source describes, and rooting it resurrected objects
  the author kept out of the scene: MotionBuilder binds its seven Producer
  viewport cameras and its Camera Switcher only by `CurrentCamera` property
  records, and a rewrite emitted them as eight real scene objects per file,
  where Blender's importer skips them. They are now dropped with an
  `unconnected-model-dropped` warning instead. The change was checked against
  the 1456-document ufbx corpus first: thirteen documents hold such Models,
  none reaches geometry this reader represents through one, and none parents
  another Model, so the only reads that changed are the thirteen files' own
  strays.
- A rig's joints are rewritten as joints. The writer learned "this Model is a
  joint" only from skin-cluster membership, so a joint that carries no weights
  — a bone's `*_end` tail helper — and a `Null` grouping node such as an
  armature's root were rewritten as plain `Mesh` Models, and an importer that
  forms bone chains from parented `LimbNode` Models found the chain broken at
  every tail: Blender 5 imported the rewrite of a skinned corpus file as
  eleven loose `_end` empties and no armature. `FbxSceneNode` now carries the
  Model's own class as `FbxNodeKind` (`Joint` for `LimbNode`/`Limb`, `Null`
  for `Null`/`Root`), the reader sets it, and the writer classes such Models
  as `LimbNode` or `Null` and gives every joint a `Skeleton` NodeAttribute
  whether or not a cluster names it. Skinned files that decoded before are
  unchanged: cluster membership still marks a joint where the source class
  did not.

### Changed

- This crate permits `unsafe` in narrow, audited paths, where `SECURITY.md`
  previously ruled it out for the whole workspace at once. Every block must
  carry a `// SAFETY:` comment naming its invariant **and where that invariant
  was established**; `undocumented_unsafe_blocks` is on and CI runs clippy with
  `-D warnings`, so an unjustified block does not build. **No path in the
  library uses `unsafe` today** and nothing that ships changes; what changes is
  that a measured optimisation in the byte-shuffling, endian-swapping,
  fixed-stride part of a file-format layer no longer has to relitigate the
  policy to land. `draco-core` keeps the rule absolute and now has the compiler
  holding it — a bitstream decoder running on attacker-controlled indices and a
  container parser are not the same risk, and one rule for both was too strict
  for one of them. `SECURITY.md` carries the requirements, including that the
  invariant must come from the code and never from the file, and the table of
  such paths.
- The four mesh writers refuse a mesh whose attributes do not cover its points,
  at their entry points, instead of reading `point * byte_stride` through the
  panicking `DataBuffer::read` at nine call sites. Those reads are sound exactly
  when each attribute holds at least `num_points` values, and nothing between a
  decoder and a writer re-checked it — the counts come from a `.drc` header. One
  precondition per writer is what makes the nine provably in range; guarding at
  the reads instead would mean deciding, nine times, what to emit for a value
  that is not there.

### Fixed

- A build with FBX writing but without `compression` no longer warns that
  `WriterOptions::compression_level` is never read. The field arrived with the
  deflate level knob and is read only where deflate is compiled in; it stays
  present in either build so the struct has one shape and the writers filling
  it need no `cfg` of their own.
- The PLY and STL writers no longer panic on a mesh whose position or normal
  attribute is not `Float32x3`. A `.drc` declares its own attribute data types,
  so `MeshDecoder` returns Uint8x3 positions or Int16x3 normals as readily as
  float ones, and both writers sliced a fixed twelve bytes per point regardless.
  The STL writer now refuses what it cannot read, as the OBJ writer already did;
  the PLY writer converts each component at its own width.
- The PLY writer handles a mesh that carries normals, colors or texture
  coordinates but no position attribute. The padding it inserts measured how far
  behind the position list the other lists were, which underflowed when nothing
  had been appended to it.
- Adding a mesh without normals, colors or texture coordinates to a `PlyWriter`
  no longer drops those attributes from the meshes added before it. The writer
  flattens every mesh into shared per-vertex lists and emits a property only
  while its list matches the vertex count; the lists were padded before a mesh's
  values were appended but not after, so the same two meshes wrote different
  files depending on the order they were added in.
- The OBJ reader refuses a `v`/`vt`/`vn` line whose components do not parse,
  instead of dropping it. OBJ indices are 1-based and count the file's own
  vertex lines, so a dropped one shifted every later index: `f 1 2 4` silently
  named a different vertex and the reader returned a mesh the file does not
  describe. Upstream C++ Draco fails the same file.
- The OBJ reader separates a keyword from its values on any ASCII whitespace. It
  matched on a trailing space, so a tab-delimited file - which is valid OBJ -
  was invisible to the parser and decoded to an empty mesh.
- The PLY reader no longer indexes past the four-channel colour array when a
  header names more colour properties than a colour has channels - the same one
  twice, for instance.
- The PLY reader's work is bounded by the file rather than by the counts printed
  in it. An `element` count is unrelated to how many lines or bytes follow, so a
  169-byte file declaring four billion faces spun for seconds and reserved
  48 GB before reading a single vertex; the line loops now end with the text and
  the reservations are bounded by what the body can hold.

## [0.3.2](https://github.com/Filyus/draco-rust/compare/draco-io-v0.3.1...draco-io-v0.3.2) - 2026-07-30

### Changed

- `keywords` names STL, which the crate has read and written since 0.3.1 without
  saying so anywhere a search would find. Five is the crates.io maximum, so
  `gltf` came off rather than STL being left out: `draco-gltf` owns glTF
  documents, and this crate's glTF role is containers and accessors, so a search
  for glTF is better answered there.
- STL is named in the format and feature tables, in the crate's opening line and
  `API.md`, and in the `traits` module docs. It had been announced in the 0.3.1
  changelog and nowhere else.
- FBX documentation moves to [`FBX.md`](FBX.md), which ships with the crate. It
  was 179 of the README's 335 lines, in a document whose job is to say what the
  crate reads and writes. The support matrix now leads it, and the prose behind
  it is split by subject rather than being one block under a single heading.
- A claim that `Definitions` property templates are not resolved is removed. They
  are — `fbx_templates` resolves them, with the object's own value winning — and
  the paragraph contradicted both the section above it and the support table.
- The crate's opening line no longer says "binary FBX": the ASCII container has
  been read and written since 0.3.0.

## [0.3.1](https://github.com/Filyus/draco-rust/compare/draco-io-v0.3.0...draco-io-v0.3.1) - 2026-07-30

### Added

- STL, read and written, behind the new `stl-reader` and `stl-writer` features.
  Both join `all-readers` / `all-writers` and therefore `default`. `StlReader`
  offers the same `open` / `from_bytes` / `read_from_bytes` / `read_mesh` shape as
  the other readers; `StlWriter` implements `Writer` and `WriteToBytes` and takes
  `with_format(StlFormat::Binary | StlFormat::Ascii)`, binary by default.
  - Which container a file is in is decided by its length -- `84 + 50 · N` for the
    declared triangle count -- rather than by whether it opens with `solid`. That
    keyword is not a discriminator: binary writers put arbitrary text in the
    80-byte header, and plenty of them start it with the word. The declared count
    is checked against the bytes actually present, so a truncated file is an
    error instead of a short mesh.
  - The binary header this writer emits starts with `Draco`, so a file it wrote
    can never be mistaken for ASCII by a reader that does look at the keyword.
  - STL stores no indices and no shared vertices, so a mesh is written as
    unshared corners with a facet normal derived from the winding.

### Changed

- The `decode_drc` and `decode_rust_lamp` examples require the `.drc` path as an
  argument and print a usage line without one. Each defaulted to a file in one
  machine's C++ Draco output directory, which no other checkout has.

### Fixed

- PLY faces are read from the element's index list rather than from whichever
  list came first. A face element may carry several: `vertex_indices` alongside a
  per-corner `texcoord`, which some exporters write and which the Draco corpus
  itself contains. The reader took the first one it met, so those files decoded
  with texture coordinates in place of indices -- vertex numbers far past the end
  of the mesh, and a parse error rather than a wrong picture. `vertex_indices` is
  now preferred by name, with the first list as the fallback for files that spell
  it otherwise, and the remaining lists are skipped by their declared type in
  both the ASCII and the binary path.
- The FBX writer's default `GlobalSettings` are Y-up and metres, which is glTF's
  own orientation and what every FBX in the wild on hand declares. They were
  Z-up, with `UpAxisSign` and `FrontAxisSign` contradicting even that, so a file
  could describe an orientation it did not contain and a reader that resolved
  those fields turned the scene over. The defaults describe nothing a caller
  wrote: a caller that converts coordinates supplies its own `GlobalSettings`,
  which every path in the web converter now does. A document carrying its own
  settings -- every rewrite of a source file -- is unaffected.
- `create_fbx`'s options are no longer discarded. `globalSettings` on them is
  written verbatim, so a flat mesh list can state the space its coordinates are
  in instead of inheriting defaults that described a different one.

## [0.3.0](https://github.com/Filyus/draco-rust/compare/draco-io-v0.2.0...draco-io-v0.3.0) - 2026-07-29

### Added

- `EXT_meshopt_compression` buffer views are decoded, through the new `meshopt`
  module and the re-exported `MeshoptMode` and `MeshoptFilter`. All three modes
  (attribute, triangle, index sequence) and all four filters are implemented,
  including the pre-release `EXT_meshopt_compression`/`COLOR` filter spelling
  that older exporters wrote, so a meshopt-packed document reads as ordinary
  geometry rather than being rejected.
- `PackedAttribute::from_gltf_accessor` keeps the source glTF component type
  alongside the packed bytes, so a storage type the Draco geometry model has no
  scalar for -- `HALF_FLOAT` from the 2.1 draft above all -- survives a read
  instead of being widened or dropped.
- Strict GLB v2/v3 inspection and byte-level container serialization.
- Explicit lossy `FbxScene` views with `from_bytes` / `to_bytes`; the streaming
  `FbxReader::read_scene` path also works with in-memory readers.
- `FbxWriter::add_scene` writes model names, hierarchy, and local affine TRS
  transforms from `FbxScene`.
- **Breaking.** FBX `LayerElementTangent` and `LayerElementBinormal` are read
  and written. `FbxMeshInstance` gains `tangent_sets` and `binormal_sets`, and
  `FbxRenderMesh` gains `tangents` and `binormals`. Handedness is merged into a
  fourth component on read and split back into the `TangentsW` sibling array on
  write, only for sets that had one. Draco has no tangent attribute, so these
  never reach the Draco mesh.
- **Breaking.** FBX `LayerElementSmoothing`, `LayerElementEdgeCrease` and
  `LayerElementVertexCrease` are read and written. `FbxMeshInstance` gains
  `smoothing_layers` and `crease_layers`, typed `i32` and `f64` respectively so
  a crease weight is not rounded through a flag's type. A layer whose length
  disagrees with the domain its mapping names is dropped with a warning; a
  `ByEdge` layer in a geometry with no `Edges` array is preserved unchecked,
  since it addresses edges this crate does not reconstruct.
- **Breaking.** FBX `Camera` and `Light` node attributes are read onto the new
  `FbxSceneNode::attribute` field, as `FbxNodeAttribute::Camera(FbxCamera)` or
  `::Light(FbxLight)`, and written back. The writer emits the `NodeAttribute`
  object with the `TypeFlags` and declared property types importers expect,
  classes the owning `Model` as `Camera` or `Light` rather than `Mesh`, and
  declares the attribute in `Definitions`. `FbxCamera` also carries the film
  back -- `film_width`, `film_height`, `film_aspect_ratio` and `aperture_mode`
  -- because a consumer needs it with `focal_length` to reach a field of view:
  Blender derives `sensor_width` from `FilmWidth` and falls back to its own
  32 mm default without it, silently reframing every camera. Other attribute
  classes raise `FbxWarningCode::DroppedNodeAttribute` on read and are not
  written.
- The ASCII FBX container is written as well as read. `FbxWriter::with_format`
  takes the new `FbxFormat`, `FbxScene::to_ascii_bytes` writes text where
  `to_bytes` writes records, and `fbx_rewrite --ascii` produces one. Both
  spellings come off the same document tree, so the two containers can differ
  only in how a record is written down -- a corpus check compares the trees
  they read back as, over all 565 comparable files. Three things ASCII cannot
  record are reported as errors rather than written wrong: a node with two
  array properties, a non-finite float, and raw bytes on a node no reader
  decodes as base64. Two it records less precisely and cannot be helped: an
  integer's width, and an object named `"` against one named `&quot;`.
- The ASCII FBX container is read, for versions 7000 and later, through the
  new `fbx_ascii` module. It produces the same node tree as the binary reader,
  so `FbxReader`, `FbxScene::from_bytes` and the `Reader` traits all accept it
  without a separate entry point. The web app's regex ASCII fallback, which
  recovered geometry only, is removed.
- **Breaking.** The seven layer-element families move off `FbxMeshInstance`
  into a new `FbxMeshLayers` struct behind one `layers` field, taking the
  instance from fifteen fields to nine. `FbxMeshInstance` and `FbxMeshLayers`
  both derive `Default`, so a literal need only name the fields it cares
  about; that is what keeps the next layer family from touching every
  construction site again.
- **Breaking.** `expand_to_render_mesh` takes an `FbxGeometryLayers` borrow
  struct instead of five positional slices. That struct now also carries
  `smoothing_layers` and `crease_layers`, which the writer previously received
  as separate arguments.
- The FBX binary container decoder moves to a new `fbx_container` module;
  `fbx_reader` keeps only the scene layer above the node tree and re-exports
  `FbxNode`, `FbxProperty`, `FbxReader` and `FbxMemoryReader`, so existing
  paths still resolve.
- `FbxUvSet`, `FbxNormalSet` and `FbxColorSet` are now aliases of a shared
  `FbxLayerSet<N>`. Field names and public paths are unchanged.
- `FbxWarningCode::DroppedLayerElement` names each `LayerElement*` the reader
  does not import, instead of discarding it silently.
- `FbxWarningCode::NameKeyedObjectModel` reports a pre-7000 document, whose
  name-keyed object model this crate does not read; such a file decodes to an
  empty scene rather than failing, so the notice is how a caller tells that
  apart from a file with no meshes.
- FBX scene round-tripping now retains skin clusters/bind poses, morph targets,
  authored node-TRS animation, and all decoded UV layers through the typed
  scene writer. Tangents and non-default transform inheritance remain explicit
  unsupported/untested cases rather than being silently discarded.
- FBX Phong/Lambert materials with diffuse/specular/emissive/ambient colors,
  scalar factors (`DiffuseFactor`, `SpecularFactor`, `Shininess`,
  `EmissiveFactor`, `ReflectionFactor`, `TransparencyFactor`, `Opacity`,
  `BumpFactor`), and diffuse/normal/emissive texture bindings (embedded
  `Content` bytes or external filename). Materials live on `FbxScene::materials`
  and reference textures via `FbxTextureBinding`.
- FBX textures (`FbxTexture`) with embedded image bytes (`Video.Content`) and
  `RelativeFilename`/`FileName`, collected on `FbxScene::textures`.
- FBX per-polygon material indices via `LayerElementMaterial`, exposed on
  `FbxMeshInstance::material_indices` (expanded from `AllSame`/`ByPolygon`).
- FBX normals and UVs read into Draco `Normal`/`TexCoord` mesh attributes and
  written back as `LayerElementNormal`/`LayerElementUV`.
- FBX node-TRS animation: the `AnimationStack`/`AnimationLayer`/
  `AnimationCurveNode`/`AnimationCurve` graph is resolved into `FbxAnimation`
  takes with `FbxAnimChannel` TRS channels in seconds. FBX KTime
  ticks-per-second (V7 `46186158000` or V8 `141120000`) is resolved from the
  file version and optional `FBXHeaderExtension/OtherFlags/TCDefinition`.
- `FbxScene::warnings` records tolerated container deviations and FBX semantics
  the decoded scene cannot express, as typed `FbxWarning` values with a stable
  `FbxWarningCode`, an occurrence count, and `is_data_loss()` to separate
  "unusual file" from "content missing from the result".
- Binary FBX big-endian reading. The endian marker at header offset 22 selects
  the byte order for node records, scalars, string/blob lengths and array
  payloads, matching `ufbx`: any non-zero marker means big-endian.
- `FbxReadOptions` and `FbxDecodeLimits` bound what one document may allocate
  (file size, node depth and count, properties per node, string and blob
  payloads, array element counts, per-array and per-document decoded bytes) and
  select strict container validation. Limit violations report
  `ErrorKind::OutOfMemory`, structural violations `ErrorKind::InvalidData`.
  Reachable through `FbxReader::{new,open,from_bytes}_with_options` and
  `FbxScene::from_bytes_with_options`; the `Reader`/`ReadFromBytes` traits keep
  their signatures and use the defaults.
- FBX vertex colours (`LayerElementColor`) on `FbxMeshInstance::color_sets`,
  read and written as linear RGBA. The first set also becomes a 4-component
  `Color` attribute on the Draco mesh.
- The raw FBX `Edges` array on `FbxMeshInstance::edges`, preserved verbatim.
- `FbxRenderMesh` and `FbxMeshInstance::to_render_mesh`, which resolve every
  layer element onto the polygon-corner domain and expose the corner-to-control-
  point and corner-to-polygon maps needed to re-index skin weights and morph
  deltas.
- Property type `Z` (`FbxProperty::U8`, read unsigned as `ufbx` does) and the
  `c` byte-array type.

### Changed

- **Breaking.** `draco-io` now provides OBJ, PLY, FBX, and low-level glTF
  container, resource, and accessor contracts only. Full glTF scene operations
  live in `draco-gltf` 0.2.
- **Breaking.** Renamed the ambiguous `gltf` feature to `gltf-container`; use
  `gltf-geometry` for accessor-to-mesh contracts and `draco-decode` for Draco
  payload decoding.
- **Breaking.** `FbxScene::warnings` is `Vec<FbxWarning>` rather than
  `Vec<String>`. `FbxWarning` implements `Display`, so a consumer that rendered
  the strings ports by calling `to_string()`.
- **Breaking.** `FbxMeshInstance` gains `color_sets` and `edges`, and
  `FbxProperty` gains a `U8` variant; struct literals and exhaustive matches
  need updating.
- Removed `serde`, `serde_json`, and `nanoserde` from runtime dependencies.
- The FBX writer assembles the whole document as a tree of `FbxNode` -- the
  same type the readers produce -- and encodes that tree in one place, instead
  of spelling document structure directly in byte calls. Written bytes are
  unchanged over the whole corpus. `FbxWriter::write_to` accordingly takes
  `W: Write` rather than `W: Write + Seek`; the backpatched node header still
  seeks, but inside a buffer of its own. Relaxing the bound accepts everything
  it accepted before.
- The `Definitions/Count` node is now the number of `ObjectType` blocks rather
  than a hand-maintained literal that had to be kept in step with them by eye.
- FBX layer elements resolve on the polygon-corner domain instead of being
  collapsed onto control points, which silently averaged away UV and hard-normal
  seams. The Draco mesh welds corners agreeing on every attribute, so seams
  survive without tripling the point count: across the `ufbx` corpus 78,509
  control points become 117,345 welded points rather than 320,967 raw corners.
- Each `AnimationLayer` becomes its own `FbxAnimation` instead of all layers
  being merged into one. Merging produced several channels driving the same node
  and path, and a consumer applying them in order kept only the last. This is
  what Blender's importer does; layer blending is still not applied.
- Reading is deterministic: object order, animation channel order and bind-pose
  resolution follow FBX object ids rather than hash iteration, so two reads of
  the same bytes compare equal.
- FBX versions below 6000 are rejected with an explicit error instead of
  decoding to an empty scene.
- `FbxMeshInstance` now carries `material_indices`; existing constructors must
  supply it (an empty `Vec` preserves previous behavior).
- `FbxWriter` accepts `Position`, `Normal`, and `TexCoord` mesh attributes
  (previously `Position`-only); other attribute types still return
  `InvalidInput` so geometry data is not dropped silently.
- `FbxReader::read_scene` now populates `FbxScene::materials`, `textures`,
  `animations`, and `warnings` in addition to the existing `root_nodes`.

### Fixed

- A PLY header delimited by bare carriage returns, which Rhino writes, was read
  as a single line and rejected as missing `end_header`. Header lines now split
  on CR, LF and CRLF alike.
- Euler angles and key times drifted a little further on every rewrite,
  because each was narrowed to `f32` before the arithmetic rather than after
  it. `f32::to_radians` and the writer's inverse each rounded twice, so `90`
  became `89.99997` and then `89.99996` without ever settling; a key's tick
  count, around 2e10 for one second, was narrowed before being divided, where
  one `f32` step is 2048 ticks. Both now compute in `f64` and narrow once, and
  a corpus check rewrites all 566 files twice over and requires the second and
  third generations to be byte-identical.
- A mesh with no vertices lost its `Geometry` object on the second rewrite.
  The writer omitted an empty `Vertices` array, but that array is what
  identifies a geometry, so the record described nothing the reader
  recognized. Both arrays are now written whatever their length.
- A bind pose was dropped from an ASCII document whose object ids fit in 32
  bits. `PoseNode/Node` was the one id in the reader matched against `I64`
  alone rather than through `object_id`, and ASCII does not record an
  integer's width. Authored exports use ids far above that range, so only a
  document with small ids showed it -- which is what this crate now writes.
- A written `Model`'s `Shading` record is a boolean, as it is in every FBX
  file: the ufbx corpus types it `C` roughly 1400 times and `Y` never, while
  this crate wrote the 16-bit integer `Y`. No reader in this crate consults
  `Shading`, so the visible effect is one byte per model, but the type was
  also the only thing a `Model` carried that the ASCII container cannot spell.
- Values an exporter states once in `Definitions/PropertyTemplate` were not
  read, so a property stated there and not on the object was lost. The object
  always wins -- 5553 properties in the corpus are declared in both places,
  `Lcl Translation` on 928 models among them. Recovers the whole
  field-of-view and focal-length block of the Revit cameras, which declare
  almost nothing directly and previously opened at focal length zero in
  Blender's ufbx importer.
- A material's `ShadingModel` is read from the `ShadingModel` node beside its
  `Properties70`, which Maya writes and this crate never looked at. Both of
  the material's own spellings now rank above the class template, which
  otherwise relabels every `phong` material `Lambert`.
- A `Model` whose local matrix could not be decomposed -- a zero scale, which
  Maya writes for a collapsed pivot -- failed the whole document, even when it
  carried an authored transform stack that made the decomposition unnecessary.
  It is now computed only where it is used.
- Animation key times, values, flags and tangents were written uncompressed
  whatever the document's options said, because every curve writer passed a
  freshly defaulted `WriterOptions` instead of the document's. Uncompressed
  output is unaffected.
- A document whose Model connections form a cycle, or a chain deeper than 256,
  recursed until the stack was exhausted. Reaching it required an ASCII-only
  corpus file, so the binary path had never exercised it.
- `collect_transform_warnings` indexed a vector before checking its length,
  because `bool::then_some` evaluates its argument eagerly. Any `Lcl Scaling`
  with fewer than three numeric values panicked.
- Object ids, float arrays and scalar doubles were matched at one width only,
  so a document that stored them at another read as absent rather than as
  present. This is invisible in the binary container, which tags every width.
- The writer named an unnamed `Texture` and `Video` after its class, so a
  document without texture names acquired them by being rewritten.
- Layer elements mapped `ByPolygon` were resolved on the control-point domain,
  returning an unrelated polygon's value. Five corpus files carry
  `LayerElementNormal` with that mapping.
- A geometry carrying only colour layers wrote a `LayerElementColor` that no
  `Layer` node referenced, so a strict importer did not find it.
- FBX reader no longer loops forever on a node record whose `end_offset` points
  backwards, and no longer aborts the process on an array header claiming more
  memory than exists. Record offsets are bounds checked against the record start
  and the file length, array sizing uses checked multiplication (`usize` is
  32-bit on wasm32), and decompression is bounded with an exact-output-size
  check, so a zip bomb is an error rather than an out-of-memory abort.
- FBX reader no longer panics on a negative `IndexToDirect` layer index, which
  became a huge `usize` and overflowed the value offset.
- FBX writer no longer destroys per-polygon material assignment on n-gon meshes.
  `LayerElementMaterial` is addressed ByPolygon while `material_indices` is per
  triangle, so writing the triangle list verbatim made the reader take its first
  N entries as the polygon assignments: on a two-quad mesh `[0, 0, 1, 1]` came
  back as `[0, 0, 0, 0]`.
- FBX writer keeps material slots when a file carries a material layer but
  defines no `Material` objects, instead of collapsing them all to zero.
- FBX writer emits `PolygonVertexIndex` for a vertices-only `Geometry`, which
  the reader needs to recognize it as a mesh; such objects used to disappear on
  round-trip.
- FBX writer emits the conventional binary footer. It previously wrote 20 zero
  bytes and the first four bytes of the footer id, with no repeated version and
  no closing magic.
- FBX writer no longer invents a `ShadingModel` for materials that declare none,
  or names an unnamed `Geometry` after its `Model`.
- FBX writer now emits `UnitScaleFactor = 100.0` (and
  `OriginalUnitScaleFactor = 100.0`) instead of `1.0`. FBX documents its base
  unit as the centimeter, and Blender's `io_scene_fbx` importer applies a
  `UnitScaleFactor / 100` multiplier; the legacy `1.0` value caused every
  exported scene to come in 100× too small in Blender.

## [0.2.0](https://github.com/Filyus/draco-rust/compare/draco-io-v0.1.0...draco-io-v0.2.0) - 2026-07-16

### Added

- Shared strict glTF/GLB container, data-URI, companion-resource, and embedded
  serialization APIs, including configurable external-file policies and
  resource quotas.
- A clean compression-options API (`GltfCompressionOptions`, optional
  per-semantic quantization, encoding method/speeds, and `OutputFormat`) shared
  by the document compressor and writer.
- Typed `CompressionReport` / `PreserveReason` output for valid primitives that
  remain uncompressed, including morph targets, sparse accessors, shared
  accessors, unsupported layouts, and existing Draco payloads.
- Strict shared `KHR_draco_mesh_compression` parsing and validation, with Draco
  unique IDs represented as `u32` and looked up by unique ID when decoding.

### Changed

- `compress_gltf_bytes*` and `compress_gltf_value` now return
  `CompressionOutput`; invalid input is always an error rather than a preserve
  reason. Default compression remains lossy (`14/10/8/12/8` quantization).
- Resource resolution is synchronous and policy-driven; native convenience
  loading remains uncapped unless the caller supplies `ResourceLimits`.
- Repository-only fixture and Blender tests use the `test` feature and are
  physically excluded from the published crate, while self-contained tests
  remain in the package.

### Fixed

- Reject malformed GLB chunks, invalid base64/percent escapes, opaque unknown
  extension binary references, out-of-range views/accessors, invalid fallback
  accessors, and accessor-contract mismatches with controlled errors.
- Harden buffer/accessor/GLB sizing and allocation paths with checked arithmetic
  and fallible reservations; declared lengths and GLB 32-bit limits are now
  enforced before materialization where possible.
- Preserve custom semantics, `extras`, known extension JSON, side attributes,
  images, animations, skins, and morph-target bytes when repacking understood
  references. External images remain external unless explicitly embedded by
  the caller.

## [0.1.0] - 2026-06-24

### Added

- Format readers and writers for OBJ, PLY, binary FBX, glTF, and GLB.
- `KHR_draco_mesh_compression` support (encode and decode) through `draco-core`.
- Document-preserving glTF/GLB Draco compression (`compress_gltf_bytes`) that
  rewrites only the compressible geometry and carries materials, textures,
  animations, skins, and unknown extensions through untouched.
- Hardening of the glTF compressor against malformed/untrusted input.
- Reader-agnostic geometry decode (`decode_geometry` + `AccessorSource`) reusable
  by external glTF front ends without linking the glTF reader.
