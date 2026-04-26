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
micro-profile, clone/setup overhead, and Rust vs C++ breakdowns.

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
