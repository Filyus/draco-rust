# Changelog — draco-core

Notable changes to the `draco-core` crate. This crate is versioned and released
independently; its release tags are `draco-core-vX.Y.Z`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.0.0](https://github.com/Filyus/draco-rust/compare/draco-core-v1.2.0...draco-core-v2.0.0) - 2026-08-02

A breaking release. Its through-line is that a refusal should say what it
refused. Ninety-odd functions across the decode and encode paths returned a
`bool` whose `false` covered a truncated buffer, an uninitialised transform and
an unsupported feature alike, and the reason — where one existed at all — went
to a `debug_log!` the release build drops. `DracoError` itself became an opaque
struct with an `ErrorKind`, which is the change every caller sees. Alongside
that, the bitstream versions an encode accepts are now an enumeration of
combinations that have a round-trip test, the header count guard is replaced by one whose
premise holds, and the encoder reports the choices it makes for itself.

### Added

- `PointCloudEncoder::encoded_point_cloud_info` and `EncodedPointCloudInfo`
  report what a point-cloud encode decided, so the KD-tree-versus-sequential
  choice — which the encoder makes on its own whenever every attribute is
  eligible — is visible. `EncodedMeshInfo` gains the resolved bitstream version,
  EdgeBreaker traversal, speed and split-connectivity flag; `EncodedAttributeInfo`
  gains the prediction scheme, prediction transform and the quantization bits a
  transform actually applied. The values are the resolved ones, not the requested
  ones: several encoder arms fall back to `Difference` when the attribute or the
  mesh cannot support what was asked for, and a `quantization_bits` set on an
  integer attribute is not repeated back as though it had been used. Both structs
  are `#[non_exhaustive]`.
- `ErrorKind::AllocationExceedsInput` tells apart a decode refused for asking to
  allocate more than its input could describe.
- `EncoderOptions::set_compression_level`/`get_compression_level` and
  `set_attribute_quantization`/`get_attribute_quantization` cover what the
  `draco_encoder` CLI's `-cl` and `-qp`/`-qt`/`-qn`/`-qg` resolve to at this
  layer, for callers porting CLI settings. `EncoderOptions` already mirrors
  the C++ *library's* `encoding_speed`/`decoding_speed`/`quantization_bits`
  keys byte-for-byte; the CLI's `-cl` is a different, inverted number
  (`speed = 10 - compression_level`) that exists one layer up, in the CLI
  binary alone, and the two were easy to conflate — passing a CLI compression
  level straight into `encoding_speed` silently asks for the opposite of what
  was intended. The new methods are the CLI's own conversion and nothing more;
  see `API.md` for the full comparison.
- `PointAttribute::read_f32s` and `DataBuffer::update_f32s_le` move a float
  channel out of and into an attribute buffer in one pass, without a per-value
  intermediate. They are inverses and are meant to be used as a pair.
  `read_f32s` widens whatever component type the attribute stores, follows the
  value index mapping — so an attribute with fewer unique values than points
  still lands on the right one — and always returns `num_points * components`
  values, zero-filling what the attribute has nothing to give. The length is
  the point of it: the result is a channel addressed by vertex index, so a
  short row for an attribute with fewer components than asked for shifts every
  later vertex onto the wrong values, which is worse than a zero. The four
  copies of this reader that preceded it had drifted into three different
  answers, and two of them indexed the buffer by point directly, which is only
  correct while the mapping is the identity. `update_f32s_le` is named for
  `update` rather than for `write` because it grows the buffer past its end,
  which `write` panics on and `try_write` refuses, and it counts as an update
  so a buffer whose count did not move genuinely was not touched.

### Changed

- The crate's freedom from `unsafe` is now enforced by the compiler rather than
  asserted in a document. `SECURITY.md` has long promised it and nothing
  checked it; the manifest carries `[lints.rust] unsafe_code = "forbid"`, so an
  `unsafe` block is a build failure, and `forbid` rather than `deny` so no
  module can lift it locally. Nothing in the crate changes — there was none to
  remove. The rule is now stated per crate, and it is deliberately not the same
  one `draco-io` follows: the algorithms here run on bitstream-controlled
  indices throughout, which is the wrong place to be unable to reason about a
  crash.
- **Breaking.** `DracoError` is an opaque struct one pointer wide, in the shape
  of `std::io::Error`, and the variants are now the `#[non_exhaustive]`
  `ErrorKind` enum reached through `kind()`. Construct with
  `DracoError::general(msg)`, `DracoError::buffer(msg)` and their siblings, or
  `DracoError::new(kind, msg)`; read the text back with `message()`, or `Display`
  for the kind and the message together. `DracoError::DracoError` disappears in
  the process — the variant mirrored upstream's `Status::DRACO_ERROR`, which
  upstream qualifies as `Status::`, so the stutter was an artifact of this port
  naming the enum `DracoError`; it is `ErrorKind::General`, and the `Display`
  text has always read "General error".

  The shape is not a style preference. Almost every decode and encode function
  in this crate returns `Status`, so the size of the failure case is paid by the
  success case at every call site: with the message stored inline,
  `Result<(), DracoError>` was 32 bytes and needed dropping, so those functions
  returned through a hidden out-pointer and every `?` expanded to `String` drop
  glue. Boxed, `Ok(())` is a null pointer in a register and the drop glue is one
  shared function.
- The `thiserror` dependency is gone; `Display` and `Error` for `DracoError` are
  written out, and it was the only thing the crate used the derive for.
- **Breaking.** The fallible methods of `AttributeTransform`, of the
  `PredictionScheme` family, and of the sequential attribute coders return
  `Status` instead of `bool`, and their failures propagate to `MeshDecoder`,
  `PointCloudDecoder`, `MeshEncoder` and `PointCloudEncoder` rather than being
  replaced by a fixed sentence one hop below the API. A truncated stream now
  reports which structure ran out, three layers up. `is_initialized`,
  `is_valid_quantization_bits` and `are_corrections_positive` stay `bool` — they
  are predicates, not outcomes.
- **Breaking.** `DirectBitDecoder::decode_next_bit` returns `Option<bool>`: its
  bits are a `Vec<u32>` filled by `start_decoding`, so a read past the end is
  exactly knowable and no longer indistinguishable from an encoded zero. Its rANS
  siblings keep `bool` and document why they cannot answer the same question —
  the state legitimately falls below `l_base` on 26% of reads in streams this
  crate encodes and decodes back byte-exactly, so there is no local signal, and
  the guard belongs at the call sites where the read count is bounded
  structurally.
- **Breaking.** The bitstream versions an encode accepts are an enumeration per
  geometry/coder combination — `version::EncodeTarget::claimed_versions` — each
  entry carrying an encode/decode round-trip test, rather than the interval
  from 1.0 to the newest: 259 values for a mesh, including minors that never
  existed, most of which produced a stream this crate's own decoder rejects.
  `EncoderOptions::set_version` still takes `(major, minor)` and still accepts
  any pair; what changed is where an unsupported one is refused. The check is
  `version::validate_encodable_version`, run by `MeshEncoder::encode` and
  `PointCloudEncoder::encode` once the target is known — it cannot run in the
  setter, which does not yet know whether it is configuring a mesh or a point
  cloud, EdgeBreaker or sequential. So a version that used to encode into an
  unreadable stream now fails at `encode()`, with an `UnsupportedVersion` error
  naming what the target does claim, and `(0, 0)` still means "use the
  default". The list is shorter than what the setter accepted and longer than
  what it wrote correctly: the pre-2.2 layout fixes below are what a legacy
  version can now carry, and an EdgeBreaker mesh is claimed at 2.2, 2.1, 2.0,
  1.2 and 1.1. Narrowing cannot diverge from upstream: C++ Draco has no version
  setter at all.
- **Breaking.** `DracoError::is_count_exceeds_bitstream` and the message-prefix
  constructor behind it are gone with the guard they described.
- **Breaking.** `version::OLDEST_ENCODABLE_VERSION` is gone. It named 1.0 as the
  floor of an interval that no longer exists, nothing had read it since
  `claimed_versions` replaced that interval, and no released Draco ever wrote
  bitstream 1.0 — the oldest that turns up is 1.1, from Draco 0.9.1.
- The decoder bounds allocations two ways, and which one applies depends on
  what is being counted.

  Buffers sized from a declared count of *values* are bounded by a ratio
  against the input — 2^20 bytes per input byte — rather than by refusing a
  count above the remaining bitstream in bits. That premise was false: geometry
  whose values are all equal entropy-codes to a size independent of the count,
  so this crate wrote 100,000 points into 171 bytes and then refused to read the
  file back.

  A declared count of *symbols* is bounded at one bit each instead, because a
  ratio is the wrong instrument for a count. A ratio scales with the input, and
  the input is what an attacker supplies: a 26 KB stream naming 1,095,910,464
  faces reserves 13 GB of indices and clears the ratio with room to spare, since
  13 GB over 2^20 is 12 KB. Lowering the constant would not change that shape —
  whatever it is, twice the input buys twice the allocation. This was found by
  the `decode_drc` fuzz target and is pinned by
  `a_face_count_beyond_one_bit_each_is_refused_before_allocation`.

  Upstream's own `faces > remaining/3` is still not adopted: at three bytes per
  face it is 24 times stricter than one bit per symbol, so a file this crate
  accepts and upstream refuses still decodes here, which is the direction the
  difference is meant to run.
- A tex-coord prediction scheme forced onto an attribute that is not a texture
  coordinate is refused. Both tex-coord predictors work on two components, and a
  normal presents two once the octahedron transform has folded it from three, so
  the scheme was accepted for normals and wrote values the normal decoder cannot
  read back. Three-component attributes were already refused, which is why only
  normals slipped through.
- The EdgeBreaker traversal reports `DracoError` rather than `String`, removing
  the last error type in the crate that was not `DracoError`. Behaviour-neutral.
- The texture-coordinate orientation count is bounded by the entry count rather
  than by the remaining stream in bits. `compute_original_values` pops exactly one
  orientation per predicted entry, so a count above that can never be consumed
  whatever the stream holds; the previous bound assumed one rANS bit per
  orientation, which a run of identical ones beats.
- A prediction scheme is no longer named in a bitstream that predates it. The
  encoder now carries the version each scheme was introduced at and substitutes
  the one its era used: `MeshPredictionConstrainedMultiParallelogram` arrived in
  Draco 0.10.0, which writes bitstream 1.2, and
  `MeshPredictionTexCoordsPortable` and `MeshPredictionGeometricNormal` in Draco
  1.0.0, which writes 2.0 — neither of the latter two identifiers occurs
  anywhere in the 0.9.1 or 0.10.0 sources. Naming one anyway is not a graceful
  degradation: Draco 0.9.1 hands any id it does not recognise to
  `PredictionSchemeDifference`, so the correction values are read as plain
  deltas and the file opens, reports the right vertex count and reconstructs
  different geometry with no error anywhere. The two schemes that predict from
  position are worse still, because a modern decoder accepts them and answers
  wrongly: below 2.0 upstream's `InitPredictionScheme` gives the predictor the
  final position attribute rather than the portable one, and at that version the
  final attribute already holds dequantized floats, which
  `GetPositionForDataId` then truncates into `VectorD<int64_t, 3>` — on a
  unit-scale mesh every coordinate collapses to 0 or ±1 and every cross product
  is degenerate, whatever the encoder predicted against. That branch is not an
  oversight; it exists for `MESH_PREDICTION_TEX_COORDS_DEPRECATED`, the one
  pre-2.0 scheme with a parent attribute, whose predictor really is
  float-domain. So there is no encoder-side domain that makes a pre-2.0
  geometric normal readable — the combination is outside the format, and the
  encoder declines to write it rather than trying to agree with it. Before the
  rule existed, roughly 40% of a 289-point mesh's normals landed on the wrong
  point at 1.1 and 1.2, with the point and face counts still right; it was found
  only by checking a real decoder, since this crate's own encoder and decoder
  read the same wrong location and agreed with each other. The decoder still
  reads a genuine old file that used either scheme below 2.0, which needed the
  mirror of upstream's two branches: below 2.0 the parent is the attribute
  itself, dequantized in place as soon as its own turn ends, and from 2.0 it is
  the portable copy.

### Fixed

- An encoding speed outside the documented 0..=10 produced different bytes from
  C++ Draco. The entropy coder's compression level is `10 - speed`, and upstream
  sets it through `SetSymbolEncodingCompressionLevel`, which refuses anything
  outside 0..=10 and leaves the option unset, so `EncodeSymbols` falls back to
  its own default of 7; this crate passed the raw value through, so a speed of
  11 became level -1 and shortened the symbol bit length by 2 where upstream
  adjusted nothing. Byte parity now holds across -20..=20, pinned by
  `parity_encode_bytes_speeds_outside_the_documented_range` — which also gave
  the neighbouring `parity_encode_bytes_all_speeds` the assertions it had been
  missing, having only ever printed its verdict.
- Quantizing an attribute with parameters computed for a narrower one panicked
  with "the len is 2 but the index is 2", reachable through the public
  `AttributeTransform::transform_attribute`. The inverse direction had checked
  that length since it was written.
- `AttributeQuantizationTransform::encode_parameters` wrote `-1` truncated to
  `0xFF` as the quantization-bits byte and reported success, emitting a value no
  decoder accepts. The sibling octahedron transform had always gated the same
  method.
- The normal encoder accepted a quantization bit count of 1 or above 30 — the
  octahedron transform carries 2..=30 — leaving the transform uninitialised while
  reporting success; the encode failed later, at the folding step, naming nothing.
- Two `unwrap()` calls on the portable texture-coordinate encode path panicked on
  a scheme that had mesh data but no corner map, after a check covering only the
  first.
- `CornerTable::opposite` and `vertex` return the invalid sentinel for a corner
  past the table instead of indexing out of bounds. Roughly twenty-five index
  expressions in that file funnel through the two.
- A KD-tree stream claiming more splits than its half bits cover read zeros past
  the end and kept building; it is refused.
- Encoding below 2.2 wrote 2.2 layouts inside older streams in five places, all
  one root cause: a sub-buffer built with `EncoderBuffer::new()` reports version
  0, which every version branch reads as "newest". The EdgeBreaker traversal
  block, the deprecated tex-coords, geometric-normal and portable tex-coords
  schemes, and the pre-2.0 quantization parameters are written in the layout the
  target version specifies.
- A stream written at compression level 0 by Draco 0.9.1 or 0.10.0 decodes to
  its geometry rather than to the origin. Those streams carry no prediction
  scheme, and upstream's sentinel for that is `PREDICTION_NONE` = -2 = `0xFE`;
  the pre-2.0 shims that walk the prediction header to reach the quantization
  parameters behind it tested `0xFF` alone, stepped one byte in, and read the
  parameters shifted. The range came out zero, so every position dequantized to
  the origin — with the point and face counts right and the decode reporting
  success, which is why the fixture suite never saw it.
- **Breaking, on the bitstream.** A point cloud below 2.0 carries its
  quantization parameters inline, before the integer values, as a mesh already
  did. Upstream decides the placement on the version alone —
  `DecodeQuantizedDataInfo` is called from `DecodeIntegerValues` below 2.0 and
  from `DecodeDataNeededByPortableTransform` above it, in an attribute decoder
  shared by both geometry types — and this crate split the rule by geometry
  type on both sides at once, so its own round trip agreed with itself while no
  C++ decoder could read a 1.3 point cloud. A 1.3 point cloud written by 1.x is
  not readable by 2.0 and the reverse, which is the point: only one of them was
  ever Draco.
- A pre-1.2 stream no longer carries the rANS zero-run token. Token 3 in a
  probability byte means "a run of zero-probability symbols follows" only from
  Draco 0.10.0, whose bitstream is 1.2; before that it meant "three extra
  probability bytes". An old decoder reads the run byte as a length prefix and
  loses the rest of the table. The asymmetry is why this survived: an old stream
  never contains token 3, so every later decoder reads one, and only writing the
  old version breaks.
- The prediction scheme is chosen from what the target bitstream can name.
  Constrained multi-parallelogram, portable tex-coords and geometric normal all
  postdate 1.1, and speed 0 selects the first of them, so a 1.1 stream written at
  speed 0 was readable by no released decoder — Draco 0.9.1's factory returns
  null for an unknown scheme and drops the prediction silently, and modern Draco
  refuses the stream outright.
- The position values are ordered by prediction degree only when the target can
  say so. That order is declared by an attribute traversal byte that arrived in
  1.2; below it a decoder assumes depth first, so a speed-0 encode wrote values
  in an order nothing reconstructs and produced a mesh with every vertex in the
  wrong place.
- Topology split events below 1.2 are written in that version's layout — two
  absolute `u32` ids and a byte for the edge, with no bit-coded section after
  them — rather than the delta/varint pair and packed edge bits that arrived in
  1.2. The decoder had always read both. This is why 1.1 is back on the claimed
  list: it worked for a mesh with no split events, which is every grid, and
  silently produced wrong faces for anything that splits.
- Sequential meshes switch counts *and* face indices to varints at 2.2, not at
  the major. The face-index branch had no version gate at all, so a 1.3 mesh with
  65,536 or more points decoded without error, with the right face count, and
  with different faces.
- The four decode-path allocations sized from `num_points * num_components` are
  fallible, and the product is computed with `checked_mul` — `usize` is 32 bits
  on the wasm32 target this ships to, where a large point count times 255
  components wraps rather than saturating.
- `DecoderBuffer::decode_least_significant_bits32` rejected `nbits > 32` but
  not `nbits == 32` itself, and computed `1u32 << nbits` to build the mask —
  the exact width the tagged symbol scheme uses for a prediction residual
  whose top bit is set, which its own decode loop already treated as valid
  (`len == 0 || len > 32` rejects only the two neighbors of 32). A debug build
  panics with "attempt to shift left with overflow"; a release build does not
  panic and instead returns 0 for the read while reporting success, so the
  decoded attribute is silently wrong rather than absent. `DirectBitDecoder`,
  `RAnsBitDecoder` and `FoldedBit32Decoder` already handled width 32 correctly
  in the same crate; only this one bit reader had not. Found while checking
  `quantization_bits` at its upper bound — reachable with no quantization
  involved at all, from `symbol_encoding::encode_symbols`/`decode_symbols`
  directly on a single `u32::MAX` value.
- `AttributeQuantizationTransform` accepted `quantization_bits` up to 31.
  Upstream's own `IsQuantizationValid` caps it at 30, same ceiling as the
  octahedron transform's `2..=30`; a prior comment on the Rust constant read
  the two ranges as deliberately different sizes, which is not why they
  differ — 30 is the shared ceiling, and 31 quantized and encoded without
  complaint into a stream that a real mesh's prediction residuals could drive
  straight into the bit-reader bug above, on decode by this crate or any
  other. This narrows the range to match upstream rather than relying on the
  bit-reader fix alone to make 31 safe.
- `decoder` without `point_cloud_decode` failed to compile. The pre-1.2 shim
  `carries_transform_byte` sat behind `point_cloud_decode` while the mesh
  decoder's own legacy path calls it too; it is now gated on both of its
  callers — `point_cloud_decode` and `legacy_bitstream_decode` — which also
  keeps it from reading as dead code in a `decoder` build that compiles
  neither. Every combination of the crate's seven encode/decode features — 46
  distinct sets once the ones an implied feature makes identical are folded
  together — builds with no warnings, as do `debug_logs` and
  `force_sequential_seeds` on top of the defaults. CI checks the powerset now
  rather than a picked list, because a picked list is what let this through.

## [1.2.0](https://github.com/Filyus/draco-rust/compare/draco-core-v1.1.0...draco-core-v1.2.0) - 2026-08-01

A hardening release for the encoding side. Geometry handed to an encoder
normally came out of a file some other library parsed, so its point count, index
buffer, attribute mappings and quantization settings are as untrusted as the
file was, and nothing between the two re-checked them. Found by a new
encoder-side libFuzzer target and by an audit of that target's own first fixes;
every case below is pinned as a regression test.

### Added

- `DracoError::is_count_exceeds_bitstream` tells apart the decoder's
  count-versus-size header refusal. It is the one decode refusal that can be a
  false positive: the bound assumes at least one bit per point or face, which
  highly repetitive geometry beats.
- `PointAttribute::is_mapping_identity` reports whether point ids are used
  directly as attribute value ids, so a caller validating a mapping answers in
  one comparison instead of a `mapped_index` call per point.
- `version::validate_encodable_version` and `version::OLDEST_ENCODABLE_VERSION`
  state which bitstream versions this crate writes: 1.0 up to the newest for the
  geometry type.

### Changed

- The encoders validate the geometry they are given before encoding it, and
  refuse what they cannot encode instead of panicking. An attribute must have
  components, a valid data type, a point map that lands inside its own value
  array, a stride at least as wide as its element, and a buffer long enough for
  the values it reports; a mesh face must reference points the mesh has; the
  target bitstream version must be one this crate writes; and
  `force_predictive_traversal` requires a target below 2.0, which its own
  comment already said and nothing checked. Callers passing geometry assembled
  from a file now get a `DracoError` where the encoder previously indexed off
  those numbers.
- A mesh needing more attribute groups than the bitstream's one-byte count field
  holds is refused rather than truncated. At 256 groups the count wrapped to
  zero and the decoder read the following bytes as attribute data; 255 groups
  still round-trip.
- `KeyframeAnimation::add_keyframes` returns `-1` instead of writing past the
  attribute buffer when the component count does not fit the `u8` that stores
  it, or when the declared scalar type is not the element type of the slice.

### Fixed

- Encoding a mesh at a high quantization setting no longer allocates a frequency
  table the size of the largest prediction residual. A 100-point mesh at
  `-qp 30 -cl 10`, both legitimate settings, asked for roughly 17 GB and took 13
  seconds; symbols above 2^18 now live in a map, so the same frequencies - and
  therefore byte-identical output - cost the number of distinct symbols instead
  of their magnitude.
- Reusing a `MeshEncoder` for a second mesh no longer inherits the first one's
  connectivity. Each encode caches a corner table and its maps for the attribute
  stage and the sequential path does not rebuild all of it, so encoding an
  attributed mesh with EdgeBreaker and then a plain mesh sequentially produced a
  stream this crate's own decoder rejects.
- A point cloud encoded at a bitstream version below 1.3 omitted the header
  flags field, leaving the stream two bytes short of what its own decoder reads,
  so an explicit `set_version(1, 0)` produced a `.drc` nothing could decode.
  Upstream writes that field for every version, and the mesh encoder already
  did.
- The quantization transform and its parameter scan report a failure instead of
  indexing when a source offset falls outside the attribute buffer, matching the
  octahedron transform's counterpart.
- Three arithmetic operations this port performed in a signed type where
  upstream uses an unsigned one no longer panic in a debug build: the portable
  tex-coord predictor's squared-norm product, the rANS table-size estimate, and
  the constrained multi-parallelogram scorer's zig-zag of a full-range residual.
  Release output is unchanged; the random-mesh parity sweep against the
  reference encoder now passes on all 388 meshes.

## [1.1.0](https://github.com/Filyus/draco-rust/compare/draco-core-v1.0.5...draco-core-v1.1.0) - 2026-07-31

### Changed

- A point cloud encoded without an explicit `encoding_method` now uses the
  KD-tree encoder, as Draco does, instead of the sequential one. Upstream picks
  KD-tree for any cloud whose attributes it can handle at any speed below 10,
  and the default speed is 5, so this was the common case: the same input
  produced a different method byte and an entirely different payload from the
  reference encoder. Callers that want the previous output should ask for it
  with `set_encoding_method(0)`; those that already did are unaffected. The
  KD-tree encoder reorders points, so anything comparing decoded points by index
  needs to compare them as a set instead.
- Normals and texture coordinates are predicted the way Draco predicts them at
  encoder speeds 0 to 3: geometric-normal prediction for normals, and portable
  tex-coord prediction reading quantized positions. The encoder now agrees with
  C++ Draco byte for byte on every attributed mesh in the parity suite, at every
  speed. On meshes whose normals are already smooth, those schemes can cost more
  than a plain delta -- that is upstream's tradeoff, and the output is now
  identical to what C++ Draco produces for the same input.

### Fixed

- Encoding a mesh with vertices no face references no longer panics. Point
  deduplication rewrote each attribute's buffer to hold only the surviving
  points but left the attribute reporting its old `size()`, so anything walking
  it by that count read past the buffer's end -- which the quantization
  transform does while computing min/max. Clean meshes hid this, because there
  the surviving count equals the original; raw scanned geometry does not. The
  Stanford bunny in `testdata` carries 35,947 vertices of which 1,113 are in no
  triangle, and loading and encoding it failed outright.
- The mesh encoder no longer panics on a mesh whose faces are degenerate. Three
  distinct faults, all reachable through `encode()` on geometry a caller can
  legitimately hand it: the tex-coord predictor indexed a corner table entry
  that a zero-area face leaves unset; a mesh whose faces are *all* degenerate
  reached code assuming at least one encoded point, where C++ Draco rejects the
  input outright and now so does this; and a vertex reachable only through a
  degenerate face was written an attribute value the connectivity header's
  vertex count never accounted for, which corrupted every byte after it while
  `encode()` still returned `Ok`. The last is the serious one -- it produced a
  silently wrong stream rather than an error.
- The constrained multi-parallelogram predictor picks the same configuration
  C++ Draco picks when two configurations cost the same. Its per-vertex search
  enumerated candidates in bitmask order; Draco enumerates them by increasing
  number of parallelograms used and, within each count, in `std::next_permutation`
  order. Both searches cover the same set and both find a genuinely optimal
  configuration, so no decoded value was ever wrong -- but on a tie the winner
  is whichever was visited first, and which configuration won is itself written
  to the stream as crease flags. This was the dominant source of byte
  differences on meshes dense enough to offer a vertex several parallelograms.
- Decoding a malformed mesh no longer allocates for geometry the stream only
  claims to contain. The edgebreaker decoder sized the mesh, the corner table
  and the per-vertex hole table from the face and vertex counts in the header,
  before the checks that would reject the stream had run; a 374-byte fuzz input
  claiming 724 million faces cost 8.7 GB and a second, and was then rejected on
  a payload size that could never have fit the buffer. The corner table now
  grows a face at a time as faces are decoded, and the mesh is sized from it,
  so memory follows the geometry actually present. No accepted stream changes,
  and the counts still bound the traversal. Notably without a
  faces-per-byte cap: edgebreaker connectivity is rANS-coded and can go below a
  bit per face, so any such limit would be a guess about compression that a
  valid stream could fall foul of.
- Encoding a mesh at high quantization no longer allocates gigabytes for
  predictions it discards. The entropy tracker's frequency table is indexed by
  symbol value, so it costs memory proportional to the largest symbol rather
  than to the number of distinct ones, and `peek` -- which scores a candidate
  the predictor may well reject -- grew it just as `push` does and never gave
  the growth back. One rejected candidate whose averaged prediction overflowed
  held a couple of billion entries for the rest of the encode. Measured across
  400 randomly generated meshes at 1 to 30 quantization bits, peak memory fell
  from 21.9 GB to 7.0 GB and wall clock from 65 s to 41 s, with byte-identical
  output.
- The portable tex-coord predictor applies Draco's three overflow checks when
  encoding, not only when decoding. Upstream shares one predictor between its
  encoder and decoder so both sides check; the two halves here are separate and
  the encoding one did not, so it wrapped silently and produced a stream for
  non-manifold meshes C++ Draco declines to encode at all.
- Point clouds now encode byte for byte as C++ Draco does, on both the
  sequential and the KD-tree path, at every speed. Four faults stood between:
  an integer attribute with quantization requested for it was announced as
  quantized and then written raw, desyncing every attribute after it in the
  stream; a normal attribute nobody asked to quantize selected the octahedral
  encoder, which cannot encode one, and the encode failed; the KD-tree method on
  a cloud with no attributes indexed attribute zero and panicked; and the
  KD-tree traversal left points in a different order than upstream's within each
  split, which the bitstream records wherever a node holds one or two points.
  Which encoder writes an attribute is now decided once, from the data type
  first, and the identifier byte in the stream is that same decision rather than
  a second guess at it.
- The point-cloud bitstream version is 2.3 for both methods, as upstream writes
  it. The sequential path claimed 1.3, which also wrote the attribute count as a
  fixed `u32` where every reader expects a varint.
- An attribute with interior seams -- a vertex carrying more than one texture
  coordinate -- was encoded with its values in the wrong order, so a decoder
  returned them attached to the wrong points. The attribute's own corner table
  is now walked depth first, seeded by the edgebreaker corner order, as Draco
  does; it was previously enumerated by vertex index, which is the identity
  permutation rather than an encoding order. Affected encoder speeds 0 to 5,
  where each attribute keeps its own connectivity; from speed 6 up the mesh is
  split on seams into a single connectivity and was already correct. Reachable
  from this crate's API, not from the glTF, OBJ or FBX readers, which hand the
  encoder one attribute value per point.
- Texture coordinates were predicted from the original float positions while the
  decoder predicts from the quantized ones, so the two disagreed and the decoded
  UVs were wrong at speeds 0 to 3. The portable position attribute now also
  carries the point map its predictors index it by, without which the lookup
  returned whichever vertex happened to sit at that entry.
- Corrections from a mesh prediction scheme are no longer zig-zagged when the
  transform already makes them positive. The decoder decides this from the
  transform written to the stream, and the encoder now decides it the same way
  instead of consulting a scheme object that mesh predictors never populate.
- A normal attribute with integral values no longer fails the encode outright.
  It selects geometric-normal prediction, which has no octahedral quantization
  to work from, and now falls back to a delta as Draco's own scheme factory does
  when a mesh scheme cannot be built.
- At speed 0 the position attribute alone is walked in max-prediction-degree
  order; every other attribute is walked depth first. The encoder declared and
  used the position's order for all of them, producing streams that named an
  order they were not written in.
- Octahedral normal quantization is computed in `f64`, as upstream does, rather
  than in `f32`. The rounding rule was already the same; only the width of the
  intermediate differed, and that decides the result whenever the value being
  floored lands exactly on `.5`. Ordinary normals do that regularly -- for
  `(0, 0.7071, 0.7071)` the octahedral coordinate is exactly `511.5` at 10 bits
  -- so the encoder picked the neighbouring coordinate and wrote a normal
  roughly one quantization step away from the one C++ Draco writes for the same
  input. Byte parity with the C++ encoder on attributed meshes went from 24 of
  55 cases to 38, with every remaining difference confined to speeds 0 to 3.

## [1.0.5](https://github.com/Filyus/draco-rust/compare/draco-core-v1.0.4...draco-core-v1.0.5) - 2026-07-30

### Changed

- `SUPPORT_MATRIX.md` records what happens to a glTF extension carried across a
  Draco transform: extensions with no binary references are declared, the two
  that own references remap them, and an unregistered one refuses rather than
  being guessed at. `EXT_structural_metadata` / `EXT_mesh_features` says what is
  actually preserved — property-table buffer views kept alive across compaction,
  feature IDs riding on attributes the encoder returns unchanged — instead of
  "known slots participate in safe remapping".
- STL is named where the crate boundary is described: the module docs, the
  workspace-crate list, and the matrix row for the formats `draco-io` owns. It
  has been read and written there since `draco-io` 0.3.1.
- The C++ reference is described as a local checkout located through the
  environment, rather than by one machine's path.

### Fixed

- Feature-disabled builds no longer compile unused codec internals or encoder
  helpers, so supported minimal feature combinations pass with `-D warnings`.

## [1.0.3](https://github.com/Filyus/draco-rust/compare/draco-core-v1.0.2...draco-core-v1.0.3) - 2026-06-27

### Performance

- Reduced allocation and setup overhead in selected mesh codec hot paths.

## [1.0.2] - 2026-06-24

### Fixed

- Malformed legacy EdgeBreaker streams now validate symbol-count invariants
  before allocating mesh face storage, avoiding a fuzz-discovered OOM path.
- `PointCloudDecoder` now rejects mesh bitstream headers directly.

## [1.0.1] - 2026-06-24

### Fixed

- EdgeBreaker mesh encoding now reports a clear error when a pre-2.2 bitstream
  or forced predictive traversal is requested without `legacy_bitstream_encode`,
  instead of producing an invalid legacy-shaped stream.
- Legacy prediction-scheme encode requests now report a clear error when
  `legacy_bitstream_encode` is disabled.

## [1.0.0] - 2026-06-24

First stable release. The public API is now covered by SemVer (breaking changes
mean a new major version). The `.drc` bitstream it reads and writes is Google
Draco's stable format.

### Added

- Pure-Rust encoder and decoder for Draco `.drc` meshes and point clouds.
- Sequential, KD-tree, and EdgeBreaker connectivity paths; attribute
  quantization, prediction schemes, and normal octahedron transforms.
- Metadata and legacy-bitstream compatibility paths for older Draco streams.
- Feature flags to build decoder-only, encoder-only, or point-cloud-only
  configurations for smaller binaries.
