# Security and resource policy

`draco-core` decodes Draco `.drc` byte streams. In most deployments those bytes
arrive from outside the trust boundary (uploaded assets, third-party glTF/GLB
files, networked content), so this document describes the decode threat model
and the operational limits a caller should enforce.

It complements [`hardening_status.yaml`](hardening_status.yaml) (current
hardening evidence and residual risk) and [`FUZZING.md`](FUZZING.md) (how the
decode path is fuzzed).

## Threat model

| Asset class | Description | Decode posture |
|---|---|---|
| Controlled assets | Known-good streams from a trusted pipeline or fixtures. | Strong positive-path parity with C++ Draco. |
| Untrusted assets | Externally supplied `.drc` bytes that may be truncated, corrupted, inconsistent, oversized, or adversarial. | Decode must fail as a `DracoError`, not panic, hang, or over-allocate. |

What the crate guarantees today:

- `draco-core` contains no `unsafe`, and the compiler enforces it — see
  [Memory safety](#memory-safety-unsafe) for what that means per crate.
- Malformed, truncated, and byte-mutated streams over the supported fixture set
  return a `DracoError` instead of panicking (covered by
  `crates/draco-core/tests/drc_edge_cases_test.rs`).
- A bitstream-controlled count — faces, points, attribute values, symbols —
  does not allocate. It is the ceiling a decode works towards; the buffers grow
  as the data arrives, or are sized after the step that produced it. A header
  naming a billion elements therefore costs one small reservation and an error,
  not a reservation for a billion. A ratio against the input size remains
  behind that as a backstop, and every allocation on these paths is fallible.
  See [Bounding what a decode allocates](#bounding-what-a-decode-allocates).
- Entropy, prediction, transform, and KD-tree decode paths use checked indexing
  and fallible buffer access on the audited paths.
- A decode refusal names what it refused. The attribute transforms, the
  prediction schemes and the sequential attribute coders return `DracoError`
  rather than a bare `bool`, and the reason reaches `MeshDecoder::decode` from
  the layer that found it.

What the crate intentionally does **not** do:

- It does not impose artificial mesh-size caps. Draco is designed for large
  geometry, so a hard internal cap would break legitimate inputs. Bounding
  input cost is therefore a **caller** responsibility — see below.
- It does not provide its own timeout, sandbox, or memory accounting.

## Memory safety (`unsafe`)

The rule is per crate, because the crates do different work and a single rule
for all of them was either too strict for some or too loose for others. Which
rule a crate is under is decided by the build, not by this page: CI runs
`cargo clippy --workspace --all-targets --all-features -- -D warnings`.

| crate | rule | enforced by |
|---|---|---|
| `draco-core` | **No `unsafe`.** | `[lints.rust] unsafe_code = "forbid"` — a build failure, and `forbid` so no module can lift it locally |
| `draco-texture` | **No `unsafe`.** | the same lint |
| `draco-io` | `unsafe` **permitted in narrow, audited paths**, each block carrying a `// SAFETY:` justification | `[lints.clippy] undocumented_unsafe_blocks` — a block with no comment at all fails the build |
| `draco-gltf` | the same as `draco-io` | the same lint |

`draco-cpp-test-bridge` is outside the table: it exists to run the C++ library
next to this one and is not shipped in any form. So are the `web/*-wasm`
wrappers — none of them writes `unsafe`, but `#[wasm_bindgen]` expands to some,
and `forbid` rejects what a macro generates as readily as what a human types.
The rule belongs on the crates that hold the decoding, which is where a bug
would be.

The two halves are not enforced to the same depth, and it is worth being plain
about it. `unsafe_code = "forbid"` checks the property itself — there is no way
to have `unsafe` and a green build. `undocumented_unsafe_blocks` checks that a
comment is *present*, not that it is true, and nothing machine-checks the
requirements listed below. On the permissive side the build catches an omission
and review catches everything else.

**Why `draco-core` is strict.** Its algorithms — rANS, edgebreaker
traversal, the prediction schemes, the KD-tree coder — are intricate, they run
on bitstream-controlled indices throughout, and a memory-safety bug inside one
of them would be hard to find by review and hard to attribute from a crash.
Whatever `unsafe` could buy there is not worth being unable to reason about the
result, so the door is shut and the compiler holds it.

**`draco-texture` is strict for the same reason**, and it is in the table
despite `publish = false`. That flag says where the crate is distributed, not
what it is exposed to: it ships inside `ktx2-wasm` in the WASM release assets
and transcodes whatever KTX2 the converter is handed. Its work is the same
shape as `draco-core`'s — table-driven block decoding on indices that come out
of the stream — so it belongs on the same side, and since it contains no
`unsafe` today the rule costs nothing to adopt and fixes what already holds.

**Why the other two are not.** They are the file-format and document layers, and
the work in their hot paths is a different shape: byte shuffling, endian swaps,
fixed-stride copies between typed buffers, accessor strides and buffer views,
and container parsing whose bounds are established at the point of use rather
than carried through an algorithm. That is exactly the shape SIMD and unchecked
indexing help with, and each such path is small enough to audit in isolation. It
is a different risk profile, not a lower standard.

The split is by *what the code does*, not by how much it is trusted. A
`draco-gltf` accessor walk reads offsets and strides straight out of a `.gltf`
that a hostile caller wrote, so nothing here says its input is safer than
`draco-core`'s — it says the bound on each read is established a line or two
away rather than carried through a decoder's state, which is what makes a
`// SAFETY:` comment able to say something true and checkable.

**What is required of an `unsafe` block here**, and it is all of it, not a
choice:

- a `// SAFETY:` comment naming the invariant and *where it was established* —
  a precondition checked three functions away is not a justification, a
  precondition checked on the line above is;
- the invariant established by the code, not by the format. A file is untrusted
  input; "the header says the array is this long" is not a precondition, it is
  an attacker-controlled value that has to be checked first;
- an entry in the table below, so an audit reads one list rather than grepping;
- coverage by the fuzz targets in [`FUZZING.md`](FUZZING.md). A path that
  `unsafe` made faster and that nothing fuzzes is the one combination this
  policy exists to prevent;
- a measurement of the safe version it replaces, stated with the block. The
  permission exists so that a demonstrated win does not have to reargue the
  policy — which means the win has to be demonstrated. `chunks_exact`,
  `from_le_bytes` and `copy_from_slice` compile to the same instructions
  often enough that "this is the fast way" is a hypothesis until a benchmark
  says otherwise.

**Paths using `unsafe` in the `draco-io` and `draco-gltf` libraries today:
none.** The permission is in place so that a measured optimisation does not have
to relitigate the policy to land; it is not an invitation to reach for it first.
Safe Rust that measures the same wins. When the first block lands, it is listed
here.

One *test* uses it, and it is listed because a policy whose table is
selectively complete is worth nothing:

| path | what | why it is sound |
|---|---|---|
| `crates/draco-io/tests/reader_hardening_test.rs` | a counting `GlobalAlloc`, so a test can assert on what a reader *reserves* rather than on how long it takes to fail | every method forwards to `System` with the layout and pointer unchanged; the only addition is an atomic counter that allocates nothing |

## Bounding what a decode allocates

A count in the bitstream is a ceiling to check against, never a size to
allocate. Two shapes satisfy that, and every decode path here uses one of them:

- **Grow.** The buffer starts at what the input could plausibly back and
  extends as output is produced, so a count nothing backs costs one small
  reservation and then an error when the coder runs out. `decode_symbols`
  appends into the caller's `Vec` starting at eight symbols per remaining input
  byte; `DynamicIntegerPointsKdTreeDecoder` reserves on the same allowance;
  `EdgebreakerConnectivityDecoder` and both EdgeBreaker traversals grow as
  faces and vertices are decoded; `draco-texture` grows an ETC1S image one
  block row at a time.
- **Size after the data exists.** Where a consumer writes at computed offsets
  and needs the buffer whole, the allocation follows the step that produced or
  verified the bytes. A sequential attribute's buffer is left unreserved by
  `PointAttribute::init_deferred` and sized by whichever decoder writes it; the
  KD-tree sizes its attributes once the decoded array's length has been checked
  against the count; dequantization and the inverse octahedral transform size
  their target against the source they are reading; raw attribute values size
  theirs from a stream that has to carry them literally.

Two things this deliberately does *not* rely on:

- **A ratio is not a bound.** It limits bytes allocated per byte read, and the
  input is the attacker's to choose, so any constant is walked past by a larger
  file. `decode_budget` keeps one as a backstop; it is not what makes these
  paths safe.
- **A per-element floor holds only where it is measured.** One bit per symbol
  is a floor for a *count* — `ensure_symbols_are_backed` uses it for
  connectivity — and it is false for entropy-coded values, and false for ETC1S
  blocks, where the fixtures reach 0.98 bits per block.

Allocations on these paths are fallible. `vec![0; n]` and
`Vec::with_capacity(n)` abort through `handle_alloc_error`, which in the WASM
modules would take the page, so a size that can come from a file goes through
`try_reserve` and reports the module's own error instead. That holds even where
a large allocation is a deliberate ceiling: `draco-texture` keeps the reference
reader's 16384-texel dimension limit, and reaching it is an error.

The property is measured rather than asserted. Two fuzz artifacts that reserved
13 GB from 26 KB and 8 GB from 9 KB now allocate 26,386 and 9,034 bytes — the
size of their own input. Across the 2,367-file `decode_drc` corpus the largest
single allocation is 8.5 MB, down from 31.4 MB, with every file decoding to an
identical verdict.

## Recommended caller limits for untrusted input

Decoding hostile input safely is a shared responsibility. The decoder avoids
overflow and unbounded allocation on audited paths; the caller should still
bound the work it is willing to do.

| Control | Recommendation | Rationale |
|---|---|---|
| Input byte size | Reject streams larger than your application maximum **before** decoding. | A small compressed stream can describe a much larger mesh; capping input bytes is the cheapest first gate. |
| Output / memory budget | Run decode where a memory ceiling is enforceable (container limit, cgroup, or a dedicated allocator/arena). | The decoder does not cap reconstructed geometry size by design. |
| Timeout / cancellation | Decode untrusted input on a worker with a wall-clock timeout and drop it on expiry. | Guards against pathological inputs that are slow rather than large. |
| Process / worker isolation | Decode untrusted input off the main thread, ideally in a separate process or WASM instance. | Contains any residual unforeseen failure without taking down the host. |
| Feature surface | Build untrusted-decode profiles with `default-features = false` and only `decoder` (+ `point_cloud_decode`). | Drops legacy bitstream and deprecated prediction decode paths that hostile callers do not need. |

A WASM instance is a natural fit for several of these controls at once (memory
ceiling + isolation) and is a supported target for this crate.

## Residual risk

- Sustained fuzzing is operationalized (see [`FUZZING.md`](FUZZING.md)) but
  hostile-input confidence depends on running it regularly with a persisted
  corpus; arbitrary-hostile-input safety is improving, not yet claimed absolute.
- An rANS bit decoder cannot tell an encoded zero from a read past the encoded
  bits, so a read count that overshoots yields deterministic garbage rather
  than an error. The bound has to be structural at each call site, and every
  call site has been audited: the crease-edge flags and the deprecated
  texture-coordinate orientations against the corner count, the portable
  orientations and the geometric-normal flip bits against the entry count,
  the EdgeBreaker seam bits against the decoded face count, and the start-face
  and predictive-traversal bits against the symbol count. This is a
  wrong-geometry rather than a memory-safety concern: the loops are driven by
  counts that have their own guards. The audit is recorded in
  `hardening_status.yaml` under `rans-over-read-call-site-bounds`.
- A `draco-texture` level at the 16384-texel dimension limit legitimately costs
  a gigabyte, and the ETC1S block table for a level with alpha costs 134 MB.
  Both are bounded, fallible, and reached only as far as the blocks that
  actually decoded — but a caller handing this untrusted files should still put
  a memory ceiling around it, as the table below recommends for geometry.

## Reporting a vulnerability

This is not an official Google Draco release. Report suspected decode panics,
out-of-bounds reads, or unbounded-allocation issues through the project's issue
tracker with a minimized reproducer (`cargo fuzz tmin` output is ideal).
