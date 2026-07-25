# draco-io

[![Crates.io](https://img.shields.io/crates/v/draco-io.svg)](https://crates.io/crates/draco-io)
[![Docs.rs](https://docs.rs/draco-io/badge.svg)](https://docs.rs/draco-io)
[![Rust CI](https://github.com/Filyus/draco-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/Filyus/draco-rust/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/Filyus/draco-rust/blob/main/LICENSE)

`draco-io` is the low-level format I/O layer for the Draco Rust workspace. It
reads and writes OBJ, PLY, and binary FBX geometry and provides strict glTF/GLB
container, resource, accessor, and Draco-geometry contracts.

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
| FBX | Yes | Yes | Binary FBX 7.x scene data. See the FBX matrix below. |
| glTF / GLB | Containers and geometry contracts | Containers | GLB inspection, JSON/bin extraction, resource resolution, accessors, and optional `KHR_draco_mesh_compression` geometry decode. Full-document operations belong to `draco-gltf`. |

All mesh formats use the `draco-core` geometry model. `Position` is required;
the supported optional attributes depend on the source and destination format.
Writers do not claim to preserve data that their target cannot represent.

### FBX support

Binary FBX only; the ASCII container is rejected. The container is read for
versions 6000 through 8000, in either byte order — a non-zero endian marker
selects big-endian, as `ufbx` does. Output is FBX 7500 little-endian.

Scene content, however, is read only from **FBX 7000 and later**. Earlier
versions use a different object model: objects are identified by a
`"name\0\x01Class"` string instead of an `i64` id, connections reference those
strings, geometry lives on the `Model` rather than on a separate `Geometry`,
and array payloads are stored as repeated scalar properties. A pre-7000
document therefore decodes to a structurally valid but empty scene, and says so
through `FbxWarningCode::NameKeyedObjectModel` rather than looking like a file
that simply has no meshes. 81 of the 308 binary files in the `ufbx` corpus are
version 6100 and fall in this category.

| Data | Read | Write |
| --- | :---: | :---: |
| Mesh geometry | Yes | Yes |
| Normals and UV layers | Yes | Yes |
| Vertex colors | Yes | Yes |
| Tangents and binormals | Yes | Yes |
| `Edges` array | Yes | Yes |
| Edge smoothing (`LayerElementSmoothing`) | Yes | Yes |
| Edge and vertex creases | Yes | Yes |
| Mesh and model names | Yes | Yes |
| Node hierarchy | Yes | Yes |
| Node transforms | Yes | Yes |
| Materials and textures | Yes | Yes |
| Node-TRS animation | Yes | Yes |
| Multiple animation layers | Yes | Yes |
| Animation layer blending | No | No |
| Skins, bind poses, and influences | Yes | Yes |
| Blend shapes / morph targets | Yes | Yes |
| `Definitions` property templates | No | n/a |

Layer elements are resolved on the polygon-corner domain, so a UV or hard-normal
seam survives instead of being averaged onto its control point. The Draco mesh
welds corners that agree on every attribute, which keeps seams while collapsing
interior duplicates. Every UV, normal, colour and tangent set is preserved on
`FbxMeshInstance`; only the first of each reaches the Draco mesh, since Draco
has no concept of multiple sets.

Tangents are stored as four components, with the handedness sign in `w` -- the
layout glTF's `TANGENT` uses. FBX itself splits them across two sibling arrays,
`Tangents` and a `TangentsW` that only 7500 and later write, so a set records
whether its handedness was authored or defaulted to `+1`; the writer emits the
sibling array only when it was. Binormals are read and written for the same
reason `Edges` is kept raw -- they are the only carrier of tangent sign in files
that have no `TangentsW` -- but they stop at `FbxMeshInstance`, since glTF has
no binormal to lower them onto.

Draco's `GeometryAttributeType` has no tangent, so tangents never enter the
Draco mesh or its weld key; they travel on `FbxMeshInstance` and
`FbxRenderMesh`, the same route extra UV sets take.

`Edges` is kept verbatim rather than normalized: FBX does not require it to list
every topological edge, and importers reconstruct the rest from faces, so
discarding the distinction would lose information. It is also the domain
`ByEdge` layers address.

Smoothing flags and crease weights address edges, polygons or control points --
never polygon corners -- so they are preserved raw on `FbxMeshInstance` beside
`Edges` rather than resolved onto the render mesh, and they have separate types
because smoothing is an integer flag while a crease is a floating-point weight
that an integer would flatten. glTF has no equivalent for either, so they
survive an FBX-to-FBX rewrite and travel no further.

A layer whose length disagrees with the domain its mapping names is dropped with
a warning instead of being kept as misaligned data; seven layers in the corpus
are in that state. A `ByEdge` layer in a geometry that has no `Edges` array is a
separate case -- it addresses the edges an importer would reconstruct, which
this crate does not do -- so it is preserved unchecked rather than discarded.

Property templates in `Definitions` are not resolved. The specification allows a
property to be omitted from an object and supplied by its class template, but
across 272 binary files in the `ufbx` corpus no `Material` or `Model` relies on
that for any property this crate reads.

FBX materials cover the canonical Phong/Lambert property set (`DiffuseColor`,
`SpecularFactor`, `Shininess`, `EmissiveColor`/`EmissiveFactor`,
`ReflectionFactor`, `TransparencyFactor`/`Opacity`, `BumpFactor`) with diffuse,
normal, and emissive textures (embedded `Content` or external filename), and
per-polygon material indices. Animation resolves the
`AnimationStack → AnimationLayer → AnimationCurveNode → AnimationCurve` graph
into per-node TRS channels in seconds, one clip per layer — the same choice
Blender's importer makes. Layers are not blended. Cameras, lights and other
`NodeAttribute` objects are not read at all: a node carrying one keeps its
transform and hierarchy, but the attribute itself is absent from the scene and
is not currently reported. Scene export preserves
local affine translation, rotation, scale, skins, bind poses, morph targets,
and authored animation channels. FBX pivot settings and inheritance rules are
not represented by `FbxTransform`.

Decoding a document twice gives the same result: object order, animation
channel order and bind-pose resolution follow FBX object ids rather than hash
iteration.

### Reading untrusted input

FBX is a length-prefixed binary container with a decompression path, so
`FbxReadOptions` bounds what one document may allocate and how strictly its
layout is enforced:

```rust
use draco_io::{FbxDecodeLimits, FbxReadOptions, FbxScene};

let options = FbxReadOptions::default()
    .with_limits(FbxDecodeLimits::default().with_max_blob_bytes(16 << 20));
let scene = FbxScene::from_bytes_with_options(&bytes, options)?;
# Ok::<(), std::io::Error>(())
```

Limit violations fail with `ErrorKind::OutOfMemory` and structural violations
with `ErrorKind::InvalidData`, so a caller can tell "too big, retry with
`FbxDecodeLimits::permissive()`" from "corrupt". The defaults are calibrated
against real assets, not guessed; see `FbxDecodeLimits::default`.

`FbxReadOptions::strict()` additionally rejects anything the container layout
does not permit, including a malformed binary footer. It is off by default
because shipping exporters emit slop that every practical reader tolerates:
222 of 308 real files in the `ufbx` corpus do not begin their trailing region
with the conventional footer id at all. Deviations accepted in the default mode
are reported through `FbxScene::warnings` as typed `FbxWarningCode` values
rather than passing silently.

The web converter adds a source-neutral `SceneDocument` adapter above this
crate. That adapter preserves extra UV sets and up to eight skin influences in
its GLB and typed-FBX paths; its WebGL preview reports when it uses only the
first four influences. Vertex colours travel end to end as `COLOR_0`; tangents
remain lossless in SceneDocument/glTF but are reported as unsupported when
lowering to the typed FBX writer. Non-default `RotationOrder`/`InheritType`
behavior remains unvalidated beyond the Mixamo, Samba Dancing, and Fox
controls.

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
| `all-readers` / `all-writers` | Yes | Enable all OBJ, PLY, and FBX readers or writers. |
| `obj-reader` / `obj-writer` | Yes | Wavefront OBJ support. |
| `ply-reader` / `ply-writer` | Yes | Stanford PLY support. |
| `fbx-reader` / `fbx-writer` | Yes | Binary FBX support. |
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
