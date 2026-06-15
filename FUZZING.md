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
| `decode_drc` | [`fuzz/fuzz_targets/decode_drc.rs`](fuzz/fuzz_targets/decode_drc.rs) | Feeds each input through both `MeshDecoder` and `PointCloudDecoder` with default-disabled legacy features. |

The fuzz crate builds `draco-core` with `default-features = false` and only the
`decoder` + `point_cloud_decode` features, matching the smallest realistic
untrusted-decode profile.

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
(`0xc0000135`). Because `draco-core` contains no `unsafe`, ASan adds little here
— out-of-bounds access surfaces as a normal Rust panic that libFuzzer already
catches as a crash. Run with the sanitizer disabled on Windows:

```powershell
cargo +nightly fuzz run -O decode_drc --fuzz-dir fuzz --sanitizer none -- -max_total_time=120 -rss_limit_mb=4096
```

(If you have the ASan runtime on `PATH`, drop `--sanitizer none` to keep it on.)

## Seeding the corpus

The corpus directory (`fuzz/corpus/`) is git-ignored, so reconstruct it from the
committed fixtures before the first run. The seed script copies every `*.drc`
file under `testdata/` into `fuzz/corpus/decode_drc/`:

```powershell
pwsh fuzz/seed_corpus.ps1
```

```bash
./fuzz/seed_corpus.sh
```

Seeding from the real fixture inventory (point clouds, sequential/EdgeBreaker
meshes, legacy 1.0.0/1.1.0 streams, KD-tree streams) gives the fuzzer good
coverage immediately instead of rediscovering the container format from scratch.

## Running

This repository keeps the workspace manifest under `crates/` and has no
`Cargo.toml` at the root, so every `cargo fuzz` invocation must point at the
fuzz project with `--fuzz-dir fuzz`.

```powershell
# Bounded smoke run (CI / pre-release gate)
cargo +nightly fuzz run -O decode_drc --fuzz-dir fuzz --sanitizer none -- -max_total_time=120 -rss_limit_mb=4096

# Longer soak run
cargo +nightly fuzz run -O decode_drc --fuzz-dir fuzz --sanitizer none -- -max_total_time=3600 -rss_limit_mb=4096
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
```

## Reproducing and triaging a crash

A failing input is written to `fuzz/artifacts/decode_drc/`. Re-run it
deterministically and minimize it:

```powershell
# Replay a specific crashing input (use -O to match the CI gate's build)
cargo +nightly fuzz run -O decode_drc --fuzz-dir fuzz fuzz/artifacts/decode_drc/crash-<hash>

# Minimize the crashing input to the smallest reproducer
cargo +nightly fuzz tmin -O decode_drc --fuzz-dir fuzz fuzz/artifacts/decode_drc/crash-<hash>
```

When a crash is confirmed, add the minimized reproducer as a deterministic
regression in
[`crates/draco-core/tests/drc_edge_cases_test.rs`](crates/draco-core/tests/drc_edge_cases_test.rs)
(see the `*_do_not_panic` tests) so the case is covered on stable in CI without
requiring the fuzzing toolchain.

## CI

Fuzzing runs in CI on Linux (`ubuntu-latest`), where libFuzzer and
AddressSanitizer work out of the box — no Windows sanitizer-runtime workaround
needed. Two layers run there:

**Level 1 — lightweight in-repo gate ([`.github/workflows/fuzz.yml`](.github/workflows/fuzz.yml)):**

- Bounded smoke run (`-max_total_time=120`) on every pull request and push to
  `main`, plus a nightly soak (`-max_total_time=1800`) on a schedule.
- The corpus is persisted across runs via the GitHub Actions cache and
  re-seeded from the committed fixtures each run, so coverage never starts from
  zero even if the cache entry is evicted.
- A crash fails the job and uploads the reproducer as a build artifact.

**Level 2 — ClusterFuzzLite continuous fuzzing
([`.github/workflows/cflite_*.yml`](.github/workflows), [`.clusterfuzzlite/`](.clusterfuzzlite)):**

- `cflite_pr.yml` fuzzes only the code changed in a pull request.
- `cflite_batch.yml` runs a nightly batch campaign.
- `cflite_cron.yml` prunes the corpus nightly.
- Build integration lives in `.clusterfuzzlite/` (OSS-Fuzz Rust base image +
  `build.sh` that calls `cargo fuzz build --fuzz-dir fuzz`); corpus and crashes
  are stored in the GitHub Actions cache by default. This is also the stepping
  stone to full OSS-Fuzz onboarding.

Stable CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) additionally
runs the deterministic malformed-input regressions in `drc_edge_cases_test.rs`
on every change with no nightly toolchain needed.

Any new crash must be fixed and then pinned as a deterministic regression test
before the hostile-input readiness in `hardening_status.yaml` is upgraded.
