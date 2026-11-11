# glTF 2.1 draft snapshot

`draco-gltf` targets the glTF 2.1 draft on KhronosGroup/glTF's `draft-2.1`
branch at commit
[`fab13e27aa347f22b57a829e76401b760a25cbe7`](https://github.com/KhronosGroup/glTF/commit/fab13e27aa347f22b57a829e76401b760a25cbe7),
resolved on 2026-11-10. It replaces `8e4bd403`, resolved on 2026-09-29.
The compatibility update was implemented on 2026-11-11.

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

- GLB version 3: 63-bit lengths in 64-bit fields, chunks starting on 8-byte
  boundaries, and padding outside chunk lengths. Import resolves explicit
  buffer references to any plain chunk type and reads the declared buffer
  prefix. Inspection exposes encoding metadata; payload loading requires a
  known encoding. The writer consolidates buffers into one BIN chunk.
- `files` references (`mimeType` plus exactly one of URI or buffer view),
  `externalAssets` model indirection, and explicit loading of nested assets.
- One preferred scene with read compatibility for legacy multiple scenes.
- Shapes, including `mesh` and `convexMesh` references, node bounding volumes,
  thumbnails, and the existing partial UID support described in the support
  table. Mesh shape validation covers references and primitive structure;
  geometric convexity, volume, and winding remain caller responsibilities.
- Accessor and built-in attribute formats from the pinned specification, with
  the additional formats enabled by a required `KHR_mesh_quantization`.
- Consecutive indexed attribute semantics starting at zero, without leading
  zeroes, and with at most nine digits under the draft profile.

## Compatibility changes in this update

The accessor tables and schema at this SHA enumerate `5120`, `5121`, `5122`,
`5123`, `5125`, and `5126`. Strict document validation and primitive reads and
writes now reject the proposed `i32`, `f16`, `f64`, `i64`, and `u64` storage
previously accepted by `Gltf21Draft`. `ComponentType` still represents these
codes, and `Document::from_json_bytes` preserves their original JSON.

The previous documentation also advertised non-consecutive texture-coordinate
and color sets. Both this snapshot and its predecessor require consecutive
indices. Validation now enforces the tables and indexed-semantic syntax.

Extension promotion and expanded component definitions remain tracked by
[#2592](https://github.com/KhronosGroup/glTF/issues/2592) and
[#2596](https://github.com/KhronosGroup/glTF/issues/2596). The pinned normative
text continues to require the extension declaration for quantized attributes,
including when `asset.minVersion` is `"2.1"`.

The parser preserves all unknown JSON and extension payloads. Strict
validation and transformations only claim the subset listed above plus the
stable glTF 2.0 core needed by Draco operations.
