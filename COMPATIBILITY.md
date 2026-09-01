# Compatibility with upstream Draco

This port targets byte-exact parity with C++ Draco 1.5.7, and mostly reaches it.
The same mesh and options give the same bytes, and each implementation reads
what the other writes.

A few differences are deliberate. This file lists them: what differs, what it
means for your files, and what it would take to remove.

For which algorithms exist at all, see
[`crates/draco-core/SUPPORT_MATRIX.md`](crates/draco-core/SUPPORT_MATRIX.md).

## `uint32` attribute values above `i32::MAX`

### What this means for a file

This encoder accepts integer attribute values that C++ Draco refuses:

- A mesh with a `uint32` attribute holding values above `i32::MAX` encodes here.
  C++ Draco refuses the same mesh with `Failed to encode point attributes.` and
  writes no bytes.
- The `.drc` this encoder writes for it decodes correctly in C++ Draco.

Only this encoder writes such files; both decoders read them. So you lose no
interoperability. What you gain is a capability the reference implementation
does not have — which is worth knowing if you come to depend on it.

### What each side does

Upstream converts every value into its portable `int32` attribute with
`ConvertValue<int32_t>`. `ConvertComponentValue` range-checks first — for a
`uint32` source it computes `kOutMin = 0` and rejects anything above
`INT32_MAX` (`attributes/geometry_attribute.h`). `PrepareValues` turns that
`false` into a failed encode.

This port carries the bits through instead. `read_value_as_i32`
([`sequential_integer_attribute_encoder.rs`](crates/draco-core/src/sequential_integer_attribute_encoder.rs))
reinterprets the `uint32` as `i32`, the portable attribute holds it as a
negative number, and on the way out `write_value_from_i32`
([`sequential_integer_attribute_decoder.rs`](crates/draco-core/src/sequential_integer_attribute_decoder.rs))
writes it back under the attribute's declared type. The bits survive unchanged.

Upstream's decoder reads such a stream correctly because its `StoreTypedValues`
(`compression/attributes/sequential_integer_attribute_decoder.cc`) is a plain
`static_cast<AttributeTypeT>` of the portable `int32` with no range check.

Wider scalar types never reach this path: `select_sequential_encoder`
([`sequential_attribute_encoder.rs`](crates/draco-core/src/sequential_attribute_encoder.rs))
routes `Int64`, `Uint64` and `Float64` to the generic encoder, which stores raw
bytes losslessly.

### The prediction half

A prediction scheme reads its parent position as a number, so both halves of the
codec have to read it the same way. Twice they did not.

The encoder read the portable `int32` and got `-256`; the decoder read the same
bytes as the `uint32` the attribute declares and got `4294967040`. Predictions
`2^32` apart overflowed the texture-coordinate scheme's guard, and the decoder
refused a stream this encoder had just written. The `encode_drc` fuzz oracle
found it.

The second case came through the same door. The parent was read in whatever type
the attribute declared, so a `Uint64` position — which never reaches the
*encoder* path above, but does reach a *predictor* — arrived at the ends of the
`i64` range and overflowed the arithmetic after it.

Both are closed at the source rather than case by case. A prediction scheme no
longer holds a `PointAttribute`. It holds a `PredictionParent`
([`portable_attribute.rs`](crates/draco-core/src/portable_attribute.rs)), which
offers no buffer, no byte stride and no data type — only the point-to-entry
lookup and one read. That read widens a `Uint32` as the portable `int32`, and
building the parent refuses a float or 64-bit attribute, exactly where upstream's
decoder refuses it. There is nowhere else this decision is made.

### What it costs in compressed size

Measured with
[`examples/wide_uint32_ratio.rs`](crates/draco-core/examples/wide_uint32_ratio.rs)
on a 16×16 grid, against a control with the same geometry below the boundary.
Sizes are deterministic, so one run per case is the measurement.

| prediction scheme | control (low values) | all values wide | values straddling the boundary |
| --- | --- | --- | --- |
| portable texture coordinates (5) | 1259 B | 1260 B (+0.1%) | 1652 B (+31.2%) |
| default | 424 B | 425 B (+0.2%) | 792 B (+86.8%) |

An attribute whose values sit *entirely* above `i32::MAX` costs nothing:
prediction works on differences, and a uniform offset leaves them unchanged. The
penalty belongs to data that *crosses* the boundary, where neighbouring values
are `2^32` apart as integers. That is a property of the data rather than a tax on
the widening — but a producer packing unrelated ranges into one `uint32`
attribute should know the compressor will pay for it.

### What pins it

| test | where | what it would catch |
| --- | --- | --- |
| `a_uint32_attribute_keeps_values_above_i32_max_through_a_round_trip` | [`draco-core/tests/attribute_integration_test.rs`](crates/draco-core/tests/attribute_integration_test.rs) | The encode starting to refuse these values, and the two halves disagreeing on a parent that straddles the boundary. |
| `a_texcoord_predicts_from_a_uint32_position_as_the_encoder_read_it` | [`draco-core/tests/encoder_hardening_test.rs`](crates/draco-core/tests/encoder_hardening_test.rs) | The original fuzz reproducer, replayed from `fuzz/seeds/encode_drc/texcoord_predicts_from_a_uint32_position.bin`. |
| `test_read_component_as_i64_reads_a_uint32_position_as_the_portable_int32` | [`draco-core/src/portable_attribute.rs`](crates/draco-core/src/portable_attribute.rs) | The parent reader's `Uint32` arm alone, without going through a full encode. |
| `cpp_decodes_a_uint32_attribute_above_i32_max_the_same_way` | [`draco-cpp-test-bridge/tests/parity_wide_uint32_attributes.rs`](crates/draco-cpp-test-bridge/tests/parity_wide_uint32_attributes.rs) | Upstream C++ Draco disagreeing with this decoder on the decoded bytes. |

The first three run on ordinary CI. The fourth links the C++ library through
`draco-cpp-test-bridge`, which needs a build of it (`DRACO_CPP_SOURCE_DIR`,
`DRACO_CPP_BUILD_DIR`) and **compiles itself out without one**, so a green CI run
says nothing about it. It was last run by hand against C++ Draco 1.5.7 on
2026-08-30 and passed. If you touch anything on this page, run it again and
update that date — a test that was never built reports success.

The comparisons that spawn the upstream *command line* tools rather than linking
its library are no longer in that position: the `draco C++ parity` job builds
Draco at the 1.5.7 tag and runs them with `DRACO_REQUIRE_CPP_TOOLS` set, which
makes a missing tool fail the run instead of skipping it. Only the bridge above
still depends on someone remembering.

### If this should become a refusal instead

The alternative is to refuse the value at encode time, as upstream does. That was
built and measured once already. Here is what it takes, so nobody has to work it
out twice.

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
   unsigned. One arm in one file — before the parent reader was unified this step
   meant editing two copies that had drifted apart.
4. Delete the four tests above, or rewrite the first two to assert the refusal.

The cost, measured rather than assumed:

- `a_color_attribute_without_a_position_attribute_round_trips_at_speed_zero`
  ([`encoder_hardening_test.rs`](crates/draco-core/tests/encoder_hardening_test.rs))
  **fails**. Its fixture pins a per-attribute connectivity bug, and it happens to
  carry a `uint32` value above `i32::MAX`. The encode it tests would stop
  happening, so the test would stop testing the bug. You need a replacement
  fixture with the same connectivity and in-range values first.
- Corpus coverage narrows. `encode_drc`'s `build_attribute` fills attribute bytes
  straight from the fuzz payload, so roughly half of all `Uint32` entries carry a
  component at or above `2^31`. Those inputs would bail at the encode call
  instead of exercising the integer encode path.
- Files this port has already written stop round-tripping through it. The
  decoder is unaffected, so they stay readable, but they cannot be re-encoded.

The refusal buys no interoperability: C++ Draco already reads what this encoder
writes, which is what the bridge test shows. It buys a narrower accepted domain,
identical to upstream's, at the price of the capability.

## The wrap transform's reconstruction, and the tex-coord predictor's overflow discipline

### What this means for a file

Two related behaviours around prediction corrections differ from upstream, both
in the direction of reconstructing what the encoder coded:

- Where a prediction and its correction overflow `int32` when added back
  together, this decoder reconstructs the value the encoder coded. C++ Draco
  performs that addition in `uint32` (its own guard against signed overflow),
  and where the `uint32` sum wraps, its single wrap step cannot reach: the
  reconstruction lands a whole value span away, and every later prediction
  reads the aliased number. Upstream then either decodes different geometry or
  -- for inputs whose drifted data trips its own overflow guards -- refuses a
  stream it wrote itself.
- The portable tex-coord predictor wraps wherever upstream wraps and refuses
  only where upstream refuses (the three guards in its `ComputePredictedValue`).
  An earlier state of this port checked every step on the decode side and
  refused streams whose scaled arithmetic left `i64` -- streams C++ Draco
  decodes -- while the encode side wrapped; the two halves of this codec could
  not read each other's work. They are one predictor now, and the discipline is
  upstream's.

The divergence is on the decode side alone, and upstream's own behaviour on
these meshes is measured, not inferred. Built through C++ Draco 1.5.7's
`ExpertEncoder` -- the same six points and forty-five faces, the same `uint16`
position and `int32` tex coords, EdgeBreaker at speeds 0/4 with the same two
prediction schemes -- the reproducer mesh encodes to 911 bytes **byte-identical
to what this encoder writes**, and C++ Draco then fails to decode its own
stream: "Failed to decode point attributes", its overflow guards firing on the
aliased reconstruction. So no encoder can help: upstream cannot round-trip this
mesh through itself, and the one stream both encoders agree on is a stream only
this decoder reads.

Files that decode identically in both implementations still do: every
correction that stayed inside the wrap range reconstructs exactly as before.
The differences appear only where C++'s own arithmetic aliases.

### What each side does

The encoder wraps each correction into `min_correction..=max_correction` -- half
the value span -- exactly as upstream does. The decoder adds prediction and
correction in `i64` when the `int32` sum would overflow
(`checked_add` with an `i64` fallback in `compute_original_value`,
[`prediction_scheme_wrap.rs`](crates/draco-core/src/prediction_scheme_wrap.rs)):
the exact sum sits at most half a span outside `[min, max]`, so the transform's
single wrap step lands on the original value, provably, for every correction the
encoder can produce. Upstream's `ComputeOriginalValue`
(`prediction_scheme_wrap_decoding_transform.h`) does the same addition in
`uint32`, where the wrap loses the information that a `i64` add keeps.

The tex-coord predictor is one body of code
(`MeshPredictionSchemeTexCoordsPortablePredictor`,
[`prediction_scheme_tex_coords_portable.rs`](crates/draco-core/src/prediction_scheme_tex_coords_portable.rs))
run by both schemes with an `is_encoder` flag selecting only the orientation
handling, mirroring upstream's shared predictor header. Past the three guards
its arithmetic wraps in two's complement, which is bit-identical to upstream's
mix of C++ signed and unsigned 64-bit math.

### What pins it

| test | where | what it would catch |
| --- | --- | --- |
| `a_texcoord_prediction_that_wraps_round_trips_through_the_shared_predictor` | [`draco-core/tests/encoder_hardening_test.rs`](crates/draco-core/tests/encoder_hardening_test.rs) | The round trip of `fuzz/seeds/encode_drc/texcoord_portable_encoder_wraps_where_decoder_refuses.bin` -- the input that needs both halves of this page: a prediction past the guards would refuse at encode, and an aliased reconstruction would refuse at decode. |
| `encoder_and_decoder_produce_the_same_prediction_when_the_arithmetic_wraps` | [`draco-core/src/prediction_scheme_tex_coords_portable.rs`](crates/draco-core/src/prediction_scheme_tex_coords_portable.rs) | The two sides disagreeing about a prediction whose intermediate arithmetic wraps. |

## A vertex no face uses is dropped, not carried

### What this means for a file

A mesh file whose vertex list includes an entry no face refers to encodes to
different bytes here than through C++ Draco. Both sides decode the same
triangles and the same points -- neither writes the unused vertex into the
stream -- so what differs is precision, and this side has more of it.

The vertex still reaches the encoder upstream, because the quantization range
is computed over the values an attribute holds rather than over the ones the
connectivity reaches. One stray entry far from the mesh therefore spends bits
on empty space. Measured on a unit triangle carrying a fourth vertex at
`1000, 1000, 1000`, quantized to 16 bits: the encoded size does not move at all,
and the coordinate that should return as `1.0` returns as `1.007095`.

Nothing is lost by dropping it. Upstream's own encoder writes only the geometry
the connectivity reaches, so the unused vertex never arrives at any decoder
either way -- keeping it buys no data and no bytes, only the wider range.

How often it costs anything depends entirely on where the file came from.
Across eight delivered glTF assets -- 1,046 primitives and 8,753,680 vertices
-- not one vertex was unreferenced and not one duplicated another. That is a
statement about those eight, not about glTF: the Khronos `Fox` sample this
repository carries ships one vertex per corner, 1,294 of its 1,728 duplicating
another exactly, and merging them takes its encoded point count to 434. What
those eight had in common was an exporter that welds; a sample authored by hand
need not.

Raw geometry is the opposite. The Stanford Bunny this repository carries as a
fixture reaches it immediately: 35,947 vertices in the PLY, 1,113 of which no
face names. A scan is exactly the shape that carries them, which is why this
divergence is not a corner case in the formats where it applies -- and why
those 1,113 vertices, all of them dead, were setting the quantization range
until this dropped them.

### What each side does

Upstream's readers size an attribute from the vertex list before they know which
entries the faces use, and no later step revisits the question: its
`PointAttribute::RemoveUnusedValues` exists, but is compiled into the transcoder
alone and no reader calls it.

Every reader here that builds a mesh from scratch ends through
[`mesh_finalize`](crates/draco-io/src/mesh_finalize.rs), which does what
upstream's `TriangleSoupMeshBuilder::Finalize` does -- merge bit-identical
attribute values, then merge the points those values made identical -- and then
one step further: drop the points no face names, and the values no point names.
That last step is this divergence, and it is the same in OBJ, PLY and glTF,
where before it the readers disagreed with each other as well as with upstream.

A file with a *duplicated* vertex, rather than an unused one, encodes byte for
byte the same on both sides. That case is upstream's rule faithfully ported, and
the merge is what makes triangles that share a position share an edge.

## Smaller divergences in the same family

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

All of these come from the same place. Upstream gives every integer-decoded
attribute an `int32` portable copy and lets predictors read nothing else, failing
outright when the copy is missing; this port grew the portable copy along the
dequantization path only. `PredictionParent` now enforces that rule here too,
which is why this list is three items and not a work plan.
