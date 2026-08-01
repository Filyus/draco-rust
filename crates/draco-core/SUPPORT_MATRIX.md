# Support Matrix: draco-core vs C++ Draco

How `draco-core` maps onto upstream C++ Draco. `draco-core` is the raw `.drc`
**bitstream** layer only; file and scene formats live one level up in `draco-io`
(OBJ / PLY / STL / FBX) and `draco-gltf` (full glTF / GLB scenes). The C++ reference is
a local checkout of upstream Draco, wherever it is kept; the tools that compare
against it take its path from the environment rather than assuming one.

## Status labels

| Label | Meaning |
|---|---|
| yes | Implemented. |
| explicit | Supported, but only by manual selection — never auto-chosen. |
| — | Supported, but not a default choice for new streams (only on explicit request). |

## Crate boundary

Which crate owns each area — so nothing reads as a gap when it is simply handled
at a different level of abstraction.

| Area | C++ | Crate |
|---|---:|---|
| `.drc` point-cloud / triangle-mesh / keyframe bitstream | yes | `draco-core` |
| `.drc` geometry & attribute metadata | yes | `draco-core` |
| glTF / GLB scenes + `KHR_draco_mesh_compression` | yes | `draco-gltf` (geometry via `draco-core`, document-preserving compress via `draco-io`) |
| glTF materials / textures / nodes / animations / skins / lights / extensions | yes (transcoder) | `draco-gltf` — understood references are preserved across a Draco round-trip; opaque unknown binary references are rejected rather than guessed |
| glTF `EXT_structural_metadata` / `EXT_mesh_features` | yes (glTF path) | `draco-gltf` — property-table buffer views are kept alive and remapped across compaction; feature IDs ride on vertex attributes the encoder returns unchanged. Richer semantic interpretation is future scene-layer work |
| glTF extensions across a Draco transform | yes (glTF path) | `draco-gltf` — extensions with no binary references are declared, the two that own references remap them, and anything unregistered still refuses rather than being guessed at. See `GLTF_2_1_SUPPORT.md` |
| OBJ / PLY / STL / FBX | yes | `draco-io` |

## Raw geometry bitstreams

All decode and encode in `draco-core`.

| Draco path | C++ | `draco-core` | Notes |
|---|---:|:--|---|
| Point cloud, sequential | yes | yes | Byte-identical to C++ Draco over the 132 encodes in `parity_point_clouds.rs`: four clouds, every speed, and the method left unset as well as forced. |
| Point cloud, KD-tree | yes | yes | Same suite. Reproducing the order `std::partition` leaves is part of it -- see `PointDVector::partition`. |
| Triangle mesh, sequential | yes | yes | Byte-identical over the 66 encodes in `parity_encode_attributes.rs` and the 33 seamed ones in `parity_attribute_seams.rs`, at every speed. |
| Triangle mesh, EdgeBreaker standard | yes | yes | Main compressed mesh path. |
| Triangle mesh, EdgeBreaker valence | yes | yes | Behind `edgebreaker_valence_*`; decode covers every version, encode writes current streams by default and, with `legacy_bitstream_encode`, round-trips at 2.2, 2.1, 2.0, 1.2 and 1.1. See [Legacy & compatibility](#legacy--compatibility) for why encode is an enumeration and decode is not. |
| Triangle mesh, EdgeBreaker predictive (type `1`) | yes (≤ `0.9.1`) | decode yes, encode explicit | Legacy connectivity; behind the legacy features. See [Legacy & compatibility](#legacy--compatibility). |

## Attribute encoders & semantics

All four sequential attribute encoders decode and encode in `draco-core`:
`GENERIC`, `INTEGER`, `QUANTIZATION`, `NORMALS` (octahedral transform).

Semantics `POSITION`, `NORMAL`, `TEX_COORD`, `COLOR`, `GENERIC` are all
supported. Draco attributes are typed point attributes plus a semantic tag;
format-specific meaning (glTF accessors, FBX layers) belongs above `draco-core`.

## Prediction schemes

| Scheme | Id | C++ | `draco-core` | Default |
|---|---:|:--|:--|:--|
| `PREDICTION_NONE` | -2 | yes | yes | — |
| `PREDICTION_DIFFERENCE` | 0 | yes | yes | yes |
| `MESH_PREDICTION_PARALLELOGRAM` | 1 | yes | yes | yes |
| `MESH_PREDICTION_MULTI_PARALLELOGRAM` | 2 | decode only* | decode yes, encode explicit | — |
| `MESH_PREDICTION_TEX_COORDS_DEPRECATED` | 3 | decode only* | decode yes, encode explicit | — |
| `MESH_PREDICTION_CONSTRAINED_MULTI_PARALLELOGRAM` | 4 | yes | yes | yes |
| `MESH_PREDICTION_TEX_COORDS_PORTABLE` | 5 | yes | yes | yes |
| `MESH_PREDICTION_GEOMETRIC_NORMAL` | 6 | yes | yes | yes |

\* The public C++ encoder rejects ids 2 and 3; they remain real C++ *decoders*.
`draco-core` keeps their encode behind `legacy_bitstream_encode` + manual
selection, never auto-chosen.

## Prediction transforms

| Transform | C++ | `draco-core` | Notes |
|---|---:|:--|---|
| Default / delta, `PREDICTION_TRANSFORM_WRAP` | yes | yes | Simple prediction paths. |
| `NORMAL_OCTAHEDRON` (id 2) | encode ≤ `0.9.1` | decode yes, encode explicit | Legacy normal transform; see [Legacy & compatibility](#legacy--compatibility). |
| `NORMAL_OCTAHEDRON_CANONICALIZED` (id 3) | yes (since `0.10.0`) | yes | Modern normal transform. |

## Entropy & bit coding

All decode and encode in `draco-core`: rANS bit coding, rANS symbol coding,
tagged symbols, raw symbols, direct bit coding, folded bit32 coding.

## Metadata

C++ Draco stores metadata entries as untyped byte blobs, with typed (`int32`,
`double`, array, string) APIs layered over the same bytes. `draco-core` follows
the same model and round-trips it through encode/decode.

| Feature | `draco-core` | Notes |
|---|:--|---|
| Geometry-level entries | yes | On `PointCloud`; `Mesh` inherits via its base. |
| Attribute metadata (by unique id) | yes | Keyed by Draco unique id, not vector index. |
| Lookup by string entry | yes | Mirrors C++ `GetAttributeMetadataByStringEntry`. |
| Nested sub-metadata | yes | C++-matching nesting limit. |
| Binary + typed (`int32`/`double`/array/string) values | yes | Raw bytes are the base API; typed helpers mirror C++ byte layout (explicit little-endian). |
| Encode/decode round-trip | yes | When bitstream header flags are available. |

Empty values are rejected, matching C++ Draco's metadata decoder.

## Keyframe animation

> Likely legacy. C++ Draco's `KeyframeAnimation` (2017, ~Draco `1.3.4`) is used
> nowhere in Draco's own pipeline — no CLI, no glTF/USD I/O, no JS binding, only
> its own unit tests. glTF does not use it either (it compresses only geometry).
> Treat the Rust port as bitstream-parity completeness, not a recommended path.

`draco-core` ports the container, encoder, and decoder as a thin typed wrapper
over the sequential point-cloud path (`KeyframeAnimation`,
`KeyframeAnimationEncoder`, `KeyframeAnimationDecoder`): timestamp track
(unique id `0`), multiple tracks, and quantized data all work. It adds no
dependencies and could be dropped without affecting any other path. This is
unrelated to glTF node animation, which `draco-gltf` preserves at the scene
level (see [Crate boundary](#crate-boundary)).

## Legacy & compatibility

`draco-core` matches observable C++ behavior for existing streams, including
awkward but compatibility-sensitive details. Every bitstream version from
`0.9.1` to current decodes.

Encoding is an enumeration rather than a range.
`version::EncodeTarget::claimed_versions` is the list, and the rule for
membership is that the combination has an encode/decode round-trip test
comparing values rather than counts:

| Target | Versions written |
|---|---|
| Triangle mesh, EdgeBreaker | 2.2, 2.1, 2.0, 1.2, 1.1 |
| Triangle mesh, sequential | 2.2, 1.3 |
| Point cloud, sequential | 2.3, 1.3 |
| Point cloud, KD-tree | 2.3 |

`set_version` refuses anything else. It used to accept the whole interval from
1.0 to the newest — 259 values for a mesh, including minors that never existed
such as 1.42 — and most of them produced, with no error, a stream this crate's
own decoder rejects. So the list is shorter than what the setter accepted, and
longer than what it wrote correctly.

**What a legacy version can carry grew at the same time.** Six writers used to
put a newer layout inside an older stream. Five share one root cause: a
sub-buffer built with `EncoderBuffer::new()` reports version 0, and every
version branch reads 0 as "newest", so the EdgeBreaker traversal block, the
three prediction schemes that carry an rANS size prefix, and the pre-2.0
quantization parameters all wrote 2.2. The sixth is the topology split events,
which below 1.2 are two absolute `u32` ids and a byte for the edge with no
bit-coded section after them, and which were written in the 1.2 form at every
version. All six now follow the target.

Before that, 1.2 worked only for a mesh that avoided quantization and those
schemes, and 1.1 only for a mesh with no split events at all — which is every
grid, since EdgeBreaker walks a disc without meeting itself. The version numbers
in the table barely moved; the geometry they can be written from did.

All three traversals write every version in the EdgeBreaker row. What the table
cannot express is that the predictive traversal needs a target below 2.0, since
2.x connectivity has no predictive traversal to read back; that pairing is
refused on its own.

- **Feature flags.** Legacy decode lives behind `legacy_bitstream_decode`;
  legacy encode lives behind `legacy_bitstream_encode`; valence lives behind
  `edgebreaker_valence_*`.
- **EdgeBreaker predictive (type `1`).** C++ emitted it in `0.9.1` and replaced
  it with valence (type `2`) in `0.10.0`; `1.0.0`+ never emit it. `draco-core`
  decodes it, and encodes it only via the `force_predictive_traversal` option —
  never auto-selected.
- **Normal octahedron (id 2).** C++ used it through `0.9.1`, then switched to the
  canonicalized transform (id 3) in `0.10.0`. `draco-core` decodes id 2 and, for
  pre-2.0 streams, also uses the historical `0.9.1` octahedron-to-vector float
  conversion so byte output matches the old decoder exactly.
- **Pre-2.2 layout.** The pre-2.2 valence and constrained-multi-parallelogram
  layouts decode with the default compatibility feature and encode with
  `legacy_bitstream_encode`. They differ from
  current streams in: a separate main traversal symbol stream, raw-bit (not
  rANS) start faces, a split-count/mode prefix, hole events after topology
  splits, a 2-bit split-edge selector, fixed-u32 counts before bitstream 2.0,
  and an always-present header flags field.
- **Portable texcoord prediction** preserves the C++ cast/wrapping order around
  unsigned intermediate arithmetic.
