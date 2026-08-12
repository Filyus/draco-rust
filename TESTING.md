# Testing

Correctness, parity, and compatibility tests: byte-level encode parity against
C++ Draco, encoding-speed and encoder-option compatibility, and C++ I/O smoke
examples. Performance benchmarks and profiling live in
[`PERFORMANCE.md`](PERFORMANCE.md); the commands to run the whole suite are in
[`AGENTS.md`](AGENTS.md).

## Compatibility And Parity

These show whether Rust encode/decode output stays compatible with C++ Draco
-- useful next to performance work, since a faster path is only valuable if it
remains correct.

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
