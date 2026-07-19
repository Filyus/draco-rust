# Changelog — draco-gltf

Notable changes to the `draco-gltf` crate. This crate is versioned and released
independently; its release tags are `draco-gltf-vX.Y.Z`. It depends on published
`draco-core` and `draco-io`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `AccessorData::accessor_type` and
  `DocumentAccessorSource::read_buffer_view` for generic accessor and embedded
  payload consumers, including WebAssembly bindings.
- Domain feature `accessors` for generic matrix-capable accessor
  materialization; primitive geometry reading remains in `read`.

### Fixed

- Draco compression now derives accessor counts, component layouts, and
  `POSITION` bounds from the encoded topology, including when the encoder drops
  unused points. Decode rejects accessor counts that disagree with the stream.
- Strict validation requires finite, ordered three-component bounds on
  `POSITION` accessors.

## [0.2.0] - 2026-07-18

### Added

- Lossless `Document` typed views and index types for full glTF scenes; unknown
  fields, `extras`, and unregistered extension JSON survive edits and writes.
- Pinned glTF 2.1-draft validation surface, GLB v3 containers, explicit
  `files` asset loading, extension contracts, and packed geometry views.
- Document-preserving Draco compression with measured output reporting.
- `Import::to_gltf_output` for portable JSON plus companion buffers, alongside
  GLB v2/v3 serialization through `Import::to_bytes`.
- Bidirectional `PackedGeometry` primitive reads and writes, including minimal
  standalone scenes, raw accessors, explicit Draco storage, and GLB v2/v3.
- `Document::to_minified_json_bytes` for forced whitespace-free output.

### Changed

- Breaking: replaced the previous external document API. Use `Document`, typed
  views, and index types described in `MIGRATING_0_2.md`.
- Full-scene operations now live in `draco-gltf`; `draco-io` provides only
  container, resource, accessor, and bitstream contracts.
- `CompressionMode::DracoOnly` is the default and requires Draco; use
  `CompressionMode::Fallback` to retain ordinary geometry for non-Draco
  readers.
- `Import` is the single geometry read/write entry point; feature `read`
  enables ordinary accessors, while Draco decode and encode remain explicit.
- Packed geometry belongs to `draco-gltf`; `draco-io` remains the low-level
  container, resource, accessor and Draco contract layer.

### Fixed

- Draco compression and decompression are atomic; shared accessors, unknown
  JSON, registered extension references, and retained scene resources are
  preserved safely.
- Strict KHR Draco validation checks extension lists, primitive modes,
  attribute mappings, unique IDs, and Draco-only accessor layouts.
- Binary range arithmetic, resource quotas, output limits, and compaction use
  checked operations; overlapping retained ranges are coalesced.
- Raw primitive writes emit exact `POSITION` bounds, validate topology and
  well-known attribute layouts, and reject incompatible morph targets before
  mutating the document.

## [0.1.0] - 2026-06-24

### Added

- Load and save full glTF scenes with Draco-compressed geometry.
- glTF/GLB import, explicit Draco decoding, and materialization into ordinary
  geometry.
- `compress` to Draco-compress a full scene while preserving materials,
  textures, nodes, animations, skins, and unknown extensions.
- Draco-aware, panic-safe validation on import.
- WebAssembly support, and an optional `image` feature to shrink the build when
  texture pixels are not needed.
