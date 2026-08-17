# Performance

Decode and encode benchmarks, profiling tests, and the Rust-vs-C++ speed
snapshot. Use `--release` for timing runs; add `-- --nocapture` to see the
printed comparison output. Correctness and parity tests live in
[`TESTING.md`](TESTING.md).

## Speed Snapshot

Measured on the seeded and real-corpus profiles below, against the C++
reference build. Absolute microsecond figures are comparable only within a
single run -- they track machine state, while the C++ side is unchanged code
built on 2026-02-05. How much machine state matters is worth stating in
numbers: between the 2026-07-31 snapshot and this one the C++ side alone got
`25%` faster on seeded encode and `36%` faster on seeded decode, on the same
binary and the same seeded workload. Every speedup below is therefore lower
than the figures it replaces without anything having regressed -- what moved
is the denominator.

Method for this snapshot: the test binary launched directly with one logical
core per physical (`0x55`) and `High` priority, `4` interleaved runs of the
seeded sweep and `5` of the real corpus, each cell the median across runs of
the harness's own `avg [p10..p90]`. Run-to-run agreement on the seeded
speedup column is `1.2%` to `5.1%` depending on the speed, so treat a
difference under `5%` between two snapshots as not measured. Pinning is not
what moved the numbers: an unpinned run reproduces the C++ side to `0.6%`.

Seeded mesh sweep encode, `avg [p10..p90]` across `12` stratified samples:

Samples: `3` grid, `3` fan, `3` boundary ribbon, `3` torus.

Sampled points: avg `12,156`, p50 `9,664`, p10..p90 `[7,578..21,435]`.

Sampled faces: avg `18,641`, p50 `19,327`, p10..p90 `[15,151..21,487]`.

| Speed | C++ raw us | Rust raw us | C++ us/1k faces | Rust us/1k faces | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `23,994 [17,104..35,837]` | `9,759 [4,700..21,893]` | `1,278 [906..1,784]` | `542 [254..1,316]` | `3.33x [1.35..4.98]` |
| 1 | `23,334 [16,531..35,276]` | `9,483 [4,350..20,928]` | `1,242 [895..1,678]` | `525 [238..1,258]` | `3.31x [1.33..4.62]` |
| 2 | `19,504 [12,269..29,981]` | `7,296 [2,054..19,830]` | `1,040 [668..1,601]` | `409 [112..1,192]` | `4.84x [1.33..6.95]` |
| 3 | `19,588 [12,143..29,958]` | `7,267 [2,034..19,786]` | `1,044 [663..1,595]` | `408 [112..1,189]` | `4.90x [1.33..7.07]` |
| 4 | `19,447 [11,893..30,087]` | `7,233 [2,049..19,753]` | `1,035 [658..1,599]` | `406 [111..1,187]` | `4.92x [1.33..7.28]` |
| 5 | `19,092 [11,604..29,669]` | `6,982 [1,784..19,604]` | `1,016 [641..1,551]` | `392 [98..1,178]` | `5.38x [1.31..7.78]` |
| 6 | `19,067 [11,626..29,539]` | `6,951 [1,759..19,526]` | `1,015 [634..1,560]` | `391 [98..1,174]` | `5.34x [1.33..7.79]` |
| 7 | `19,125 [11,777..29,955]` | `6,950 [1,789..19,562]` | `1,018 [634..1,547]` | `391 [97..1,176]` | `5.39x [1.32..7.88]` |
| 8 | `18,927 [11,396..29,657]` | `6,837 [1,688..19,506]` | `1,008 [627..1,580]` | `385 [92..1,173]` | `5.63x [1.35..8.19]` |
| 9 | `18,972 [11,400..29,832]` | `6,852 [1,696..19,506]` | `1,010 [632..1,561]` | `386 [93..1,173]` | `5.57x [1.33..8.23]` |
| 10 | `777 [552..1,208]` | `621 [395..1,084]` | `41 [34..56]` | `32 [25..50]` | `1.30x [1.14..1.40]` |

Seeded mesh sweep decode, `avg [p10..p90]` across `12` stratified samples:

| Speed | C++ raw us | Rust raw us | C++ us/1k faces | Rust us/1k faces | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `32,463 [23,746..44,298]` | `1,598 [1,024..2,083]` | `1,717 [1,564..2,070]` | `84 [68..99]` | `20.75x [16.84..23.16]` |
| 1 | `32,277 [23,470..44,173]` | `1,514 [787..2,384]` | `1,706 [1,551..2,061]` | `78 [52..112]` | `23.07x [18.27..29.83]` |
| 2 | `31,801 [23,389..43,434]` | `1,274 [810..2,142]` | `1,681 [1,528..2,036]` | `66 [44..101]` | `27.18x [20.35..35.06]` |
| 3 | `31,782 [23,862..43,623]` | `1,205 [698..2,129]` | `1,682 [1,528..2,029]` | `62 [44..99]` | `29.21x [20.48..35.83]` |
| 4 | `31,824 [22,994..44,614]` | `1,226 [707..2,152]` | `1,680 [1,525..2,063]` | `63 [44..101]` | `28.87x [20.33..35.54]` |
| 5 | `31,510 [22,989..43,951]` | `1,111 [590..2,021]` | `1,667 [1,518..2,029]` | `57 [38..95]` | `32.42x [21.63..40.17]` |
| 6 | `31,488 [23,029..43,528]` | `1,101 [581..2,001]` | `1,664 [1,518..2,031]` | `56 [38..95]` | `32.55x [21.62..40.62]` |
| 7 | `31,637 [22,933..43,930]` | `1,102 [590..2,038]` | `1,672 [1,517..2,045]` | `56 [38..95]` | `32.72x [21.56..40.71]` |
| 8 | `31,254 [22,884..43,542]` | `1,015 [521..1,897]` | `1,652 [1,509..2,026]` | `52 [34..88]` | `35.41x [22.94..45.06]` |
| 9 | `31,328 [22,959..43,829]` | `1,017 [520..1,923]` | `1,655 [1,513..2,023]` | `52 [35..89]` | `35.31x [22.99..44.26]` |
| 10 | `295 [196..473]` | `223 [140..380]` | `15 [13..22]` | `12 [9..18]` | `1.35x [1.25..1.43]` |

Real `.drc` corpus normal-distribution sample:

Samples: `24` draws from `16` compatible real mesh fixtures, seed
`0x6a7573737265616c`.

Sampled points: avg `2,096`, p50 `97`, p10..p90 `[9..4,959]`, min..max
`[8..34,834]`.

Sampled faces: avg `4,001`, p50 `170`, p10..p90 `[9..8,525]`, min..max
`[8..69,451]`.

This sample is mostly small: half its meshes are under `170` faces, so it
measures per-call cost where the seeded sweep measures throughput. The two
answer different questions and disagree about the same change below.

Source `.drc` decode speedup: avg `10.51x`, p50 `9.38x`, p10..p90
`[3.36..17.03]`, decoded size match `24/24`.

Real corpus re-encode/decode speedup:

| Speed | Encode avg | Encode p50 | Encode p10..p90 | Decode avg | Decode p50 | Decode p10..p90 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `4.26x` | `4.17x` | `[2.73..5.62]` | `11.84x` | `11.70x` | `[6.92..17.39]` |
| 1 | `4.37x` | `4.28x` | `[2.90..5.92]` | `12.76x` | `12.26x` | `[6.64..19.56]` |
| 2 | `5.61x` | `5.79x` | `[2.98..7.84]` | `14.84x` | `14.24x` | `[8.00..22.46]` |
| 3 | `5.75x` | `6.02x` | `[2.85..7.97]` | `14.99x` | `13.60x` | `[7.63..22.38]` |
| 4 | `5.73x` | `5.85x` | `[3.03..7.90]` | `15.01x` | `13.63x` | `[7.64..24.72]` |
| 5 | `5.90x` | `6.20x` | `[3.34..8.00]` | `14.90x` | `13.64x` | `[7.54..23.70]` |
| 6 | `4.26x` | `4.33x` | `[2.53..5.56]` | `19.41x` | `20.72x` | `[7.19..30.90]` |
| 7 | `4.31x` | `4.40x` | `[2.44..5.55]` | `18.25x` | `21.16x` | `[6.68..29.19]` |
| 8 | `4.11x` | `4.37x` | `[2.34..5.72]` | `21.16x` | `23.88x` | `[7.80..33.43]` |
| 9 | `4.08x` | `4.36x` | `[2.49..5.71]` | `20.77x` | `23.54x` | `[8.00..34.75]` |
| 10 | `1.44x` | `1.44x` | `[1.10..1.83]` | `1.72x` | `1.58x` | `[1.43..2.15]` |

Every speed decodes the whole sample without failures, in all `5` runs. Speed
`0` once reported `3/24`, which was the separate-connectivity seam ordering
defect rather than a timing result.

### What The 2.0 Optimization Series Changed

The series between `7a277ff` and `f18f3e1` -- kd-tree walk, parallelogram
prediction bounds, corner-table construction, the entropy and normal memos --
measured as a paired A/B rather than against the snapshot above, since the
snapshot's own denominator moves. Both revisions were built with the same
toolchain and profile, and each binary carries its own copy of the unchanged
C++ side, which is the control: it agrees between the two to within `0.9%` on
the seeded sweep, so the Rust-side differences are the change.

Seeded sweep, `4` interleaved pairs, `us/1k faces` avg over the 12 samples.
The newer revision was faster in all `88` comparisons -- `4` pairs times `11`
speeds times encode and decode -- so the direction is not in question. Speeds
`3`, `4`, `6` and `7` track their neighbours and are left out of the table:

| Speed | Encode before | Encode after | Gain | Decode before | Decode after | Gain |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `601` | `542` | `1.11x` | `86.8` | `84.2` | `1.03x` |
| 1 | `584` | `525` | `1.11x` | `81.2` | `78.3` | `1.04x` |
| 2 | `416` | `409` | `1.02x` | `68.8` | `65.8` | `1.05x` |
| 5 | `398` | `392` | `1.01x` | `59.5` | `56.7` | `1.05x` |
| 8 | `389` | `385` | `1.01x` | `55.5` | `51.8` | `1.07x` |
| 9 | `390` | `386` | `1.01x` | `55.3` | `51.9` | `1.07x` |
| 10 | `32.9` | `32.1` | `1.03x` | `13.8` | `11.5` | `1.20x` |

So on meshes of this size the series bought `1%` to `2%` on encode at speeds
`2` and above, `11%` at speeds `0` and `1`, and `3%` to `7%` on decode, with
`20%` at speed `10`.

The real corpus disagrees on encode. Over `5` interleaved pairs the newer
revision was faster in `11/55` encode comparisons against `41/55` decode ones:
encode is `3%` to `13%` slower across speeds `1` to `9` and unchanged at speed
`0`, while decode runs from `2%` slower to `5%` faster, most speeds landing
between `1%` and `3%` faster. Speeds `6` and `10` are left out of both ranges
because the C++ control moved `8%` between the two binaries' runs there, which
is larger than the effect being read.

The difference between the two profiles is mesh size, so the reading that fits
is per-call setup: a memo table earns its keep across thousands of faces and
is pure overhead on a mesh of eight, and half this corpus is under `170`
faces. That is a hypothesis this snapshot does not settle -- what it measures
is the sign and the size on both profiles, with the same control. Anyone
taking it further should measure the memo allocations directly rather than
inferring them from these two tables.

## Main C++ vs Rust Benchmarks

### Decode Through The C++ Bridge

File: `crates/draco-cpp-test-bridge/tests/bench_decode_cpp_vs_rust.rs`

Package: `draco-cpp-test-bridge`

Purpose: in-process decode benchmark, C++ bridge vs Rust. The timed region is
matched between C++ and Rust, and the reported result uses median batches.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_decode_cpp_vs_rust --release -- --nocapture
```

### Encode Through The C++ Bridge

File: `crates/draco-cpp-test-bridge/tests/bench_encode_cpp_vs_rust.rs`

Package: `draco-cpp-test-bridge`

Purpose: in-process encode benchmark, C++ bridge vs Rust, without external
process startup cost.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_encode_cpp_vs_rust --release -- --nocapture
```

### Encode/Decode Matrix

File: `crates/draco-cpp-test-bridge/tests/bench_encode_decode_matrix.rs`

Package: `draco-cpp-test-bridge`

Purpose: encode/decode performance and correctness across multiple speeds and
mesh sizes.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_encode_decode_matrix --release -- --nocapture
```

### Decode Real Files

File: `crates/draco-cpp-test-bridge/tests/bench_decode_real_files.rs`

Package: `draco-cpp-test-bridge`

Purpose: decode timing on real `.drc` files from testdata, C++ bridge vs Rust.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_decode_real_files --release -- --nocapture
```

### Rust vs External C++ Tools

File: `crates/draco-core/tests/bench_external_cpp_encode.rs`

Package: `draco-core`

Purpose: Rust encode/decode compared with external C++ encoder/decoder tools.
Note that C++ runs here include process startup overhead.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-core --test bench_external_cpp_encode --release -- --nocapture
```

### Point Cloud Smoke Benchmark

File: `crates/draco-core/tests/bench_point_cloud.rs`

Package: `draco-core`

Purpose: point cloud encode/decode performance smoke test.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-core --test bench_point_cloud --release -- --nocapture
```

## Profiling And Micro-Benchmarks

### Sequential Pipeline Profile

File: `crates/draco-cpp-test-bridge/tests/profile_sequential_pipeline.rs`

Package: `draco-cpp-test-bridge`

Purpose: detailed sequential encoder/decoder stage profiling, rANS loop
micro-profile, clean and seeded topology cases, clone/setup overhead, and Rust
vs C++ breakdowns.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test profile_sequential_pipeline --release -- --nocapture
```

Useful test functions in this file:

- `profile_sequential_pipeline`
- `profile_detailed_breakdown`
- `profile_encoding_stages`
- `profile_symbol_encoding_details`
- `profile_rans_loop_micro`
- `profile_full_encode_breakdown`
- `profile_clean_topologies`
- `profile_seeded_mesh_sweep`
- `profile_real_corpus_gaussian_sweep`
- `profile_mesh_clone_overhead`
- `profile_point_ids_creation`
- `profile_rust_vs_cpp_breakdown`
- `profile_decode_rust_vs_cpp`
- `profile_decode_sequential_breakdown`

To turn profile data into a faster binary (a separate, build-time step rather
than a test), see [`PGO.md`](PGO.md).

