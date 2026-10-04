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

### Draco glTF From glTF-Transform

File: `crates/draco-gltf/tests/gltf_transform_fixtures_test.rs`

Package: `draco-gltf`

Purpose: Draco glTF as glTF-Transform writes it -- placeholder accessors, the
extension's own attribute ids, several primitives in one binary chunk --
imports, decodes to the counts its accessors declare, and decodes to the same
triangles through EdgeBreaker (standard and valence traversals) and the
sequential coder. The fixtures and how they were made are in
`testdata/gltf_transform/README.md`. Runs in CI.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-gltf --test gltf_transform_fixtures_test
```

### Someone Else's Files Against C++

File: `crates/draco-cpp-test-bridge/tests/external_draco_corpus_probe.rs`

Package: `draco-cpp-test-bridge`

Purpose: a directory no test can carry -- files a user reports, a sample-model
repository, what another tool writes -- walked whole. Every `.drc`, and every
Draco primitive's stream in a `.gltf` or `.glb`, is decoded here and by C++
Draco and must have the same fingerprint; every glTF must also import and
decode through draco-gltf under both validation profiles. Ignored unless
`DRACO_CORPUS_DIR` is set; it needs the C++ bridge.

```sh
DRACO_CORPUS_DIR=<dir> cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test external_draco_corpus_probe --release -- --ignored --nocapture
```

Corpora it has passed, 2026-10-04, on this tree and on the draco-core 2.2.1
release alike: the Draco variants of a public glTF sample-model collection
(253 streams in 16 files, written by gltf-pipeline), and those 16 models
re-encoded by glTF-Transform 4.5.1 in seven modes (1,771 streams in 112 files).
To build the second, with `@gltf-transform/cli` installed:

```sh
gltf-transform draco <in> <out>.glb                                    # EdgeBreaker
gltf-transform draco <in> <out>.glb --encode-speed 10 --decode-speed 10
gltf-transform draco <in> <out>.glb --encode-speed 0 --decode-speed 0  # valence traversal
gltf-transform draco <in> <out>.glb --method sequential
gltf-transform draco <in> <out>.glb --quantize-position 16 --quantize-normal 16 --quantize-texcoord 16 --quantize-color 16 --quantize-generic 16
gltf-transform draco <in> <out>.glb --quantize-position 6 --quantize-normal 4 --quantize-texcoord 6 --quantize-color 4 --quantize-generic 4
gltf-transform draco <in> <out>.glb --quantization-volume scene
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
