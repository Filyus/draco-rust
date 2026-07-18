# glTF 2.1 draft snapshot

`draco-gltf` 0.2 targets the glTF 2.1 draft at KhronosGroup/glTF commit
[`77b44be7bef26e01fb0b140e3d5bb1716421c5e9`](https://github.com/KhronosGroup/glTF/commit/77b44be7bef26e01fb0b140e3d5bb1716421c5e9),
resolved on 2026-07-18.

The draft is not a moving build dependency. Updating this snapshot requires a
dedicated compatibility change: update this file, add/adjust fixtures and
validation tests, and document every public API or serialization change.

## Targeted draft surface

- GLB version 3: 64-bit file and chunk lengths plus zero-valued reserved chunk
  encodings, while retaining GLB version 2 support.
- `files` references and explicit loading of nested external assets.
- One preferred scene with read compatibility for legacy multiple scenes.
- Shapes, node bounding volumes, thumbnails, and object UIDs.
- Core accessor component-type definitions for signed 32-bit, half/double
  precision floats, and signed/unsigned 64-bit integers.
- Non-sequential `TEXCOORD_n` and `COLOR_n` primitive semantics.

The parser preserves all unknown JSON and extension payloads. Strict
validation and transformations only claim the subset listed above plus the
stable glTF 2.0 core needed by Draco operations.
