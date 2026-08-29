# Compatibility with upstream Draco

This port aims at byte-exact parity with C++ Draco 1.5.7, and mostly reaches it:
the same mesh and options produce the same bytes, and either implementation
reads what the other writes. This file records the places where that is
deliberately not true — what the difference is, what it means for the files you
end up with, and what removing it would take.

Coverage per algorithm is a different question, answered by
[`crates/draco-core/SUPPORT_MATRIX.md`](crates/draco-core/SUPPORT_MATRIX.md).

## `uint32` attribute values above `i32::MAX`

### What this means for a file

This encoder accepts integer attribute values that C++ Draco refuses, and both
decoders read the result back byte for byte. Concretely:

- A mesh with a `uint32` attribute holding values above `i32::MAX` encodes here.
  C++ Draco fails the same mesh with `Failed to encode point attributes.` and
  writes no bytes.
- A `.drc` this encoder produced from such a mesh decodes correctly in C++
  Draco. The asymmetry is one-sided: only this encoder produces those files, but
  either decoder consumes them.

So the widening costs you no interoperability. What it costs is that a producer
relying on it is relying on something the reference implementation will not do.

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

A value is only carried faithfully if both halves of the codec agree on what it
*is*. A prediction scheme reads its parent position as a number, and the two
halves once disagreed: the encoder read the portable `int32` (`-256`), the
decoder read the mesh attribute labelled `uint32` (`4294967040`). At `2^32`
apart the scaled products left the range the portable texture-coordinate
scheme's overflow guard allows, and the decode refused a stream this encoder had
just written. The `encode_drc` round-trip fuzz oracle found it.

There turned out to be a second door into the same room. The parent was read at
the attribute's own declared type, so a `Uint64` position — one the paragraph
above says never reaches the *encoder* path — reached a *predictor* at both ends
of the `i64` range and overflowed the arithmetic downstream of it.

Both are closed structurally rather than case by case. A prediction scheme no
longer holds a `PointAttribute`: it holds a `PredictionParent`
([`portable_attribute.rs`](crates/draco-core/src/portable_attribute.rs)), which
exposes no buffer, no byte stride and no data type — only the point-to-entry
lookup and one canonical widening read, whose `Uint32` arm reads the portable
`int32`. Constructing one validates the attribute against the types the portable
pass writes, so a float or 64-bit parent is refused where upstream's decoder
refuses it. That is the single place this behaviour now lives.

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

The first three run on ordinary CI. The fourth needs a C++ Draco build
(`DRACO_CPP_SOURCE_DIR`, `DRACO_CPP_BUILD_DIR`) and **skips itself without one**,
so a green CI run says nothing about it. It was last run by hand against C++
Draco 1.5.7 on 2026-08-30 and passed. If you touch anything on this page, run it
again and update that date — a skipped test reports success.

### If this should become a refusal instead

The upstream-faithful alternative is to refuse the value at encode time. It was
implemented and measured once already; this is what it takes, so the decision can
be made on facts rather than re-derived.

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
  **fails**. Its fixture is a fuzz artifact that pins a per-attribute
  connectivity bug, and it carries a `uint32` value above `i32::MAX`; the test
  would stop covering what it was written for, because the encode it exercises
  would no longer happen. A replacement fixture with the same connectivity shape
  and in-range values has to be produced first.
- Corpus coverage narrows. `encode_drc`'s `build_attribute` fills attribute bytes
  straight from the fuzz payload, so roughly half of all `Uint32` entries carry a
  component at or above `2^31`. Those inputs would bail at the encode call
  instead of exercising the integer encode path.
- Files this port has already written stop round-tripping through it. They stay
  readable — the decoder is unaffected — but they could not be re-encoded.

What the refusal would *not* buy is interoperability: files produced today are
already read correctly by C++ Draco, as the bridge test above demonstrates. The
gain is a narrower, upstream-identical accepted domain; the loss is the
capability itself.

## Smaller divergences in the same family

- **A pre-2.0 `uint32` parent is read signed.** Below bitstream 2.0 upstream
  binds the mesh attribute itself as a prediction parent and reads it in its
  declared type, so a `DT_UINT32` parent is read unsigned there. This port reads
  it as the portable `int32` at every version, because one canonical widening
  serves both bindings.
- **The decode-side portable attribute is declared `Uint32`.** Upstream declares
  `DT_INT32` for quantized and normal attributes
  ([`sequential_quantization_attribute_decoder.rs`](crates/draco-core/src/sequential_quantization_attribute_decoder.rs)).
  Harmless today: quantization is capped at 30 bits, so the signed and unsigned
  readings coincide.
- **Narrower integer positions are narrowed again on the way out.** `Uint8`,
  `Int16` and friends pass through `write_value_from_i32` into the destination
  attribute. Unreachable through this encoder, whose portable values come from
  that same attribute and therefore fit; reachable only with a hand-built or
  corrupted stream.

These share a root with the divergence above: upstream gives every
integer-decoded attribute an `int32` portable copy and lets predictors read
nothing else, while this port grew the portable concept along the dequantization
path only. That invariant is now ported — the `PredictionParent` type above is
what enforces it — which is why the list is this short.
