# Changelog — draco-io

## 0.3.0

- Breaking: `draco-io` now provides OBJ, PLY, FBX, and low-level glTF
  container/resource/accessor contracts only. Full glTF scene operations live
  in `draco-gltf` 0.2.
- Removed `serde`, `serde_json`, and `nanoserde` from runtime dependencies.
- Added strict GLB v2/v3 inspection and byte-level container serialization.

This crate is versioned and released independently. Its release tags are
`draco-io-vX.Y.Z`.
