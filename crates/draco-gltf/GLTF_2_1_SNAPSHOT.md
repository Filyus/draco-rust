# glTF 2.1 draft snapshot

`draco-gltf` targets the glTF 2.1 draft at the tip of the `draft-2.1` branch of
KhronosGroup/glTF, commit
[`8e4bd40310d84bfb34f454fc696eafb2c20d8e51`](https://github.com/KhronosGroup/glTF/commit/8e4bd40310d84bfb34f454fc696eafb2c20d8e51),
resolved on 2026-09-29. It replaces the earlier snapshot, `77b44be7`, resolved
on 2026-07-18, which predates the draft's specification text and schemas.

The draft is not a moving build dependency. Updating this snapshot requires a
dedicated compatibility change: update this file, add/adjust fixtures and
validation tests, and document every public API or serialization change.

At this SHA the branch carries the draft specification text and JSON schemas
under `specification/2.1`. Strict checks follow them for `files` and their
`aliases`, `externalAssets`, shapes, bounding volumes, `asset.thumbnail`, the
GLB version 3 layout and `buffer.chunk`. The branch defines no schema for UIDs,
so their character rules stay unchecked, and no speculative schema is accepted
as strict validation. The remaining draft fields are preserved losslessly.

The branch is a work in progress and has moved since earlier snapshots, for
example the GLB version 3 chunk header. Compare a new snapshot against
`specification/2.1` before adopting it, not against the explainers.

## Targeted draft surface

- GLB version 3: 64-bit file and chunk lengths plus zero-valued reserved chunk
  encodings, while retaining GLB version 2 support.
- `files` references (`mimeType` plus exactly one of URI or buffer view),
  `externalAssets` model indirection, and explicit loading of nested assets.
- One preferred scene with read compatibility for legacy multiple scenes.
- Shapes, node bounding volumes, thumbnails, and object UIDs.
- Core accessor component-type definitions for signed 32-bit, half/double
  precision floats, and signed/unsigned 64-bit integers.
- Non-sequential `TEXCOORD_n` and `COLOR_n` primitive semantics.

The parser preserves all unknown JSON and extension payloads. Strict
validation and transformations only claim the subset listed above plus the
stable glTF 2.0 core needed by Draco operations.
