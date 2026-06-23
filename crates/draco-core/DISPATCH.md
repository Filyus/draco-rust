# Dispatch & Polymorphism: draco-core vs C++ Draco

This note explains a recurring stylistic difference between upstream C++ Draco
and this Rust port, so that anyone reading both side by side understands why the
Rust code looks more verbose in places — and where that verbosity actually buys
something. Every snippet below is real code from both trees.

## TL;DR

C++ Draco leans on **runtime polymorphism** in many places: a single
`std::unique_ptr<SomeInterface>` field, a `Create…` factory that picks the
concrete type, and virtual calls at the use site. `draco-core` replaces most of
that with **compile-time dispatch**: generics + traits, `enum` + `match`, or
`Option<ConcreteType>`.

| | C++ (`unique_ptr<Interface>` + factory) | draco-core (generics / enum / `Option<Concrete>`) |
|---|---|---|
| Dispatch | virtual call (vtable) | monomorphized / matched — inlined |
| Allocation | heap (`new` / `unique_ptr`) | usually none |
| Lifetimes | raw pointers, unchecked | borrow-checked |
| Code | terser | more verbose |
| Extensibility | open (add a subclass) | closed (touch the `enum`/`match`) |

Neither is universally "better". C++'s shape is more elegant where Rust's borrow
checker gets in the way; the Rust shape is faster and safer where it applies.

---

## 1. Attribute decoder selection

The cleanest illustration. C++ stores decoders behind a base class and builds
them with a factory `switch`:

```cpp
// sequential_attribute_decoders_controller.{h,cc}
std::vector<std::unique_ptr<SequentialAttributeDecoder>> sequential_decoders_;

sequential_decoders_[i] = CreateSequentialDecoder(decoder_type);   // factory
sequential_decoders_[i]->Init(GetDecoder(), GetAttributeId(i));
// ... later: sequential_decoders_[i]->DecodeValues(...);   // virtual

std::unique_ptr<SequentialAttributeDecoder>
CreateSequentialDecoder(uint8_t decoder_type) {
  switch (decoder_type) {
    case SEQUENTIAL_ATTRIBUTE_ENCODER_GENERIC:
      return std::unique_ptr<...>(new SequentialAttributeDecoder());
    case SEQUENTIAL_ATTRIBUTE_ENCODER_INTEGER:
      return std::unique_ptr<...>(new SequentialIntegerAttributeDecoder());
    case SEQUENTIAL_ATTRIBUTE_ENCODER_QUANTIZATION:
      return std::unique_ptr<...>(new SequentialQuantizationAttributeDecoder());
    case SEQUENTIAL_ATTRIBUTE_ENCODER_NORMALS:
      return std::unique_ptr<...>(new SequentialNormalAttributeDecoder());
  }
}
```

draco-core matches on the same byte but constructs the concrete type inline and
calls it directly — no base class, no `unique_ptr`, no virtual:

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

Cost of the Rust form: the `match` is duplicated in the two callers
(point-cloud vs mesh) because their `decode_values` arguments differ
(mesh passes a corner table + maps; point-cloud passes `None`). That is a
genuine downside — but it is *parallel code*, not the copy-paste a single
factory would remove, since the argument lists are not the same.

---

## 2. Prediction scheme — the forced case

C++ holds **one** polymorphic scheme and a factory:

```cpp
// sequential_integer_attribute_decoder.{h,cc}
std::unique_ptr<PredictionSchemeTypedDecoderInterface<int32_t>> prediction_scheme_;

prediction_scheme_ = CreateIntPredictionScheme(method, transform_type);   // factory
...
prediction_scheme_->DecodePredictionData(in_buffer);     // virtual, no switch
prediction_scheme_->ComputeOriginalValues(...);          // virtual, no switch
```

draco-core cannot do this for the *locally built* predictors. It keeps one
boxed scheme **only** for the externally-set case (which is `'static`), and one
`Option<ConcreteType>` per scheme otherwise:

```rust
// only the externally-supplied scheme can be type-erased (it is 'static)
prediction_scheme: Option<Box<dyn PredictionSchemeDecoder<'static, i32, i32>>>,

// locally built predictors cannot be erased -> one Option per concrete type (x8)
let mut predictor_parallelogram_opt:
    Option<MeshPredictionSchemeParallelogramDecoder<i32, i32, PredictionSchemeWrapDecodingTransform<i32>>> = None;
// ... 7 more ...

// apply phase: pick the right Option, a generic helper removes the repeated
// extract/call/log boilerplate (see commit "dedup predictor dispatch")
match selected_method {
    PredictionSchemeMethod::MeshPredictionParallelogram => {
        if !run_decode_prediction_data(predictor_parallelogram_opt.as_mut(), in_buffer) {
            return false;
        }
    }
    // ...
}

fn run_decode_prediction_data<'a, P: PredictionSchemeDecoder<'a, i32, i32> + ?Sized>(
    predictor: Option<&mut P>,
    buffer: &mut DecoderBuffer,
) -> bool { /* extract Option, call, log + fail */ }
```

### Why it is forced, not lazy

It comes down to **how `mesh_data` (corner table + traversal maps) is stored**:

- **C++:** `MeshPredictionSchemeData` holds **raw `const` pointers** to the
  corner table and maps. Lifetimes are not checked — it is assumed the data
  (owned by the decoder) outlives the scheme. So one `unique_ptr<Interface>`
  works: type erasure with no lifetime obligations.
- **draco-core:** the locally built predictors **borrow** the
  `vertex_to_data_map` / `data_to_corner_map` constructed right inside
  `decode_values`. `Box<dyn PredictionSchemeDecoder + 'static>` requires
  `'static`, so the type cannot be erased — hence the per-scheme `Option`s.

The "nice" C++ abstraction rests precisely on a raw pointer with no lifetime
guarantee — exactly what Rust refuses to express implicitly.

### Reproducing the C++ shape in Rust safely

Two options, both with a cost:

1. **Make each predictor own its `mesh_data`** (move/clone the maps in instead
   of borrowing). Then `Box<dyn … + 'static>` is valid again → one boxed scheme
   + a factory, just like C++. **Cost:** a copy of the maps per attribute — an
   extra allocation on a warm path.
2. **Store the maps in `self`** (the way C++ keeps them on the decoder). But the
   lifetime becomes `'self`, not `'static`, which is the same obstacle for a
   boxed field.

The current design is the deliberate trade: faster and borrow-safe, more code.

---

## 3. EdgeBreaker traversal — where both monomorphize

It is tempting to call this a Rust win, but the honest comparison is closer.
**Both** trees monomorphize the hot per-symbol path.

C++ parameterizes the impl over the traversal type (a by-value template field,
so `DecodeSymbol()` is **not** virtual — it inlines):

```cpp
template <class TraversalDecoderT>
class MeshEdgebreakerDecoderImpl : public MeshEdgebreakerDecoderImplInterface {
  TraversalDecoderT traversal_decoder_;            // by value -> monomorphized
  // ... const uint32_t symbol = traversal_decoder_.DecodeSymbol();   // not virtual
};

// but the decoder stores the impl behind an interface and dispatches once:
std::unique_ptr<MeshEdgebreakerDecoderImplInterface> impl_;
impl_ = ...(new MeshEdgebreakerDecoderImpl<MeshEdgebreakerTraversalDecoder>());
impl_->DecodeConnectivity();                        // ONE virtual call per mesh
```

draco-core threads the traversal as a generic type parameter and skips the outer
interface layer entirely:

```rust
pub trait EdgebreakerTraversalDecoder {
    fn decode_symbol(&mut self) -> Result<u32, String>;
    // ...
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

So the per-symbol loop is inlined in **both**. The only difference: C++ wraps the
whole impl in `MeshEdgebreakerDecoderImplInterface` (one virtual indirection per
mesh, so the decoder can hold `unique_ptr<Interface> impl_` and select the
traversal at runtime), while draco-core selects the traversal with a `match` and
calls the right monomorphization directly — one fewer layer, but the same hot
path. Not a speed story, just less indirection.

---

## Where C++ uses this pattern, and what draco-core does instead

| Site | C++ | draco-core | Note |
|---|---|---|---|
| Attribute decoder | `CreateSequentialDecoder` → `unique_ptr<base>` + virtual | `match decoder_type` + concrete types | Rust form duplicated across mesh/PC callers (different args) |
| Prediction scheme | `unique_ptr<…TypedDecoderInterface>` + factory | 8 `Option<Concrete>` + `match` (+1 `Box<dyn 'static>` external) | **forced by the borrow checker** |
| EdgeBreaker traversal | `MeshEdgebreakerDecoderImpl<TraversalT>` behind `…ImplInterface` | `decode_connectivity<T: …>` | both monomorphize; Rust drops one interface layer |
| PointsSequencer | `unique_ptr<PointsSequencer> sequencer_` | inlined; no trait-object layer | — |
| AttributesDecoder | `unique_ptr<AttributesDecoderInterface>` | concrete types | — |
| Prediction transforms | interface / template | concrete + `enum` tag (`TransformType`) | monomorphized |

In the whole crate, `Box<dyn …>` survives in exactly **one** field: the
externally-settable `prediction_scheme` (and even there it is `'static`).

## Rule of thumb in this codebase

- Prefer **generics + traits** when the implementation set is closed and the
  call is hot (traversal, transforms).
- Use **`enum` + `match`** for closed, data-driven dispatch (method / transform /
  decoder-type selection).
- Reach for **`Box<dyn Trait>`** only when the implementation is genuinely
  open/external *and* `'static` (the one prediction-scheme field).
- Accept **`Option<ConcreteType>` fan-out** when type erasure is blocked by a
  borrow (predictors borrowing local data) — and leave a comment saying why, so
  it is not "cleaned up" into something that will not compile.
