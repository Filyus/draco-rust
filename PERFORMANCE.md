# Performance

Decode and encode benchmarks, profiling tests, and the Rust-vs-C++ speed
snapshot. Use `--release` for timing runs; add `-- --nocapture` to see the
printed comparison output. Correctness and parity tests live in
[`TESTING.md`](TESTING.md).

## Speed Snapshot

Measured on the seeded and real-corpus profiles below, against the C++
reference build. Absolute microsecond figures are comparable only within a
single run -- they track machine state, while the C++ side is unchanged code.
The speedup columns are the stable quantity: repeat runs of the seeded sweep
agree to within about `2%`.

Seeded mesh sweep encode, `avg [p10..p90]` across `12` stratified samples:

Samples: `3` grid, `3` fan, `3` boundary ribbon, `3` torus.

Sampled points: avg `12,156`, p50 `9,664`, p10..p90 `[7,578..21,435]`.

Sampled faces: avg `18,641`, p50 `19,327`, p10..p90 `[15,151..21,487]`.

| Speed | C++ raw us | Rust raw us | C++ us/1k faces | Rust us/1k faces | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `31,738 [22,468..45,124]` | `11,896 [6,207..24,846]` | `1,680 [1,242..2,086]` | `660 [342..1,495]` | `3.49x [1.39..5.36]` |
| 1 | `31,514 [21,657..45,565]` | `11,740 [5,728..24,125]` | `1,670 [1,249..2,115]` | `647 [314..1,451]` | `3.47x [1.44..5.01]` |
| 2 | `26,716 [17,303..39,788]` | `8,153 [2,335..21,990]` | `1,416 [974..1,913]` | `458 [128..1,322]` | `6.24x [1.45..9.14]` |
| 3 | `26,687 [17,229..39,170]` | `8,107 [2,325..22,156]` | `1,412 [973..1,922]` | `454 [126..1,331]` | `6.32x [1.45..9.45]` |
| 4 | `26,951 [17,366..40,047]` | `8,216 [2,679..22,950]` | `1,427 [1,003..1,916]` | `461 [138..1,379]` | `6.27x [1.39..9.54]` |
| 5 | `26,403 [16,779..39,449]` | `7,938 [2,027..22,010]` | `1,399 [925..1,976]` | `446 [111..1,323]` | `6.94x [1.45..10.29]` |
| 6 | `26,396 [16,968..39,370]` | `7,881 [2,005..21,365]` | `1,399 [939..1,968]` | `445 [109..1,329]` | `7.06x [1.48..10.33]` |
| 7 | `26,406 [17,031..40,647]` | `7,838 [2,044..22,203]` | `1,395 [949..1,900]` | `440 [112..1,334]` | `7.00x [1.43..10.19]` |
| 8 | `26,172 [16,964..39,154]` | `7,616 [1,995..21,706]` | `1,385 [950..1,880]` | `429 [104..1,305]` | `7.36x [1.44..11.13]` |
| 9 | `26,550 [16,388..39,300]` | `7,797 [1,872..22,190]` | `1,410 [930..2,031]` | `438 [101..1,334]` | `7.43x [1.48..11.08]` |
| 10 | `848 [592..1,314]` | `704 [446..1,227]` | `44 [37..62]` | `36 [28..58]` | `1.26x [1.08..1.36]` |

Seeded mesh sweep decode, `avg [p10..p90]` across `12` stratified samples:

| Speed | C++ raw us | Rust raw us | C++ us/1k faces | Rust us/1k faces | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `50,126 [40,415..67,123]` | `1,762 [1,238..2,288]` | `2,662 [2,402..3,132]` | `93 [76..108]` | `28.98x [23.57..32.68]` |
| 1 | `49,683 [36,680..68,162]` | `1,631 [877..2,520]` | `2,629 [2,409..3,170]` | `85 [57..117]` | `32.65x [26.01..42.13]` |
| 2 | `49,363 [37,223..68,809]` | `1,393 [881..2,340]` | `2,611 [2,371..3,192]` | `72 [48..110]` | `38.25x [29.42..49.09]` |
| 3 | `48,956 [36,509..67,569]` | `1,331 [748..2,330]` | `2,592 [2,380..3,142]` | `68 [48..110]` | `40.96x [29.01..50.32]` |
| 4 | `49,264 [35,561..67,563]` | `1,357 [716..2,415]` | `2,605 [2,367..3,152]` | `69 [47..113]` | `40.94x [27.98..50.57]` |
| 5 | `49,053 [35,583..71,245]` | `1,225 [651..2,196]` | `2,590 [2,346..3,122]` | `63 [43..104]` | `44.94x [30.40..54.86]` |
| 6 | `48,848 [35,685..69,828]` | `1,227 [657..2,261]` | `2,582 [2,337..3,146]` | `63 [43..106]` | `45.27x [29.98..56.31]` |
| 7 | `49,373 [36,131..67,590]` | `1,269 [674..2,261]` | `2,612 [2,377..3,142]` | `65 [44..106]` | `43.70x [29.90..54.74]` |
| 8 | `49,175 [35,518..69,149]` | `1,172 [605..2,219]` | `2,597 [2,342..3,219]` | `60 [40..104]` | `48.20x [31.19..59.18]` |
| 9 | `49,951 [37,041..67,218]` | `1,157 [612..2,118]` | `2,644 [2,388..3,186]` | `59 [40..104]` | `49.76x [30.86..60.96]` |
| 10 | `317 [208..505]` | `272 [169..447]` | `16 [14..24]` | `14 [11..21]` | `1.19x [1.10..1.27]` |

Real `.drc` corpus normal-distribution sample:

Samples: `24` draws from `16` compatible real mesh fixtures, seed
`0x6a7573737265616c`.

Sampled points: avg `2,096`, p50 `97`, p10..p90 `[9..4,959]`, min..max
`[8..34,834]`.

Sampled faces: avg `4,001`, p50 `170`, p10..p90 `[9..8,525]`, min..max
`[8..69,451]`.

Source `.drc` decode speedup: avg `13.76x`, p50 `13.04x`, p10..p90
`[4.02..23.58]`, decoded size match `24/24`.

Real corpus re-encode/decode speedup:

| Speed | Encode avg | Encode p50 | Encode p10..p90 | Decode avg | Decode p50 | Decode p10..p90 | Decode failures |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `4.81x` | `4.87x` | `[3.20..6.19]` | `14.58x` | `13.08x` | `[8.24..23.41]` | `0/24` |
| 1 | `4.83x` | `5.34x` | `[3.29..6.27]` | `15.40x` | `14.48x` | `[8.65..25.20]` | `0/24` |
| 2 | `7.36x` | `8.01x` | `[3.88..9.71]` | `18.63x` | `16.48x` | `[9.47..29.89]` | `0/24` |
| 3 | `7.16x` | `7.48x` | `[3.87..9.72]` | `18.58x` | `16.84x` | `[8.28..29.25]` | `0/24` |
| 4 | `7.85x` | `7.83x` | `[4.07..10.98]` | `17.98x` | `16.77x` | `[9.22..28.68]` | `0/24` |
| 5 | `7.36x` | `8.24x` | `[4.02..9.52]` | `18.18x` | `16.75x` | `[8.58..26.52]` | `0/24` |
| 6 | `5.32x` | `5.37x` | `[2.98..6.76]` | `24.61x` | `25.02x` | `[8.87..43.17]` | `0/24` |
| 7 | `5.42x` | `5.70x` | `[3.00..7.38]` | `24.46x` | `25.56x` | `[9.17..43.52]` | `0/24` |
| 8 | `5.45x` | `5.81x` | `[3.21..7.52]` | `26.03x` | `30.64x` | `[9.83..41.65]` | `0/24` |
| 9 | `5.40x` | `5.65x` | `[3.30..7.43]` | `25.13x` | `29.04x` | `[9.08..41.19]` | `0/24` |
| 10 | `1.49x` | `1.50x` | `[1.06..1.90]` | `1.55x` | `1.51x` | `[1.29..1.94]` | `0/24` |

Every speed now decodes the whole sample without failures. Speed `0`
previously reported `3/24`, which was the separate-connectivity seam ordering
defect rather than a timing result.

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

