# Changelog — draco-core

Notable changes to the `draco-core` crate. This crate is versioned and released
independently; its release tags are `draco-core-vX.Y.Z`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Changed

- A tex-coord prediction scheme forced onto an attribute that is not a texture
  coordinate is refused. Both tex-coord predictors work on two components, and a
  normal presents two once the octahedron transform has folded it from three, so
  the scheme was accepted for normals and wrote values the normal decoder cannot
  read back. Three-component attributes were already refused, which is why only
  normals slipped through.

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
