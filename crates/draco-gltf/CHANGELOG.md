# Changelog — draco-gltf

Notable changes to the `draco-gltf` crate. This crate is versioned and released
independently; its release tags are `draco-gltf-vX.Y.Z`. It depends on published
`draco-core` and `draco-io`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-06-21

### Added

- Load and save full glTF scenes with Draco-compressed geometry, bridging
  [`gltf`](https://crates.io/crates/gltf) (the whole scene model) and the Draco
  crates.
- `import` / `import_slice` for reading Draco glTF/GLB, `decode_primitive` to
  decompress geometry, and `decompress_in_place` for transparent reading with
  the plain gltf-rs API.
- `compress` to Draco-compress a full scene while preserving materials,
  textures, nodes, animations, skins, and unknown extensions.
- Draco-aware, panic-safe validation on import (gltf-rs rejects Draco assets
  outright).
- WebAssembly support, and an optional `image` feature to shrink the build when
  texture pixels are not needed.
