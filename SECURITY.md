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

- No `unsafe` in `draco-core` or `draco-io` source.
- Malformed, truncated, and byte-mutated streams over the supported fixture set
  return a `DracoError` instead of panicking (covered by
  `crates/draco-core/tests/drc_edge_cases_test.rs`).
- Bitstream-controlled counts (face counts, point counts, attribute counts) are
  range-checked before large allocations.
- Entropy, prediction, transform, and KD-tree decode paths use checked indexing
  and fallible buffer access on the audited paths.

What the crate intentionally does **not** do:

- It does not impose artificial mesh-size caps. Draco is designed for large
  geometry, so a hard internal cap would break legitimate inputs. Bounding
  input cost is therefore a **caller** responsibility — see below.
- It does not provide its own timeout, sandbox, or memory accounting.

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
- Some low-level decode helpers still return `bool` rather than a structured
  `DracoError`; this loses failure context but does not bypass the bounds
  checks. Tracked in `hardening_status.yaml`.

## Reporting a vulnerability

This is not an official Google Draco release. Report suspected decode panics,
out-of-bounds reads, or unbounded-allocation issues through the project's issue
tracker with a minimized reproducer (`cargo fuzz tmin` output is ideal).
