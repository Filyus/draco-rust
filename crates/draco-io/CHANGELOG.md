# Changelog — draco-io

Notable changes to the `draco-io` crate. This crate is versioned and released
independently; its release tags are `draco-io-vX.Y.Z`. It depends on a published
`draco-core`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Serde-free compact glTF reader front end (`gltf_compact`, `parse_compact_document`)
  gated behind the new opt-in `gltf-compact` feature. It reuses the strict GLB
  container validation and shared `GltfError` type but parses the JSON document
  with `nanoserde`, keeping the `serde`/`serde_json` dependencies out of the
  binary for size-constrained front ends (WASM). The contract (FLOAT-only
  POSITION/NORMAL, integer TEXCOORD_0/COLOR_0 with normalization, multi-buffer,
  multi-primitive, no sparse accessors, `KHR_draco_mesh_compression` as the only
  allowed required extension) is documented on the module.
- `CompactLimits` and `parse_compact_document_with_limits` for bounded compact
  parsing. The WASM reader applies conservative JSON, resource, buffer, and
  decoded-geometry quotas by default.

### Changed

- The compact glTF reader now decodes FLOAT `COLOR_0` attributes, rejects
  unsupported attribute semantics, invalid default scenes, and undeclared Draco
  usage instead of silently producing incomplete scene data.
- The compact glTF reader now accepts the common production-asset shapes the
  native `GltfReader` already supports: integer `TEXCOORD_0`/`COLOR_0` accessors
  (UNSIGNED_BYTE/UNSIGNED_SHORT with normalization), multi-primitive meshes
  (flattened into `CompactDocument::meshes`), and multi-buffer documents.

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
