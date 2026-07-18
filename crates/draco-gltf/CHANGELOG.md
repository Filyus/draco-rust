# Changelog — draco-gltf

Notable changes to the `draco-gltf` crate. This crate is versioned and released
independently; its release tags are `draco-gltf-vX.Y.Z`. It depends on published
`draco-core` and `draco-io`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-18

### Changed

- Breaking: introduced lossless `Document` typed views and indices.
  See `MIGRATING_0_2.md`.
- Added the pinned glTF 2.1 draft profile, GLB v3 support, extension
  contracts, explicit `files` asset loading, and Draco decode/compress/
  decompress operations.

### Added

- Clean compression API on `Import`: shared `GltfCompressionOptions`,
  `CompressionOutput`, typed preserve reports, `SameAsInput`/embedded/GLB
  output, and `to_bytes` after decompression.
- `ImportOptions` with synchronous resource resolvers, external-file policy,
  and optional per-resource, total-buffer, and image-pixel quotas.
- Custom `_*` semantics, `extras`, unknown extension JSON, and arbitrary object
  field preservation, plus a deterministic `gltf_tool` interoperability
  example.

### Changed

- `draco_attribute_map` is strict and returns
  `Result<Option<BTreeMap<String, u32>>>`; IDs are Draco unique IDs, not
  positional attribute indices.
- Compression uses the common `draco-io` container, resource, KHR schema, and
  options/report types. The former free `compress(document, buffers)` API is
  replaced by `Import::compress[_with_options]`.

### Fixed

- `decompress_in_place` is atomic, validates its replacement document, clones
  shared accessors, preserves unmapped attributes, and always materializes
  indexed `TRIANGLES` for strips and non-indexed primitives.
- Decoded attributes and indices are checked against accessor semantic, layout,
  component type, normalization, and count contracts.
- Checked range arithmetic, preflight bounds checks, fallible allocations, and
  fallible Draco buffer reads replace panic/OOM-prone paths.

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
