# Changelog — draco-io

Notable changes to the `draco-io` crate. This crate is versioned and released
independently; its release tags are `draco-io-vX.Y.Z`. It depends on a published
`draco-core`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
  `AnimationCurveNode`/`AnimationCurve` graph is flattened into
  `FbxAnimation` takes with `FbxAnimChannel` TRS channels in seconds. FBX KTime
  ticks-per-second (V7 `46186158000` or V8 `141120000`) is resolved from the
  file version and optional `FBXHeaderExtension/OtherFlags/TCDefinition`.
- `FbxScene::warnings` records skipped skin deformers and blend shapes so
  callers can surface them without failing the parse.

### Changed

- `FbxMeshInstance` now carries `material_indices`; existing constructors must
  supply it (an empty `Vec` preserves previous behavior).
- `FbxWriter` accepts `Position`, `Normal`, and `TexCoord` mesh attributes
  (previously `Position`-only); other attribute types still return
  `InvalidInput` so geometry data is not dropped silently.
- `FbxReader::read_scene` now populates `FbxScene::materials`, `textures`,
  `animations`, and `warnings` in addition to the existing `root_nodes`.

### Fixed

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
