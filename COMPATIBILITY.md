# Compatibility with upstream Draco

This port targets byte-exact parity with C++ Draco 1.5.7, and mostly reaches it.
The same mesh and options give the same bytes, and each implementation reads
what the other writes.

The deliberate differences fall into two layers, kept apart below:

1. **[The codec, `draco-core`](#the-codec-draco-core)** — what a given `Mesh` or
   `PointCloud` encodes to, and what a stream decodes to. This is what parity
   means, and it comes first.
2. **[File readers, `draco-io` and `draco-gltf`](#file-readers-draco-io-and-draco-gltf)**
   — how a file becomes a `Mesh`. A difference here changes the mesh that
   reaches the encoder, not the codec.

For which algorithms exist at all, see
[`crates/draco-core/SUPPORT_MATRIX.md`](crates/draco-core/SUPPORT_MATRIX.md).

## The codec, `draco-core`

| difference | same bytes as C++? | C++ decodes ours? | we decode C++'s? |
|---|---|---|---|
| [`uint32` > `i32::MAX`][uint32] | C++ refuses | yes | yes |
| [Overflowing corrections][wrap] | yes | no, nor its own | yes |
| [Integer details][minor] | yes | yes | old `uint32` parents differ |
| [Point-cloud options][options] | only when off | yes | yes |
| [`Mesh::finalize`][finalize] | yes; the mesh differs | yes | yes |
| [64-bit dedup][dedup] | n/a | yes | yes |

[uint32]: #uint32-attribute-values-above-i32max
[wrap]: #prediction-corrections-that-overflow-int32
[minor]: #minor-differences-in-integer-attribute-handling
[options]: #point-cloud-encoder-options-not-in-upstream
[finalize]: #meshfinalize-drops-unused-points-and-values
[dedup]: #deduplication-of-64-bit-attribute-values

"C++ refuses": C++ Draco will not encode that input. "n/a": nothing upstream
builds reaches that code.

### `uint32` attribute values above `i32::MAX`

#### Effect on streams

This encoder accepts `uint32` attribute values above `i32::MAX`. C++ Draco
refuses the same mesh with `Failed to encode point attributes.` and writes
nothing.

Both decoders read the resulting `.drc` correctly, so no interoperability is
lost. Only this encoder can write such a file, though, so a pipeline that
relies on it cannot switch to the C++ encoder.

#### How each implementation handles it

Upstream copies every value into a portable `int32` attribute with
`ConvertValue<int32_t>`. For a `uint32` source, `ConvertComponentValue` rejects
anything above `INT32_MAX` (`attributes/geometry_attribute.h`), and
`PrepareValues` turns that into a failed encode.

This port keeps the bits instead. `read_value_as_i32`
([`sequential_integer_attribute_encoder.rs`](crates/draco-core/src/sequential_integer_attribute_encoder.rs))
reinterprets the `uint32` as an `i32`, so the portable attribute holds a negative
number. On decode, `write_value_from_i32`
([`sequential_integer_attribute_decoder.rs`](crates/draco-core/src/sequential_integer_attribute_decoder.rs))
writes it back under the declared type, bits unchanged.

Upstream's decoder gets the same value, because its `StoreTypedValues`
(`compression/attributes/sequential_integer_attribute_decoder.cc`) is a plain
`static_cast<AttributeTypeT>` of the portable `int32`, with no range check.

`Int64`, `Uint64` and `Float64` never reach this path: `select_sequential_encoder`
([`sequential_attribute_encoder.rs`](crates/draco-core/src/sequential_attribute_encoder.rs))
sends them to the generic encoder, which stores raw bytes.

#### Predicting from a `uint32` attribute

A prediction scheme reads its parent attribute as a number, and the encoder and
decoder must read it the same way. Twice they did not:

- The encoder read the portable `int32` and got `-256`; the decoder read the
  same bytes as the declared `uint32` and got `4294967040`. The two predictions,
  `2^32` apart, overflowed the texture-coordinate scheme's guard, and the decoder
  refused a stream this encoder had just written. The `encode_drc` fuzz oracle
  found it.
- A `Uint64` position was read in its declared type. It never reaches the
  encoder path above, but it does reach a predictor, where values at the ends of
  the `i64` range overflowed the arithmetic that follows.

Both are fixed in one place. A prediction scheme now holds a `PredictionParent`
([`portable_attribute.rs`](crates/draco-core/src/portable_attribute.rs)) instead
of a `PointAttribute`. It exposes only the point-to-entry lookup and a single
read: that read widens a `Uint32` as the portable `int32`, and building the
parent refuses a float or 64-bit attribute, where upstream's decoder refuses it
too.

#### Size cost

Measured with
[`examples/wide_uint32_ratio.rs`](crates/draco-core/examples/wide_uint32_ratio.rs)
on a 16×16 grid, against a control with the same geometry and values below the
boundary. Sizes are deterministic, so one run per case is enough.

| prediction scheme | control (low values) | all values wide | values straddling the boundary |
| --- | --- | --- | --- |
| portable texture coordinates (5) | 1259 B | 1260 B (+0.1%) | 1652 B (+31.2%) |
| default | 424 B | 425 B (+0.2%) | 792 B (+86.8%) |

Values that all sit above `i32::MAX` cost nothing extra: prediction codes
differences, and a uniform offset does not change them. The cost comes from data
that crosses the boundary, where neighbouring values are `2^32` apart as
integers. That is a property of the data, but a producer packing unrelated
ranges into one `uint32` attribute pays for it.

#### Tests

| test | where | what it would catch |
| --- | --- | --- |
| `a_uint32_attribute_keeps_values_above_i32_max_through_a_round_trip` | [`draco-core/tests/attribute_integration_test.rs`](crates/draco-core/tests/attribute_integration_test.rs) | The encode starting to refuse these values, and the two halves disagreeing on a parent that straddles the boundary. |
| `a_texcoord_predicts_from_a_uint32_position_as_the_encoder_read_it` | [`draco-core/tests/encoder_hardening_test.rs`](crates/draco-core/tests/encoder_hardening_test.rs) | The original fuzz reproducer, replayed from `fuzz/seeds/encode_drc/texcoord_predicts_from_a_uint32_position.bin`. |
| `test_read_component_as_i64_reads_a_uint32_position_as_the_portable_int32` | [`draco-core/src/portable_attribute.rs`](crates/draco-core/src/portable_attribute.rs) | The parent reader's `Uint32` arm alone, without going through a full encode. |
| `cpp_decodes_a_uint32_attribute_above_i32_max_the_same_way` | [`draco-cpp-test-bridge/tests/parity_wide_uint32_attributes.rs`](crates/draco-cpp-test-bridge/tests/parity_wide_uint32_attributes.rs) | Upstream C++ Draco disagreeing with this decoder on the decoded bytes. |

All four run on CI. The first three need nothing outside this repository. The
fourth links the C++ library through `draco-cpp-test-bridge`, so it runs in the
`draco C++ parity` job, which builds Draco at the 1.5.7 tag and compares against
it both by spawning its command-line tools and by linking its library.

That job sets `DRACO_REQUIRE_CPP_TOOLS` and `DRACO_REQUIRE_CPP_BRIDGE`. Keep this
in mind before trusting any parity result: every comparison stands down
silently when it cannot find a C++ Draco. The tool-spawning tests return early,
and the bridge's build script compiles its tests out, and both report success.
The two variables turn that into a failure, so the job cannot pass without
having compared.

**The randomized sweep runs on CI only over the region it asserts.** The sweep
in `draco-cpp-test-bridge/tests/parity_random_meshes.rs` also explores 30-bit
quantization, where prediction residuals leave `int32` and both encoders
overflow; there it measures rather than asserts. In that region upstream built
with gcc asks for memory without bound on one case, reaching 11.5 GB before
`std::bad_alloc` ends the process. An MSVC build of the same sources survives
it, so the compiler decides this, not the version: released 1.5.7 and upstream
`main` have identical entropy sources and both do it.

`SWEEP_ASSERTED_ONLY` limits the sweep to the asserted region, and the parity
job sets it. Nothing asserted is lost: 349 comparisons across grid, soup and
degenerate meshes, in 37 seconds. Exploring the 30-bit region stays a local run.

#### Alternative: refuse these values, as upstream does

This has been built and measured once. The steps and the cost:

1. In
   [`sequential_integer_attribute_encoder.rs`](crates/draco-core/src/sequential_integer_attribute_encoder.rs),
   give `read_value_as_i32` a fallible sibling returning `None` for a `Uint32`
   above `i32::MAX`. Every other arm widens totally and cannot start failing.
2. Route both call sites through it and turn `None` into an encode error:
   `encode_values` in that file, and `integral_portable_attribute` in
   [`mesh_encoder.rs`](crates/draco-core/src/mesh_encoder.rs). Both already
   return a `Result`.
3. Change the `Uint32` arm of `read_component_as_i64` in
   [`portable_attribute.rs`](crates/draco-core/src/portable_attribute.rs) to read
   unsigned. It is one arm in one file.
4. Delete the four tests above, or rewrite the first two to assert the refusal.

What it costs:

- `a_color_attribute_without_a_position_attribute_round_trips_at_speed_zero`
  ([`encoder_hardening_test.rs`](crates/draco-core/tests/encoder_hardening_test.rs))
  **fails**. Its fixture pins a per-attribute connectivity bug and happens to
  carry a `uint32` value above `i32::MAX`, so the encode it tests would stop
  happening. It needs a replacement fixture with the same connectivity and
  in-range values first.
- Fuzz coverage narrows. `encode_drc`'s `build_attribute` fills attribute bytes
  straight from the fuzz payload, so about half of all `Uint32` entries carry a
  component at or above `2^31`. Those inputs would stop at the encode call
  instead of exercising the integer encode path.
- Files this port has already written can no longer be re-encoded by it. They
  still decode.

The refusal buys no interoperability, since C++ Draco already reads what this
encoder writes. It only narrows the accepted input to upstream's.

### Prediction corrections that overflow `int32`

#### Effect on streams

Two behaviours around prediction corrections differ from upstream. In both,
this port reconstructs exactly what the encoder coded:

- When a prediction plus its correction overflows `int32`, this decoder
  recovers the coded value. C++ Draco does that addition in `uint32` to avoid
  signed overflow, and where the `uint32` sum wraps, its single wrap step cannot
  undo it: the value lands a whole span away, and every later prediction reads
  the wrong number. Upstream then either decodes different geometry or, when the
  drifted values trip its own overflow guards, refuses a stream it wrote itself.
- The portable tex-coord predictor wraps wherever upstream wraps and refuses
  only where upstream refuses (the three guards in its `ComputePredictedValue`).
  An earlier version of this port checked every step on decode, and refused
  streams whose scaled arithmetic left `i64` — streams C++ Draco decodes —
  while its encoder wrapped, so the two halves of this codec could not read each
  other's output. They now share one predictor with upstream's rules.

Only decoding differs, and upstream's behaviour here is measured. Built through
C++ Draco 1.5.7's `ExpertEncoder` with the same six points and forty-five
faces, the same `uint16` position and `int32` tex coords, and EdgeBreaker at
speeds 0/4 with the same two prediction schemes, the reproducer mesh encodes to
911 bytes, **byte-identical to what this encoder writes**. C++ Draco then fails
to decode its own stream with "Failed to decode point attributes", because its
overflow guards fire on the wrong reconstruction. So no encoder choice helps:
upstream cannot round-trip this mesh, and only this decoder reads the stream
both encoders produce.

Files that decode identically in both implementations still do. Every
correction that stays inside the wrap range reconstructs exactly as before; the
results differ only where C++'s own arithmetic wraps.

#### How each implementation handles it

The encoder wraps each correction into `min_correction..=max_correction`, half
the value span, exactly as upstream does. When the `int32` sum of prediction
and correction would overflow, the decoder adds them in `i64` instead
(`checked_add` with an `i64` fallback in `compute_original_value`,
[`prediction_scheme_wrap.rs`](crates/draco-core/src/prediction_scheme_wrap.rs)).
The exact sum is at most half a span outside `[min, max]`, so the single wrap
step provably lands on the original value for every correction the encoder can
produce. Upstream's `ComputeOriginalValue`
(`prediction_scheme_wrap_decoding_transform.h`) does the addition in `uint32`,
where the wrap loses information that the `i64` sum keeps.

Both tex-coord schemes run one predictor
(`MeshPredictionSchemeTexCoordsPortablePredictor`,
[`prediction_scheme_tex_coords_portable.rs`](crates/draco-core/src/prediction_scheme_tex_coords_portable.rs)),
with an `is_encoder` flag that selects only the orientation handling, as
upstream's shared predictor header does. Past the three guards its arithmetic
wraps in two's complement, which gives the same bits as upstream's mix of
signed and unsigned 64-bit C++ math.

#### Tests

| test | where | what it would catch |
| --- | --- | --- |
| `a_texcoord_prediction_that_wraps_round_trips_through_the_shared_predictor` | [`draco-core/tests/encoder_hardening_test.rs`](crates/draco-core/tests/encoder_hardening_test.rs) | The round trip of `fuzz/seeds/encode_drc/texcoord_portable_encoder_wraps_where_decoder_refuses.bin`, the input that needs both fixes: a prediction past the guards would refuse at encode, and a wrong reconstruction would refuse at decode. |
| `encoder_and_decoder_produce_the_same_prediction_when_the_arithmetic_wraps` | [`draco-core/src/prediction_scheme_tex_coords_portable.rs`](crates/draco-core/src/prediction_scheme_tex_coords_portable.rs) | The two sides disagreeing about a prediction whose intermediate arithmetic wraps. |

### Minor differences in integer attribute handling

- **A pre-2.0 `uint32` parent is read signed.** Below bitstream 2.0 upstream
  binds the attribute itself and reads it in its declared type, so it reads a
  `DT_UINT32` parent unsigned. This port reads it as the portable `int32` at
  every version, because one read serves both bindings.
- **The decode-side portable attribute is declared `Uint32`.** Upstream declares
  `DT_INT32` for quantized and normal attributes
  ([`sequential_quantization_attribute_decoder.rs`](crates/draco-core/src/sequential_quantization_attribute_decoder.rs)).
  Harmless today: quantization is capped at 30 bits, so the signed and unsigned
  readings coincide.
- **Narrower integer positions are narrowed again on the way out.** `Uint8`,
  `Int16` and the rest pass through `write_value_from_i32` into the destination
  attribute. Streams this encoder writes cannot hit it, since their portable
  values came from that same attribute and therefore fit; a hand-built or
  corrupted stream can.

All three come from the same place. Upstream gives every integer-decoded
attribute an `int32` portable copy and lets predictors read nothing else, failing
outright when the copy is missing; this port grew the portable copy along the
dequantization path only. `PredictionParent` now enforces that rule here too,
which is why this list is three items and not a work plan.

### Point-cloud encoder options not in upstream

`EncoderOptions::set_prediction_search` and `set_spatial_point_order` exist only
in this port. Both are off by default, and with them off a point cloud encodes
to the same bytes as in C++ Draco.

When turned on, each option makes files smaller than upstream's for the same
input. The bytes differ, but every decoder, C++ Draco's included, reads the
file:

| option | what the encoder does differently | why upstream's decoder reads it |
| --- | --- | --- |
| `set_prediction_search` | May code an attribute with no prediction (`PREDICTION_NONE`) where upstream always uses `Difference`. | Upstream's decoder has read `PREDICTION_NONE` since bitstream 1.1; its encoder just never picks it for a point cloud. |
| `set_spatial_point_order` | Writes the points in a spatial order over their positions instead of input order; a Morton curve today, open to change. | The format is unchanged; only the point order differs. |

The spatial order also changes the order of the decoded points. That matters
only to data outside the file that refers to points by index.

#### Tests

`parity_point_cloud_options.rs` in `draco-cpp-test-bridge` decodes streams
written with each option using upstream's decoder and compares every value of
every attribute. Each case first checks that the option changed the bytes, so
an option that silently did nothing would fail rather than pass.

### `Mesh::finalize` drops unused points and values

`Mesh::finalize` does what upstream's `TriangleSoupMeshBuilder::Finalize`
does — merges bit-identical attribute values, then the points those values made
identical — and one step more: it drops points no face names and values no
point names. Upstream has that last step as `PointAttribute::RemoveUnusedValues`,
but compiles it only into its transcoder.

The encoder is unaffected: the same `Mesh` encodes to the same bytes. What
differs is the mesh `finalize` hands it, and that matters for precision, because
the quantization range covers every value an attribute holds, not only those the
faces reach. One stray value far from the mesh therefore widens the range. On
a unit triangle with a fourth vertex at `1000, 1000, 1000`, quantized to 16
bits, the encoded size does not change, and a coordinate that should decode as
`1.0` decodes as `1.007095`.

Dropping the value loses nothing: upstream's encoder writes only the geometry
the faces reach, so it never arrives at any decoder either way.

A *duplicated* vertex, as opposed to an unused one, encodes to the same bytes on
both sides. That rule is ported unchanged, and the merge is what lets
triangles that share a position share an edge.

### Deduplication of 64-bit attribute values

`Mesh::finalize` merges bit-identical values within each attribute, as
upstream's `TriangleSoupMeshBuilder::Finalize` does. Upstream's `DeduplicateValues` handles types up to 32
bits and fails on `DT_FLOAT64`, `DT_INT64` and `DT_UINT64`; this port merges
those as well.

No stream changes: nothing upstream builds hands its `Finalize` a 64-bit
attribute, and for every type it handles the result is the same. The
difference lets a caller finalize a mesh carrying a 64-bit attribute, which
used to fail here as it does upstream.

## File readers, `draco-io` and `draco-gltf`

Where these readers and upstream's turn the same file into different meshes:

| difference | readers here | upstream's readers | effect on the stream |
|---|---|---|---|
| [Unused vertices][readers-unused] | dropped | kept | same geometry, more precision here |
| PLY texture coordinates | read | not read | carries UVs upstream's lacks |
| Other PLY vertex properties | carried on request | dropped | only with `with_generic_attributes` |

[readers-unused]: #unused-vertices

Upstream's PLY reader takes positions, normals and colours and nothing else. A
`u`/`v` or `s`/`t` pair is read here as texture coordinates only when both
halves are `float`; anything else is an ordinary property.

### Unused vertices

Every reader here that builds a mesh from scratch ends with
[`Mesh::finalize`](#meshfinalize-drops-unused-points-and-values), so a file
whose vertex list has entries no face uses encodes to different bytes than
through upstream's readers, which keep them. Both decode the same triangles and
points; this side keeps more precision. The step behaves the same in the OBJ,
PLY and glTF readers, which before it disagreed with each other as well as with
upstream.

How often this matters depends on where the file came from. Eight delivered
glTF assets — 1,046 primitives and 8,753,680 vertices — had no unused vertex and
no duplicate. That describes those eight, not glTF: the Khronos `Fox` sample in
this repository ships one vertex per corner, 1,294 of its 1,728 duplicating
another exactly, and merging them leaves 434 points. The eight shared an
exporter that welds vertices; a hand-authored file need not.

Raw geometry is the opposite. The Stanford Bunny PLY in this repository has
35,947 vertices, 1,113 of which no face uses, and until they were dropped they
set its quantization range. Scans commonly carry such vertices, so in the
formats where this applies it is not a corner case.
