# Changelog — draco-io

Notable changes to the `draco-io` crate. This crate is versioned and released
independently; its release tags are `draco-io-vX.Y.Z`. It depends on a published
`draco-core`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
