# glTF 2.1 support

`draco-gltf` and `draco-io` target **glTF 2.0** and support it fully. This note
explains how they behave with **glTF 2.1**.

## Status

glTF 2.1 was announced on 2026-06-11 and, as of 2026-06-21, is at an early
"explainer" stage — a **backward-compatible revision of the glTF 2.0 core** with
no ratified schema yet. These crates do **not** implement 2.1-specific features
yet. There is no urgency: every glTF 2.0 asset remains valid under 2.1, so
anything you read or produce today keeps working.

## What glTF 2.1 adds

Most of 2.1 is about composing large scenes — referencing external glTF files,
packaging them together, implicit shapes, bounding-volume hierarchies,
thumbnails, and unique IDs. A few smaller additions touch geometry data: new
accessor component types, relaxed (non-sequential) attribute indices, and a
64-bit GLB container.

## What happens if you use a glTF 2.1 asset today

- **glTF 2.0 content** — fully supported, unchanged.
- **New scene-level 2.1 content** (external assets, packaging, shapes, BVH,
  thumbnails, unique IDs) — **preserved**. Compression rewrites only the mesh
  geometry it compresses and carries the rest of the document through untouched,
  so content the library does not model still survives a round-trip.
- **New accessor component types** — `SIGNED_INT` (i32), `DOUBLE` (f64),
  `HALF_FLOAT` (f16), `SIGNED_INT64` (i64), `UNSIGNED_INT64` (u64): a primitive
  whose attribute uses one of these is **left uncompressed and preserved
  verbatim — never silently corrupted**. (In 2.1 these types are opt-in through
  extensions; standard meshes still use the 2.0 types.)
- **64-bit GLB (binary version 3)** — not yet read or written; the byte API works
  with standard (version 2) GLB. `draco-gltf` itself emits embedded glTF, so it
  is unaffected.

In short: your 2.0 assets are unaffected, and a 2.1 asset is handled safely —
compressed where possible, preserved where not.

## Why 2.1 features are not implemented yet

As of 2026-06-21, implementing them would be speculative, for reasons largely
outside this library:

- the 2.1 specification is at the **explainer stage** with **no ratified schema**
  ([tracking issue #2585](https://github.com/KhronosGroup/glTF/issues/2585),
  opened 2026-05-27);
- the underlying parser, [gltf-rs](https://crates.io/crates/gltf) (latest release
  **1.4.1**, 2024-05), does not model the new accessor component types — its
  `accessor::DataType` is still the six glTF 2.0 values — so they cannot even be
  parsed;
- the **Draco codec has no encoding** for half-, double-, or 64-bit-integer
  attributes (only `i32` maps to an existing Draco type).

(Versions and status above are point-in-time; re-check the tracking issue and the
gltf-rs release notes before relying on them.)

Support will be added once the specification and the surrounding ecosystem
stabilize. Until then, the safe preserve/skip behavior above is the intended
response — and it is covered by a regression test so it cannot silently change.

## References

- Announcement: <https://www.khronos.org/blog/introducing-gltf-2.1-with-complex-scenes>
- Tracking issue: <https://github.com/KhronosGroup/glTF/issues/2585>

---

<details>
<summary>Notes for contributors</summary>

When 2.1 features are implemented, the relevant extension points are:

- `draco-io` `gltf_geometry`: the glTF component-type constants,
  `GENERIC_COMPONENT_TYPES`, and `supported_semantic_spec`.
- `draco-io` `gltf_writer`: `component_type_for_data_type`,
  `validate_attribute_for_gltf`.
- `draco-gltf`: `draco_data_type` (gltf-rs `DataType` → `draco_core::DataType`),
  once gltf-rs models the new types.
- `draco-io` `gltf_compress`: `build_glb` / `split_glb` for GLB version 3.

Plus Draco encoder support for any non-`i32` type before claiming it. The safe
behavior is guarded by `compress_skips_primitive_with_gltf_2_1_component_type` in
`crates/draco-io/tests/gltf_compress_test.rs`.

</details>
