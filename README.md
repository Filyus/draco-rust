# draco-rust

Rust implementation of Draco geometry encoding and decoding.

- `draco-core`: bitstream and geometry codec.
- `draco-io`: OBJ, PLY, FBX plus low-level GLB/resource/accessor contracts.
- `draco-gltf`: lossless glTF 2.0 and pinned 2.1-draft documents,
  compact views, Draco transforms and GLB v2/v3 output.

Full glTF applications should depend on `draco-gltf`; `draco-io` intentionally
does not expose a glTF scene API.
