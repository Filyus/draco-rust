# Draco for Rust

Pure Rust encoder/decoder crates for the [Draco](https://github.com/google/draco)
geometry compression format.

This repository contains an independent Rust implementation of Draco mesh and
point-cloud compression. It is designed for applications that want Draco
bitstream compatibility without linking the C++ library, including native Rust,
WASM, and format-conversion workflows.

The project is not an official Google Draco release.

## Crates

| Crate | Purpose |
|---|---|
| [`draco-core`](crates/draco-core) | Core Draco mesh and point-cloud encode/decode implementation. |
| [`draco-io`](crates/draco-io) | Format I/O helpers for OBJ, PLY, FBX, glTF, and GLB, including glTF Draco extension support. |

## Status

Current focus:

- Mesh and point-cloud decoding.
- Mesh and point-cloud encoding.
- Sequential and EdgeBreaker mesh paths.
- Attribute prediction schemes, including position, normal, color, texcoord,
  and generic attributes.
- C++ interop tests in both directions:
  C++ encode -> Rust decode and Rust encode -> C++ decode.
- Legacy `.drc` fixture coverage and malformed-input hardening.
- `no unsafe` in `draco-core` and `draco-io` source code.

The public API is still young. Expect refinements before a stable 1.0 release.

## Installation

```toml
[dependencies]
draco-core = "0.1"
draco-io = "0.1"
```

Use only the crate you need. `draco-core` is enough for raw Draco bitstreams;
`draco-io` adds file-format readers and writers.

Decoder-only builds are supported:

```toml
[dependencies]
draco-core = { version = "0.1", default-features = false, features = ["decoder"] }
draco-io = { version = "0.1", default-features = false, features = ["decoder"] }
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
| glTF / GLB | yes | yes | Includes `KHR_draco_mesh_compression`. |
| OBJ | yes | yes | Meshes and point clouds. |
| PLY | yes | yes | ASCII and binary paths with mesh/point data. |
| FBX | yes | yes | Rust-side I/O helpers, optional compression support. |

## Compatibility

The implementation targets compatibility with the official C++ Draco bitstream.
The test suite includes reference fixtures, legacy Draco files, C++ encode ->
Rust decode checks, and Rust encode -> C++ decode checks.

For local C++ interop benchmarks, `draco-cpp-test-bridge` can be pointed at a
local C++ Draco checkout/build through environment variables:

```powershell
$env:DRACO_CPP_SOURCE_DIR = "D:\Projects\Draco\src"
$env:DRACO_CPP_BUILD_DIR = "D:\Projects\Draco\build-original"
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge
```

That bridge crate is test infrastructure only and is marked `publish = false`.

## Performance

Benchmarks are still evolving, so treat the numbers as a snapshot rather than a
contract. On local generated mesh encode/decode tests, the Rust implementation
was faster than the C++ reference in that setup:

| Case | Encode | Decode |
|---|---:|---:|
| Sphere 24x48, speeds 0-9 | about 5.6x-15.6x faster | about 50x-126x faster |
| Cube subdiv20, speeds 0-9 | about 6.4x-15.9x faster | about 65x-126x faster |
| Speed 10 sequential path | about 1.7x faster | about 1.1x faster |

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

## Repository Layout

```text
crates/
  draco-core/              Core Draco bitstream implementation
  draco-io/                OBJ/PLY/FBX/glTF/GLB helpers
web/                       WASM conversion modules and demo workspace
fuzz/                      Decode fuzz target wiring
testdata/                  Fixtures used by compatibility and hardening tests
```

## License

Apache-2.0, matching the upstream Draco project.

## Related

- [Google Draco](https://github.com/google/draco)
