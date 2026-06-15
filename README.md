# Draco for Rust

Pure Rust crates for the [Draco](https://github.com/google/draco) geometry
compression format.

This repository provides an independent Rust implementation of Draco mesh and
point-cloud compression, plus format I/O crates around it. It is intended for
native Rust, WASM, and conversion pipelines that need Draco bitstream
compatibility without linking the C++ library.

This project is not an official Google Draco release.

## Crates

| Crate | Purpose |
|---|---|
| [`draco-core`](crates/draco-core) | Raw Draco `.drc` mesh and point-cloud encode/decode implementation. |
| [`draco-io`](crates/draco-io) | Readers and writers for OBJ, PLY, FBX, glTF, and GLB. |

## Features

`draco-core` handles raw Draco data:

- Decode and encode `.drc` point clouds and triangle meshes.
- Use sequential, KD-tree, and EdgeBreaker paths.
- Preserve positions, normals, colors, texture coordinates, and generic
  attributes.
- Roundtrip raw and typed Draco metadata.

`draco-io` handles file formats:

- Read and write glTF, GLB, OBJ, PLY, and binary FBX.
- Use `KHR_draco_mesh_compression` for Draco-compressed glTF and GLB.

The main paths are covered by Rust roundtrips, fixtures, and C++ interop tests.

Neither `draco-core` nor `draco-io` uses `unsafe` in its source code.

For detailed compatibility and scope notes, see the crate docs and
[`SUPPORT_MATRIX.md`](crates/draco-core/SUPPORT_MATRIX.md).

## Installation

```toml
[dependencies]
draco-core = "0.1"
draco-io = "0.1"
```

Use only the crate you need. `draco-core` is enough for raw Draco bitstreams;
`draco-io` adds file-format readers and writers.

For raw `.drc` decode-only builds:

```toml
[dependencies]
draco-core = { version = "0.1", default-features = false, features = ["decoder"] }
```

Format-specific `draco-io` builds can enable only the readers or writers needed:

```toml
[dependencies]
draco-io = { version = "0.1", default-features = false, features = ["gltf-reader", "gltf-writer"] }
```

## Quick Start

Decode a Draco mesh from bytes:

```rust
use draco_core::{DecoderBuffer, Mesh, MeshDecoder};

fn decode_mesh(bytes: &[u8]) -> Result<Mesh, draco_core::DracoError> {
    let mut buffer = DecoderBuffer::new(bytes);
    let mut decoder = MeshDecoder::new();
    let mut mesh = Mesh::new();

    decoder.decode(&mut buffer, &mut mesh)?;
    Ok(mesh)
}
```

Encode a mesh:

```rust
use draco_core::{EncoderBuffer, EncoderOptions, Mesh, MeshEncoder};

fn encode_mesh(mesh: &Mesh) -> Result<Vec<u8>, draco_core::DracoError> {
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", 5);
    options.set_global_int("decoding_speed", 5);

    let mut out = EncoderBuffer::new();
    encoder.encode(&options, &mut out)?;
    Ok(out.data().to_vec())
}
```

Write a Draco-compressed GLB:

```rust
use draco_io::gltf_writer::GltfWriter;

fn write_glb(mesh: &draco_core::Mesh, path: &str) -> Result<(), draco_io::GltfWriteError> {
    let mut writer = GltfWriter::new();
    writer.add_draco_mesh(mesh, Some("mesh"), None)?;
    writer.write_glb(path)
}
```

Read Draco-compressed meshes from a GLB:

```rust
use draco_io::gltf_reader::GltfReader;

fn read_glb(path: &str) -> Result<(), draco_io::GltfError> {
    let reader = GltfReader::open(path)?;

    for (info, mesh) in reader.decode_all_draco_meshes()? {
        println!(
            "{}: {} points, {} faces",
            info.mesh_name.as_deref().unwrap_or("mesh"),
            mesh.num_points(),
            mesh.num_faces()
        );
    }

    Ok(())
}
```

## Format Support

| Format | Read | Write | Notes |
|---|---:|---:|---|
| Draco `.drc` | yes | yes | Meshes and point clouds through `draco-core`. |
| glTF / GLB | yes | yes | Focused support for Draco triangle meshes through `KHR_draco_mesh_compression`. |
| OBJ | yes | yes | Meshes and point clouds. |
| PLY | yes | yes | ASCII and binary paths with mesh/point data. |
| FBX | yes | yes | Binary FBX with optional array compression. |

`draco-io` is not a full scene SDK. Materials, textures, cameras, lights,
animation, skinning, and semantic glTF metadata are outside the supported
format scope.

## Compatibility

The implementation targets compatibility with the official C++ Draco bitstream.
The test suite includes reference fixtures, legacy Draco files, C++-encoded
streams decoded by Rust, and Rust-encoded streams decoded by C++.

For local C++ interop benchmarks, `draco-cpp-test-bridge` can be pointed at a
local C++ Draco checkout/build through environment variables:

```powershell
$env:DRACO_CPP_SOURCE_DIR = "D:\Projects\Draco\src"
$env:DRACO_CPP_BUILD_DIR = "D:\Projects\Draco\build-original"
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge
```

That bridge crate is test infrastructure only and is marked `publish = false`.

## Performance

Local Rust/C++ interop benchmarks show a clear advantage in this setup,
especially on compressed mesh decoding. Encoding also measures faster on the
generated mesh cases tracked by the benchmark suite. Decode can be dramatically
faster; encode also shows solid gains. Exact results depend on the mesh,
encoder settings, and hardware.

The benchmark suite lives mostly under `draco-cpp-test-bridge`:

```powershell
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --release -- --nocapture
```

See [`BENCHMARK_TESTS.md`](BENCHMARK_TESTS.md) for the available benchmark
targets and what each one measures.

## Development

Run the Rust workspace tests:

```powershell
cargo test --manifest-path crates/Cargo.toml --all
```

Run the WASM/web workspace tests:

```powershell
cargo test --manifest-path web/Cargo.toml --all
```

Run the decode fuzz target check:

```powershell
cargo check --manifest-path fuzz/Cargo.toml --bins
```

Run the coverage-guided decode fuzzer (nightly + `cargo-fuzz`):

```powershell
pwsh fuzz/seed_corpus.ps1
cargo +nightly fuzz run -O decode_drc --fuzz-dir fuzz -- -max_total_time=120 -rss_limit_mb=4096
```

See [`FUZZING.md`](FUZZING.md) for the full fuzzing routine and
[`SECURITY.md`](SECURITY.md) for the decode threat model and recommended caller
resource limits for untrusted input.

## Repository Layout

```text
crates/
  draco-core/              Core Draco bitstream implementation
  draco-io/                OBJ/PLY/FBX/glTF/GLB readers and writers
web/                       WASM conversion modules and demo workspace
fuzz/                      Decode fuzz target wiring
testdata/                  Fixtures used by compatibility and hardening tests
```

## License

Apache-2.0, matching the upstream Draco project.

## Related

- [Google Draco](https://github.com/google/draco)
