# Tricks

Dated, code-level optimization techniques found in this repo, both the ones
landed as discrete commits and the ones baked into the port from the first
commit. Every entry shows real before/after code trimmed from the actual
diff (or, where the comparison is against upstream, the real C++ next to the
real Rust) -- prose alone hides which line actually did the work.

This is a technique reference, not a changelog: [`PERFORMANCE.md`](PERFORMANCE.md)
has the current speed snapshot, [`CHANGELOG.md`](CHANGELOG.md) has release
notes, and the crate `CHANGELOG.md`s have per-release detail. A trick here may
span several commits (the KD-tree section is one five-commit sequence) or be
a single line that has held since 2026-04-26's `feat: add initial draco rust
implementation` with no later commit to point to -- those are dated
`2026-04-26` and labelled "day one" rather than credited to a hash that just
happens to touch the file.

Numbers are quoted from the commit or profiling doc that produced them, never
invented. Where a technique was measured and did **not** pay off, it is kept
in [Measured and rejected](#measured-and-rejected-what-looked-like-a-trick-and-wasnt)
rather than silently dropped -- a documented dead end saves the next person
from re-deriving it.

**Entries marked "re-measured 2026-08-14"** were checked against the tree as
it stands, because their original number was either missing (a day-one design
nobody had ever benchmarked) or taken in the old 8s x 6-10 run regime that
cannot resolve below ~2%. The method is the one
[`dev/profiling/README.md`](dev/profiling/README.md) prescribes: revert the
trick in place, confirm the revert still decodes bit-identically, and run both
binaries interleaved at `SECONDS_PER_RUN=2` over 18-24 rounds, with a second
build per condition wherever the gap is small enough for code layout to
explain it. One of those re-measurements overturned its own entry -- see
[the rANS remainder](#take-the-rans-quotient-once-derive-the-remainder-by-subtraction).

## Contents

- [Dispatch: no vtable where the borrow checker allows it](#dispatch-no-vtable-where-the-borrow-checker-allows-it)
- [Straight into the buffer: no scratch copy](#straight-into-the-buffer-no-scratch-copy)
- [Error layout: pay for failure only on the failure path](#error-layout-pay-for-failure-only-on-the-failure-path)
- [Corner table and mesh connectivity](#corner-table-and-mesh-connectivity)
- [Branchless: when it wins, and when it loses](#branchless-when-it-wins-and-when-it-loses)
- [Allocate from decoded data, not the header's claim](#allocate-from-decoded-data-not-the-headers-claim)
- [Entropy coding](#entropy-coding)
- [Attribute prediction schemes](#attribute-prediction-schemes)
- [KD-tree point-cloud decode: a five-commit sequence](#kd-tree-point-cloud-decode-a-five-commit-sequence)
- [I/O and WASM: allocation off the hot path](#io-and-wasm-allocation-off-the-hot-path)
- [Binary size: dead code elimination](#binary-size-dead-code-elimination)
- [Measured and rejected](#measured-and-rejected-what-looked-like-a-trick-and-wasnt)

---

## Dispatch: no vtable where the borrow checker allows it

C++ Draco leans on runtime polymorphism: a `unique_ptr<Interface>` field, a
`Create...` factory, virtual calls at the use site. This port replaces most of
that with compile-time dispatch -- generics, `enum` + `match`, or
`Option<ConcreteType>` -- for a design reason as much as a speed one: the
locally built predictors *borrow* maps built inside the decode call, and
`Box<dyn Trait>` defaults to `'static`, so a borrowing predictor cannot be
type-erased into one anyway. The full comparison, with more C++/Rust pairs
than fit here, lives in
[`crates/draco-core/DISPATCH.md`](crates/draco-core/DISPATCH.md).

### Attribute decoder selection: `match` on concrete types instead of a factory

**2026-06-23** (documented; the pattern is day one, 2026-04-26)

C++ builds a decoder behind a base class with a factory `switch`, then calls
it virtually:

```cpp
// sequential_attribute_decoders_controller.{h,cc}
std::vector<std::unique_ptr<SequentialAttributeDecoder>> sequential_decoders_;

sequential_decoders_[i] = CreateSequentialDecoder(decoder_type);   // factory
sequential_decoders_[i]->Init(GetDecoder(), GetAttributeId(i));
// later: sequential_decoders_[i]->DecodeValues(...);   // virtual
```

`match` on the same byte constructs the concrete type inline and calls it
directly -- no base class, no heap allocation for the decoder object, no
virtual indirection:

```rust
// point_cloud_decoder.rs / mesh_decoder.rs
match decoder_type {
    0 => {
        let mut att_decoder = SequentialGenericAttributeDecoder::new();
        att_decoder.init(self, att_id);
        att_decoder.decode_values(...)?;
    }
    1 => {
        let mut att_decoder = SequentialIntegerAttributeDecoder::new();
        att_decoder.init(self, att_id);
        att_decoder.decode_values(pc, point_ids, buffer, None, None, None, None, None, None);
    }
    // 3 => SequentialNormalAttributeDecoder, ...
}
```

Cost: the `match` is duplicated between the mesh and point-cloud callers,
since their `decode_values` argument lists differ (mesh passes a corner table
+ traversal maps, point-cloud passes `None`) -- parallel code, not the
copy-paste a shared factory would remove.

### Prediction-scheme dispatch: forced from one boxed interface to eight `Option`s

**2026-06-23**

C++ holds one polymorphic scheme and calls it virtually with no switch at the
use site:

```cpp
// sequential_integer_attribute_decoder.{h,cc}
std::unique_ptr<PredictionSchemeTypedDecoderInterface<int32_t>> prediction_scheme_;

prediction_scheme_ = CreateIntPredictionScheme(method, transform_type);   // factory
prediction_scheme_->DecodePredictionData(in_buffer);     // virtual, no switch
prediction_scheme_->ComputeOriginalValues(...);          // virtual, no switch
```

This is not reachable the same way here. The locally built predictors borrow
`vertex_to_data_map`/`data_to_corner_map` constructed inside `decode_values`,
and `Box<dyn Trait>` defaults to a `'static` object lifetime, so a borrowing
predictor cannot be erased into one. One `Option<ConcreteType>` per scheme (8
of them) stands in, plus a single `Box<dyn ... + 'static>` reserved for the
one scheme that actually is externally supplied and `'static`:

```rust
// only the externally-supplied scheme can be type-erased (it is 'static)
prediction_scheme: Option<Box<dyn PredictionSchemeDecoder<'static, i32, i32>>>,

// locally built predictors cannot be erased -> one Option per concrete type (x8)
let mut predictor_parallelogram_opt:
    Option<MeshPredictionSchemeParallelogramDecoder<i32, i32, PredictionSchemeWrapDecodingTransform<i32>>> = None;
// ... 7 more ...

match selected_method {
    PredictionSchemeMethod::MeshPredictionParallelogram => {
        if !run_decode_prediction_data(predictor_parallelogram_opt.as_mut(), in_buffer) {
            return false;
        }
    }
    // ...
}
```

Not a benchmarked win -- the `Option` fan-out avoids the heap allocation the
C++ factory pays per attribute, at the cost of eight fields instead of one.
It's the shape the borrow checker leaves once the maps are borrowed rather
than owned; owning/cloning them into each predictor would restore the C++
shape at the cost of a copy on a warm path.

### Dedup the resulting 16 near-identical match arms with generic helpers

**2026-06-23**, `5c664f0` &middot; `crates/draco-core/src/sequential_integer_attribute_decoder.rs`

The eight-way fan-out above meant `decode_values`' two apply-phases repeated
the same extract-`Option` / call-method / log-and-fail block once per scheme,
~16 near-identical copies:

```rust
PredictionSchemeMethod::MeshPredictionParallelogram => {
    let Some(predictor) = predictor_parallelogram_opt.as_mut() else {
        debug_log!("Parallelogram predictor was selected but not initialized");
        return false;
    };
    if !predictor.decode_prediction_data(in_buffer) {
        debug_log!("Failed to decode prediction data (att_id={}, ...)", att_id, ...);
        return false;
    }
}
```

Two generic helpers collapse each arm to a one-line call:

```rust
fn run_decode_prediction_data<'a, P: PredictionSchemeDecoder<'a, i32, i32> + ?Sized>(
    predictor: Option<&mut P>,
    buffer: &mut DecoderBuffer,
) -> bool {
    let Some(predictor) = predictor else {
        debug_log!("Predictor was selected but not initialized");
        return false;
    };
    if !predictor.decode_prediction_data(buffer) {
        debug_log!("Failed to decode prediction data");
        return false;
    }
    true
}
```

Because the helper is generic (not `dyn`), it monomorphizes per concrete
predictor type at each call site -- `?Sized` also lets the same helper serve
the one `dyn`-typed `self.prediction_scheme` path -- so the dedup costs
nothing at runtime. 294 lines changed, net -68; no runtime-cost change
claimed, none expected.

### EdgeBreaker traversal: one fewer interface layer, both trees inline the hot loop

**2026-06-23**

Tempting to call this a Rust win, but the honest comparison is closer: both
trees monomorphize the per-symbol decode loop. C++ makes the impl a template
over the traversal type, so `DecodeSymbol()` is not virtual -- but the impl
itself sits behind an interface, paying one virtual call *per mesh* to reach
it:

```cpp
template <class TraversalDecoderT>
class MeshEdgebreakerDecoderImpl : public MeshEdgebreakerDecoderImplInterface {
  TraversalDecoderT traversal_decoder_;            // by value -> monomorphized
  // const uint32_t symbol = traversal_decoder_.DecodeSymbol();   // not virtual
};

std::unique_ptr<MeshEdgebreakerDecoderImplInterface> impl_;
impl_ = ...(new MeshEdgebreakerDecoderImpl<MeshEdgebreakerTraversalDecoder>());
impl_->DecodeConnectivity();                        // ONE virtual call per mesh
```

Rust threads the traversal decoder as a generic parameter directly and drops
the outer interface:

```rust
pub trait EdgebreakerTraversalDecoder {
    fn decode_symbol(&mut self) -> Result<u32, String>;
}

pub fn decode_connectivity<T: EdgebreakerTraversalDecoder>(
    &mut self,
    num_symbols: i32,
    traversal_decoder: &mut T,
    remove_invalid_vertices: bool,
) -> Result<i32, String> {
    for symbol_id in 0..num_symbols {
        let symbol = traversal_decoder.decode_symbol()?;   // monomorphized, inlined
        // ...
    }
}
```

Not a hot-loop speed story -- both inline the loop identically. One fewer
indirection to *reach* it, per mesh rather than per symbol.

---

## Straight into the buffer: no scratch copy

**2026-04-26** (day one), across `crates/draco-core/src/mesh.rs`,
`sequential_generic_attribute_decoder.rs`, `sequential_attribute_encoder.rs`,
`point_cloud_decoder.rs`, `point_cloud_encoder.rs`, `decoder_buffer.rs`,
`rans_symbol_encoder.rs`, `attribute_quantization_transform.rs`

A recurring shape in upstream C++ Draco's per-point loops: heap-allocate one
scratch buffer of `entry_size` bytes, then for every point read into the
scratch and copy the scratch into its real destination -- two small
operations and (on the raw-attribute paths) one allocation, per point. None
of these loops need the scratch: the source and destination are both fully
addressable up front, so a slice straight into one or the other does the same
work with fewer copies and no per-call allocation. This shape recurs enough
across the codec that it is worth naming as one pattern rather than listing
each site as an unrelated find.

### Decode a generic attribute in one slice copy, not a per-point Decode+Write

The clearest case. Upstream's base decoder reads every point's entry into a
reused scratch buffer with one `Decode()` call and writes it out with one
`Write()` call, so *N* points pay 2*N* small buffer operations:

```cpp
bool SequentialAttributeDecoder::DecodeValues(
    const std::vector<PointIndex> &point_ids, DecoderBuffer *in_buffer) {
  const int32_t num_values = static_cast<uint32_t>(point_ids.size());
  const int entry_size = static_cast<int>(attribute_->byte_stride());
  std::unique_ptr<uint8_t[]> value_data_ptr(new uint8_t[entry_size]);
  uint8_t *const value_data = value_data_ptr.get();
  int out_byte_pos = 0;
  for (int i = 0; i < num_values; ++i) {
    if (!in_buffer->Decode(value_data, entry_size)) {
      return false;
    }
    attribute_->buffer()->Write(out_byte_pos, value_data, entry_size);
    out_byte_pos += entry_size;
  }
  return true;
}
```

The generic decoder here has no per-point loop at all: it computes the
attribute's total byte size once, takes that many bytes as a single borrowed
slice straight from the input buffer, and copies it into the (once-resized)
attribute buffer with one `copy_from_slice`:

```rust
let total_size = num_points
    .checked_mul(num_components)
    .and_then(|size| size.checked_mul(data_type_size))
    .ok_or_else(|| DracoError::general("Generic attribute size overflow".to_string()))?;
attribute.buffer_mut().try_resize(total_size)
    .map_err(|_| DracoError::general("Failed to allocate generic attribute".to_string()))?;

let bytes = buffer.decode_slice(total_size)
    .map_err(|_| DracoError::general("Failed to decode generic attribute".to_string()))?;
attribute.buffer_mut().data_mut().copy_from_slice(bytes);
```

### The same shape recurs on three more paths

**Raw point-cloud attribute decode** (`point_cloud_decoder.rs`) slices the
destination into `entry_size` chunks up front and decodes each point directly
into its final chunk:

```rust
for chunk in dst[..required_size].chunks_exact_mut(entry_size) {
    buffer.decode_bytes(chunk).map_err(|_| {
        DracoError::general("Failed to decode raw point cloud attribute values".to_string())
    })?;
}
```

**Raw point-cloud attribute encode** (`point_cloud_encoder.rs`) and
**sequential attribute encode** (`sequential_attribute_encoder.rs`) compute
each point's byte range directly in the attribute's own already-resident
buffer and hand that borrowed slice straight to `encode_data`, instead of
upstream's per-point `GetValue()`-into-scratch then `Encode()`-from-scratch:

```rust
let offset = mapped_index * entry_size;
let bytes = &buffer_data[offset..offset + entry_size];
out_buffer.encode_data(bytes);
```

None of these four were separately benchmarked -- they were the shape of the
port from the first commit, not a later rewrite -- but the pattern (avoid an
allocation and a copy per point on every attribute path in the codec) is
large enough in aggregate that it is worth reading as one decision, not four
coincidences.

### Bulk-convert raw connectivity with `chunks_exact`, not one scalar decode per component

`crates/draco-core/src/mesh.rs`, `mesh_decoder.rs`

Upstream decodes a raw (uncompressed) face's three point indices as three
separate scalar reads, each with its own bounds check, then pushes the face:

```cpp
for (uint32_t i = 0; i < num_faces; ++i) {
  Mesh::Face face;
  for (int j = 0; j < 3; ++j) {
    uint8_t val;
    if (!buffer()->Decode(&val)) {
      return false;
    }
    face[j] = val;
  }
  mesh()->AddFace(face);
}
```

The Rust path reads the whole index block as one bounds-checked slice, then
converts it into the pre-sized face vector with a single `chunks_exact` +
`zip` pass:

```rust
// mesh_decoder.rs: one bounds-checked read for the whole index block
let bytes = buffer.decode_slice(bytes_needed)?;
mesh.try_set_num_faces(num_faces)?;
mesh.set_faces_from_u8_indices(bytes);

// mesh.rs: chunks_exact + zip converts the whole block in one pass
for (face, chunk) in self.faces.iter_mut().zip(bytes.chunks_exact(3)) {
    *face = [
        PointIndex(chunk[0] as u32),
        PointIndex(chunk[1] as u32),
        PointIndex(chunk[2] as u32),
    ];
}
```

The per-scalar decode calls and per-face pushes both disappear; the same
function also has `chunks_exact(6)`/`chunks_exact(12)` siblings for the
16-bit and 32-bit index widths.

### Read bit-decode words 8 bytes at a time, not bit by bit

`crates/draco-core/src/decoder_buffer.rs`

Upstream's `BitDecoder::GetBits` calls `GetBit()` once per requested bit,
each paying its own bounds check plus byte-offset/shift arithmetic:

```cpp
inline bool GetBits(uint32_t nbits, uint32_t *x) {
  if (nbits > 32) {
    return false;
  }
  uint32_t value = 0;
  for (uint32_t bit = 0; bit < nbits; ++bit) {
    value |= GetBit() << bit;   // per-bit bounds check + byte/shift lookup
  }
  *x = value;
  return true;
}
```

`decode_least_significant_bits32_fast` loads a whole `u64` (8 bytes) in one
`copy_from_slice` whenever 8 bytes remain past the current position, then
pulls all `nbits` out of it with a single shift and mask; only the last few
bytes of the stream fall back to a per-byte loop.

**Re-measured 2026-08-14** (it had no number): replacing the body with the
per-bit loop above, output verified bit-identical, 24 interleaved rounds --

```
bunny_cpp_standard.drc   2444.6 -> 1923.8 us   -21.3%   (C++-encoded, default settings)
bunny_norm.obj decode    13797.1 -> 13905.1 us  +0.8%   (this crate at speed 1)
```

The largest single win in this document, and it only shows on one of the two
paths: a C++-default stream drives this bit reader hard (tagged symbol
lengths, edgebreaker traversal), while this crate's own speed-1 stream barely
reaches it, so there the per-bit loop is if anything marginally ahead. A
trick's size is a property of the stream, not of the function.

```rust
let raw = if remaining >= 8 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&self.data[byte_offset..byte_offset + 8]);
    u64::from_le_bytes(bytes)
} else {
    // ... per-byte fallback, only reached near the end of the stream ...
};
let mask = (1u64 << nbits) - 1;
let value = ((raw >> bit_shift) & mask) as u32;
```

### Take the rANS quotient once, derive the remainder by subtraction

`crates/draco-core/src/rans_symbol_encoder.rs`

The renormalization loop's divide/mod by `ANS_IO_BASE` (256, a compile-time
constant) folds to shift/mask in both languages -- a wash. The remaining
division is by `p`, the symbol's *runtime* probability, and upstream's
`rans_write` computes `state / p` and `state % p` as two independent
expressions:

```cpp
inline void rans_write(const struct rans_sym *const sym) {
  const uint32_t p = sym->prob;
  while (ans_.state >= l_rans_base / rans_precision * DRACO_ANS_IO_BASE * p) {
    ans_.buf[ans_.buf_offset++] = ans_.state % DRACO_ANS_IO_BASE;
    ans_.state /= DRACO_ANS_IO_BASE;
  }
  ans_.state =
      (ans_.state / p) * rans_precision + ans_.state % p + sym->cum_prob;
}
```

The Rust port takes the quotient once and derives the remainder by
subtracting `quot * p`, on the theory that this costs one hardware division
per symbol regardless of whether the compiler manages to prove the two C++
expressions share an operand:

```rust
fn rans_write(&mut self, sym: RAnsSymbol) {
    let p = sym.prob;
    let renorm_bound = (Self::L_RANS_BASE / Self::RANS_PRECISION) * crate::ans::ANS_IO_BASE * p;
    let mut state = self.ans.state;
    while state >= renorm_bound {
        self.ans.buf.push((state & 0xFF) as u8);
        state >>= 8;
    }
    // Compute quotient once; derive remainder without an extra division.
    let quot = state / p;
    let rem = state - quot * p;
    state = quot * Self::RANS_PRECISION + rem + sym.cum_prob;
    self.ans.state = state;
}
```

**Re-measured 2026-08-14 -- and this one is backwards.** Replacing
`state - quot * p` with the plain `state % p` the theory says is wasteful,
two builds per condition, 20 interleaved rounds on Bunny encode at speed 1:

```
state - quot * p   34851.0  34863.6 us      <- what the code does now
state % p          34186.3  34115.2 us      -2.1%
```

Both clusters are tight (0.04% and 0.2% apart) and disjoint, so this clears
the layout floor comfortably. LLVM lowers `/` and `%` on the same operands to
a single `div`, which already yields both results -- the hand-written
subtraction adds a multiply and a subtract that the hardware was giving away
for free. **The comment in the source is wrong and the "optimization" costs
2.1% of encode.** Left in place here rather than changed as a side effect of
documenting it; it is a one-line fix to a hot encoder path and deserves its
own commit and its own confirmation run.

### Dispatch dequantization by component count instead of checked per-component arithmetic

`crates/draco-core/src/attribute_quantization_transform.rs`

Not a C++ comparison -- an internal fast path next to the general one. The
generic dequantization loop reproves overflow safety per component: a
`checked_mul`/`checked_add` chain plus a slice bounds check for every
component of every point:

```rust
for i in 0..num_values {
    let Some(src_offset) = i.checked_mul(src_stride) else { return Err(overflow()); };
    for c in 0..num_components {
        let Some(component_offset) = c.checked_mul(4) else { return Err(overflow()); };
        let Some(src_pos) = src_offset.checked_add(component_offset) else { return Err(overflow()); };
        let Some(src_bytes) = src_data.get(src_pos..src_pos + 4) else { return Err(truncated()); };
        let q_val = i32::from_le_bytes([src_bytes[0], src_bytes[1], src_bytes[2], src_bytes[3]]);
        let val = dequantizer.dequantize_float(q_val) + self.min_values[c];
        // ... dst offset computed the same checked way, then copy_from_slice ...
    }
}
```

When the source is `Uint32` with a tight (unpadded) stride and 1-4 components
-- positions, UVs, colors, the common case -- the whole buffer length is
checked once up front, then a `match` on component count dispatches to an
unrolled loop that indexes both buffers with plain arithmetic, no per-element
`Option` chain left in the body:

```rust
if attribute.data_type() == DataType::Uint32
    && (1..=4).contains(&num_components)
    && src_stride == tight_stride && dst_stride == tight_stride
{
    if src_data.len() < required_src || dst_data.len() < required_dst {
        return Err(truncated());
    }
    match num_components {
        1 => for i in 0..num_values {
            let offset = i * tight_stride;
            let q_x = i32::from_le_bytes([src_data[offset], src_data[offset + 1],
                src_data[offset + 2], src_data[offset + 3]]);
            let x = dequantizer.dequantize_float(q_x) + self.min_values[0];
            dst_data[offset..offset + 4].copy_from_slice(&x.to_le_bytes());
        },
        // 2, 3, 4: same shape, one unrolled arm per component count
        _ => return Err(unsupported()),
    }
    return Ok(());
}
```

The generic checked path stays as the fallback for padded strides, more than
four components, or a non-`Uint32` source.

**Re-measured 2026-08-14** (it had no number): disabling the fast path so
every attribute takes the generic checked loop, output verified
bit-identical, 24 interleaved rounds --

```
bunny_cpp_standard.drc   2309.4 -> 1923.8 us   -16.7%   (C++-encoded, default settings)
bunny_norm.obj decode    14082.9 -> 13905.1 us  -1.3%   (this crate at speed 1)
```

Same lesson as the bit reader above: on a C++-default stream dequantization
is a large share of decode, while a speed-1 stream spends most of its time in
the constrained multi-parallelogram predictor instead, so the same fast path
is worth twelve times less there.

The remaining entries in this section (the four scratch-copy eliminations and
the `chunks_exact` connectivity conversion) were **not** separately
benchmarked. Reverting them means reintroducing an allocation per point on
paths that only raw/generic attributes reach, which the fixtures here barely
exercise; they are read as a consistent design decision applied across the
codec, not as individually measured wins.

---

## Error layout: pay for failure only on the failure path

**2026-08-01**, `70efae73` &middot; `crates/draco-core/src/status.rs`

Every fallible function returns `Status` (a `Result<T, DracoError>`). With the
message stored inline, `DracoError` was a 32-byte enum that needed dropping --
so the *success* case paid for it too: functions returned through a hidden
out-pointer, and every `?` expanded to `String` drop glue at the call site.

```rust
#[derive(Error, Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DracoError {
    #[error("General error: {0}")]
    General(String),
    #[error("IO error: {0}")]
    IoError(String),
    #[error("Buffer decode error: {0}")]
    BufferError(String),
    #[error("Decode would allocate {requested_bytes} bytes from a {stream_bytes} byte stream")]
    AllocationExceedsInput { requested_bytes: usize, stream_bytes: usize },
}
```

The error becomes an opaque struct boxing an `{ErrorKind, String}` pair behind
one pointer, and the box is built only on a cold, never-inlined path:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind { General, Io, Buffer, AllocationExceedsInput }

struct Inner { kind: ErrorKind, message: String }

pub struct DracoError { inner: Box<Inner> }

impl DracoError {
    /// Cold and never inlined so the allocation is emitted once rather than
    /// at each of the several hundred sites that construct an error.
    #[cold]
    #[inline(never)]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { inner: Box::new(Inner { kind, message: message.into() }) }
    }
}
```

`Ok(())` is now a null pointer in a register; drop glue collapses to one
shared function. Measured on the glTF WASM module, that's **2.6 KiB** of
gzipped code -- and the doc comment on `DracoError` is explicit that this
isn't what dominates: the message text and the `format!` that builds it are
worth 16 KiB in the same module, and no shape of the error type reaches
those. ("It is not what dominates, and this comment says so because the
first guess was wrong.")

The `#[cold] #[inline(never)]` pair on `new` is not decoration. Measured by
temporarily removing it (`dev/profiling/README.md`, round 4):

```
binary        903,168 -> 999,936 bytes   (+96 KiB, +10.7%)
decode sp1    +7.2%
encode sp5    +0.1%
```

Several hundred call sites construct an error; without the attribute, each
one inlines the allocation and the `String` machinery into a hot function.
One annotation, on a function that is already off the hot path by
construction, was worth more than every bounds-check or memoization change in
the rounds around it. The rule this codebase draws from it: `#[cold]`
anything that constructs an error or otherwise handles a failure, and leave
inlining of the hot leaves to the compiler.

---

## Corner table and mesh connectivity

### Build the corner table a face at a time, not a corner at a time

**2026-08-13**, `dfd038b` &middot; `crates/draco-core/src/corner_table.rs`

`compute_opposite_corners` asked for each corner's three vertices through
`vertex(next(c))` and `vertex(previous(c))` -- two wrap computations and three
checked lookups per corner -- when all three vertices belong to the corner's
own face and sit contiguously in the map:

```rust
for c in 0..self.num_corners() {
    let c_idx = CornerIndex(c as u32);
    let tip_v = self.vertex(c_idx);
    let source_v = self.vertex(self.next(c_idx));
    let sink_v = self.vertex(self.previous(c_idx));

    if tip_v == source_v || source_v == sink_v || sink_v == tip_v {
        continue;
    }
    // ... half-edge matching, indexed by `c` ...
}
```

Reading the triple once per face with `chunks_exact(3)` produces the same
corners in the same order, and declines to invent a face from a map whose
length isn't a multiple of three:

```rust
const NEXT_LOCAL: [usize; 3] = [1, 2, 0];
const PREV_LOCAL: [usize; 3] = [2, 0, 1];

for (face, face_base) in self.corner_to_vertex_map
    .chunks_exact(3)
    .map(<[VertexIndex; 3]>::try_from)
    .map(|face| face.expect("chunks_exact(3) yields three vertices"))
    .zip((0..).step_by(3))
{
    for local in 0..3 {
        let c = face_base + local;
        let tip_v = face[local];
        let source_v = face[NEXT_LOCAL[local]];
        let sink_v = face[PREV_LOCAL[local]];
        if tip_v == source_v || source_v == sink_v || sink_v == tip_v {
            continue;
        }
        // ... half-edge matching follows ...
    }
}
```

Table build is 8.4% of a speed-5 encode: `encode speed 5 -1.7%`, `encode speed
1 -0.7%`, `decode -0.6%` (control -- the decoder builds its table elsewhere).
Attributing this needed care: the same binaries showed speed 10 *3.6% slower*,
where an instrumented build counts **zero** calls into the changed function.
A null pad of the same instruction count reproduced 2.4% of that regression,
and a codegen diff showed why -- the edit moved shared helpers
(`core::slice::index` grew 320 bytes, `CornerTable::previous` shrank 64), so
every path's code placement shifted. The −1.7%/−0.7%/−0.6% figures are from
the paths that actually run the changed code, measured against null edits of
the same shape; see [How to tell a real win from code
placement](dev/profiling/README.md#how-to-tell-a-real-win-from-code-placement)
for the full method.

### Fold a consistency scan into one branch-free comparison

**2026-08-13**, `9f7ad8c` &middot; `crates/draco-core/src/corner_table.rs`

`is_index_consistent` walked both index maps on the way into every attribute
traversal -- 200k corners on a Bunny-sized mesh -- and every element cost two
comparisons with a short-circuiting branch: "is this the sentinel", then "is
it past the bound":

```rust
if self.corner_to_vertex_map.iter()
    .any(|&v| v != INVALID_VERTEX_INDEX && (v.0 as usize) >= num_vertices)
{
    return false;
}
if self.opposite_corners.iter()
    .any(|&o| o != INVALID_CORNER_INDEX && (o.0 as usize) >= num_corners)
{
    return false;
}
```

The sentinel is `u32::MAX`, which wraps to `0` on `+1` -- below every real
bound -- and every real value shifts up by one, so a single `>` catches both
"is the sentinel" and "is past the bound" at once. Folding with `max` instead
of short-circuiting with `any` also keeps the loop branch-free, so it
vectorises:

```rust
fn exceeds(values: impl Iterator<Item = u32>, bound: usize) -> bool {
    let bound = u32::try_from(bound).unwrap_or(u32::MAX);
    values.fold(0u32, |worst, v| worst.max(v.wrapping_add(1))) > bound
}

if exceeds(self.corner_to_vertex_map.iter().map(|v| v.0), num_vertices) {
    return false;
}
if exceeds(self.opposite_corners.iter().map(|c| c.0), num_corners) {
    return false;
}
```

Bunny decode, interleaved runs pinned to one core, two independent rounds of
10: median **-0.9%** both times, against build-to-build layout noise of
0.2-0.4%.

**Re-measured 2026-08-14**, reverting to the `.any()` form above, output
verified bit-identical, 24 interleaved rounds on a C++-encoded fixture:
`bunny_cpp_standard.drc` **1962.8 -> 1923.8 us, -2.0%** -- twice the
originally reported figure, on a fixture the original run did not use.

### Count vertices in one pass instead of resizing per element

**2026-06-27**, `6bc3c1f` &middot; `crates/draco-core/src/corner_table.rs`

Counting outgoing half-edges per vertex grew a `Vec` one element at a time,
`resize`-ing inside the loop, and read each vertex through a method call
instead of the backing array directly:

```rust
let mut num_corners_on_vertices = Vec::new();
for c in 0..self.num_corners() {
    let v1 = self.vertex(CornerIndex(c as u32));
    if v1 == INVALID_VERTEX_INDEX { continue; }
    let v1_val = v1.0 as usize;
    if v1_val >= num_corners_on_vertices.len() {
        num_corners_on_vertices.resize(v1_val + 1, 0);
    }
    num_corners_on_vertices[v1_val] += 1;
}
```

One pass over the raw map finds the vertex count, the counts vector is
allocated once, then filled by iterating the raw map directly:

```rust
let mut num_vertices_seen = 0;
for &v1 in &self.corner_to_vertex_map {
    if v1 == INVALID_VERTEX_INDEX { continue; }
    num_vertices_seen = num_vertices_seen.max(v1.0 as usize + 1);
}

let mut num_corners_on_vertices = vec![0; num_vertices_seen];
for &v1 in &self.corner_to_vertex_map {
    if v1 == INVALID_VERTEX_INDEX { continue; }
    num_corners_on_vertices[v1.0 as usize] += 1;
}
```

Not separately measured on its own; landed with a rANS output-buffer
preallocation and a per-symbol LUT-fill-by-slice change in the same commit,
"tightening the same encode/decode hot paths from the other direction."

---

## Branchless: when it wins, and when it loses

This codebase has one measured failure and one measured win for the same
rewrite, a few files apart. The difference is not the arithmetic -- it is
whether the branch predictor can learn the branch.

### The failure: the corner table's wrap

`corner_table.rs`, recorded in `9f7ad8c`. `next` wraps every third corner, so
the branch is taken in a fixed 1-in-3 pattern the predictor gets right
essentially always. Replacing it with `c + 1 - 3 * wraps` measured **+8.8%
slower** end-to-end: the branch cost nothing, and the arithmetic put a
multiply and a subtract on the dependency chain every vertex-fan swing waits
on. The source carries the result as a comment so it is not retried.

### The win: the octahedral `mod_max`

**2026-08-14** &middot; `crates/draco-core/src/normal_compression_utils.rs`

The same shape, on data instead of a pattern. `mod_max` folds a decoded
octahedral coordinate back into range with two branches:

```rust
pub fn mod_max(&self, x: i32) -> i32 {
    if x > self.center_value {
        return x - self.max_quantized_value;
    }
    if x < -self.center_value {
        return x + self.max_quantized_value;
    }
    x
}
```

The two conditions are mutually exclusive -- both would need
`center < x < -center` -- so the whole function is one expression, and `x` is
decoded data, so which way it falls is not a pattern to learn:

```rust
let over = (x > self.center_value) as i32;
let under = (x < -self.center_value) as i32;
x + (under - over) * self.max_quantized_value
```

`mod_max` was 5.5% of a C++-default mesh decode across three lines, all of it
branches. 22 interleaved rounds, decoded attributes and faces bit-identical:
**1930.0 -> 1802.3 us, -6.6%** on `bunny_cpp_standard.drc`.

The rule the pair supports: **branchless pays when the condition is data and
costs when the condition is structure.** A wrap, a stride, an every-Nth
pattern -- leave the branch. A threshold on a decoded value -- take the
arithmetic.

### Bulk-copy the values array when the layout already matches

**Re-measured 2026-08-14** &middot; `crates/draco-core/src/sequential_integer_attribute_decoder.rs`

A claim in the same audit, never benchmarked: when the attribute is
`Int32`/`Uint32` and its stride is exactly the packed row, the decoded values
already have the destination's byte layout, so the whole array is one
`copy_from_slice` instead of a per-component write with type conversion.

```rust
if (data_type == DataType::Int32 || data_type == DataType::Uint32) && byte_stride == packed_row {
    let src: &[u8] = bytemuck::cast_slice(&values[..num_values_required]);
    let dst = attr.buffer_mut().data_mut();
    let Some(dst) = dst.get_mut(..src.len()) else { return false; };
    dst.copy_from_slice(src);
    return true;
}
```

Forcing the slow per-component path instead, output bit-identical, 22
interleaved rounds: **1997.1 -> 1930.0 us, -3.4%** on
`bunny_cpp_standard.drc`. The claim is real.

---

## Allocate from decoded data, not the header's claim

A count in a `.drc` header is just an assertion the bitstream makes about
itself -- nothing checks it before the code that trusts it runs. Sizing a
buffer from that count instead of from what decoding actually produces makes
a handful of malformed bytes cost gigabytes. This is a security fix as much
as a performance one: the pathological case it removes *is* an algorithmic
cost that scales with a value the input controls, so closing it is also
closing the tail of the decode-time distribution. The Shannon
entropy-tracker fixes under [Entropy coding](#entropy-coding) are the same
pattern applied to frequency tables; these are the connectivity and symbol
paths.

### Grow the corner table and hole table from decoded faces, not a claimed header count

**2026-07-31**, `a85f87f1` &middot; `crates/draco-core/src/edgebreaker_connectivity_decoder.rs`, `mesh_edgebreaker_decoder.rs`, `corner_table.rs`

`num_faces`/`num_encoded_vertices` are read straight off the bitstream and
trusted immediately: a 374-byte fuzz input claimed 724,249,387 faces (`0x2B2B2B2B`
read out of a run of `0x2B` bytes), and every size ported from upstream
accepted that self-consistent nonsense. The corner table and the per-vertex
hole table were sized from the claim before the consistency check that would
eventually reject the stream had run:

```rust
pub fn new(num_faces: i32, max_num_vertices: i32) -> Self {
    Self {
        corner_table: CornerTable::new(num_faces as usize),
        is_vert_hole: vec![true; max_num_vertices as usize],
        // ...
    }
}
// ... inside decode_connectivity, on TOPOLOGY_C:
let vertex_x_index = self.vertex_index(vertex_x, "TOPOLOGY_C")?;
self.is_vert_hole[vertex_x_index] = false;
```

Faces are created strictly in order, and every corner belongs to a face
already built, so the corner table can grow a face at a time inside the
decode loop instead, and the hole table can grow lazily to whatever vertex is
actually touched:

```rust
pub fn new(num_faces: i32, max_num_vertices: i32) -> Self {
    Self {
        corner_table: CornerTable::new(0),
        is_vert_hole: Vec::new(),
        declared_num_faces: num_faces,
        max_num_vertices: max_num_vertices.max(0) as usize,
        // ...
    }
}

fn mark_vert_not_hole(&mut self, vertex: VertexIndex, context: &str) -> Result<(), DracoError> {
    let index = self.vertex_index(vertex, context)?;
    if index >= self.is_vert_hole.len() {
        self.is_vert_hole.resize(index + 1, true);
    }
    self.is_vert_hole[index] = false;
    Ok(())
}
// ... inside decode_connectivity, once per face:
self.corner_table.try_grow_to_face(face.0 as usize)?;
```

The mesh itself is now sized from the corner table's real face count only
after decode has agreed with the declared count, not from the claim up
front. On the fuzz artifact: **1.07 s and 8.7 GB become 22.6 &micro;s and no
allocation**, with the same error returned. All 15 stored `decode_drc`
artifacts decode in under 2 ms. No change to any accepted stream -- 524
tests, 41 parity tests, and a 1500-mesh randomized sweep against C++ Draco
are unchanged.

### Let the decode paths grow their buffers instead of reserving from a header

**2026-08-07**, `07e247ba` &middot; touches 18 files; the three below are the
representative cases

One commit closing the same gap across every remaining path that still sized
up front from a declared count: `mesh_decoder.rs`'s two connectivity scratch
buffers, `point_cloud_decoder.rs`'s sequential point-id array, and
`symbol_encoding.rs`'s decoded-symbol sink.

**`mesh_decoder.rs`** sized both connectivity buffers from the header's
`num_indices` before decoding anything:

```rust
let mut encoded_indices = make_zeroed_indices(num_indices, buffer.size())?;
// ... decode_symbols fills encoded_indices up to num_indices ...
let mut indices = make_zeroed_indices(num_indices, buffer.size())?;
```

`decode_symbols` now appends into an unsized `Vec` that grows only as symbols
actually decode, and the second buffer is sized from what that produced:

```rust
// Empty on purpose: `decode_symbols` grows it as symbols
// arrive, so a count the stream cannot deliver costs one
// small reservation instead of the whole array.
let mut encoded_indices = Vec::new();
// ... decode_symbols appends into encoded_indices ...
// Sized from what the decode produced rather than what
// the header claimed: on success the two are equal, and on
// failure this line is not reached.
let mut indices = make_zeroed_indices(encoded_indices.len(), buffer.size())?;
```

**`point_cloud_decoder.rs`** allocated a `Vec<PointIndex>` sized from the
header's declared point count and filled it `0, 1, 2, ..., n-1` -- the
identity, written out anyway. A fuzz artifact declaring 33,686,016 points
turned that into a 134 MB allocation from a 9 KB stream:

```rust
fn make_point_ids(num_points: usize, stream_bytes: usize) -> Result<Vec<PointIndex>, DracoError> {
    // ...
    let mut point_ids = Vec::new();
    point_ids.try_reserve_exact(num_points)?;
    for i in 0..num_points {
        point_ids.push(PointIndex(i as u32));
    }
    Ok(point_ids)
}
```

An `Identity(usize)` variant on the existing `EntryToPointIdMap` enum
represents the mapping by its length alone and looks it up by arithmetic,
never materializing it:

```rust
// The identity, and not written out. Entry `i` is point `i` here, so
// materializing it bought nothing and cost four bytes per point of a
// count the header supplies -- 134 MB from a 9 KB stream on one artifact.
let point_ids = if decoder_types.iter().any(|&decoder_type| decoder_type != 0) {
    Some(EntryToPointIdMap::identity(num_points))
} else {
    None
};
```

**`symbol_encoding.rs`**'s `decode_symbols` took a caller-preallocated slice
sized to the stream's declared symbol count, so a crafted 26 KB stream naming
1,095,910,464 faces could force a multi-gigabyte allocation before a single
symbol was read. It now grows a `Vec` by `push`, with an initial reserve
capped at **8 symbols per remaining input byte** -- an earlier attempt at 1
symbol per byte was too conservative and paid for reallocations instead:

```rust
pub fn decode_symbols(..., symbols: &mut Vec<u32>) -> bool {
    symbols.clear();
    if num_values == 0 { return true; }
    if num_components == 0 || !num_values.is_multiple_of(num_components) { return false; }
    reserve_within_input(symbols, num_values, in_buffer);
```

Across the commit: the largest single allocation on each of two fuzz
artifacts is now **the size of the input itself** (26,386 and 9,034 bytes, no
amplification), and 8.4 MB across the 2,367-file `decode_drc` corpus; 705
tests pass. Decode throughput is unchanged within the benchmark's own 11%
spread -- the same benchmark caught the 1-symbol-per-byte reserve costing
17% on a 10,000-point decode, which is why the final number is 8, not 1.

---

## Entropy coding

### Store rANS decode precision as a runtime field, not a monomorphized constant

**2026-04-26** (day one) &middot; `crates/draco-core/src/rans_symbol_decoder.rs`

The rANS precision (12..=20 bits) is data-dependent -- it isn't known until
the symbol population is inspected. Upstream C++ Draco resolves this by
template: `DecodeRawSymbols` in `symbol_decoding.cc` is an 18-case `switch`
instantiating `SymbolDecoderT<1>` through `SymbolDecoderT<18>`, each a
separate template specialization of `RAnsSymbolDecoder<N>`/`RAnsDecoder<N>`
with its own compiled copy of `Create`/`StartDecoding`/`DecodeSymbol`/
`rans_build_look_up_table`. This crate's own encoder mirrors that shape for
its 9-arm case:

```rust
match rans_precision_bits {
    12 => encode_raw_symbols_internal::<12>(symbols, frequencies, target_buffer),
    13 => encode_raw_symbols_internal::<13>(symbols, frequencies, target_buffer),
    // ... one arm per precision, 14 through 19 ...
    20 => encode_raw_symbols_internal::<20>(symbols, frequencies, target_buffer),
    other => Err(DracoError::general(format!("no encoder for precision {other}"))),
}
```

`RAnsSymbolDecoder` deliberately does not: precision bits and mask are struct
fields computed once in `new()`, and decode uses them instead of compile-time
constants --

```rust
pub struct RAnsSymbolDecoder<'a> {
    pub ans: AnsDecoder<'a>,
    probability_table: Vec<RAnsSymbol>,
    lut: Vec<u32>,
    num_symbols: usize,
    rans_precision_bits: u32, // Store bits for shift operations
    rans_precision_mask: u32, // (1 << bits) - 1 for fast modulo
    // ...
}

self.ans.read_normalize();
let quo = self.ans.state >> self.rans_precision_bits; // Fast division
let rem = self.ans.state & self.rans_precision_mask;   // Fast modulo
let symbol_id = *self.lut.get(rem as usize)?;
```

One compiled copy of the decode logic instead of nine (or C++'s eighteen), at
the cost of the shift amount and mask being two struct-field reads and a
variable-width shift/mask instead of values the compiler could bake in. No
separate timing was taken -- the size cost being avoided is demonstrated
qualitatively by upstream's own 18-way instantiation, which is exactly the
pattern this design was written to sidestep.

### Memoise `f * log2(f)` in the Shannon entropy tracker

**2026-08-12**, `f9a331d` &middot; `crates/draco-core/src/shannon_entropy.rs`

`update_symbols` recomputed `(frequency as f64) * (frequency as f64).log2()`
for the old and new frequency of every symbol on every peek/push. The
constrained encoder peeks every candidate prediction config, so the same
small integer frequencies recur thousands of times:

```rust
let mut old_symbol_entropy_norm = 0.0;
if frequency > 1 {
    old_symbol_entropy_norm = (frequency as f64) * (frequency as f64).log2();
}
// ...
let new_symbol_entropy_norm = (frequency as f64) * (frequency as f64).log2();
```

A lazily-filled `Vec` keyed by frequency memoises the exact `f64` the inline
expression would have produced -- no approximation, so every entropy
estimate (and every prediction-config decision made from it) is bit-for-bit
unchanged:

```rust
fn f_times_log2_f(&mut self, f: i32) -> f64 {
    if f < 2 { return 0.0; }
    let i = f as usize;
    if i >= self.entropy_norm_cache.len() {
        let old_len = self.entropy_norm_cache.len();
        self.entropy_norm_cache.resize(i + 1, 0.0);
        for j in old_len..=i {
            let jf = j as f64;
            self.entropy_norm_cache[j] = jf * jf.log2();
        }
    }
    self.entropy_norm_cache[i]
}
```

Pinned to one core, 10 interleaved runs (grid 100x100, quantization 10, speed
1): median **5909.9 -> 5373.7 us, -9.1%** end-to-end encode. `log2`'s share of
encode time in a `samply` profile drops 30.3% -> 18.8%. A checksum of one
encode is identical with and without the change.

**Re-measured 2026-08-14** on a different asset and in the current regime
(make the memo always recompute; 18 interleaved rounds, Bunny encode at speed
1): **38101.4 -> 35043.2 us, -8.0%**. The original -9.1% holds up.

The memo's own memory bound was later measured and documented rather than
changed -- see [Measured and rejected](#measured-and-rejected-what-looked-like-a-trick-and-wasnt).

### Hoist the per-config overhead estimate out of the permutation loop

**2026-08-13**, `27601f2` &middot; `crates/draco-core/src/prediction_scheme_constrained_multi_parallelogram.rs`

The constrained multi-parallelogram encoder scores every configuration of
every entry, and each score called `log2` three times: once for the
`n * log2(n)` data-bits term, twice inside the overhead estimate's binary
entropy -- 9.9% of encode on the Stanford Bunny, the single largest item in
the profile. The overhead term depends only on *how many* parallelograms a
config uses, not which ones, so it was being recomputed identically for every
permutation of a fixed count:

```rust
loop {
    // ... build candidate config ...
    let entropy_data = self.entropy_tracker.peek(&entropy_symbols);
    error.num_bits =
        ShannonEntropyTracker::get_number_of_data_bits_static(&entropy_data)
            + ShannonEntropyTracker::get_number_of_r_ans_table_bits_static(&entropy_data);

    let overhead_bits = Self::compute_overhead_bits(
        total_used_parallelograms[context] + num_used as i64,
        total_parallelograms[context],
    );
    error.num_bits += overhead_bits;
}
```

Hoisted above the loop over permutations, computed once per count:

```rust
let overhead_bits = Self::compute_overhead_bits(
    total_used_parallelograms[context] + num_used as i64,
    total_parallelograms[context],
);

loop {
    // ... build candidate config ...
    let entropy_data = self.entropy_tracker.peek(&entropy_symbols);
    error.num_bits = self.entropy_tracker.number_of_data_bits(&entropy_data)
        + ShannonEntropyTracker::get_number_of_r_ans_table_bits_static(&entropy_data);
    error.num_bits += overhead_bits;
}
```

The `n * log2(n)` term is handled the same way, memoising the last `(n, n *
log2(n))` seen since `n` (the running value count) is identical for every
candidate of one entry. Both keep the exact `f64` the uncached expression
produced. Bunny, two builds per side, 20 interleaved 2-second runs each:
`encode 35170-35517 -> 34136-34154 us`, **-3.4%**; decode clusters overlap
(untouched control).

### Bound memory to what was decoded, not to the value a symbol could reach

**2026-07-31 to 2026-08-01**, `be37ec49` and `8a982199` &middot; `crates/draco-core/src/shannon_entropy.rs`

The frequency table was a `Vec` indexed by raw symbol value, so a rejected
candidate's magnitude -- not the number of distinct symbols actually seen --
set the table's size:

```rust
for &symbol in symbols {
    let symbol = symbol as usize;
    if self.frequencies.len() <= symbol {
        self.frequencies.resize(symbol + 1, 0);
    }
    let frequency = self.frequencies[symbol];
    self.frequencies[symbol] += 1;
}
if push_changes {
    self.entropy_data = ret_data;
} else {
    for &symbol in symbols { self.frequencies[symbol as usize] -= 1; }
}
```

A symbol is a zig-zagged prediction residual bounded only by `u32`; one
candidate whose averaged prediction overflowed toward `u32::MAX` grew the
table to billions of entries just to be scored and discarded. Growth is now
gated on `push_changes`; a rejected `peek` instead counts how often the
symbol already appeared earlier in the same call (a symbol the table doesn't
cover has frequency zero by definition):

```rust
for (i, &symbol) in symbols.iter().enumerate() {
    let index = symbol as usize;
    let mut frequency = 0;
    if index < self.frequencies.len() {
        frequency = self.frequencies[index];
    } else if push_changes {
        self.frequencies.resize(index + 1, 0);
    } else {
        for &earlier in &symbols[..i] {
            if earlier == symbol { frequency += 1; }
        }
    }
    frequency += 1;
    if index < self.frequencies.len() { self.frequencies[index] = frequency; }
}
```

Measured over 400 randomly generated meshes (1-30 quantization bits, all
speeds; the same fix applied to both this crate's and C++ Draco's own
encoder): **peak RSS 21.9 GB -> 7.0 GB, wall clock 65s -> 41s**, output
byte-identical.

A day later, a legitimate encode (`-qp 30 -cl 10` on a 100-point mesh) still
asked for 17 GB, because the table's cost tracks residual magnitude even
without the discarded-candidate case. Symbols at or above `2^18` (the symbol
coder's own cutoff for its raw scheme) now live in a `HashMap` instead of a
dense `Vec` slot:

```rust
const MAX_DENSE_SYMBOL: usize = 1 << 18;

pub struct ShannonEntropyTracker {
    entropy_data: EntropyData,
    frequencies: Vec<i32>,
    sparse_frequencies: std::collections::HashMap<u32, i32>,
}
```

Memory now bounded by the number of *encoded values*, not by residual
magnitude: the same 100-point mesh at `-qp 30 -cl 10` went from **~17 GB and
13 seconds to 0 ms and no oversized allocation**, frequencies unchanged so
output stays byte-for-byte identical. Both fixes double as hardening --
neither changes what a well-formed encode produces, both remove a
pathological-input cost that scaled with a value the input controls, which is
itself a performance property (see `SECURITY.md`'s allocation-bound section).

---

## Attribute prediction schemes

### Elide bounds checks the guard above already proved

**2026-08-11 / 2026-08-12**, `70af635` and `94e67cd` &middot;
`crates/draco-core/src/prediction_scheme_constrained_multi_parallelogram.rs`,
`crates/draco-core/src/prediction_scheme_parallelogram.rs`

Both the constrained decoder's inner loop and the shared
`compute_parallelogram_prediction` helper indexed three neighbour regions by
`base_off + k` once per component, each carrying a bounds check -- even
though an explicit guard just above (`vert_* < data_entry_id`) had already
proven every index in range:

```rust
for k in 0..num_components {
    let p = DataType::compute_parallelogram_prediction(
        out_data[v_next_off + k],
        out_data[v_prev_off + k],
        out_data[v_opp_off + k],
    );
    multi_pred_vals[k] = DataType::add_as_unsigned(multi_pred_vals[k], p);
}
```

Slicing each neighbour region to exactly `num_components` and zip-iterating
removes the per-element check from the inner loop while the guard above still
returns `Err`/`false` on a malformed stream -- nothing upstream of the slice
changes:

```rust
let v_opp = &out_data[v_opp_off..v_opp_off + num_components];
let v_next = &out_data[v_next_off..v_next_off + num_components];
let v_prev = &out_data[v_prev_off..v_prev_off + num_components];
for (((pv, &n), &pr), &op) in multi_pred_vals.iter_mut().zip(v_next).zip(v_prev).zip(v_opp) {
    let p = DataType::compute_parallelogram_prediction(n, pr, op);
    *pv = DataType::add_as_unsigned(*pv, p);
}
```

Constrained decoder inner loop (speed 1): min **1291.9 -> 1252.1 us (-3.1%)**,
median ~1362 -> ~1324 us (-2.6%) end-to-end decode; the predictor's own share
of a `samply` profile falls 33.4% -> 31.7% (~8% faster itself). The shared
helper (speed 2, basic parallelogram): median **920.2 -> 901.5 us (-2.0%)**
end-to-end; its own profile share falls 8.11% -> 5.33% (~34% faster itself).

**Re-measured 2026-08-14**, reverting each to its indexed form, 20-22
interleaved rounds. Both hold up on the position-only 100x100 grid they were
originally measured on -- and both disappear on a mesh carrying normals:

| | grid (100x100, positions only) | Bunny (pos+norm) |
|---|---|---|
| constrained MP, speed 1 | 1541.5 -> 1480.0 us, **-4.0%** | 13584.9 -> 13545.8, **-0.3%** |
| shared helper, speed 2 | 1016.4 -> 995.5 us, **-2.1%** | 11723.2 -> 11826.0, **+0.9%** |

The grid figures confirm the originals (-3.1% and -2.0%) almost exactly. On
the Bunny both sit at or under the layout floor, and the helper even reads
backwards -- not because the elision stopped working, but because a mesh with
normals spends its decode in the octahedral normal path instead, so the
position predictor it speeds up is a much smaller share of the total. Quote
these against the asset, not in the abstract.

### Predict from the decoded values in place, not from a copy of them

**2026-08-13**, `50c2b48` &middot; `crates/draco-core/src/prediction_scheme_delta.rs`

The delta scheme's prediction for an entry is simply the entry right before
it -- already sitting in `out_data` -- but it was copied into a scratch
buffer first, a memcpy per entry. On a point cloud, where this scheme carries
every value, that was 8% of decode:

```rust
let mut predicted = vec![DataType::default(); num_components];
for i in (num_components..size).step_by(num_components) {
    predicted.copy_from_slice(&out_data[i - num_components..i]);
    let corr = &in_corr[i..i + num_components];
    let out = &mut out_data[i..i + num_components];
    self.transform.compute_original_value(&predicted, corr, out);
}
```

`split_at_mut` hands the previous entry over as a slice directly instead of
copying it:

```rust
for i in (num_components..size).step_by(num_components) {
    let (decoded, rest) = out_data.split_at_mut(i);
    let predicted = &decoded[i - num_components..];
    let corr = &in_corr[i..i + num_components];
    let out = &mut rest[..num_components];
    self.transform.compute_original_value(predicted, corr, out);
}
```

Two builds per side, 12-15 interleaved 2-second runs, clusters 0.2% wide
where the effect is large: `pc_color.drc` (sequential point cloud + colour)
**-15.6%**, `bunny_cpp_standard.drc` (mesh, C++ default settings) **-10.4%**,
Bunny at speed 10 **-10.0%**, `lamp_cpp_std.drc` -1.3%, `pc_kd_color.drc`
unchanged (no delta path), Bunny at speed 1 unchanged (constrained
multi-parallelogram, different code path), encode unaffected (control).

### Memoise vertex positions in the geometric normal predictor

**2026-08-13**, `c8e1be6` &middot; `crates/draco-core/src/prediction_scheme_geometric_normal.rs`

The predictor resolved a corner to a decoded position through
`vertex_to_data_map`, `entry_to_point_id_map`, the attribute's index map, and
a byte-level read of three components -- once per call, with no cache (C++'s
`GetPositionForCorner` has none either). It visits every corner around a
vertex while reading both neighbours' positions, so a regular mesh redecoded
each position about six times over, once more for each neighbour's own
prediction:

```rust
fn get_position_for_corner(&self, corner_id: CornerIndex) -> [i32; 3] {
    if corner_id == INVALID_CORNER_INDEX { return [0, 0, 0]; }
    let Some(mesh_data) = self.mesh_data.as_ref() else { return [0, 0, 0]; };
    let Some(corner_table) = mesh_data.corner_table() else { return [0, 0, 0]; };
    let Some(vertex_to_data_map) = mesh_data.vertex_to_data_map() else { return [0, 0, 0]; };
    let Some(pos_attribute) = self.pos_attribute else { return [0, 0, 0]; };
    let v = corner_table.vertex(corner_id);
    // ... resolve data_id, point_id, pos_val_id, then a fresh 3-component read ...
}
```

The parent position attribute is fully decoded before any normal that
predicts from it, and neither map changes during the pass, so the maps are
resolved once and the decoded position is cached per vertex:

```rust
struct CornerPositions<'b> {
    corner_table: &'b CornerTable,
    vertex_to_data_map: &'b [i32],
    entry_to_point_id_map: EntryToPointIdMap<'b>,
    pos_attribute: &'b PointAttribute,
    cache: Vec<[i32; 3]>,
    cached: Vec<bool>,
}

impl<'b> CornerPositions<'b> {
    fn get(&mut self, corner_id: CornerIndex) -> [i32; 3] {
        if corner_id == INVALID_CORNER_INDEX { return [0, 0, 0]; }
        let vi = self.corner_table.vertex(corner_id).0 as usize;
        if self.cached[vi] { return self.cache[vi]; }
        let pos = position_for_vertex(self.vertex_to_data_map, self.entry_to_point_id_map, self.pos_attribute, self.corner_table.vertex(corner_id));
        self.cache[vi] = pos;
        self.cached[vi] = true;
        pos
    }
}
```

Stanford Bunny (36k vertices, pos+norm, qp 11/qn 8), interleaved runs pinned
to one core, 10 rounds each: decode median **14884.0 -> 13689.1 us, -8.0%**
(min -8.1%); encode +0.3% (untouched control). `prediction_scheme_geometric_normal`'s
self time in a `samply` profile falls 17.2% -> 12.6% -- what remains is
corner-table walking, not position decoding.

**Re-measured 2026-08-14** (force the cache to always miss; 20 interleaved
rounds, same asset and speed): **16464.9 -> 13905.1 us, -15.5%** -- nearly
twice the original figure. The memo did not get better; everything around it
got faster, so the share of decode it removes grew. A win measured against a
2026-08-13 baseline is not the same number against today's.

---

## KD-tree point-cloud decode: a five-commit sequence

**2026-08-13 to 2026-08-14** &middot; `crates/draco-core/src/kd_tree_attributes_decoder.rs`,
`crates/draco-core/src/dynamic_integer_points_kd_tree.rs`

One profiling target, followed to the end: `pc_kd_color.drc` decode went from
476.3 to 316.4 &micro;s across five commits, **-33.6%** total. Kept together
here because each step's diff is the clean illustration of a pattern that
recurs individually elsewhere in this document -- per-scalar bounds checks,
scratch-buffer round trips, table lookups that are really a rotation, and a
runtime-length copy that should have been a compile-time one.

### 1. Bound a point's row once, write every component into it

**2026-08-13**, `5b4023c`

Each of a point's components ran its own overflow-checked index math and its
own `copy_from_slice` of one to four bytes -- a memcpy call per scalar, 19%
of decode on a kd-tree point cloud:

```rust
for c in 0..num_components {
    let decoded_index = p
        .checked_mul(total_dimensionality)
        .and_then(|v| v.checked_add(offset))
        .and_then(|v| v.checked_add(c))
        .ok_or_else(|| DracoError::general(format!("Point {p} component {c} overflows")))?;
    let &v = decoded.get(decoded_index).ok_or_else(|| DracoError::general("oob".into()))?;
    let component_offset = c.checked_mul(component_size)
        .and_then(|delta| base.checked_add(delta))
        .ok_or_else(|| DracoError::general("offset overflows".into()))?;
    write_unsigned_component(target_attribute.buffer_mut(), component_offset, target_type, v)
        .map_err(|err| DracoError::general(format!("Point {p} component {c}: {err}")))?;
}
```

Bounding the whole row once and taking the destination slice up front lets
every component write straight into it:

```rust
let dst = target_attribute.buffer_mut().data_mut()
    .get_mut(base..row_end)
    .ok_or_else(|| DracoError::general(format!("Point {p} writes {base}..{row_end} past the attribute buffer")))?;
match target_type {
    DataType::Uint8 => {
        for (d, &v) in dst.iter_mut().zip(src) { *d = v as u8; }
    }
    DataType::Uint16 => {
        for (d, &v) in dst.chunks_exact_mut(2).zip(src) {
            d.copy_from_slice(&(v as u16).to_le_bytes());
        }
    }
    DataType::Uint32 => {
        for (d, &v) in dst.chunks_exact_mut(4).zip(src) {
            d.copy_from_slice(&v.to_le_bytes());
        }
    }
    _ => {}
}
```

The overflow/out-of-range refusal moves from per-component to per-row, so a
short buffer or truncated stream is still an `Err`, not a panic.
`pc_kd_color.drc` **-7% to -12%** (two builds per side); `pc_color.drc` and
`bunny_cpp_standard.drc` unchanged as controls.

### 2. Walk the tree over the stack rows in place

**2026-08-14**, `edaba23`

Each node copied its base and level rows out of two flat stacks into scratch
vectors, then copied the result back -- six memcpys per node -- because the
rows live behind `&mut self` while the entropy decoders also need `&mut
self`, so the borrow checker would not let a row slice outlive a decode call:

```rust
let row_start = stack_pos * dimension;
old_base.copy_from_slice(&self.base_stack[row_start..row_start + dimension]);
levels.copy_from_slice(&self.levels_stack[row_start..row_start + dimension]);
// ...
let child_start = (stack_pos + 1) * dimension;
self.base_stack[child_start..child_start + dimension].copy_from_slice(&old_base);
self.base_stack[child_start + axis as usize] += modifier;
// ...
levels[axis as usize] += 1;
self.levels_stack[row_start..row_start + dimension].copy_from_slice(&levels);
self.levels_stack[child_start..child_start + dimension].copy_from_slice(&levels);
```

Moving both stacks out of `self` for the duration of the walk lets the rows
be read where they sit; propagating to a child (whose row is the next one
along) becomes a `copy_within` instead of a round trip through scratch:

```rust
let mut base_stack = std::mem::take(&mut self.base_stack);
let mut levels_stack = std::mem::take(&mut self.levels_stack);
let ok = self.decode_walk(num_points, out, &mut base_stack, &mut levels_stack);
self.base_stack = base_stack;
self.levels_stack = levels_stack;
// ...
base_stack.copy_within(row_start..row_end, child_start);
base_stack[child_start + axis] += modifier;
// ...
levels_stack[row_start + axis] += 1;
levels_stack.copy_within(row_start..row_end, child_start);
```

`pc_kd_color.drc` **-23.4%**, `bpy_point_cloud.kd` **-18.3%**, mesh decode
unchanged (0.0%); decoded attributes and faces stay bit-identical on all four
kd-tree fixtures.

### 3. Assemble a point where it is stored, not in a scratch row

**2026-08-14**, `377bf84`

A leaf decoded its point into a scratch row field, then appended that row to
the output vector -- one more memcpy per point, the largest single item left
in the profile after the walk itself:

```rust
for j in 0..self.dimension as usize {
    self.p[self.axes[j] as usize] = 0;
    let num_bits = self.bit_length - levels[self.axes[j] as usize];
    if num_bits != 0 {
        let ok = self.remaining_bits_decoder.decode_least_significant_bits32(
            num_bits, &mut self.p[self.axes[j] as usize]);
        if !ok { return false; }
    }
    self.p[self.axes[j] as usize] |= old_base[self.axes[j] as usize];
}
out.extend_from_slice(&self.p);
```

Growing the output vector by one point and writing each axis slot directly
into its tail removes the scratch row entirely; the axis permutation still
covers every dimension exactly once, so nothing is left at the zero the
growth put there:

```rust
let start = out.len();
out.resize(start + dimension, 0);
let p = &mut out[start..];
for j in 0..dimension {
    let axis_j = self.axes[j] as usize;
    let num_bits = self.bit_length - levels[axis_j];
    let mut value = 0u32;
    if num_bits != 0 {
        let ok = self.remaining_bits_decoder
            .decode_least_significant_bits32(num_bits, &mut value);
        if !ok { return false; }
    }
    p[axis_j] = value | old_base[axis_j];
}
```

`pc_kd_color.drc` **-7.6%** on top of the in-place walk (**-28.4%** against
both combined), the four kd-tree fixtures still bit-identical.

### 4. Carry the leaf's axis order in a variable, not a table

**2026-08-14**, `ec0524c`

Every leaf rebuilt the axis order into a `self.axes` vector, then read it
back one bounds-checked element at a time -- but the order is just the
rotation starting at the node's own axis:

```rust
self.axes[0] = axis as u32;
for i in 1..self.dimension as usize {
    self.axes[i] = increment_mod(self.axes[i - 1], self.dimension);
}
// ... later, per component:
let axis_j = self.axes[j] as usize;
```

Advancing a single `axis_j` variable with `increment_mod` each iteration
reproduces the same sequence without ever materialising or indexing a table:

```rust
let mut axis_j = axis;
for _ in 0..dimension {
    let num_bits = self.bit_length - levels[axis_j];
    let mut value = 0u32;
    // ... decode into value ...
    p[axis_j] = value | old_base[axis_j];
    axis_j = increment_mod(axis_j as u32, self.dimension) as usize;
}
```

`pc_kd_color.drc` **-4.2%**; the field and its allocation disappear along
with the table.

### 5. Copy a node's row at a length the compiler knows

**2026-08-14**, `d674fd6`

The two `copy_within` calls left after step 2 still cost more than what they
moved: a run-time-only length lowers to a `memmove` call, but a node's row is
only a handful of words.

```rust
base_stack.copy_within(row_start..row_end, child_start);
base_stack[child_start + axis] += modifier;
// ...
levels_stack[row_start + axis] += 1;
levels_stack.copy_within(row_start..row_end, child_start);
```

Dispatching the dimensions that occur in practice (1 through 12) to a
const-generic helper turns each into inlined loads and stores; anything wider
falls back to the call, where the row is big enough to justify it:

```rust
fn copy_row_to_next(stack: &mut [u32], src: usize, dim: usize) {
    fn fixed<const N: usize>(stack: &mut [u32], src: usize) {
        let Some(window) = stack.get_mut(src..src + 2 * N) else { return; };
        let (row, next) = window.split_at_mut(N);
        next.copy_from_slice(row);
    }
    match dim {
        1 => fixed::<1>(stack, src),
        2 => fixed::<2>(stack, src),
        // ... 3 through 11, same pattern ...
        12 => fixed::<12>(stack, src),
        _ => stack.copy_within(src..src + dim, src + dim),
    }
}
```

`pc_kd_color.drc` **-3.4%**; `memmove` is gone from the profile of that path.
A block-copy formulation that avoided the twelve match arms was measured too
and came out **slower** (+1.0% against this change's -3.4%), so the arms
stay -- see [Measured and rejected](#measured-and-rejected-what-looked-like-a-trick-and-wasnt).

---

## I/O and WASM: allocation off the hot path

These are outside the codec proper (`draco-io`'s file readers/writers, the
`web/` WASM bridges), but the pattern -- a collection or `String` built once
per element instead of once for the whole call -- is the same one the codec
side applies, so it belongs in the same reference.

### Tokenize forward, don't collect into a `Vec` you read once

**2026-08-03**, `0732e565` &middot; `web/obj-wasm/src/lib.rs`

`parse_obj_internal` collected `line.split_whitespace()` into a `Vec` for
every non-blank line, and `part.split('/')` into another `Vec` for every
face-vertex token, purely to hold values read once in order:

```rust
let parts: Vec<&str> = line.split_whitespace().collect();
if parts.is_empty() { continue; }
match parts[0] {
    "v" if parts.len() >= 4 => {
        if let (Ok(x), Ok(y), Ok(z)) = (parts[1].parse::<f32>(), parts[2].parse::<f32>(), parts[3].parse::<f32>()) {
            positions.push([x, y, z]);
        } else {
            warnings.push(format!("Line {}: Invalid vertex coordinates", line_num + 1));
        }
    }
```

Rewritten as forward-only `.next()` calls on the same split iterators, with
`if parts.len() >= N` guards replaced by tuple patterns that fail to match at
exactly the same point the length guard would have:

```rust
let mut tokens = line.split_whitespace();
let Some(keyword) = tokens.next() else { continue; };
match keyword {
    "v" => {
        if let (Some(x), Some(y), Some(z)) = (tokens.next(), tokens.next(), tokens.next()) {
            if let (Ok(x), Ok(y), Ok(z)) = (x.parse::<f32>(), y.parse::<f32>(), z.parse::<f32>()) {
                positions.push([x, y, z]);
            } else {
                warnings.push(format!("Line {}: Invalid vertex coordinates", line_num + 1));
            }
        }
    }
```

On the 51.1 MB benchmark file: roughly 1.31M line-tokenizations plus 1.57M
corner-tokenizations, about **2.9M short-lived heap `Vec`s eliminated**.
Verified byte-for-byte against the previous parsing behaviour across 20
fixtures.

### Size the buffer once instead of allocating per element

**2026-08-03**, `82d87e97` and `56e9d738` &middot; `crates/draco-io/src/fbx_encoder.rs`, `web/ply-wasm/src/lib.rs`

`write_array_property` built its byte buffer as one heap-allocated `Vec` per
array element (`flat_map` + `collect`), for every position/normal/uv/color/
index value in the mesh:

```rust
let raw_data: Vec<u8> = values.iter().flat_map(&to_bytes).collect();
```

A `Vec` sized once up front, filled in place, never regrows:

```rust
let mut raw_data = Vec::with_capacity(values.len() * std::mem::size_of::<T>());
for value in values {
    write_element(value, &mut raw_data);
}
```

The same shape recurs in the wasm writers, which built a fresh `Vec<u8>` per
*vertex* just to hand it to a buffer write:

```rust
for (i, chunk) in input.positions.chunks_exact(3).enumerate() {
    let bytes: Vec<u8> = chunk.iter().flat_map(|value| value.to_le_bytes()).collect();
    pos_att.buffer_mut().write(i * 12, &bytes);
}
```

replaced by a single-pass writer with no intermediate `Vec` at all:

```rust
pos_att.buffer_mut().write_f32s_le(0, &input.positions);
```

FBX write (263k verts/524k tris grid, combined with a deflate-level change
below): **848ms -> ~211ms**. PLY: 31.5 -> 23.3ms; STL: 58.2 -> 50.5ms (best of
7). FBX/DRC writers moved under 3% from this specific change alone -- zlib
and the entropy coder already dominated those, so allocation wasn't their
bottleneck. Decoded content hashed identical before/after in every case.

### `write!` into the destination instead of `format!` + `push_str`

**2026-08-03**, `6d8451e8` &middot; `web/obj-wasm/src/lib.rs`

Both text writers built every line with `format!` and pushed the result --
one `String` allocated and dropped per line, about 1.8 million allocations
for a 263k-vertex OBJ mesh:

```rust
let mut output = String::new();
output.push_str(&format!("v {:.*} {:.*} {:.*}\n", precision, x, precision, y, precision, z));
```

`write!` formats directly into the destination, sized up front by estimate
rather than doubled into from empty:

```rust
let mut output = String::with_capacity(estimated_bytes(meshes, precision));
for position in mesh.positions.chunks_exact(3).take(vertex_count) {
    let _ = writeln!(output, "v {:.*} {:.*} {:.*}", precision, position[0], precision, position[1], precision, position[2]);
}
```

OBJ export: **318.4 -> 269.1 ms** median. Output verified byte-identical
across all four OBJ face forms.

### Drive the glTF compressor from the in-memory document, not a serialize/reparse round trip

**2026-06-21**, `9ad7dee4` &middot; `crates/draco-gltf/src/lib.rs`

`compress()` cloned the document to JSON, base64-encoded every buffer into a
data URI, serialized the whole thing to bytes, and handed those bytes to
`draco-io`'s byte-oriented compressor -- which then re-parsed the JSON and
re-resolved the buffers it had *just* held in memory:

```rust
pub fn compress(document: &gltf::Document, buffers: &[Vec<u8>]) -> Result<Vec<u8>> {
    let mut root = document.clone().into_json();
    for (i, buf) in buffers.iter().enumerate() {
        let entry = root.buffers.get_mut(i)?;
        entry.uri = Some(format!("data:application/octet-stream;base64,{}", base64_encode(buf)));
        entry.byte_length = gltf::json::validation::USize64(buf.len() as u64);
    }
    let bytes = serde_json::to_vec(&root)?;
    draco_io::compress_gltf_bytes(&bytes, None)?
}
```

A shared core takes the already-parsed document and already-resolved buffers
directly:

```rust
pub fn compress(document: &gltf::Document, buffers: &[Vec<u8>]) -> Result<Vec<u8>> {
    let doc_value = serde_json::to_value(document.clone().into_json())?;
    let reader = draco_io::GltfReader::from_value(&doc_value, buffers.to_vec())?;
    let (mut out_doc, bin) = draco_io::compress_gltf_value(doc_value, buffers, None, |mesh, prim| {
        reader.decode_primitive_with_semantics(mesh, prim)
    })?;
    embed_single_buffer(&mut out_doc, &bin);
    Ok(serde_json::to_vec(&out_doc)?)
}
```

Removes the redundant serialize-to-bytes / re-parse / re-resolve-buffers /
base64 encode+decode cycle entirely. Size-optimized wasm, full import +
compress: ~304 -> ~295 KB gzip, since the now-unused byte-path code is also
dead-code-eliminated.

### A dependency bump can be the optimization

**2026-08-03**, `d97f1ffd` &middot; `crates/draco-io/Cargo.toml`

`draco-io` pinned `miniz_oxide` at `"0.6"` (0.6.2), well short of 0.8.4 --
where upstream `miniz_oxide` worked around a Rust compiler codegen regression
([rust-lang/rust#132636](https://github.com/rust-lang/rust/issues/132636))
that costs roughly 60% of deflate's compression speed on affected `rustc`
versions. This repo builds on 1.95, an affected version:

```toml
miniz_oxide = { version = "0.6", optional = true }
```

```toml
miniz_oxide = { version = "0.9.1", optional = true }
```

On 0.9.1, level 1 is fastest again as expected: **184ms/4.66MB** vs level
2's 215ms/4.00MB on the 263k-vertex benchmark mesh. This reversed an earlier
finding that had blamed level 1's relative slowness on the *data* -- it was
actually the 0.6.2 regression distorting the measurement. No breaking API
change between 0.6.2 and 0.9.1; full test suites pass unchanged.

---

## Binary size: dead code elimination

Not runtime speed, but the same "pay only for what you use" instinct applied
to the WASM footprint -- included because the technique (feature-gate,
`#[cfg]`, dead-code fallout) is the same shape as the speed tricks above, just
aimed at a different metric.

### Route diagnostics through a macro that compiles away

**2026-06-23**, `54d94b38` &middot; `crates/draco-core/src/lib.rs`

`draco-core` is a library, but 179 `eprintln!`/`println!` call sites in the
decode/encode paths printed unconditionally in release builds -- including a
per-element formatting loop in the sequential integer decoder's hot path:

```rust
let method_byte = match in_buffer.decode_u8() {
    Ok(v) => v,
    Err(_) => {
        eprintln!("Failed to decode prediction method");
        return false;
    }
};
```

Every diagnostic now goes through a macro that wraps the call in a
compile-time-evaluable condition. The format arguments stay type-checked, so
the code doesn't rot, but when the feature is off the branch is dead and the
compiler removes both the formatting work and the call:

```rust
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if cfg!(feature = "debug_logs") {
            eprintln!($($arg)*);
        }
    };
}

let method_byte = match in_buffer.decode_u8() {
    Ok(v) => v,
    Err(_) => {
        debug_log!("Failed to decode prediction method");
        return false;
    }
};
```

No before/after benchmark taken; zero runtime cost by construction once the
feature is off, across 179 converted sites.

### Feature-gate a dependency that most callers never touch

**2026-06-16**, `793132d8` &middot; `crates/draco-gltf/src/lib.rs`

The `image` crate (PNG/JPEG codecs), pulled in transitively just to decode
texture pixels, was the largest part of a `draco-gltf` wasm build even though
geometry-only callers never touch `Import::images`:

```rust
pub fn import_slice(bytes: &[u8], base: Option<&Path>) -> Result<Import> {
    let gltf::Gltf { document, blob } = gltf::Gltf::from_slice_without_validation(bytes)?;
    validate(&document)?;
    let buffers = gltf::import_buffers(&document, base, blob)?;
    let images = gltf::import_images(&document, base, &buffers)?;
    Ok(Import { document, buffers, images })
}
```

An opt-in, default-on `image` Cargo feature; `default-features = false` drops
the codec entirely and falls back to a small built-in buffer loader (data
URIs and the GLB BIN chunk only):

```rust
pub fn import_slice(bytes: &[u8], base: Option<&Path>) -> Result<Import> {
    let gltf::Gltf { document, blob } = gltf::Gltf::from_slice_without_validation(bytes)?;
    validate(&document)?;
    #[cfg(feature = "image")]
    { /* full import with images, as before */ }
    #[cfg(not(feature = "image"))]
    {
        let buffers = load_buffers(&document, blob, base)?;
        Ok(Import { document, buffers })
    }
}
```

Size-optimized wasm: **~408 KB -> ~304 KB gzip** (~1227 -> ~957 KB raw)
without `image`.

### Gate write-only code behind the write feature

**2026-07-27**, `adb270c5` &middot; `crates/draco-gltf/src/extensions.rs`

Three extension handlers exist only to answer whether a *binary transform*
may touch the document -- a question a read-only build never asks -- yet the
default registry registered all twenty unconditionally:

```rust
registry.register(DracoExtension)?;
registry.register(MeshGpuInstancingExtension)?;
registry.register(StructuralMetadataExtension)?;
for name in BINARY_FREE_EXTENSIONS {
    registry.register(BinaryFreeExtension(name))?;
}
```

Wrapped in `#[cfg(feature = "write")]`, so a decode-only WASM reader never
links the handler code at all, rather than skipping it at runtime:

```rust
registry.register(DracoExtension)?;
#[cfg(feature = "write")]
{
    registry.register(MeshGpuInstancingExtension)?;
    registry.register(StructuralMetadataExtension)?;
    for name in BINARY_FREE_EXTENSIONS {
        registry.register(BinaryFreeExtension(name))?;
    }
}
```

WASM reader release profile: **115.0 KiB -> 113.2 KiB** (1.8 KiB of a 115 KiB
budget it was already close to).

---

## Measured and rejected: what looked like a trick and wasn't

Kept for the same reason the wins above are kept -- so nobody re-derives
these and burns the time this session already spent.

| tried | where | result |
|---|---|---|
| Branchless `next`/`previous` on the corner table (`c + 1 - 3 * wraps` instead of the branch on face wrap) | `corner_table.rs`, `9f7ad8c` | **+8.8% slower** end-to-end. The wrap pattern is perfectly predictable; the arithmetic landed on the dependency chain every vertex-fan swing waits on. |
| Rewrite `opposite`/`vertex` as an explicit range test + direct index | `corner_table.rs`, `432344b` | Originally claimed -0.8%; re-measured in a regime that resolves 0.1% (not the original's 2%), it is **+0.2% -- nothing**. Reverted; the finding is kept, the change is not. |
| Remove the corner-table bounds check via `get_unchecked` | `corner_table.rs`, `432344b` | **2.0%** real, on Bunny decode -- the actual price of `unsafe_code = "forbid"` on this path (`SECURITY.md`). Not taken: `draco-core` forbids `unsafe` by policy, not by omission. |
| Cap the entropy `f*log2(f)` memo at 2^16 entries | `shannon_entropy.rs`, `e568bb6` | **+1.2% slower** encode. The function is hot enough that one more comparison per call outweighs the memory the cap would save; documented rather than applied. |
| Rewrite the wrap-transform's inner loop as a zip over equal-length slices, the same shape as the parallelogram-helper fix above | `prediction_scheme_wrap.rs`, mentioned in `50c2b48` | Accounts for **none** of that commit's measured win and costs **1.4%** on top of it. Not applied. |
| Eliding rANS LUT bounds checks, two variants | rANS decode path, `dev/profiling/README.md` | **+0.7%** and **+1.4%** respectively -- both slower. Not applied. |
| Copy a kd-tree node's row in constant-size 4-word blocks, to avoid the twelve `match` arms in the const-generic dispatch | `dynamic_integer_points_kd_tree.rs`, considered alongside `d674fd6` | **+1.0%** against the `copy_within` it would have replaced -- slower than doing nothing. The dispatch-by-dimension version (**-3.4%**) shipped instead. |
| Pre-size the kd-tree decoder's output `Vec` and fill it through a cursor, to drop the per-point `resize` | `dynamic_integer_points_kd_tree.rs`, considered alongside `d674fd6` | **0.1%**, i.e. nothing. `resize` within existing capacity already costs about what the bounds check it replaces costs. |
| Reserve the kd-tree traversal stack instead of letting it grow | `dynamic_integer_points_kd_tree.rs`, considered alongside `d674fd6` | **0.0%**. |
| `DecoderBuffer` borrowing the input slice zero-copy, as a claimed Rust-vs-C++ win | `decoder_buffer.rs` vs. upstream `decoder_buffer.h` | **No real difference.** Upstream's `Init(const char*, size_t)` stores a raw, uncopied pointer too -- both are O(1) zero-copy. The actual difference is that Rust's borrow carries a lifetime the compiler checks at every call site, where C++ has a comment asking the caller to keep the buffer alive. A safety story, not a speed one. |
| Deriving the rANS remainder as `state - quot * p` to avoid a second division | `rans_symbol_encoder.rs`, day-one design, re-measured 2026-08-14 | **Backwards: it costs 2.1% of encode.** Plain `state % p` is faster, because LLVM lowers `/` and `%` on the same operands to one `div` that already produces both. Two builds per condition, clusters 0.04% and 0.2% wide and disjoint. Fixed in `22c459e`. |
| The encode-side "3-component float -> uint32" quantize fast path | `attribute_quantization_transform.rs:263`, measured 2026-08-14 | **0.1% -- nothing measurable.** Disabling it so every attribute takes the generic path moved Bunny encode at speed 5 from 15486.9 to 15499.7 us. Quantization runs once per attribute; a speed-5 encode is spent in the traversal and the prediction search, so the fast path has almost nothing to be fast *of*. Kept (it costs nothing either), but it is not a lever. |
| Eliding the bounds checks around the rANS symbol LUT | `rans_symbol_decoder.rs`, two attempts before 2026-08-14, re-attributed after | **+0.7% and +1.4%** for the two formulations tried. The re-attribution explains why the share looks so tempting: `slice::index::get` (6.0%) plus `unlikely` (5.1%) is ~11% of a mesh decode, and nearly all of it is one function -- `try_decode_symbol` performs **five** fallible operations per symbol (`lut.get?`, `probability_table.get?`, `checked_mul?`, `checked_sub?`, `checked_add?`). That is the design, not an oversight: every index there is bitstream-controlled, and `unsafe_code = "forbid"` makes the check non-negotiable. Its price on a comparable path was already quantified at 2.0% (`432344b`). |
| Skipping the zero-fill on an attribute buffer that is about to be overwritten whole | `data_buffer.rs` + `sequential_integer_attribute_decoder.rs`, tried 2026-08-14 | **Nothing.** `try_resize` zeroes `required` bytes that the bulk copy replaces immediately, and memset was 3.7% of the profile, so a `try_replace_with` that reserves and `extend_from_slice`s looked free. One build per side read -0.6%; two builds per side put all four inside 0.3% with the conditions interleaved. Both passes are memory-bandwidth bound on pages the copy has to fault in anyway, so removing one of them buys back much less than its share suggests. Reverted. |
