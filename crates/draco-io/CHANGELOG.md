# Changelog — draco-io

Notable changes to the `draco-io` crate. This crate is versioned and released
independently; its release tags are `draco-io-vX.Y.Z`. It depends on a published
`draco-core`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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

### Changed

- The FBX writer assembles the whole document as a tree of `FbxNode` -- the
  same type the readers produce -- and encodes that tree in one place, instead
  of spelling document structure directly in byte calls. Written bytes are
  unchanged over the whole corpus. `FbxWriter::write_to` accordingly takes
  `W: Write` rather than `W: Write + Seek`; the backpatched node header still
  seeks, but inside a buffer of its own. Relaxing the bound accepts everything
  it accepted before.
- The `Definitions/Count` node is now the number of `ObjectType` blocks rather
  than a hand-maintained literal that had to be kept in step with them by eye.

### Fixed

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

- **Breaking.** `FbxScene::warnings` is `Vec<FbxWarning>` rather than
  `Vec<String>`. `FbxWarning` implements `Display`, so a consumer that rendered
  the strings ports by calling `to_string()`.
- **Breaking.** `FbxMeshInstance` gains `color_sets` and `edges`, and
  `FbxProperty` gains a `U8` variant; struct literals and exhaustive matches
  need updating.
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
  exported scene to come in 100× too small in Blender. The accompanying
  `applyRootScale` workaround in `web/www/fbx-scene-adapter.js` (and its
  `legacyCompatibility` plumbing in `gltf-loader.js`) has been removed because
  the unit-scale factor now carries the correct conversion on its own —
  scaling coordinates on top of it made the scene come in 100× too large.

## [0.3.0](https://github.com/Filyus/draco-rust/compare/draco-io-v0.2.0...draco-io-v0.3.0) - 2026-07-18

### Added

- Strict GLB v2/v3 inspection and byte-level container serialization.
- Explicit lossy `FbxScene` views with `from_bytes` / `to_bytes`; the streaming
  `FbxReader::read_scene` path also works with in-memory readers.
- `FbxWriter::add_scene` writes model names, hierarchy, and local affine TRS
  transforms from `FbxScene`.

### Changed

- **Breaking:** `draco-io` now provides OBJ, PLY, FBX, and low-level glTF
  container, resource, and accessor contracts only. Full glTF scene operations
  live in `draco-gltf` 0.2.
- Renamed the ambiguous `gltf` feature to `gltf-container`; use
  `gltf-geometry` for accessor-to-mesh contracts and `draco-decode` for Draco
  payload decoding.
- Removed `serde`, `serde_json`, and `nanoserde` from runtime dependencies.

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
