# Fuzzing draco-rust

Draco decodes attacker-controllable byte streams, so the decode path is fuzzed
to confirm that malformed, truncated, or adversarial `.drc` input fails as a
controlled `DracoError` instead of panicking, hanging, or over-allocating.

This document is the operational routine for that fuzzing. The decode hardening
status, threat model, and known residual risk live in
[`hardening_status.yaml`](hardening_status.yaml) and [`SECURITY.md`](SECURITY.md).

## What is fuzzed

| Target | Path | Surface |
|---|---|---|
| `decode_drc` | [`fuzz/fuzz_targets/decode_drc.rs`](fuzz/fuzz_targets/decode_drc.rs) | Feeds each input through both `MeshDecoder` and `PointCloudDecoder`, including the legacy decode features used for old `.drc` streams. |
| `compress_gltf` | [`fuzz/fuzz_targets/compress_gltf.rs`](fuzz/fuzz_targets/compress_gltf.rs) | Feeds arbitrary glTF/GLB bytes into the document-preserving glTF compressor with external file resolution disabled. |
| `draco_gltf_import` | [`fuzz/fuzz_targets/draco_gltf_import.rs`](fuzz/fuzz_targets/draco_gltf_import.rs) | Imports a full scene through `draco-gltf`, decodes every Draco primitive, then exercises atomic in-place decompression. |
| `fbx_read_scene` | [`fuzz/fuzz_targets/fbx_read_scene.rs`](fuzz/fuzz_targets/fbx_read_scene.rs) | Reads arbitrary bytes as an FBX scene under tight decode limits, in both lenient and strict modes, and checks that reading the same input twice agrees. |
| `fbx_roundtrip` | [`fuzz/fuzz_targets/fbx_roundtrip.rs`](fuzz/fuzz_targets/fbx_roundtrip.rs) | Writes back whatever the FBX reader accepted and requires the result to satisfy the reader's strict mode, so the writer is fuzzed with scenes nobody would hand-build. |
| `ktx2_transcode` | [`fuzz/fuzz_targets/ktx2_transcode.rs`](fuzz/fuzz_targets/ktx2_transcode.rs) | Parses arbitrary bytes as KTX2 and transcodes every level into every target, for images small enough that an allocation failure is a finding rather than the header's own arithmetic. |

`decode_drc` builds `draco-core` with `default-features = false` and enables the
legacy decode features needed for old streams. The two glTF targets use
`draco-gltf`'s lossless document API, so malformed document parsing, decode,
compression, and atomic decompression all receive coverage.

The FBX targets cover `draco-io`, whose reader is the workspace's only
hand-rolled parser of untrusted binary: node records carry file-controlled
lengths that feed allocations directly, and array payloads pass through zlib
inflation. They run with [`FbxDecodeLimits::fuzzing()`], which is far tighter
than the shipped defaults. That is deliberate: with the shipped limits a header
may legitimately ask for hundreds of megabytes, `-rss_limit_mb` would fire on
it, and real findings would drown in that noise. Under the fuzzing limits any
allocation failure is a genuine bug.

[`FbxDecodeLimits::fuzzing()`]: crates/draco-io/src/fbx_options.rs

`ktx2_transcode` covers `draco-texture`, which is the same shape of surface: a
header of file-controlled offsets and lengths, a Zstd payload whose size the
header declares before it is decompressed, and block data indexed arithmetically
from the dimensions the file states. The reader's own limit is 16384 texels
either way, which is right for a texture and wrong for a campaign - a header
well inside it can legitimately ask for a gigabyte and `-rss_limit_mb` would
fire on that - so the target parses at any size and only decodes below a
million texels. Past that bound an allocation failure is a genuine bug.

It has a deterministic counterpart that needs no campaign to run:
[`crates/draco-texture/tests/ktx2_malformed.rs`](crates/draco-texture/tests/ktx2_malformed.rs)
sweeps every header and level-index field at eight extreme values, every
truncation of every fixture, and payloads overwritten wholesale, on every push.
The two divide the work the way they should - the sweep covers the fields
someone would actually reach for, in under three seconds; the campaign looks
for what nobody thought to sweep.

### `-O`: fuzz with production (release) semantics

Pass `-O` to `cargo fuzz` to build in release mode **without** debug assertions
and overflow checks. This matters: by default cargo-fuzz enables both (its build
is "release + debug-assertions + overflow-checks"), and a Cargo profile setting
does not override that — only `-O` does.

Use `-O` for the CI gate and any run whose job is to find *shipped-behavior*
hazards. The decoder intentionally relies on two's-complement wrapping (matching
C++ Draco) and keeps `debug_assert!` / `DRACO_DCHECK`-equivalent invariants that
are compiled out in release. Without `-O`, the fuzzer trips on those dev-time
checks and benign intentional-overflow wraps instead of real hostile-input
hazards. With `-O` the memory-safety coverage that matters is intact —
out-of-bounds indexing still panics in release Rust and is still caught, as are
OOMs, timeouts, and unbounded loops.

Omit `-O` only for **development** deep-fuzzing, where catching debug-assertion
invariant violations and intentional-overflow wraps is the goal; treat those
findings as debug-only (they do not reproduce in the shipped release build).

## Prerequisites

```powershell
rustup toolchain install nightly
cargo install cargo-fuzz
```

`cargo-fuzz` requires a nightly toolchain because it relies on libFuzzer and
sanitizer instrumentation. On Linux/macOS AddressSanitizer is the default and
recommended.

On Windows MSVC the AddressSanitizer runtime DLL is frequently not on `PATH`,
which makes the target fail at startup with `STATUS_DLL_NOT_FOUND`
(`0xc0000135`). Put the runtime that ships with Visual Studio on `PATH` for the
session — adjust the MSVC version to match your install:

```powershell
$env:PATH = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64;$env:PATH"
cargo +nightly fuzz run -O decode_drc --fuzz-dir fuzz -- -max_total_time=120 -rss_limit_mb=4096
```

`--sanitizer none` is **not** a workaround on MSVC: libFuzzer's coverage
counters (`__start___sancov_cntrs` and friends) come from the sanitizer
runtime, so dropping it fails at link time with unresolved externals.

## Seeding the corpus

The corpus directory (`fuzz/corpus/`) is git-ignored, so reconstruct it from the
committed fixtures before the first run. The seed script copies every `*.drc`
file for `decode_drc`; the glTF targets receive all repository `*.gltf`/`*.glb`
fixtures plus target-specific self-contained seeds under `fuzz/seeds/`:

```powershell
pwsh fuzz/seed_corpus.ps1
pwsh fuzz/seed_corpus.ps1 -Target compress_gltf
pwsh fuzz/seed_corpus.ps1 -Target draco_gltf_import
pwsh fuzz/seed_corpus.ps1 -Target fbx_read_scene
pwsh fuzz/seed_corpus.ps1 -Target fbx_roundtrip
```

```bash
./fuzz/seed_corpus.sh
./fuzz/seed_corpus.sh compress_gltf
./fuzz/seed_corpus.sh draco_gltf_import
./fuzz/seed_corpus.sh fbx_read_scene
./fuzz/seed_corpus.sh fbx_roundtrip
```

No `.fbx` fixtures are committed under `testdata/`, so the FBX targets are
seeded entirely from `fuzz/seeds/<target>/`. Those files are generated rather
than hand-edited — each malformed one encodes a single hazard so a crash
minimises to an obvious cause:

```bash
cargo run --manifest-path crates/Cargo.toml --example fbx_make_seeds -- fuzz/seeds
```

Seeding from the real fixture inventory (point clouds, sequential/EdgeBreaker
meshes, legacy 0.9.1/1.0.0/1.1.0 streams, KD-tree streams) gives the fuzzer good
coverage immediately instead of rediscovering the container format from scratch.
The legacy fixtures are especially important because they exercise pre-2.2
connectivity layouts, legacy valence/predictive traversal, and old normal
octahedron transform data.

## Running

This repository keeps the workspace manifest under `crates/` and has no
`Cargo.toml` at the root, so every `cargo fuzz` invocation must point at the
fuzz project with `--fuzz-dir fuzz`.

```powershell
# Bounded decode smoke run (CI / pre-release gate)
cargo +nightly fuzz run -O decode_drc --fuzz-dir fuzz -- -max_total_time=120 -rss_limit_mb=4096

# Bounded glTF compressor smoke run
cargo +nightly fuzz run -O compress_gltf --fuzz-dir fuzz -- -max_total_time=120 -rss_limit_mb=4096

# Full-scene import, Draco decode, and decompression smoke run
cargo +nightly fuzz run -O draco_gltf_import --fuzz-dir fuzz -- -max_total_time=120 -rss_limit_mb=4096

# FBX container read, lenient and strict
cargo +nightly fuzz run -O fbx_read_scene --fuzz-dir fuzz -- -max_total_time=120 -rss_limit_mb=4096

# FBX read/write/read round-trip
cargo +nightly fuzz run -O fbx_roundtrip --fuzz-dir fuzz -- -max_total_time=120 -rss_limit_mb=4096

# Longer decode soak run
cargo +nightly fuzz run -O decode_drc --fuzz-dir fuzz -- -max_total_time=3600 -rss_limit_mb=4096
```

Useful libFuzzer flags (everything after `--` is passed straight to libFuzzer):

- `-max_total_time=<seconds>` — wall-clock budget.
- `-rss_limit_mb=<mb>` — abort on a single input that exceeds this RSS; catches
  unbounded-allocation regressions.
- `-max_len=<bytes>` — cap generated input size (the real fixtures stay larger).
- `-timeout=<seconds>` — flag a single input that decodes too slowly; catches
  CPU-amplification / pathological-complexity inputs as `slow-unit-*` artifacts.
- `-jobs=<n> -workers=<n>` — parallel fuzzing.
- `-print_final_stats=1` — coverage / corpus summary on exit.

## Minimizing the corpus

Shrink the corpus to the smallest set that preserves coverage before committing
a refreshed seed set or before a long soak:

```powershell
cargo +nightly fuzz cmin -O decode_drc --fuzz-dir fuzz
cargo +nightly fuzz cmin -O compress_gltf --fuzz-dir fuzz
cargo +nightly fuzz cmin -O draco_gltf_import --fuzz-dir fuzz
cargo +nightly fuzz cmin -O fbx_read_scene --fuzz-dir fuzz
cargo +nightly fuzz cmin -O fbx_roundtrip --fuzz-dir fuzz
```

## Reproducing and triaging a crash

A failing input is written to `fuzz/artifacts/<target>/`. Re-run it
deterministically and minimize it by substituting the affected target below:

```powershell
$Target = "draco_gltf_import" # or decode_drc, compress_gltf, fbx_read_scene, fbx_roundtrip
$Crash = "fuzz/artifacts/$Target/crash-<hash>"

# Replay a specific crashing input (use -O to match the CI gate's build)
cargo +nightly fuzz run -O $Target --fuzz-dir fuzz $Crash

# Minimize the crashing input to the smallest reproducer
cargo +nightly fuzz tmin -O $Target --fuzz-dir fuzz $Crash
```

When a crash is confirmed, keep the minimized input under
`fuzz/seeds/<target>/` and add a deterministic regression to the crate that owns
the affected surface. Decoder regressions belong in
[`crates/draco-core/tests/drc_edge_cases_test.rs`](crates/draco-core/tests/drc_edge_cases_test.rs)
(see the `*_do_not_panic` tests); glTF compressor/import regressions belong in
the corresponding `draco-io` or `draco-gltf` test suite. FBX regressions belong
in the `fbx_reader` / `fbx_writer` unit tests — see
`a_short_footer_is_refused_instead_of_panicking`, which came straight from
`fbx_read_scene`. This keeps every case covered on stable CI without requiring
the fuzzing toolchain.

## CI

Fuzzing runs in CI on Linux (`ubuntu-latest`), where libFuzzer and
AddressSanitizer work out of the box — no Windows sanitizer-runtime workaround
needed. Two layers run there:

There are **no scheduled (nightly) fuzzing runs** — fuzzing fires only when code
changes, plus deeper runs on demand. Trigger a manual run with
`gh workflow run <workflow>.yml` (or the Actions tab).

**Level 1 — lightweight in-repo gate ([`.github/workflows/fuzz.yml`](.github/workflows/fuzz.yml)):**

- Bounded smoke runs (`-max_total_time=120`) for `decode_drc`,
  `compress_gltf`, `draco_gltf_import`, `fbx_read_scene` and `fbx_roundtrip`
  on every pull request and push to `main`. A manual dispatch runs longer soaks (`-max_total_time=1800`).
- The corpus is persisted across runs via the GitHub Actions cache and
  re-seeded from the committed fixtures each run, so coverage never starts from
  zero even if the cache entry is evicted.
- A crash fails the job and uploads the reproducer as a build artifact.

**Level 2 — ClusterFuzzLite continuous fuzzing
([`.github/workflows/cflite_*.yml`](.github/workflows), [`.clusterfuzzlite/`](.clusterfuzzlite)):**

- `cflite_pr.yml` fuzzes only the code changed in a pull request.
- `cflite_batch.yml` runs a batch campaign (manual dispatch).
- `cflite_cron.yml` prunes the corpus (manual dispatch).
- Build integration lives in `.clusterfuzzlite/` (OSS-Fuzz Rust base image +
  `build.sh` that calls `cargo fuzz build -O --fuzz-dir fuzz` and exports all
  fuzz binaries together with a fixture-backed seed corpus for each target);
  corpus and crashes are stored in the GitHub Actions cache by default. This is
  also the stepping stone to full OSS-Fuzz onboarding.

Stable CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) additionally
runs the deterministic malformed-input regressions in `drc_edge_cases_test.rs`
on every change with no nightly toolchain needed.

Any new crash must be fixed and then pinned as a deterministic regression test
before the hostile-input readiness in `hardening_status.yaml` is upgraded.
