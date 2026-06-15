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
| raw roundtrip | Raw bitstream data is decoded, exposed, and encoded; higher-level helpers are tracked separately. |
| explicit | Available only by manual selection; not part of default encoder choice. |
| no | Not implemented. |
| n/a | Not meaningful for this layer. |

## Scope Boundary

| Area | C++ Draco | Belongs in `draco-core`? | Belongs in `draco-io`? | Notes |
|---|---:|---:|---:|---|
| Raw `.drc` point-cloud bitstream | yes | yes | no | Core compression format. |
| Raw `.drc` triangle-mesh bitstream | yes | yes | no | Core compression format. |
| Raw `.drc` keyframe animation bitstream | yes | yes | no | Implemented as a point-cloud-like sequential stream, matching C++. |
| Geometry/attribute metadata in `.drc` | yes | yes | no | Bitstream-level metadata, not glTF metadata. |
| Scene graph | yes, mainly transcoder | no | yes | `draco-core` should not become a scene SDK. |
| glTF node animation and skins | yes, transcoder | no | possible | File/scene concern, currently rejected by `draco-io` glTF reader. |
| glTF / GLB `KHR_draco_mesh_compression` | yes | no | yes | Container I/O around Draco payloads. |
| OBJ / PLY / FBX I/O | yes | no | yes | Format import/export, not raw bitstream logic. |
| Structural metadata for glTF | yes, transcoder | no | future semantic glTF layer | Scene/file-format concern, not raw Draco metadata; support belongs with `EXT_mesh_features` / 3D Tiles-style workflows. |

## Raw Geometry Bitstreams

| Draco path | C++ decode | C++ encode | `draco-core` decode | `draco-core` encode | Practical status |
|---|---:|---:|---:|---:|---|
| Point cloud, sequential | yes | yes | yes | yes | Core parity path. |
| Point cloud, KD-tree | yes | yes | yes | yes | Core parity path; C++ notes KD-tree is not applicable to float values in some encoder tests. |
| Triangle mesh, sequential | yes | yes | yes | yes | Core parity path. |
| Triangle mesh, EdgeBreaker standard | yes | yes | yes | yes | Main compressed mesh path. |
| Triangle mesh, EdgeBreaker valence | yes | yes | yes | yes | Behind `edgebreaker_valence_decode` / `edgebreaker_valence_encode`. |
| Triangle mesh, EdgeBreaker predictive type `1` | yes | legacy through `0.9.1` | no | no | Legacy connectivity variant. C++ emitted type `1` in `0.9.1` and replaced it with valence type `2` in `0.10.0` in 2017; current C++ keeps decode compatibility. |

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
| `PREDICTION_TRANSFORM_NORMAL_OCTAHEDRON` | yes | legacy through `0.9.1` | shared base only | no | Legacy normal transform. Rust keeps the old octahedron base because canonicalized transform builds on the same math, but it does not currently accept old transform id `2` as a complete decode path. C++ emitted id `2` through `0.9.1` and switched normal encode to canonicalized in `0.10.0` in 2017; current C++ keeps decode compatibility. |
| `PREDICTION_TRANSFORM_NORMAL_OCTAHEDRON_CANONICALIZED` | yes | yes, since `0.10.0` | yes | yes | Main modern normal prediction transform; C++ normal encoder switched to it in 2017. |

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

C++ Draco metadata stores entries as untyped byte blobs. Its `int32`, `double`,
array, and string APIs are convenience helpers over those bytes, not a tagged
schema. `draco-core` follows the same model: raw bytes are the base API, and
typed helpers read/write the same entry payloads.

| Metadata feature | C++ support | `draco-core` status | Notes |
|---|---:|---|---|
| Geometry-level metadata entries | yes | yes | Stored on `PointCloud`; `Mesh` gets it through its point-cloud base. |
| Attribute metadata keyed by attribute unique id | yes | yes | Preserved by Draco attribute unique id, not attribute vector index. |
| Attribute metadata lookup by string entry | yes | yes | Mirrors C++ `GetAttributeMetadataByStringEntry` for helpers such as `"name" = "position"`. |
| Nested sub-metadata | yes | yes | Preserved with C++-matching nesting limit. |
| Binary/raw entry values | yes | yes | Raw `Vec<u8>` values remain the lossless public surface. |
| Typed helpers for `int32`, `double`, arrays, strings | yes | yes | Rust-style helpers mirror C++ byte layout without type tags. |
| Metadata encode/decode roundtrip | yes | yes | Metadata roundtrips for point clouds and meshes when bitstream header flags are available. |

Rust numeric helpers use explicit little-endian encoding for deterministic
output. Empty values are rejected because C++ Draco's metadata decoder rejects
zero-length entry data.

## Keyframe Animation

C++ Draco has a raw keyframe animation path, but it is narrower than general
scene animation.

| Animation feature | C++ support | `draco-core` status | Notes |
|---|---:|---|---|
| `KeyframeAnimation` container | yes | yes | Thin wrapper over `PointCloud` (`keyframe_animation::KeyframeAnimation`). |
| Timestamp track | yes | yes | Attribute unique id `0` reserved for `f32` timestamps. |
| Multiple keyframe tracks | yes | yes | Each track stored as a generic point attribute with matching frame count. |
| `KeyframeAnimationEncoder` | yes | yes | Routes through sequential point-cloud encoding. |
| `KeyframeAnimationDecoder` | yes | yes | Routes through sequential point-cloud decoding. |
| Quantized keyframe data | yes | yes | Reuses the existing quantization attribute path via encoder options. |
| glTF node animations | yes, transcoder | no | `draco-io` concern, not `draco-core`. |
| Skins / inverse bind matrices | yes, transcoder | no | `draco-io` concern unless a Rust scene crate appears. |

The raw keyframe animation container, encoder, and decoder are implemented as a
thin wrapper around the existing sequential point-cloud path, matching the C++
architecture. It is still separate from the mesh-focused `draco-io` glTF scope,
where node animations and skins are intentionally rejected today.

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
| EXT_structural_metadata / mesh features | yes in C++ glTF path | out of current scope | n/a | Semantic glTF/3D Tiles feature. Current `draco-io` should reject it when required, not silently claim support. |

## Practical Priorities

| Priority | Item | Why |
|---:|---|---|
| 1 | Support legacy decode only from real compatibility targets | EdgeBreaker predictive type `1` and old normal octahedron transform are still C++ decode compatibility targets, but modern C++ encoders do not emit them. |
| 2 | Keep deprecated prediction schemes explicit | C++ public encoder rejects them; supporting them is for compatibility, not defaults. |
| 3 | Add broader metadata utilities when needed | Raw and typed `.drc` metadata roundtrip exists; future work is merge/copy helpers if they become useful. |
| 4 | Keyframe animation wrapper (done) | Implemented as a point-cloud sequential encode/decode wrapper; future work is optional quantization tuning and ergonomics. |
| 5 | Keep semantic glTF features outside `draco-core` | Animations, skins, `EXT_structural_metadata`, and `EXT_mesh_features` are scene/container concerns; `draco-io` may handle them later as a separate semantic glTF layer. |

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
  `0.10.0` changed the same path to valence type `2` in 2017, and `1.0.0`
  already used standard (`0`) or valence (`2`) for encoder output. Type `1`
  remains a legacy decode compatibility target.
- `PREDICTION_TRANSFORM_NORMAL_OCTAHEDRON` follows the same compatibility shape:
  C++ emitted it for normal prediction through `0.9.1`, switched encoder output
  to `PREDICTION_TRANSFORM_NORMAL_OCTAHEDRON_CANONICALIZED` in `0.10.0` in
  2017, and still decodes both transform ids. Rust keeps the old transform base
  because the canonicalized transform reuses the same octahedron math; that is
  not the same as supporting old transform id `2` in the decoder.
- Metadata is preserved as raw bytes and roundtrips through Rust encode/decode.
  Typed helpers expose C++-compatible `int32`, `double`, array, and string
  convenience APIs over the same bytes; Rust writes numeric helpers in explicit
  little-endian order for deterministic streams.
- glTF `EXT_structural_metadata` is separate from raw `.drc` metadata. It should
  be treated as unsupported when required by a glTF asset unless a future
  semantic glTF / 3D Tiles layer is added together with `EXT_mesh_features`.
- Keyframe animation is implemented as a typed wrapper around the existing
  point-cloud path (`KeyframeAnimation`, `KeyframeAnimationEncoder`,
  `KeyframeAnimationDecoder`), mirroring the C++ point-cloud-like sequential
  stream. glTF node animations and skins remain out of scope.
