# draco-io API Reference

Complete API documentation for the draco-io crate.

## Table of Contents

- [draco-io API Reference](#draco-io-api-reference)
  - [Table of Contents](#table-of-contents)
  - [Traits](#traits)
    - [Writer](#writer)
    - [Reader](#reader)
    - [PointCloudWriter](#pointcloudwriter)
    - [PointCloudReader](#pointcloudreader)
    - [SceneWriter](#scenewriter)
    - [SceneReader](#scenereader)
  - [Scene Types](#scene-types)
    - [Scene](#scene)
    - [SceneNode](#scenenode)
    - [SceneObject](#sceneobject)
    - [Transform](#transform)
  - [OBJ Format](#obj-format)
    - [ObjReader](#objreader)
    - [ObjWriter](#objwriter)
  - [PLY Format](#ply-format)
    - [PlyReader](#plyreader)
    - [PlyWriter](#plywriter)
  - [FBX Format](#fbx-format)
    - [FbxReader](#fbxreader)
    - [FbxWriter](#fbxwriter)
  - [glTF/GLB Format](#gltfglb-format)
    - [GltfReader](#gltfreader)
    - [GltfWriter](#gltfwriter)
    - [QuantizationBits](#quantizationbits)
    - [DracoPrimitiveInfo](#dracoprimitiveinfo)
  - [Error Types](#error-types)
    - [GltfError](#gltferror)
    - [GltfWriteError](#gltfwriteerror)
  - [Complete Examples](#complete-examples)
    - [Round-Trip: Read OBJ, Write GLB with Draco](#round-trip-read-obj-write-glb-with-draco)
    - [Multi-Format Export](#multi-format-export)
    - [Scene Graph Export](#scene-graph-export)

---

## Traits

### Writer

Common interface for all mesh writers.

```rust
pub trait Writer: Sized {
    /// Create a new writer instance.
    fn new() -> Self;

    /// Add a mesh to be written.
    fn add_mesh(&mut self, mesh: &Mesh, name: Option<&str>) -> io::Result<()>;

    /// Write all added meshes to a file.
    fn write<P: AsRef<Path>>(&self, path: P) -> io::Result<()>;

    /// Get the number of vertices added.
    fn vertex_count(&self) -> usize;

    /// Get the number of faces added.
    fn face_count(&self) -> usize;
}
```

**Usage:**

```rust
use draco_io::{Writer, ObjWriter, PlyWriter, FbxWriter, GltfWriter};

// Generic function works with any writer
fn write_mesh<W: Writer>(mut writer: W, mesh: &Mesh, path: &str) -> io::Result<()> {
    writer.add_mesh(mesh, Some("Model"))?;
    println!("Vertices: {}, Faces: {}", writer.vertex_count(), writer.face_count());
    writer.write(path)
}

// Works with any format
write_mesh(ObjWriter::new(), &mesh, "out.obj")?;
write_mesh(PlyWriter::new(), &mesh, "out.ply")?;
write_mesh(FbxWriter::new(), &mesh, "out.fbx")?;
write_mesh(GltfWriter::new(), &mesh, "out.glb")?;
```

---

### Reader

Common interface for all mesh readers.

```rust
pub trait Reader: Sized {
    /// Open a file for reading.
    fn open<P: AsRef<Path>>(path: P) -> io::Result<Self>;

    /// Read multiple meshes from the file.
    fn read_meshes(&mut self) -> io::Result<Vec<Mesh>>;

    /// Read a single mesh (returns first mesh).
    fn read_mesh(&mut self) -> io::Result<Mesh>;
}
```

**Usage:**

```rust
use draco_io::{Reader, ObjReader, PlyReader, FbxReader, GltfReader};

// Generic function works with any reader
fn load_mesh<R: Reader>(path: &str) -> io::Result<Mesh> {
    let mut reader = R::open(path)?;
    reader.read_mesh()
}

// Works with any format
let mesh = load_mesh::<ObjReader>("model.obj")?;
let mesh = load_mesh::<PlyReader>("model.ply")?;
let mesh = load_mesh::<FbxReader>("model.fbx")?;
let mesh = load_mesh::<GltfReader>("model.glb")?;
```

---

### PointCloudWriter

Extended trait for writing point clouds (without faces).

```rust
pub trait PointCloudWriter: Writer {
    /// Add raw point positions.
    fn add_points(&mut self, points: &[[f32; 3]]);

    /// Add a single point.
    fn add_point(&mut self, point: [f32; 3]);
}
```

**Implementations:** `ObjWriter`, `PlyWriter`

```rust
use draco_io::{ObjWriter, PointCloudWriter, Writer};

let mut writer = ObjWriter::new();
writer.add_points(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
writer.write("points.obj")?;
```

---

### PointCloudReader

Extended trait for reading point clouds.

```rust
pub trait PointCloudReader: Reader {
    /// Read point positions only (no faces).
    fn read_points(&mut self) -> io::Result<Vec<[f32; 3]>>;
}
```

---

### SceneWriter

Extended trait for writing scene graphs with hierarchies and transforms.

```rust
pub trait SceneWriter: Writer {
    /// Add a scene graph to be written.
    fn add_scene(&mut self, scene: &Scene) -> io::Result<()>;

    /// Add multiple scenes.
    fn add_scenes(&mut self, scenes: &[Scene]) -> io::Result<()>;
}
```

**Implementations:** `GltfWriter`, `FbxWriter`

```rust
use draco_io::{GltfWriter, SceneWriter, Writer};
use draco_io::traits::{Scene, SceneNode, SceneObject};

let mut root = SceneNode::new(Some("Root".to_string()));
root.parts.push(SceneObject {
    name: Some("Mesh".to_string()),
    mesh,
    transform: None,
});

let scene = Scene {
    name: Some("MyScene".to_string()),
    parts: vec![],
    root_nodes: vec![root],
};

let mut writer = GltfWriter::new();
writer.add_scene(&scene)?;
writer.write("scene.glb")?;
```

---

### SceneReader

Extended trait for reading scene graphs.

```rust
pub trait SceneReader: Reader {
    /// Read a single scene with full hierarchy.
    fn read_scene(&mut self) -> io::Result<Scene>;

    /// Read all scenes.
    fn read_scenes(&mut self) -> io::Result<Vec<Scene>>;
}
```

**Implementations:** `GltfReader`, `FbxReader`

```rust
use draco_io::{GltfReader, SceneReader, Reader};

let mut reader = GltfReader::open("scene.glb")?;
let scene = reader.read_scene()?;

for node in &scene.root_nodes {
    println!("Node: {:?}", node.name);
    for child in &node.children {
        println!("  Child: {:?}", child.name);
    }
}
```

---

## Scene Types

### Scene

Container for a complete scene graph.

```rust
pub struct Scene {
    /// Optional scene name.
    pub name: Option<String>,
    
    /// Flat list of parts (for convenience).
    pub parts: Vec<SceneObject>,
    
    /// Root nodes forming the hierarchy.
    pub root_nodes: Vec<SceneNode>,
}
```

---

### SceneNode

A node in the scene graph hierarchy.

```rust
pub struct SceneNode {
    /// Optional node name.
    pub name: Option<String>,
    
    /// Optional transform.
    pub transform: Option<Transform>,
    
    /// Mesh parts attached to this node.
    pub parts: Vec<SceneObject>,
    
    /// Child nodes.
    pub children: Vec<SceneNode>,
}

impl SceneNode {
    pub fn new(name: Option<String>) -> Self;
}
```

---

### SceneObject

A mesh with optional metadata.

```rust
pub struct SceneObject {
    /// Optional object name.
    pub name: Option<String>,
    
    /// The mesh data.
    pub mesh: Mesh,
    
    /// Optional transform.
    pub transform: Option<Transform>,
}
```

---

### Transform

4x4 transformation matrix (row-major).

```rust
pub struct Transform {
    pub matrix: [[f32; 4]; 4],
}
```

---

## OBJ Format

### ObjReader

Reads Wavefront OBJ files.

```rust
use draco_io::{ObjReader, Reader};

let mut reader = ObjReader::open("model.obj")?;
let mesh = reader.read_mesh()?;
```

**Implements:** `Reader`, `PointCloudReader`

**Supported Features:**
- Vertex positions (`v`)
- Texture coordinates (`vt`)
- Vertex normals (`vn`)
- Faces (`f`)
- Object groups (`o`, `g`)
- Point clouds (faces with single vertex)

---

### ObjWriter

Writes Wavefront OBJ files.

```rust
use draco_io::{ObjWriter, Writer, PointCloudWriter};

let mut writer = ObjWriter::new();

// Add mesh with named group
writer.add_mesh(&mesh, Some("Cube"))?;

// Add point cloud
writer.add_points(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);
writer.add_point([0.5, 0.5, 0.5]);

writer.write("output.obj")?;
```

**Implements:** `Writer`, `PointCloudWriter`

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> ObjWriter` | Create new writer |
| `add_mesh(&mut self, mesh, name)` | Add mesh with optional group name |
| `add_points(&mut self, points)` | Add point positions |
| `add_point(&mut self, point)` | Add single point |
| `write(&self, path)` | Write to file |
| `vertex_count() -> usize` | Total vertices |
| `face_count() -> usize` | Total faces |

---

## PLY Format

### PlyReader

Reads Stanford PLY files (ASCII and binary little-endian formats).

```rust
use draco_io::{PlyReader, Reader};

let mut reader = PlyReader::open("model.ply")?;
let mesh = reader.read_mesh()?;
```

**Implements:** `Reader`, `PointCloudReader`

**Supported Features:**
- Vertex positions (x, y, z)
- Vertex colors (red, green, blue, alpha)
- Faces (vertex_indices)
- ASCII format
- Binary little-endian format

---

### PlyWriter

Writes Stanford PLY files (ASCII by default, or binary little-endian when enabled).

```rust
use draco_io::{PlyWriter, Writer};

let mut writer = PlyWriter::new();
writer.set_binary_little_endian(true);

// Add mesh
writer.add_mesh(&mesh, None)?;

// Or add points with colors
writer.add_points_with_colors(
    &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
    &[[255, 0, 0, 255], [0, 255, 0, 255]],
);

println!("Has colors: {}", writer.has_colors());
writer.write("output.ply")?;
```

**Implements:** `Writer`, `PointCloudWriter`

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> PlyWriter` | Create new writer |
| `with_binary_little_endian(self) -> PlyWriter` | Enable binary little-endian output |
| `set_binary_little_endian(&mut self, enabled)` | Toggle binary little-endian output |
| `add_mesh(&mut self, mesh, name)` | Add mesh (name ignored) |
| `add_points(&mut self, points)` | Add point positions |
| `add_points_with_colors(&mut self, points, colors)` | Add colored points |
| `has_colors() -> bool` | Check if colors present |
| `write(&self, path)` | Write to file |

---

## FBX Format

### FbxReader

Reads Autodesk FBX files (ASCII format).

```rust
use draco_io::{FbxReader, Reader, SceneReader};

let mut reader = FbxReader::open("model.fbx")?;

// Read single mesh
let mesh = reader.read_mesh()?;

// Or read full scene graph
let scene = reader.read_scene()?;
```

**Implements:** `Reader`, `SceneReader`

**Supported Features:**
- Geometry nodes
- Model hierarchy
- Basic transforms
- ASCII format

---

### FbxWriter

Writes Autodesk FBX files (ASCII format).

```rust
use draco_io::{FbxWriter, Writer, SceneWriter};

// Basic usage
let mut writer = FbxWriter::new();
writer.add_mesh(&mesh, Some("Model"))?;
writer.write("output.fbx")?;

// With compression (enabled by the default "compression" feature)
let mut writer = FbxWriter::new()
    .with_compression(true)
    .with_compression_threshold(1000);

writer.add_mesh(&mesh, Some("CompressedModel"))?;
writer.write("compressed.fbx")?;
```

**Implements:** `Writer`, `SceneWriter`

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> FbxWriter` | Create new writer |
| `with_compression(enabled: bool) -> Self` | Enable zlib compression |
| `with_compression_threshold(bytes: usize) -> Self` | Min size to compress |
| `is_compression_enabled() -> bool` | Check compression status |
| `add_mesh(&mut self, mesh, name)` | Add mesh |
| `add_scene(&mut self, scene)` | Add scene graph |
| `write(&self, path)` | Write to file |

---

## glTF/GLB Format

### GltfReader

Reads glTF 2.0 and GLB files with Draco compression support.

```rust
use draco_io::{GltfReader, Reader, SceneReader};

let mut reader = GltfReader::open("model.glb")?;

// Read single mesh
let mesh = reader.read_mesh()?;

// Read all meshes
let meshes = reader.read_meshes()?;

// Read scene graph with transforms
let scene = reader.read_scene()?;
```

**Implements:** `Reader`, `SceneReader`

**Additional Methods:**

| Method | Description |
|--------|-------------|
| `decode_all_meshes() -> Result<Vec<Mesh>>` | Decode all meshes (Draco and non-Draco) |
| `decode_all_draco_meshes() -> Result<Vec<(DracoPrimitiveInfo, Mesh)>>` | Decode only Draco-compressed meshes |
| `has_draco_extension() -> bool` | Check for Draco extension |

```rust
use draco_io::gltf_reader::GltfReader;

let reader = GltfReader::open("model.glb")?;

// Get Draco-compressed meshes with metadata
for (info, mesh) in reader.decode_all_draco_meshes()? {
    println!("Mesh: {:?}", info.mesh_name);
    println!("  Primitive: {}", info.primitive_index);
    println!("  Faces: {}", mesh.num_faces());
}
```

---

### GltfWriter

Writes glTF 2.0 and GLB files with Draco compression.

```rust
use draco_io::gltf_writer::{GltfWriter, QuantizationBits};

let mut writer = GltfWriter::new();

// Add mesh with default quantization
writer.add_draco_mesh(&mesh, Some("Model"), None)?;

// Or with custom quantization
let quant = QuantizationBits {
    position: 16,  // Higher = more precision
    normal: 10,
    color: 8,
    texcoord: 12,
    generic: 8,
};
writer.add_draco_mesh(&mesh, Some("HighQuality"), Some(quant))?;

// Multiple output formats
writer.write_glb("output.glb")?;              // Binary GLB
writer.write_gltf("out.gltf", "out.bin")?;    // JSON + binary
writer.write_gltf_embedded("embedded.gltf")?; // Pure text + base64
```

**Implements:** `Writer`, `SceneWriter`

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> GltfWriter` | Create new writer |
| `add_mesh(&mut self, mesh, name)` | Add mesh (uses default 14-bit quantization) |
| `add_draco_mesh(&mut self, mesh, name, quant)` | Add mesh with custom quantization |
| `add_scene(&mut self, scene)` | Add scene graph |
| `write(&self, path)` | Write GLB (default) |
| `write_glb(&self, path)` | Write binary GLB |
| `write_gltf(&self, json_path, bin_path)` | Write JSON + separate binary |
| `write_gltf_embedded(&self, path)` | Write JSON with embedded base64 |

---

### QuantizationBits

Configures Draco quantization precision per attribute type.

```rust
#[derive(Clone, Copy, Debug)]
pub struct QuantizationBits {
    pub position: i32,   // Default: 14
    pub normal: i32,     // Default: 10
    pub color: i32,      // Default: 8
    pub texcoord: i32,   // Default: 12
    pub generic: i32,    // Default: 8
}

impl Default for QuantizationBits {
    fn default() -> Self {
        Self {
            position: 14,
            normal: 10,
            color: 8,
            texcoord: 12,
            generic: 8,
        }
    }
}
```

**Quantization Guidelines:**

| Bits | Precision | Use Case |
|------|-----------|----------|
| 8-10 | Low | Mobile, small files |
| 12-14 | Medium | General use (default) |
| 16-18 | High | CAD, precision work |
| 20+ | Very High | Scientific data |

---

### DracoPrimitiveInfo

Metadata about a Draco-compressed primitive.

```rust
pub struct DracoPrimitiveInfo {
    /// Index of the glTF mesh.
    pub mesh_index: usize,
    
    /// Index of the primitive within the mesh.
    pub primitive_index: usize,
    
    /// Mesh name (if available).
    pub mesh_name: Option<String>,
    
    /// Byte offset into buffer view.
    pub buffer_view: usize,
}
```

---

## Error Types

### GltfError

Errors from glTF reading operations.

```rust
#[derive(Error, Debug)]
pub enum GltfError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid GLB: {0}")]
    InvalidGlb(String),

    #[error("Invalid glTF: {0}")]
    InvalidGltf(String),

    #[error("Draco decode error: {0}")]
    DracoDecode(String),

    #[error("Unsupported feature: {0}")]
    Unsupported(String),
}
```

### GltfWriteError

Errors from glTF writing operations.

```rust
#[derive(Error, Debug)]
pub enum GltfWriteError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON serialize error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Draco encode error: {0}")]
    DracoEncode(String),

    #[error("Invalid mesh: {0}")]
    InvalidMesh(String),
}
```

---

## Complete Examples

### Round-Trip: Read OBJ, Write GLB with Draco

```rust
use draco_io::{ObjReader, Reader};
use draco_io::gltf_writer::GltfWriter;

fn convert_obj_to_glb(input: &str, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Read OBJ
    let mut reader = ObjReader::open(input)?;
    let mesh = reader.read_mesh()?;
    
    println!("Read {} faces from OBJ", mesh.num_faces());
    
    // Write GLB with Draco compression
    let mut writer = GltfWriter::new();
    writer.add_draco_mesh(&mesh, Some("ConvertedMesh"), None)?;
    writer.write_glb(output)?;
    
    println!("Wrote compressed GLB");
    Ok(())
}
```

### Multi-Format Export

```rust
use draco_io::{Writer, ObjWriter, PlyWriter, FbxWriter, GltfWriter};
use draco_core::mesh::Mesh;

fn export_all_formats(mesh: &Mesh, basename: &str) -> std::io::Result<()> {
    // OBJ
    let mut obj = ObjWriter::new();
    obj.add_mesh(mesh, Some("Model"))?;
    obj.write(&format!("{}.obj", basename))?;
    
    // PLY
    let mut ply = PlyWriter::new();
    ply.add_mesh(mesh, None)?;
    ply.write(&format!("{}.ply", basename))?;
    
    // FBX
    let mut fbx = FbxWriter::new();
    fbx.add_mesh(mesh, Some("Model"))?;
    fbx.write(&format!("{}.fbx", basename))?;
    
    // GLB
    let mut gltf = GltfWriter::new();
    gltf.add_mesh(mesh, Some("Model"))?;
    gltf.write(&format!("{}.glb", basename))?;
    
    Ok(())
}
```

### Scene Graph Export

```rust
use draco_io::{GltfWriter, SceneWriter, Writer};
use draco_io::traits::{Scene, SceneNode, SceneObject, Transform};
use draco_core::mesh::Mesh;

fn export_scene(meshes: Vec<Mesh>) -> Result<(), draco_io::GltfWriteError> {
    // Build scene graph
    let mut root = SceneNode::new(Some("Root".to_string()));
    
    for (i, mesh) in meshes.into_iter().enumerate() {
        let mut child = SceneNode::new(Some(format!("Object_{}", i)));
        child.parts.push(SceneObject {
            name: Some(format!("Mesh_{}", i)),
            mesh,
            transform: Some(Transform {
                matrix: [
                    [1.0, 0.0, 0.0, i as f32 * 2.0],  // Translate along X
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
            }),
        });
        root.children.push(child);
    }
    
    let scene = Scene {
        name: Some("MyScene".to_string()),
        parts: vec![],
        root_nodes: vec![root],
    };
    
    // Export
    let mut writer = GltfWriter::new();
    writer.add_scene(&scene)?;
    writer.write_glb("scene.glb")
}
```
