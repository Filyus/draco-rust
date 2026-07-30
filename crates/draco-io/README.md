# draco-io

[![Crates.io](https://img.shields.io/crates/v/draco-io.svg)](https://crates.io/crates/draco-io)
[![Docs.rs](https://docs.rs/draco-io/badge.svg)](https://docs.rs/draco-io)
[![Rust CI](https://github.com/Filyus/draco-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/Filyus/draco-rust/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/Filyus/draco-rust/blob/main/LICENSE)

`draco-io` is the low-level format I/O layer for the Draco Rust workspace. It
reads and writes OBJ, PLY, STL and FBX geometry — each in both of its containers
where it has two — and provides strict glTF/GLB container, resource, accessor,
and Draco-geometry contracts.

For complete, lossless glTF documents, scene preservation, and
document-preserving Draco compression, use
[`draco-gltf`](https://crates.io/crates/draco-gltf). Those APIs deliberately do
not live in this crate.

## Installation

```toml
[dependencies]
draco-io = "0.3"
```

To keep a binary small, disable the default format readers and writers and
enable only the features required by the application:

```toml
[dependencies]
draco-io = { version = "0.3", default-features = false, features = ["obj-reader", "obj-writer"] }
```

## Supported formats

| Format | Read | Write | Scope |
| --- | :---: | :---: | --- |
| OBJ | Yes | Yes | Meshes, normals, texture coordinates, named groups, and point clouds. |
| PLY | Yes | Yes | ASCII and binary geometry, normals, colors, and point clouds. |
| STL | Yes | Yes | Binary and ASCII triangles. No indices or attributes: the format stores unshared corners and a facet normal, and nothing else. |
| FBX | Yes | Yes | Binary and ASCII FBX 7.x scene data: geometry, layers, materials, skins, morphs, cameras, lights and animation. See [FBX.md](FBX.md). |
| glTF / GLB | Containers and geometry contracts | Containers | GLB inspection, JSON/bin extraction, resource resolution, accessors, and optional `KHR_draco_mesh_compression` geometry decode. Full-document operations belong to `draco-gltf`. |

All mesh formats use the `draco-core` geometry model. `Position` is required;
the supported optional attributes depend on the source and destination format.
Writers do not claim to preserve data that their target cannot represent.

### FBX

Both containers are read and both are written, and either decodes to the same
node tree, so nothing above the container knows which one it was given.

The detail is in **[FBX.md](FBX.md)**: accepted versions and byte orders, the
matrix of what survives a read and a write, the decode limits an untrusted
document is held to, and the corpus each claim is measured against.

## Quick start

Read an OBJ mesh:

```rust
use draco_io::{ObjReader, Reader};
use std::path::Path;

fn print_mesh_info(path: impl AsRef<Path>) -> std::io::Result<()> {
    let mut reader = ObjReader::open(path)?;
    let mesh = reader.read_mesh()?;
    println!("{} points, {} faces", mesh.num_points(), mesh.num_faces());
    Ok(())
}
```

Convert an OBJ mesh to binary FBX:

```rust
use draco_io::{FbxWriter, ObjReader, Reader, Writer};
use std::path::Path;

fn convert_obj_to_fbx(input: impl AsRef<Path>, output: impl AsRef<Path>) -> std::io::Result<()> {
    let mut reader = ObjReader::open(input)?;
    let mesh = reader.read_mesh()?;
    let mut writer = FbxWriter::new();
    writer.add_mesh(&mesh, Some("Model"))?;
    writer.write(output)
}
```

Read and re-write a supported FBX scene while keeping its mesh hierarchy and
local transforms:

```rust,no_run
use draco_io::FbxScene;

let scene = FbxScene::from_bytes(&std::fs::read("input.fbx")?)?;
std::fs::write("output.fbx", scene.to_bytes()?)?;
# Ok::<(), std::io::Error>(())
```

Use `FbxReader::read_scene` and `FbxWriter::add_scene` when reading from a
stream or configuring FBX array compression.

For format-agnostic use, `Reader`, `Writer`, `ReadFromBytes`, and
`WriteToBytes` provide the common I/O traits. The complete API is documented at
[docs.rs/draco-io](https://docs.rs/draco-io).

## Feature flags

| Feature | Default | Purpose |
| --- | :---: | --- |
| `all-readers` / `all-writers` | Yes | Enable all OBJ, PLY, STL, and FBX readers or writers. |
| `obj-reader` / `obj-writer` | Yes | Wavefront OBJ support. |
| `ply-reader` / `ply-writer` | Yes | Stanford PLY support. |
| `stl-reader` / `stl-writer` | Yes | STL support, binary and ASCII. |
| `fbx-reader` / `fbx-writer` | Yes | FBX support, binary and ASCII. |
| `gltf-container` | No | Parse glTF/GLB and load referenced buffers; no mesh decoding. |
| `gltf-geometry` | No | Convert ordinary glTF accessors into `draco-core` meshes. |
| `draco-decode` | No | Add `KHR_draco_mesh_compression` primitive decoding. |
| `legacy-bitstream-decode` | No | Decode older Draco bitstreams. |
| `compression` | Yes | zlib compression for FBX output. |
| `point_cloud_decode` | Yes | Point-cloud decoding in `draco-core`. |

## Relationship to `draco-gltf`

Use `draco-io` when an application needs a strict GLB/container parser,
resource-resolution policy, or low-level accessor geometry. Use `draco-gltf`
when it needs a full scene document, nodes, materials, animations, skins, or
document-preserving Draco transforms. Keeping this boundary explicit prevents
low-level tooling from accidentally promising full glTF round-tripping.

## License

Apache-2.0.
