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

The **Crate** column shows which layer owns each area, so nothing reads as a
gap when it is simply handled at a different level of abstraction.

| Area | C++ Draco | Crate | Notes |
|---|---:|---|---|
| Raw `.drc` point-cloud bitstream | yes | `draco-core` | Core compression format. |
| Raw `.drc` triangle-mesh bitstream | yes | `draco-core` | Core compression format. |
| Raw `.drc` keyframe animation bitstream | yes | `draco-core` | Implemented as a point-cloud-like sequential stream, matching C++. |
| Geometry/attribute metadata in `.drc` | yes | `draco-core` | Bitstream-level metadata, not glTF metadata. |
| Scene graph | yes, mainly transcoder | `draco-io` | `draco-core` is not a scene SDK. |
| glTF node animation and skins | yes, transcoder | `draco-io` (rejected today) | File/scene concern; currently rejected by the `draco-io` glTF reader. |
| glTF / GLB `KHR_draco_mesh_compression` | yes | `draco-io` | Container I/O around Draco payloads. |
| OBJ / PLY / FBX I/O | yes | `draco-io` | Format import/export, not raw bitstream logic. |
| Structural metadata for glTF | yes, transcoder | `draco-io` (future) | Scene/file-format concern, not raw Draco metadata; belongs with `EXT_mesh_features` / 3D Tiles-style workflows. |

## Raw Geometry Bitstreams

| Draco path | C++ decode | C++ encode | `draco-core` decode | `draco-core` encode | Practical status |
|---|---:|---:|---:|---:|---|
| Point cloud, sequential | yes | yes | yes | yes | Core parity path. |
| Point cloud, KD-tree | yes | yes | yes | yes | Core parity path; C++ notes KD-tree is not applicable to float values in some encoder tests. |
| Triangle mesh, sequential | yes | yes | yes | yes | Core parity path. |
| Triangle mesh, EdgeBreaker standard | yes | yes | yes | yes | Main compressed mesh path. |
| Triangle mesh, EdgeBreaker valence | yes | yes | yes | yes | Behind `edgebreaker_valence_decode` / `edgebreaker_valence_encode`. Decode covers every bitstream version (the pre-2.2 layout — main symbol stream, raw start faces, split/mode prefix, pre-2.0 fixed-u32 counts — is handled behind `legacy_bitstream_decode`); encode round-trips bitstream 1.2 through current behind `legacy_bitstream_encode`. |
| Triangle mesh, EdgeBreaker predictive type `1` | yes | legacy through `0.9.1` | yes | explicit | Legacy connectivity variant (a binary prediction stream guessing R/C from local valence). C++ emitted type `1` in `0.9.1` and replaced it with valence type `2` in `0.10.0` in 2017. Decode is supported behind `legacy_bitstream_decode`; encode is opt-in via the `force_predictive_traversal` option behind `legacy_bitstream_encode` (never auto-selected — no modern tool emits type `1`). Round-trips both directions. |

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
| `MESH_PREDICTION_MULTI_PARALLELOGRAM` | 2 | yes | deprecated/rejected by public C++ encoder | yes | explicit | — | Compatibility/testing only. |
| `MESH_PREDICTION_TEX_COORDS_DEPRECATED` | 3 | yes | deprecated/rejected by public C++ encoder | yes | explicit | — | Compatibility/testing only. |
| `MESH_PREDICTION_CONSTRAINED_MULTI_PARALLELOGRAM` | 4 | yes | yes | yes | yes | yes | Modern multi-parallelogram family. The pre-2.2 form (a leading optimal-mode byte; crease-edge rANS streams with a fixed-u32 size prefix) round-trips both directions behind the legacy features. |
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
| `PREDICTION_TRANSFORM_NORMAL_OCTAHEDRON` | yes | legacy through `0.9.1` | yes | explicit | Legacy normal transform. C++ emitted id `2` through `0.9.1` and switched normal encode to canonicalized in `0.10.0` in 2017; Rust decodes it behind `legacy_bitstream_decode`, including the historical 0.9.1 octahedron-to-vector float conversion for byte-exact legacy output. Rust encodes it behind `legacy_bitstream_encode` when targeting pre-1.2 normal streams. |
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

> Likely legacy. C++ Draco's `KeyframeAnimation` dates to 2017 (copyright
> `2017`, first public snapshot around Draco `1.3.4` in 2018). It predates the
> glTF/USD transcoder and is **used nowhere in Draco's own pipeline** — no CLI
> tool, no glTF/USD I/O, no JavaScript binding references it; only its own unit
> tests do. glTF does not use it either: glTF compresses only mesh geometry via
> `KHR_draco_mesh_compression`, while animation/skin data stays uncompressed
> (see [Scene and Format I/O](#scene-and-format-io)). Treat the Rust port as
> bitstream-parity completeness, not a recommended path — there is no known
> consumer that needs it.

C++ Draco has a raw keyframe animation path, but it is narrower than general
scene animation. It is unrelated to glTF node animation, which is a transcoder
concern carried as uncompressed data.

| Animation feature | C++ support | `draco-core` status | Notes |
|---|---:|---|---|
| `KeyframeAnimation` container | yes | yes | Thin wrapper over `PointCloud` (`keyframe_animation::KeyframeAnimation`). |
| Timestamp track | yes | yes | Attribute unique id `0` reserved for `f32` timestamps. |
| Multiple keyframe tracks | yes | yes | Each track stored as a generic point attribute with matching frame count. |
| `KeyframeAnimationEncoder` | yes | yes | Routes through sequential point-cloud encoding. |
| `KeyframeAnimationDecoder` | yes | yes | Routes through sequential point-cloud decoding. |
| Quantized keyframe data | yes | yes | Reuses the existing quantization attribute path via encoder options. |
| glTF node animations | yes, transcoder | n/a | `draco-io` concern, not `draco-core`. |
| Skins / inverse bind matrices | yes, transcoder | n/a | `draco-io` concern unless a Rust scene crate appears. |

The raw keyframe animation container, encoder, and decoder are implemented as a
thin wrapper around the existing sequential point-cloud path, matching the C++
architecture. It is still separate from the mesh-focused `draco-io` glTF scope,
where node animations and skins are intentionally rejected today.

This was ported to complete raw-bitstream parity, not because a consumer
requires it. Because it is just a typed view over the already-supported
sequential point-cloud encode/decode, it adds no new dependencies and minimal
maintenance surface. If a future cleanup prefers to carry only actively used
surface, it can be dropped without affecting any other path.

## Scene and Format I/O

These features exist in C++ Draco's broader repository, especially transcoder
builds, but they are not raw Draco bitstream features.

| Feature | C++ Draco | Current Rust location | `draco-core` status | Practical status |
|---|---:|---|---|---|
| glTF / GLB read/write | yes | `draco-io` | n/a | Keep outside `draco-core`. |
| `KHR_draco_mesh_compression` | yes | `draco-io` | n/a | Already the right layer for container validation. |
| glTF materials/textures/cameras/lights | yes in C++ transcoder | `draco-io`: not in geometry model, but **preserved** by document-preserving compression (`gltf_compress`) | n/a | Not useful for `draco-core`. The geometry model does not interpret them; the in-place glTF compressor carries them through untouched. |
| glTF animations/skins | yes in C++ transcoder | `draco-io`: geometry model rejects; **preserved** by `gltf_compress` (animations/skins carried through; skinned geometry *is* compressed — `JOINTS_n`/`WEIGHTS_n` ride in the Draco stream as generic attributes named via the extension map, like C++) | n/a | Possible future `draco-io` scene work, not raw bitstream parity. |
| OBJ / PLY | yes | `draco-io` | n/a | File import/export helpers. |
| FBX | yes-ish/transcoder-side | `draco-io` | n/a | Keep lightweight; not a full SDK target. |
| EXT_structural_metadata / mesh features | yes in C++ glTF path | out of current scope | n/a | Semantic glTF/3D Tiles feature. Current `draco-io` should reject it when required, not silently claim support. |

## Practical Priorities

| Priority | Item | Why |
|---:|---|---|
| 1 | Legacy decode/encode from real compatibility targets (largely done) | Every bitstream version from `0.9.1` to current decodes, including EdgeBreaker predictive type `1`, the pre-2.2 valence layout, pre-2.2 constrained-multi-parallelogram prediction, and the old normal octahedron transform id `2` with historical 0.9.1 float output. Every traversal (standard/predictive/valence) round-trips, including pre-1.2 normal octahedron streams behind the legacy encode feature. |
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
  already used standard (`0`) or valence (`2`) for encoder output. `draco-core`
  decodes type `1` (behind `legacy_bitstream_decode`) and can encode it on
  request (the `force_predictive_traversal` option behind
  `legacy_bitstream_encode`); it is never auto-selected, since no current tool
  emits type `1`.
- The pre-2.2 valence and constrained-multi-parallelogram layouts round-trip in
  both directions behind the legacy features. The pre-2.2 connectivity differs
  from current streams in several ways the legacy paths handle: a separate main
  traversal symbol stream, raw-bit (not rANS) start faces, a split-count/mode
  prefix, hole events stored after the topology splits, a 2-bit split edge
  selector, fixed-u32 counts before bitstream 2.0, and an always-present header
  flags field.
- `PREDICTION_TRANSFORM_NORMAL_OCTAHEDRON` follows the same compatibility shape:
  C++ emitted it for normal prediction through `0.9.1`, switched encoder output
  to `PREDICTION_TRANSFORM_NORMAL_OCTAHEDRON_CANONICALIZED` in `0.10.0` in
  2017, and still decodes both transform ids. Rust decodes old transform id `2`
  behind `legacy_bitstream_decode` and can emit it behind
  `legacy_bitstream_encode` when targeting pre-1.2 normal streams. For pre-2.0
  normal streams, Rust also uses the historical `0.9.1`
  octahedron-to-vector float conversion instead of the modern Draco conversion
  so byte output matches the old decoder exactly.
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
  stream. This is a 2017-era, likely-legacy C++ feature that nothing in Draco's
  own pipeline (or glTF) consumes; the Rust port exists for bitstream-parity
  completeness only. glTF node animations and skins remain out of scope.
