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
- **Valid but unsupported geometry in the `draco-io` document compressor** —
  preserved with a typed report when its JSON and binary references are fully
  understood.
- **`draco-gltf` parsing** — still follows gltf-rs's glTF 2.0 model. A 2.1
  component type that gltf-rs cannot represent is rejected; it is not promised
  to round-trip through the full-scene typed API.
- **Unknown scene JSON** — preserved when it has no opaque binary-reference
  semantics. Unknown extension fields named like `buffer`, `bufferView`, or
  offsets cause an explicit `OpaqueBinaryReference` error before repacking.
- **64-bit GLB (binary version 3)** — rejected. Only GLB 2.0 is read or written.

In short: glTF 2.0 is the supported contract. 2.1 input is never silently
reinterpreted, but preservation is conditional rather than universal.

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

Support will be added once the specification and surrounding ecosystem
stabilize. Until then, typed preserve reports and controlled errors define the
boundary; there is no blanket 2.1 round-trip promise.

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
- `draco-io` `gltf_container` for any future GLB version 3 work.

Plus Draco encoder support for any non-`i32` type before claiming it. The safe
behavior is guarded by `compress_skips_primitive_with_gltf_2_1_component_type` in
`crates/draco-io/tests/gltf_compress_test.rs`.

</details>
