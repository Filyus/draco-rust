# draco-core Support Matrix

This document maps `draco-core` against the official C++ Draco implementation.
It is intentionally not a wishlist for a full 3D SDK. The baseline is:

1. What C++ Draco actually implements.
2. What is meaningful for `draco-core`, which is a raw Draco bitstream crate.
3. What should live in `draco-io` instead because it is file-format or scene I/O.

The C++ reference used here is the local checkout at `D:\Projects\Draco\src`.

## Status Labels

| Label | Meaning |
|---|---|
| yes | Implemented in the relevant project path. |
| partial | Some useful path exists, but important C++ behavior/API is missing. |
| skip-only | Parser advances over the data but does not retain or expose it. |
| explicit | Available only by manual selection; not part of default encoder choice. |
| no | Not implemented. |
| n/a | Not meaningful for this layer. |

## Scope Boundary

| Area | C++ Draco | Belongs in `draco-core`? | Belongs in `draco-io`? | Notes |
|---|---:|---:|---:|---|
| Raw `.drc` point-cloud bitstream | yes | yes | no | Core compression format. |
| Raw `.drc` triangle-mesh bitstream | yes | yes | no | Core compression format. |
| Raw `.drc` keyframe animation bitstream | yes | possible | no | C++ implements it as a point-cloud-like sequential stream. |
| Geometry/attribute metadata in `.drc` | yes | yes | no | Bitstream-level metadata, not glTF metadata. |
| Scene graph | yes, mainly transcoder | no | yes | `draco-core` should not become a scene SDK. |
| glTF node animation and skins | yes, transcoder | no | possible | File/scene concern, currently rejected by `draco-io` glTF reader. |
| glTF / GLB `KHR_draco_mesh_compression` | yes | no | yes | Container I/O around Draco payloads. |
| OBJ / PLY / FBX I/O | yes | no | yes | Format import/export, not raw bitstream logic. |
| Structural metadata for glTF | yes, transcoder | no | possible | Scene/file-format concern, not raw Draco metadata. |

## Raw Geometry Bitstreams

| Draco path | C++ decode | C++ encode | `draco-core` decode | `draco-core` encode | Practical status |
|---|---:|---:|---:|---:|---|
| Point cloud, sequential | yes | yes | yes | yes | Core parity path. |
| Point cloud, KD-tree | yes | yes | yes | yes | Core parity path; C++ notes KD-tree is not applicable to float values in some encoder tests. |
| Triangle mesh, sequential | yes | yes | yes | yes | Core parity path. |
| Triangle mesh, EdgeBreaker standard | yes | yes | yes | yes | Main compressed mesh path. |
| Triangle mesh, EdgeBreaker valence | yes | yes | yes | yes | Behind `edgebreaker_valence_decode` / `edgebreaker_valence_encode`. |
| Triangle mesh, EdgeBreaker predictive type `1` | yes | yes, through `0.9.1`; replaced by valence type `2` in `0.10.0` | no | no | Deprecated C++ connectivity variant. Public C++ `0.9.1` emitted type `1` on the predictive path; `0.10.0` changed that path to valence type `2`, and later versions keep type `1` as legacy decode compatibility. |

## Sequential Attribute Encoders

| Attribute encoder | C++ decode | C++ encode | `draco-core` decode | `draco-core` encode | Practical status |
|---|---:|---:|---:|---:|---|
| `SEQUENTIAL_ATTRIBUTE_ENCODER_GENERIC` | yes | yes | yes | yes | Raw/sequential attribute coding. |
| `SEQUENTIAL_ATTRIBUTE_ENCODER_INTEGER` | yes | yes | yes | yes | Integer attributes and prediction corrections. |
| `SEQUENTIAL_ATTRIBUTE_ENCODER_QUANTIZATION` | yes | yes | yes | yes | Quantized floating-point attributes. |
| `SEQUENTIAL_ATTRIBUTE_ENCODER_NORMALS` | yes | yes | yes | yes | Octahedral normal transform path. |

## Attribute Semantics

| Semantic | C++ support | `draco-core` support | Practical status |
|---|---:|---:|---|
| `POSITION` | yes | yes | Required for mesh encoding. |
| `NORMAL` | yes | yes | Includes normal transform and geometric normal prediction paths. |
| `TEX_COORD` | yes | yes | Includes portable texcoord prediction. |
| `COLOR` | yes | yes | Integer and normalized paths are covered through generic attribute handling. |
| `GENERIC` | yes | yes | Generic point attribute path. |

Draco attributes are fundamentally typed point attributes plus a semantic. Most
file-format-specific meaning, such as glTF accessor semantics or FBX layer
mapping, belongs above `draco-core`.

## Prediction Schemes

| Prediction scheme | Id | C++ decode | C++ encode | `draco-core` decode | `draco-core` encode | Default in new Rust streams | Practical status |
|---|---:|---:|---:|---:|---:|---:|---|
| `PREDICTION_NONE` | -2 | yes | yes | yes | yes | possible | Normal path. |
| `PREDICTION_DIFFERENCE` | 0 | yes | yes | yes | yes | yes | Normal path. |
| `MESH_PREDICTION_PARALLELOGRAM` | 1 | yes | yes | yes | yes | yes | Normal mesh path. |
| `MESH_PREDICTION_MULTI_PARALLELOGRAM` | 2 | yes | deprecated/rejected by public C++ encoder | yes | explicit | no | Compatibility/testing only. |
| `MESH_PREDICTION_TEX_COORDS_DEPRECATED` | 3 | yes | deprecated/rejected by public C++ encoder | yes | explicit | no | Compatibility/testing only. |
| `MESH_PREDICTION_CONSTRAINED_MULTI_PARALLELOGRAM` | 4 | yes | yes | yes | yes | yes | Modern multi-parallelogram family. |
| `MESH_PREDICTION_TEX_COORDS_PORTABLE` | 5 | yes | yes | yes | yes | yes | Modern texcoord path; Rust preserves C++ wrapping behavior. |
| `MESH_PREDICTION_GEOMETRIC_NORMAL` | 6 | yes | yes | yes | yes | yes | Normal-specific path. |

The legacy schemes are real C++ decoders, and C++ still has implementation
files for them. They are also explicitly rejected by the public C++ encoder
validation path. `draco-core` therefore keeps legacy encode support behind
`legacy_bitstream_encode` and manual selection instead of choosing it
automatically.

## Prediction Transforms

| Transform | C++ decode | C++ encode | `draco-core` decode | `draco-core` encode | Practical status |
|---|---:|---:|---:|---:|---|
| Default/delta transform | yes | yes | yes | yes | Used by simple prediction paths. |
| `PREDICTION_TRANSFORM_WRAP` | yes | yes | yes | yes | Integer wrap transform. |
| `PREDICTION_TRANSFORM_NORMAL_OCTAHEDRON` | yes | yes | no direct path | no direct path | C++ supports the older non-canonical transform. Rust has shared toolbox/base pieces, but normal attribute encode/decode uses the canonicalized transform. |
| `PREDICTION_TRANSFORM_NORMAL_OCTAHEDRON_CANONICALIZED` | yes | yes | yes | yes | Main modern normal prediction transform. |

## Entropy and Bit Coding

| Coder | C++ decode | C++ encode | `draco-core` decode | `draco-core` encode | Practical status |
|---|---:|---:|---:|---:|---|
| rANS bit coding | yes | yes | yes | yes | Core bit path. |
| rANS symbol coding | yes | yes | yes | yes | Core entropy path. |
| Tagged symbols | yes | yes | yes | yes | Modern symbol coding path. |
| Raw symbols | yes | yes | yes | yes | Raw symbol fallback/path. |
| Direct bit coding | yes | yes | yes | yes | Direct bitstream helper. |
| Folded bit32 coding | yes | yes | yes | yes | Folded integer bit helper. |

## Metadata

C++ Draco metadata is not just an ideal feature; it is implemented and exposed.
`draco-core` is currently behind here.

| Metadata feature | C++ support | `draco-core` status | Realistic Rust step |
|---|---:|---|---|
| Geometry-level metadata entries | yes | skip-only decode | Add `Metadata` and attach it to `PointCloud`/`Mesh`. |
| Attribute metadata keyed by attribute unique id | yes | skip-only decode | Add `AttributeMetadata` and unique-id lookup helpers. |
| Nested sub-metadata | yes | skip-only decode | Preserve recursive tree with hard nesting limits. |
| Binary entry values | yes | skip-only decode | Store raw `Vec<u8>` values first. |
| Typed helpers for `int32`, `double`, arrays, strings | yes | no public API | Add typed convenience methods after raw storage exists. |
| Metadata encode/decode roundtrip | yes | no | Add C++ fixture tests before enabling encode. |

The current Rust decoder follows the bitstream layout closely enough to advance
over metadata blocks before decoding geometry. That is useful compatibility, but
it is not metadata support.

## Keyframe Animation

C++ Draco has a raw keyframe animation path, but it is narrower than general
scene animation.

| Animation feature | C++ support | `draco-core` status | Realistic Rust step |
|---|---:|---|---|
| `KeyframeAnimation` container | yes | no | Add a thin wrapper over `PointCloud`. |
| Timestamp track | yes | no | Reserve attribute unique id `0` for float timestamps. |
| Multiple keyframe tracks | yes | no | Store each track as a point attribute with matching frame count. |
| `KeyframeAnimationEncoder` | yes | no | Route through sequential point-cloud encoding. |
| `KeyframeAnimationDecoder` | yes | no | Route through sequential point-cloud decoding. |
| Quantized keyframe data | yes | no | Reuse existing quantization attribute path. |
| glTF node animations | yes, transcoder | no | `draco-io` concern, not `draco-core`. |
| Skins / inverse bind matrices | yes, transcoder | no | `draco-io` concern unless a Rust scene crate appears. |

This is feasible in current Rust architecture because C++ itself implements
keyframe animation as a point-cloud-like wrapper. It is still separate from the
mesh-focused `draco-io` glTF scope, where animations and skins are intentionally
rejected today.

## Scene and Format I/O

These features exist in C++ Draco's broader repository, especially transcoder
builds, but they are not raw Draco bitstream features.

| Feature | C++ Draco | Current Rust location | `draco-core` status | Practical status |
|---|---:|---|---|---|
| glTF / GLB read/write | yes | `draco-io` | n/a | Keep outside `draco-core`. |
| `KHR_draco_mesh_compression` | yes | `draco-io` | n/a | Already the right layer for container validation. |
| glTF materials/textures/cameras/lights | yes in C++ transcoder | `draco-io` rejects/ignores by scope | n/a | Not useful for `draco-core`. |
| glTF animations/skins | yes in C++ transcoder | `draco-io` rejects today | n/a | Possible future `draco-io` scene work, not raw bitstream parity. |
| OBJ / PLY | yes | `draco-io` | n/a | File import/export helpers. |
| FBX | yes-ish/transcoder-side | `draco-io` | n/a | Keep lightweight; not a full SDK target. |
| EXT_structural_metadata / mesh features | yes in C++ glTF path | not supported | n/a | File/scene feature, separate from raw `.drc` metadata. |

## Practical Priorities

| Priority | Item | Why |
|---:|---|---|
| 1 | Finish raw mesh/point-cloud C++ parity tests | This is the crate's main job. |
| 2 | Add real `.drc` metadata retention | C++ supports it and Rust already has skip-only parsing, so this is a concrete gap. |
| 3 | Add keyframe animation wrapper if needed | Feasible because it reuses point-cloud sequential encode/decode. |
| 4 | Keep deprecated prediction schemes explicit | C++ public encoder rejects them; supporting them is for compatibility, not defaults. |
| 5 | Keep EdgeBreaker predictive decoder type `1` as fixture-driven legacy work | Current C++ can compile the legacy decoder, but does not emit type `1` from the normal encoder path. |
| 6 | Leave scene/glTF animation, skins, structural metadata to `draco-io` | They are not raw Draco bitstream concerns. |

## Compatibility Notes

`draco-core` aims to match observable C++ Draco behavior for existing streams,
including behavior that is awkward but compatibility-sensitive.

Important examples:

- Portable texcoord prediction preserves the C++ cast/wrapping order around
  unsigned intermediate arithmetic.
- Legacy prediction schemes are behind `legacy_bitstream_decode` and
  `legacy_bitstream_encode`.
- EdgeBreaker predictive connectivity traversal type `1` is deprecated in C++.
  Public Draco `0.9.1` emitted type `1` on the predictive path. Public Draco
  `0.10.0` changed the same path to valence type `2`, and `1.0.0` already used
  standard (`0`) or valence (`2`) for encoder output. Type `1` remains a legacy
  decode compatibility target.
- Metadata is currently only skipped, not preserved.
- Keyframe animation is not implemented yet, but is realistically implementable
  as a typed wrapper around the existing point-cloud path.
