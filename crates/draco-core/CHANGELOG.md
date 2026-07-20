# Changelog — draco-core

Notable changes to the `draco-core` crate. This crate is versioned and released
independently; its release tags are `draco-core-vX.Y.Z`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.4](https://github.com/Filyus/draco-rust/compare/draco-core-v1.0.3...draco-core-v1.0.4) - 2026-07-20

### Fixed

- Feature-disabled builds no longer compile unused codec internals or encoder
  helpers, so supported minimal feature combinations pass with `-D warnings`.

## [1.0.3](https://github.com/Filyus/draco-rust/compare/draco-core-v1.0.2...draco-core-v1.0.3) - 2026-06-27

### Performance

- Reduced allocation and setup overhead in selected mesh codec hot paths.

## [1.0.2] - 2026-06-24

### Fixed

- Malformed legacy EdgeBreaker streams now validate symbol-count invariants
  before allocating mesh face storage, avoiding a fuzz-discovered OOM path.
- `PointCloudDecoder` now rejects mesh bitstream headers directly.

## [1.0.1] - 2026-06-24

### Fixed

- EdgeBreaker mesh encoding now reports a clear error when a pre-2.2 bitstream
  or forced predictive traversal is requested without `legacy_bitstream_encode`,
  instead of producing an invalid legacy-shaped stream.
- Legacy prediction-scheme encode requests now report a clear error when
  `legacy_bitstream_encode` is disabled.

## [1.0.0] - 2026-06-24

First stable release. The public API is now covered by SemVer (breaking changes
mean a new major version). The `.drc` bitstream it reads and writes is Google
Draco's stable format.

### Added

- Pure-Rust encoder and decoder for Draco `.drc` meshes and point clouds.
- Sequential, KD-tree, and EdgeBreaker connectivity paths; attribute
  quantization, prediction schemes, and normal octahedron transforms.
- Metadata and legacy-bitstream compatibility paths for older Draco streams.
- Feature flags to build decoder-only, encoder-only, or point-cloud-only
  configurations for smaller binaries.
