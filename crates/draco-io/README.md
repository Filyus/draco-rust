# draco-io

[![Crates.io](https://img.shields.io/crates/v/draco-io.svg)](https://crates.io/crates/draco-io)
[![Docs.rs](https://docs.rs/draco-io/badge.svg)](https://docs.rs/draco-io)
[![Rust CI](https://github.com/Filyus/draco-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/Filyus/draco-rust/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/Filyus/draco-rust/blob/main/LICENSE)

A Rust library for reading and writing 3D mesh file formats with Draco compression support. Built on top of `draco-core`, this crate provides a unified API for working with OBJ, PLY, FBX, and glTF/GLB formats.

For the project overview, compatibility notes, and benchmarks, see the
[Draco Rust workspace README](https://github.com/Filyus/draco-rust).

## Features

- **Unified API**: Common `Reader` and `Writer` traits across all formats
- **In-Memory I/O**: Common `ReadFromBytes` and `WriteToBytes` traits
- **Draco Compression**: Full support for `KHR_draco_mesh_compression` in glTF/GLB
- **Document-Preserving Compression**: Draco-compress an existing glTF/GLB in
  place, keeping materials, textures, animations, `extras`, and other content
  (`compress_gltf_bytes`). Unknown JSON is retained, but extension fields with
  opaque buffer/view/offset references are rejected because they cannot be
  remapped safely.
- **Multiple Formats**: OBJ, PLY, FBX (ASCII/Binary), glTF (JSON/Binary/Embedded)
- **Scene Graph Support**: Read and write scene hierarchies with transforms
- **Point Cloud Support**: Read/write point clouds (OBJ, PLY)
- **Feature Flags**: Per-format reader and writer features

## Supported Formats

| Format | Read | Write | Draco Compression | Notes |
|--------|------|-------|-------------------|-------|
| OBJ    | ✓    | ✓     | -                 | Positions, normals, texcoords, named groups, point clouds |
| PLY    | ✓    | ✓     | -                 | ASCII/binary, normals, colors, per-vertex texcoords |
| FBX    | ✓    | ✓     | -                 | Binary 7.x, positions and triangle faces only |
| glTF   | ✓    | ✓     | ✓                 | JSON + separate .bin |
| GLB    | ✓    | ✓     | ✓                 | Binary container |

## Geometry Contract

`draco-io` is a geometry bridge for the `draco-core` data model, not a
general-purpose asset importer. The stable contract is:

- Meshes use triangle faces; readers triangulate polygon faces when the source
  format provides polygons, and reject unsupported primitive modes explicitly.
- `Position` is the required mesh attribute. `Normal`, `Color`, `TexCoord`, and
  `Generic` are preserved when the format can represent them as Draco
  attributes.
- Scene support is intentionally small: names, hierarchy, transforms, and mesh
  parts. Materials, textures, cameras, lights, animation, skinning, and
  arbitrary format extras are not exposed by the geometry model. The
  document-preserving glTF compressor carries them through without interpreting
  them when their binary references are understood.
- Writers should not silently claim to preserve attributes that the target
  format cannot encode. FBX writing currently accepts position-only meshes.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
draco-io = "0.2"
```

### Feature Flags

| Feature       | Default | Description                              |
|---------------|---------|------------------------------------------|
| `all-readers` | ✓       | Reading support for all formats          |
| `all-writers` | ✓       | Writing support for all formats          |
| `obj-reader`  | ✓       | OBJ reading support                      |
| `obj-writer`  | ✓       | OBJ writing support                      |
| `ply-reader`  | ✓       | PLY reading support                      |
| `ply-writer`  | ✓       | PLY writing support                      |
| `fbx-reader`  | ✓       | FBX reading support                      |
| `fbx-writer`  | ✓       | FBX writing support                      |
| `gltf-reader` | ✓       | glTF/GLB reading support                 |
| `gltf-writer` | ✓       | glTF/GLB writing support                 |
| `scene`       | ✓       | Scene graph API for hierarchical formats |
| `compression` | ✓       | zlib compression for FBX                 |
| `test`        | -       | Repository-only fixtures and local tools |

To use only one format direction (smaller binary):

```toml
[dependencies]
draco-io = { version = "0.2", default-features = false, features = ["gltf-reader"] }
```

## Quick Start

### Reading a Mesh

```rust
use draco_io::{Reader, ObjReader};
use std::io;

fn load_mesh(path: &str) -> io::Result<draco_core::mesh::Mesh> {
    let mut reader = ObjReader::open(path)?;
    reader.read_mesh()
}
```

### Writing a Mesh

```rust
use draco_io::{Writer, ObjWriter};
use draco_core::mesh::Mesh;
use std::io;

fn save_mesh(mesh: &Mesh, path: &str) -> io::Result<()> {
    let mut writer = ObjWriter::new();
    writer.add_mesh(mesh, Some("MyMesh"))?;
    writer.write(path)
}
```

### Generic Functions (Polymorphism)

Write format-agnostic code using the trait interface:

```rust
use draco_io::{Reader, Writer};
use draco_core::mesh::Mesh;
use std::io;

// Works with any reader implementation
fn load<R: Reader>(path: &str) -> io::Result<Mesh> {
    let mut reader = R::open(path)?;
    reader.read_mesh()
}

// Works with any writer implementation
fn save<W: Writer>(mut writer: W, mesh: &Mesh, path: &str) -> io::Result<()> {
    writer.add_mesh(mesh, Some("Model"))?;
    println!("Vertices: {}, Faces: {}", writer.vertex_count(), writer.face_count());
    writer.write(path)
}
```

### GLB with Draco Compression

```rust
use draco_io::gltf_writer::GltfWriter;
use draco_core::mesh::Mesh;

fn write_compressed_glb(mesh: &Mesh, path: &str) -> Result<(), draco_io::GltfWriteError> {
    let mut writer = GltfWriter::new();
    
    // Add mesh with Draco compression (uses default quantization)
    writer.add_draco_mesh(mesh, Some("CompressedMesh"), None)?;
    
    // Write as binary GLB
    writer.write_glb(path)
}
```

### Reading Draco-Compressed glTF

```rust
use draco_io::gltf_reader::GltfReader;

fn read_draco_glb(path: &str) -> Result<(), draco_io::GltfError> {
    let reader = GltfReader::open(path)?;
    
    // Decode all Draco-compressed meshes
    for (info, mesh) in reader.decode_all_draco_meshes()? {
        println!("Mesh '{}': {} faces, {} points",
            info.mesh_name.as_deref().unwrap_or("unnamed"),
            mesh.num_faces(),
            mesh.num_points());
    }
    Ok(())
}
```

## Unified Trait API

All readers and writers implement common traits for a consistent interface:

### Writer Trait

```rust
pub trait Writer: Sized {
    fn new() -> Self;
    fn add_mesh(&mut self, mesh: &Mesh, name: Option<&str>) -> io::Result<()>;
    fn write<P: AsRef<Path>>(&self, path: P) -> io::Result<()>;
    fn vertex_count(&self) -> usize;
    fn face_count(&self) -> usize;
}
```

### Reader Trait

```rust
pub trait Reader: Sized {
    fn open<P: AsRef<Path>>(path: P) -> io::Result<Self>;
    fn read_mesh(&mut self) -> io::Result<Mesh>;
    fn read_meshes(&mut self) -> io::Result<Vec<Mesh>>;
}
```

### In-Memory I/O Traits

```rust
pub trait ReadFromBytes: Sized {
    fn from_bytes(bytes: &[u8]) -> io::Result<Self>;
}

pub trait WriteToBytes: Writer {
    fn write_to_vec(&self) -> io::Result<Vec<u8>>;
    fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<()>;
}
```

### Scene Traits

For scene graph support with transforms and hierarchies. Scene APIs are in
`draco_io::scene` and re-exported from `draco_io` when the `scene` feature is
enabled. glTF and FBX readers expose native scene graphs; flat OBJ/PLY readers
can be wrapped explicitly with `flatten_to_scene`.

```rust
pub trait SceneReader: Reader {
    fn read_scene(&mut self) -> io::Result<Scene>;
}

pub trait SceneWriter: Writer {
    fn add_scene(&mut self, scene: &Scene) -> io::Result<()>;
}
```

## Format-Specific Features

### OBJ Writer

```rust
use draco_io::{ObjWriter, Writer, PointCloudWriter};

let mut obj = ObjWriter::new();

// Named object groups
obj.add_mesh(&mesh, Some("Cube"))?;

// Point clouds
obj.add_points(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);

obj.write("output.obj")?;
```

### PLY Writer

```rust
use draco_io::PlyWriter;

let mut ply = PlyWriter::new();
ply.set_binary_little_endian(true); // optional, ASCII is the default

// Add mesh
ply.add_mesh(&mesh, None)?;

// Or point cloud with colors
ply.add_points_with_colors(
    &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
    &[[255, 0, 0, 255], [0, 255, 0, 255]],
);

ply.write("output.ply")?;
```

### FBX Writer

```rust
use draco_io::FbxWriter;

let mut fbx = FbxWriter::new()
    .with_compression(true)           // Enable zlib output (default feature)
    .with_compression_threshold(1000); // Min size to compress

fbx.add_mesh(&mesh, Some("Model"))?;
fbx.write("output.fbx")?;
```

### glTF Writer

```rust
use draco_io::{GltfCompressionOptions, GltfWriter, QuantizationOptions};

let mut gltf = GltfWriter::new();

// Custom quantization settings
let options = GltfCompressionOptions {
    quantization: QuantizationOptions {
        position: Some(16),
        normal: Some(10),
        color: Some(8),
        texcoord: Some(12),
        generic: None, // Disable quantization for generic attributes.
    },
    ..GltfCompressionOptions::default()
};
gltf.add_draco_mesh(&mesh, Some("HighQuality"), Some(options))?;

// Multiple output formats:
gltf.write_glb("output.glb")?;                     // Binary GLB (single file)
gltf.write_gltf("out.gltf", "out.bin")?;           // JSON + separate binary
gltf.write_gltf_embedded("embedded.gltf")?;        // Pure text with base64
```

Default glTF compression is lossy (`14/10/8/12/8` quantization bits). A
`None` quantization value preserves that attribute class without quantization;
invalid bit/speed ranges are rejected rather than clamped.

### Document-Preserving glTF Compression

```rust
use draco_io::{compress_gltf_bytes_with_options, GltfCompressionOptions, OutputFormat};

let input = std::fs::read("scene.glb")?;
let options = GltfCompressionOptions {
    output_format: OutputFormat::SameAsInput,
    ..Default::default()
};
let output = compress_gltf_bytes_with_options(&input, &options)?;

std::fs::write("scene.draco.glb", output.data)?;
println!("compressed: {:?}", output.report.compressed_primitives);
println!("preserved: {:?}", output.report.preserved_primitives);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Valid but unsupported primitives are copied and listed with a typed
`PreserveReason`; malformed input is an error. `GltfEmbeddedBuffers` embeds the
consolidated glTF buffer, not external images. For companion files, use
`compress_gltf_bytes_with_resolver` with a `ResourceResolver` and optional
`ResourceLimits`, or the native base-path convenience function.

### glTF Reader with Scene Graph

```rust
use draco_io::{GltfReader, SceneReader};

let mut reader = GltfReader::open("scene.glb")?;
let scene = reader.read_scene()?;

// Traverse scene hierarchy
for node in &scene.root_nodes {
    println!("Node: {:?}", node.name);
    for part in &node.parts {
        println!("  Mesh: {:?}, {} faces", 
            part.name, 
            part.mesh.num_faces());
    }
}
```

## Examples

The crate includes several examples:

```bash
# Run unified API demo
cargo run --example unified_api

# Run polymorphic usage demo  
cargo run --example polymorphic

# Run glTF demo
cargo run --example gltf_demo

# Run FBX demo
cargo run --example fbx_demo
```

## Module Structure

```
draco-io
├── Traits
│   ├── traits::Writer       - Common writer interface
│   ├── traits::Reader       - Common reader interface
│   ├── traits::WriteToBytes - In-memory writer output
│   ├── traits::ReadFromBytes - In-memory reader input
│   ├── traits::PointCloudWriter - Point cloud writing
│   └── traits::PointCloudReader - Point cloud reading
│
├── Scene (feature = "scene")
│   ├── scene::SceneWriter   - Scene graph writing
│   ├── scene::SceneReader   - Scene graph reading
│   ├── scene::Scene         - Scene container
│   └── scene::flatten_to_scene - Flat mesh adapter
│
├── Readers (feature = "all-readers")
│   ├── obj_reader    - Wavefront OBJ
│   ├── ply_reader    - Stanford PLY
│   ├── fbx_reader    - Autodesk FBX
│   └── gltf_reader   - glTF/GLB with Draco support
│
├── Writers (feature = "all-writers")
│   ├── obj_writer    - Wavefront OBJ
│   ├── ply_writer    - Stanford PLY (ASCII/binary)
│   ├── fbx_writer    - Autodesk binary FBX
│   └── gltf_writer   - glTF/GLB with Draco compression
│
└── glTF geometry + Draco compression
    ├── gltf_geometry - Reader-agnostic geometry decode + shared glTF error type
    │                   (compiled with gltf-reader OR gltf-writer)
    └── gltf_compress - Document-preserving Draco compression. The in-memory
                        compress_gltf_value needs only gltf-writer; the byte API
                        compress_gltf_bytes also needs gltf-reader
```

The `gltf_geometry` split lets callers that already have a parsed glTF document
(for example via [`draco-gltf`](https://crates.io/crates/draco-gltf) on gltf-rs)
reuse the same geometry decoder and drive `compress_gltf_value` with only the
writer — never linking the glTF reader.

## Dependencies

- `draco-core` - Core compression/decompression
- `thiserror` - Error handling
- `byteorder` - Binary I/O
- `serde`, `serde_json` - glTF JSON parsing
- `miniz_oxide` (optional) - zlib compression for FBX

## License

Apache-2.0 (same as the original Draco library)

## See Also

- [draco-core](../draco-core) - Core compression library
- [API Reference](API.md) - Detailed API documentation
- [Google Draco](https://github.com/google/draco) - Original C++ implementation
