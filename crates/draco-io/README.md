# draco-io

A Rust library for reading and writing 3D mesh file formats with Draco compression support. Built on top of `draco-core`, this crate provides a unified API for working with OBJ, PLY, FBX, and glTF/GLB formats.

For the project overview, compatibility notes, and benchmarks, see the
[Draco Rust workspace README](https://github.com/Filyus/draco-rust).

## Features

- **Unified API**: Common `Reader` and `Writer` traits across all formats
- **Draco Compression**: Full support for `KHR_draco_mesh_compression` in glTF/GLB
- **Multiple Formats**: OBJ, PLY, FBX (ASCII/Binary), glTF (JSON/Binary/Embedded)
- **Scene Graph Support**: Read and write scene hierarchies with transforms
- **Point Cloud Support**: Read/write point clouds (OBJ, PLY)
- **Feature Flags**: Separate `encoder` and `decoder` features

## Supported Formats

| Format | Read | Write | Draco Compression | Notes |
|--------|------|-------|-------------------|-------|
| OBJ    | ✓    | ✓     | -                 | Named groups, point clouds |
| PLY    | ✓    | ✓     | -                 | ASCII, vertex colors |
| FBX    | ✓    | ✓     | -                 | ASCII format, optional zlib |
| glTF   | ✓    | ✓     | ✓                 | JSON + separate .bin |
| GLB    | ✓    | ✓     | ✓                 | Binary container |

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
draco-io = { version = "0.1.0", path = "../draco-io" }
```

### Feature Flags

| Feature       | Default | Description                              |
|---------------|---------|------------------------------------------|
| `encoder`     | ✓       | Writing support (all formats)            |
| `decoder`     | ✓       | Reading support (all formats)            |
| `compression` | ✓       | zlib compression for FBX                 |

To use only reading (smaller binary):

```toml
[dependencies]
draco-io = { version = "0.1.0", default-features = false, features = ["decoder"] }
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

### Scene Traits

For scene graph support with transforms and hierarchies:

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
use draco_io::gltf_writer::{GltfWriter, QuantizationBits};

let mut gltf = GltfWriter::new();

// Custom quantization settings
let quant = QuantizationBits {
    position: 14,
    normal: 10,
    color: 8,
    texcoord: 12,
    generic: 8,
};
gltf.add_draco_mesh(&mesh, Some("HighQuality"), Some(quant))?;

// Multiple output formats:
gltf.write_glb("output.glb")?;                     // Binary GLB (single file)
gltf.write_gltf("out.gltf", "out.bin")?;           // JSON + separate binary
gltf.write_gltf_embedded("embedded.gltf")?;        // Pure text with base64
```

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
│   ├── traits::SceneWriter  - Scene graph writing
│   ├── traits::SceneReader  - Scene graph reading
│   ├── traits::PointCloudWriter - Point cloud writing
│   └── traits::PointCloudReader - Point cloud reading
│
├── Readers (feature = "decoder")
│   ├── obj_reader    - Wavefront OBJ
│   ├── ply_reader    - Stanford PLY
│   ├── fbx_reader    - Autodesk FBX
│   └── gltf_reader   - glTF/GLB with Draco support
│
└── Writers (feature = "encoder")
    ├── obj_writer    - Wavefront OBJ
    ├── ply_writer    - Stanford PLY (ASCII)
    ├── fbx_writer    - Autodesk FBX (ASCII/compressed)
    └── gltf_writer   - glTF/GLB with Draco compression
```

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
