# Changelog — draco-gltf

Notable changes to the `draco-gltf` crate. This crate is versioned and released
independently; its release tags are `draco-gltf-vX.Y.Z`. It depends on published
`draco-core` and `draco-io`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-18

### Added

- Lossless `Document` typed views and index types for full glTF scenes; unknown
  fields, `extras`, and unregistered extension JSON survive edits and writes.
- Pinned glTF 2.1-draft validation surface, GLB v3 containers, explicit
  `files` asset loading, extension contracts, and compact geometry views.
- `Import::compress_primitive`, `CompressionOptions`, and
  `CompressionReport` for document-preserving Draco transforms.
- `Import::to_gltf_output` for portable JSON plus companion buffers, alongside
  GLB v2/v3 serialization through `Import::to_bytes`.

### Changed

- Breaking: replaced the previous external document API. Use `Document`, typed
  views, and index types described in `MIGRATING_0_2.md`.
- Full-scene operations now live in `draco-gltf`; `draco-io` provides only
  container, resource, accessor, and bitstream contracts.
- `CompressionMode::DracoOnly` is the default and requires Draco; use
  `CompressionMode::Fallback` to retain ordinary geometry for non-Draco
  readers.

### Fixed

- Draco compression and decompression are atomic; shared accessors, unknown
  JSON, registered extension references, and retained scene resources are
  preserved safely.
- Strict KHR Draco validation checks extension lists, primitive modes,
  attribute mappings, unique IDs, and Draco-only accessor layouts.
- Binary range arithmetic, resource quotas, output limits, and compaction use
  checked operations; overlapping retained ranges are coalesced.

## [0.1.0] - 2026-06-24

### Added

- Load and save full glTF scenes with Draco-compressed geometry.
- `import` / `import_slice` for reading Draco glTF/GLB, `decode_primitive` to
  decompress geometry, and `decompress_in_place` for transparent reading.
- `compress` to Draco-compress a full scene while preserving materials,
  textures, nodes, animations, skins, and unknown extensions.
- Draco-aware, panic-safe validation on import.
- WebAssembly support, and an optional `image` feature to shrink the build when
  texture pixels are not needed.
