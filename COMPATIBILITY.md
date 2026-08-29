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

The first three run on ordinary CI. The fourth needs a C++ Draco build
(`DRACO_CPP_SOURCE_DIR`, `DRACO_CPP_BUILD_DIR`) and **skips itself without one**,
so a green CI run says nothing about it. It was last run by hand against C++
Draco 1.5.7 on 2026-08-30 and passed. If you touch anything on this page, run it
again and update that date — a skipped test reports success.

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
