# Changelog

All notable changes to the publishable Draco Rust crates are documented here.

## [Unreleased]


## [0.1.0] - 2026-06-15

### Initial Release
- Publish `draco-core`, a pure Rust encoder and decoder for Draco `.drc` meshes
  and point clouds, with sequential, KD-tree, EdgeBreaker, metadata, and legacy
  bitstream compatibility paths.
- Publish `draco-io`, format readers and writers for OBJ, PLY, binary FBX,
  glTF, and GLB, including `KHR_draco_mesh_compression` support through
  `draco-core`.
- Include WASM wrapper crates and a browser demo in the repository; release
  builds package the WASM modules as GitHub Release assets.
- Document the decode threat model, fuzzing workflow, support matrix, benchmark
  suite, and C++ parity test bridge.
