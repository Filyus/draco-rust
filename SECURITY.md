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
- Bitstream-controlled counts (face counts, point counts, attribute counts) are
  checked against what the stream could plausibly describe before large
  allocations, and the allocations themselves are fallible. The check is a
  ratio against the input size, not a cap on geometry: a large mesh scales its
  own budget with it.
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

The rule is per crate, because the three crates do different work and a single
rule for all of them was either too strict for two or too loose for one. Every
line of the table is enforced by the build rather than asserted here: CI runs
`cargo clippy --workspace --all-targets --all-features -- -D warnings`.

| crate | rule | enforced by |
|---|---|---|
| `draco-core` | **No `unsafe`.** | `[lints.rust] unsafe_code = "forbid"` — a build failure, and `forbid` so no module can lift it locally |
| `draco-io` | `unsafe` **permitted in narrow, audited paths**, each block carrying a `// SAFETY:` justification | `[lints.clippy] undocumented_unsafe_blocks` — an unjustified block fails the build |
| `draco-gltf` | the same as `draco-io` | the same lint |

**Why `draco-core` is the strict one.** Its algorithms — rANS, edgebreaker
traversal, the prediction schemes, the KD-tree coder — are intricate, they run
on bitstream-controlled indices throughout, and a memory-safety bug inside one
of them would be hard to find by review and hard to attribute from a crash.
Whatever `unsafe` could buy there is not worth being unable to reason about the
result, so the door is shut and the compiler holds it.

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
  policy exists to prevent.

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
  bits, so a read count that overshoots yields deterministic garbage rather than
  an error. The bound has to be structural at each call site — the crease-edge
  flags against the corner count, the texture-coordinate orientations against
  the entry count — and those bounds have not all been audited. This is a
  wrong-geometry rather than a memory-safety concern: the loops are driven by
  counts that have their own guards. Tracked in `hardening_status.yaml` as
  `rans-over-read-call-site-bounds`.

## Reporting a vulnerability

This is not an official Google Draco release. Report suspected decode panics,
out-of-bounds reads, or unbounded-allocation issues through the project's issue
tracker with a minimized reproducer (`cargo fuzz tmin` output is ideal).
