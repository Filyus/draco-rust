# Benchmark And Test Navigation

This file is a quick map for performance benchmarks, profiling tests,
compatibility checks, and parity tests. Use `--release` for timing runs.
Add `-- --nocapture` when you want to see the printed comparison output.

## Quick Commands

Run all Rust tests in the `crates` workspace:

```sh
cargo test --manifest-path crates/Cargo.toml --release -- --nocapture
```

Run one integration test target:

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_decode_cpp_vs_rust --release -- --nocapture
cargo test --manifest-path crates/Cargo.toml -p draco-core --test bench_external_cpp_encode --release -- --nocapture
```

Required formatting checks before finalizing Rust changes:

```sh
cargo fmt --manifest-path crates/Cargo.toml --all -- --check
cargo fmt --manifest-path web/Cargo.toml --all -- --check
```

## Speed Snapshot

Seeded mesh sweep encode, `avg [p10..p90]` across `12` stratified samples:

Samples: `3` grid, `3` fan, `3` boundary ribbon, `3` torus.

| Speed | C++ raw us | Rust raw us | C++ us/1k faces | Rust us/1k faces | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `25,588 [17,618..38,628]` | `10,992 [5,505..24,049]` | `1,361 [976..1,880]` | `610 [304..1,446]` | `3.11x [1.30..4.79]` |
| 1 | `25,202 [18,340..36,485]` | `11,019 [5,332..23,717]` | `1,341 [1,023..1,742]` | `609 [298..1,425]` | `3.03x [1.22..4.52]` |
| 2 | `21,340 [13,183..33,157]` | `7,803 [2,118..21,959]` | `1,137 [733..1,709]` | `438 [114..1,320]` | `5.16x [1.37..7.28]` |
| 3 | `21,176 [13,518..32,781]` | `7,841 [2,155..21,615]` | `1,126 [738..1,658]` | `440 [122..1,299]` | `5.17x [1.28..7.75]` |
| 4 | `21,044 [13,347..32,558]` | `7,725 [2,179..22,042]` | `1,122 [735..1,694]` | `434 [119..1,324]` | `5.18x [1.28..7.64]` |
| 5 | `20,836 [12,824..32,857]` | `7,676 [2,094..21,332]` | `1,108 [711..1,633]` | `431 [108..1,282]` | `5.62x [1.28..8.35]` |
| 6 | `20,992 [13,109..32,604]` | `7,542 [1,902..21,725]` | `1,117 [728..1,659]` | `424 [104..1,305]` | `5.86x [1.27..8.59]` |
| 7 | `20,808 [12,824..32,739]` | `7,477 [1,939..21,375]` | `1,107 [717..1,648]` | `421 [106..1,284]` | `5.72x [1.28..8.50]` |
| 8 | `20,467 [12,588..32,616]` | `7,429 [1,807..20,683]` | `1,087 [698..1,652]` | `417 [98..1,243]` | `5.89x [1.33..8.64]` |
| 9 | `20,446 [12,601..31,712]` | `7,406 [1,819..22,598]` | `1,088 [703..1,643]` | `417 [100..1,357]` | `5.88x [1.26..8.78]` |
| 10 | `786 [557..1,227]` | `632 [404..1,125]` | `41 [34..57]` | `33 [26..50]` | `1.31x [1.13..1.48]` |

Seeded mesh sweep decode, `avg [p10..p90]` across `12` stratified samples:

| Speed | C++ raw us | Rust raw us | C++ us/1k faces | Rust us/1k faces | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `35,561 [26,135..49,162]` | `1,482 [906..2,040]` | `1,880 [1,715..2,281]` | `78 [60..95]` | `24.59x [19.91..28.75]` |
| 1 | `35,574 [25,986..48,497]` | `1,478 [742..2,324]` | `1,884 [1,710..2,260]` | `77 [50..108]` | `25.98x [20.85..34.14]` |
| 2 | `35,241 [26,117..48,299]` | `1,243 [816..2,152]` | `1,866 [1,721..2,254]` | `64 [45..100]` | `30.98x [22.50..39.10]` |
| 3 | `35,449 [26,159..49,090]` | `1,215 [637..2,142]` | `1,874 [1,714..2,270]` | `62 [42..100]` | `32.41x [22.75..41.04]` |
| 4 | `34,892 [25,924..48,172]` | `1,196 [666..2,133]` | `1,844 [1,694..2,256]` | `61 [44..99]` | `32.59x [22.66..39.16]` |
| 5 | `35,110 [26,027..48,089]` | `1,088 [559..1,966]` | `1,858 [1,692..2,243]` | `56 [39..93]` | `36.62x [24.46..46.22]` |
| 6 | `35,022 [26,038..48,000]` | `1,106 [556..2,134]` | `1,854 [1,691..2,240]` | `56 [36..98]` | `37.00x [22.83..47.85]` |
| 7 | `35,025 [25,535..47,804]` | `1,098 [578..2,122]` | `1,851 [1,688..2,260]` | `56 [38..97]` | `36.84x [23.34..45.33]` |
| 8 | `34,500 [25,201..47,526]` | `1,029 [525..1,988]` | `1,824 [1,664..2,222]` | `52 [35..92]` | `38.85x [24.24..48.29]` |
| 9 | `34,799 [25,764..47,998]` | `1,023 [534..1,960]` | `1,840 [1,689..2,232]` | `52 [36..91]` | `39.26x [24.51..48.13]` |
| 10 | `298 [195..484]` | `257 [186..427]` | `16 [13..22]` | `13 [11..20]` | `1.18x [1.07..1.26]` |

Real `.drc` corpus normal-distribution sample:

Samples: `24` draws from `16` compatible real mesh fixtures, seed
`0x6a7573737265616c`.

Sampled points: avg `2,096`, p50 `97`, p10..p90 `[9..4,959]`, min..max
`[8..34,834]`.

Sampled faces: avg `4,001`, p50 `170`, p10..p90 `[9..8,525]`, min..max
`[8..69,451]`.

Source `.drc` decode speedup: avg `11.34x`, p50 `10.83x`, p10..p90
`[2.95..19.65]`, decoded size match `24/24`.

Real corpus re-encode/decode speedup:

| Speed | Encode avg | Encode p50 | Encode p10..p90 | Decode avg | Decode p50 | Decode p10..p90 | Decode failures |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `4.39x` | `4.32x` | `[2.72..5.86]` | `11.94x` | `10.66x` | `[6.10..20.22]` | `3/24` |
| 1 | `4.26x` | `4.10x` | `[2.85..6.29]` | `13.07x` | `13.11x` | `[7.27..20.35]` | `0/24` |
| 2 | `6.20x` | `6.69x` | `[3.30..8.12]` | `15.27x` | `13.66x` | `[7.18..26.62]` | `0/24` |
| 3 | `6.19x` | `6.59x` | `[3.55..8.20]` | `15.74x` | `14.04x` | `[7.60..27.55]` | `0/24` |
| 4 | `6.42x` | `6.71x` | `[3.45..8.61]` | `15.24x` | `14.25x` | `[7.79..24.83]` | `0/24` |
| 5 | `6.61x` | `7.04x` | `[3.44..8.62]` | `15.35x` | `13.34x` | `[7.65..26.83]` | `0/24` |
| 6 | `4.83x` | `4.89x` | `[2.89..6.06]` | `20.71x` | `24.46x` | `[7.31..32.39]` | `0/24` |
| 7 | `4.70x` | `5.07x` | `[2.89..5.85]` | `20.30x` | `23.86x` | `[7.17..33.07]` | `0/24` |
| 8 | `4.18x` | `4.41x` | `[2.71..5.55]` | `21.55x` | `24.93x` | `[7.94..35.51]` | `0/24` |
| 9 | `4.35x` | `4.56x` | `[2.77..5.85]` | `20.76x` | `24.50x` | `[7.09..34.33]` | `0/24` |
| 10 | `1.49x` | `1.47x` | `[1.13..1.84]` | `1.54x` | `1.42x` | `[1.24..2.13]` | `0/24` |

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

## Compatibility And Parity

These are not pure benchmarks, but they are often useful next to performance
work because they show whether faster Rust output remains compatible with C++
Draco.

### Byte-Level Encode Parity

File: `crates/draco-cpp-test-bridge/tests/parity_encode_bytes.rs`

Package: `draco-cpp-test-bridge`

Purpose: byte-level comparison of Rust and C++ encoder output for selected
meshes and speed values.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test parity_encode_bytes --release -- --nocapture
```

### Encoding Speed Compatibility

File: `crates/draco-core/tests/compat_encoding_speed.rs`

Package: `draco-core`

Purpose: encoding speed compatibility and encoded-size behavior against C++
expectations.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-core --test compat_encoding_speed --release -- --nocapture
```

### Encoder Options Compatibility

File: `crates/draco-core/tests/compat_encoder_options.rs`

Package: `draco-core`

Purpose: quantization bits, compression levels, edge cases, and the
speed/quantization compatibility matrix.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-core --test compat_encoder_options --release -- --nocapture
```

### Encoding Speed Through The I/O Layer

File: `crates/draco-io/tests/encoding_speed_test.rs`

Package: `draco-io`

Purpose: end-to-end encoding speed behavior through the I/O API.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-io --test encoding_speed_test --release -- --nocapture
```

## C++ I/O Smoke Examples

### Focused Real I/O Smoke Test

File: `draco_io/examples/real_io_smoke_test.cpp`

Purpose: real file I/O operations, basic encoding, format detection, and error
handling.

### Enhanced Real I/O Smoke Test

File: `draco_io/examples/enhanced_io_smoke_test.cpp`

Purpose: expanded real file I/O validation, round trips, format detection, and
performance metrics.

Build status: the file is referenced in `draco_io/CMakeLists.txt`, but the
target is currently commented out because of complex transcoder integration.

## Rename Map

The files were renamed so similar tests sort together.

Rename date: 2026-04-26.

Directory: `crates/draco-cpp-test-bridge/tests`

| Old file | New file |
| --- | --- |
| `bench_decode_comparison.rs` | `bench_decode_cpp_vs_rust.rs` |
| `test_bridge_benchmark.rs` | `bench_encode_cpp_vs_rust.rs` |
| `decode_real_files.rs` | `bench_decode_real_files.rs` |
| `comprehensive_performance.rs` | `bench_encode_decode_matrix.rs` |
| `profile_sequential.rs` | `profile_sequential_pipeline.rs` |
| `byte_comparison.rs` | `parity_encode_bytes.rs` |

Directory: `crates/draco-core/tests`

| Old file | New file |
| --- | --- |
| `performance_comparison.rs` | `bench_external_cpp_encode.rs` |
| `point_cloud_performance.rs` | `bench_point_cloud.rs` |
| `speed_compatibility.rs` | `compat_encoding_speed.rs` |
| `encoder_options_compatibility.rs` | `compat_encoder_options.rs` |

Directory: `draco_io/examples`

| Old file | New file |
| --- | --- |
| `real_io_test.cpp` | `real_io_smoke_test.cpp` |
| `enhanced_io_test.cpp` | `enhanced_io_smoke_test.cpp` |
