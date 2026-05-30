# draco-core

`draco-core` is a pure Rust Draco bitstream encoder and decoder for triangle
meshes and point clouds.

It targets compatibility with the official C++ Draco format without linking the
C++ library. The crate is suitable for native Rust, WASM, and format conversion
pipelines that need direct access to Draco geometry data.

This project is independent and is not an official Google Draco release.

## Status

The crate is pre-1.0, but the bitstream implementation is already tested
against reference fixtures and C++ interop paths. Remaining compatibility notes
and scope boundaries are tracked in the support matrix.

Supported high-level paths:

| Path | Decode | Encode | Notes |
|---|---:|---:|---|
| Draco point cloud | yes | yes | Sequential and KD-tree attribute paths are covered. |
| Draco triangle mesh, sequential | yes | yes | Generic mesh path. |
| Draco triangle mesh, EdgeBreaker standard | yes | yes | Main compressed mesh path. |
| Draco triangle mesh, EdgeBreaker valence | yes | yes | Behind valence feature flags. |
| glTF / GLB container I/O | no | no | Use `draco-io` for file formats and `KHR_draco_mesh_compression`. |

For the detailed algorithm matrix, see
[`SUPPORT_MATRIX.md`](SUPPORT_MATRIX.md).

## Installation

```toml
[dependencies]
draco-core = "0.1"
```

Decoder-only builds:

```toml
[dependencies]
draco-core = { version = "0.1", default-features = false, features = ["decoder"] }
```

Encoder-only builds:

```toml
[dependencies]
draco-core = { version = "0.1", default-features = false, features = ["encoder"] }
```

## Feature Flags

| Feature | Default | Description |
|---|---:|---|
| `encoder` | yes | Mesh and point-cloud encoding APIs. |
| `decoder` | yes | Mesh and point-cloud decoding APIs. |
| `point_cloud_decode` | yes | Point-cloud decoder path. |
| `edgebreaker_valence_encode` | yes | EdgeBreaker valence encoder path. |
| `edgebreaker_valence_decode` | yes | EdgeBreaker valence decoder path. |
| `legacy_bitstream_encode` | yes | Manual/testing encode support for deprecated legacy prediction schemes. |
| `legacy_bitstream_decode` | yes | Decode support for deprecated legacy prediction schemes. |
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
    options.set_global_int("encoding_speed", 5);
    options.set_global_int("decoding_speed", 5);

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

## Related Crates

- `draco-io`: OBJ, PLY, FBX, glTF, and GLB readers/writers around `draco-core`.
- `draco-cpp-test-bridge`: unpublished test infrastructure for C++ parity.

## Development

Run the crate tests from the workspace:

```powershell
cargo test --manifest-path crates/Cargo.toml -p draco-core --all-features
```

Run the full Rust workspace:

```powershell
cargo test --manifest-path crates/Cargo.toml --all-features
```

## License

Apache-2.0, matching the upstream Draco project.
