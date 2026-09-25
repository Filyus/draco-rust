# draco-core

[![Crates.io](https://img.shields.io/crates/v/draco-core.svg)](https://crates.io/crates/draco-core)
[![Docs.rs](https://docs.rs/draco-core/badge.svg)](https://docs.rs/draco-core)
[![Rust CI](https://github.com/Filyus/draco-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/Filyus/draco-rust/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/Filyus/draco-rust/blob/main/LICENSE)

`draco-core` is a pure Rust Draco bitstream encoder and decoder for triangle
meshes and point clouds.

It targets compatibility with the official C++ Draco format without linking the
C++ library. The crate is suitable for native Rust, WASM, and format conversion
pipelines that need direct access to Draco geometry data.

This project is independent and is not an official Google Draco release.

## Status

Parity with C++ Draco 1.5.7 is byte-exact and mostly reached. The same mesh and
options give the same bytes, and each implementation reads what the other
writes. The deliberate exceptions are in
[`COMPATIBILITY.md`](https://github.com/Filyus/draco-rust/blob/main/COMPATIBILITY.md).

Two point-cloud encoder options go past upstream on request:
`set_prediction_search` and `set_spatial_point_order` make smaller files that
any Draco decoder reads, but not the bytes C++ Draco writes. Both are off by
default.

The crate encodes and decodes:

- EdgeBreaker standard meshes: speeds 5 to 9.
- EdgeBreaker valence meshes: below speed 5 for more compression.
- Sequential meshes: speed 10 or an explicit `set_encoding_method`.
- Point clouds: sequential and KD-tree attribute paths.

Defaults differ:

| Speed | This project | Other tools |
|---:|---|---|
| 5 | `draco-core` | the C++ Draco library |
| 4 | the web converter | Blender glTF export |
| 3 | no CLI | `draco_encoder` CLI (`-cl 7`) |

Speed 4 measured smallest or near-smallest on every mesh tried.
[`API.md`](API.md) gives the exact CLI equivalence, which differs in
quantization as well as speed.

For file formats, see `draco-gltf` (glTF, GLB) and `draco-io` (OBJ, PLY, STL,
FBX).

For the detailed algorithm matrix, see
[`SUPPORT_MATRIX.md`](SUPPORT_MATRIX.md).

## Installation

```toml
[dependencies]
draco-core = "2.2"
```

Decoder-only builds:

```toml
[dependencies]
draco-core = { version = "2.2", default-features = false, features = ["decoder"] }
```

Encoder-only builds:

```toml
[dependencies]
draco-core = { version = "2.2", default-features = false, features = ["encoder"] }
```

## Feature Flags

| Feature | Default | Description |
|---|---:|---|
| `encoder` | yes | Mesh and point-cloud encoding APIs. |
| `decoder` | yes | Mesh and point-cloud decoding APIs. |
| `point_cloud_decode` | yes | Point-cloud decoder path. |
| `edgebreaker_valence_encode` | yes | Modern EdgeBreaker valence traversal for high-compression mesh encoding. |
| `edgebreaker_valence_decode` | yes | Decode EdgeBreaker valence traversal streams. |
| `legacy_bitstream_encode` | yes | Compatibility support for writing older Draco bitstream layouts and deprecated prediction schemes. |
| `legacy_bitstream_decode` | yes | Decode older Draco bitstreams and deprecated prediction schemes. |
| `debug_logs` | no | Internal diagnostics. |
| `force_sequential_seeds` | no | Test/debug control for deterministic seed behavior. |

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

Encode a triangle mesh:

```rust
use draco_core::{EncoderBuffer, EncoderOptions, Mesh, MeshEncoder};

fn encode_mesh(mesh: &Mesh) -> Result<Vec<u8>, draco_core::DracoError> {
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());

    let mut options = EncoderOptions::new();
    options.set_compression_level(7); // draco_encoder's own -cl default

    let mut out = EncoderBuffer::new();
    encoder.encode(&options, &mut out)?;
    Ok(out.data().to_vec())
}
```

Create a minimal mesh:

```rust
use draco_core::{
    DataType, FaceIndex, GeometryAttributeType, Mesh, PointAttribute, PointIndex,
};

fn triangle() -> Mesh {
    let mut mesh = Mesh::new();

    let mut position = PointAttribute::new();
    position.init(GeometryAttributeType::Position, 3, DataType::Float32, false, 3);

    let positions: [[f32; 3]; 3] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
    ];

    for (i, value) in positions.iter().enumerate() {
        let bytes: Vec<u8> = value.iter().flat_map(|v| v.to_le_bytes()).collect();
        position.buffer_mut().write(i * 12, &bytes);
    }

    mesh.add_attribute(position);
    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh
}
```

## Compatibility

The implementation is tested against:

- Draco files encoded by the official C++ implementation.
- Rust-encoded files decoded by the C++ implementation.
- Legacy `.drc` fixtures for older bitstream versions.
- Malformed-input and edge-case fixtures.

Deprecated C++ prediction schemes are implemented for compatibility and testing,
but they are not selected by default. The official C++ public encoder rejects
`MESH_PREDICTION_MULTI_PARALLELOGRAM` and
`MESH_PREDICTION_TEX_COORDS_DEPRECATED` as deprecated; `draco-core` follows the
same spirit by keeping legacy encode support explicit.

## Workspace Crates

- `draco-io`: OBJ / PLY / STL / FBX readers and writers. Formats that carry geometry in their own encoding, so this crate's codec never enters.
- `draco-gltf`: load and save full glTF / GLB scenes with Draco-compressed geometry, containers and accessors included — decode and (re)compress via `draco-core`. The only consumer of the codec that is also a file format.
- `draco-cpp-test-bridge`: test infrastructure for C++ parity.

## Development

Run the crate tests from the workspace:

```powershell
cargo test --manifest-path crates/Cargo.toml -p draco-core --all-features
```

Run the full Rust workspace:

```powershell
cargo test --manifest-path crates/Cargo.toml --all-features
```

### Architecture notes

For why `draco-core` favors compile-time dispatch (generics, `enum`/`match`,
`Option<Concrete>`) over the `unique_ptr<Interface>` + factory pattern used by
upstream C++ Draco — with side-by-side snippets — see
[`DISPATCH.md`](DISPATCH.md).

## License

Apache-2.0, matching the upstream Draco project.
