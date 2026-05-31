# draco-io API Reference

Complete API documentation for the draco-io crate.

## Table of Contents

- [draco-io API Reference](#draco-io-api-reference)
  - [Table of Contents](#table-of-contents)
  - [Geometry Contract](#geometry-contract)
  - [Feature Flags](#feature-flags)
  - [Traits](#traits)
    - [Writer](#writer)
    - [Reader](#reader)
    - [WriteToBytes](#writetobytes)
    - [ReadFromBytes](#readfrombytes)
    - [PointCloudWriter](#pointcloudwriter)
    - [PointCloudReader](#pointcloudreader)
    - [SceneWriter](#scenewriter)
    - [SceneReader](#scenereader)
  - [Scene Types](#scene-types)
    - [Scene](#scene)
    - [SceneNode](#scenenode)
    - [MeshInstance](#meshinstance)
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

## Geometry Contract

`draco-io` maps file formats onto the `draco-core` geometry model. It supports
triangle meshes and point clouds, with `Position` as the required mesh
attribute and `Normal`, `Color`, `TexCoord`, and `Generic` preserved when the
format can represent them as Draco attributes.

Readers triangulate polygon faces where practical and return explicit
unsupported errors for primitive modes that do not map to Draco mesh geometry.
The scene layer is limited to names, hierarchy, transforms, and mesh instances.
Materials, textures/images, cameras, lights, animation, skinning, and arbitrary
format extras are intentionally out of scope.

---

## Feature Flags

`draco-io` keeps format support and Draco codec directions behind feature
flags. The default feature set enables all readers and writers, optional FBX
compression, and point-cloud decode support.

| Feature | Default | Description |
|---------|---------|-------------|
| `all-readers` | yes | Enables FBX, glTF/GLB, OBJ, and PLY readers |
| `all-writers` | yes | Enables FBX, glTF/GLB, OBJ, and PLY writers |
| `fbx-reader` | via `all-readers` | Binary FBX reader and scene graph support |
| `fbx-writer` | via `all-writers` | Binary FBX writer |
| `gltf-reader` | via `all-readers` | glTF/GLB reader; enables `draco-core/decoder` |
| `gltf-writer` | via `all-writers` | glTF/GLB writer; enables `draco-core/encoder` |
| `legacy-bitstream-decode` | no | Opt-in decode support for older Draco bitstreams inside glTF Draco payloads |
| `obj-reader` | via `all-readers` | OBJ reader |
| `obj-writer` | via `all-writers` | OBJ writer |
| `ply-reader` | via `all-readers` | PLY reader |
| `ply-writer` | via `all-writers` | PLY writer |
| `scene` | via FBX/glTF readers and glTF writer | Scene graph model and traits |
| `point_cloud_decode` | yes | Enables `draco-core/point_cloud_decode` |
| `compression` | yes | Enables optional compressed FBX property output |

`gltf-writer` intentionally depends only on `draco-core/encoder`; it does not
enable runtime decode. Legacy bitstream support is reader-only and opt-in.

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

### WriteToBytes

Extended trait for writers that can emit a complete file payload in memory.

```rust
pub trait WriteToBytes: Writer {
    fn write_to_vec(&self) -> io::Result<Vec<u8>>;
    fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<()>;
}
```

**Implementations:** `ObjWriter`, `PlyWriter`, `FbxWriter`, `GltfWriter`

```rust
use draco_io::{ObjWriter, WriteToBytes, Writer};

let mut writer = ObjWriter::new();
writer.add_mesh(&mesh, Some("Model"))?;
let bytes = writer.write_to_vec()?;
```

---

### ReadFromBytes

Extended trait for readers that can be constructed from a complete file payload.

```rust
pub trait ReadFromBytes: Sized {
    fn from_bytes(bytes: &[u8]) -> io::Result<Self>;
}
```

**Implementations:** `ObjReader`, `PlyReader`, `GltfReader`, `FbxMemoryReader`

```rust
use draco_io::{ObjReader, ReadFromBytes, Reader};

let mut reader = <ObjReader as ReadFromBytes>::from_bytes(&bytes)?;
let mesh = reader.read_mesh()?;
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
Available as `draco_io::scene::SceneWriter` and re-exported as
`draco_io::SceneWriter` when the `scene` feature is enabled.

```rust
pub trait SceneWriter: Writer {
    /// Add a scene graph to be written.
    fn add_scene(&mut self, scene: &Scene) -> io::Result<()>;

    /// Add multiple scenes.
    fn add_scenes(&mut self, scenes: &[Scene]) -> io::Result<()>;
}
```

**Implementations:** `GltfWriter`

```rust
use draco_io::{GltfWriter, SceneWriter, Writer};
use draco_io::{MeshInstance, Scene, SceneNode};

let mut root = SceneNode::new(Some("Root".to_string()));
root.mesh_instances.push(MeshInstance {
    name: Some("Mesh".to_string()),
    mesh,
    transform: None,
});

let scene = Scene {
    name: Some("MyScene".to_string()),
    root_nodes: vec![root],
};

let mut writer = GltfWriter::new();
SceneWriter::add_scene(&mut writer, &scene)?;
writer.write("scene.glb")?;
```

---

### SceneReader

Extended trait for reading scene graphs.
Available as `draco_io::scene::SceneReader` and re-exported as
`draco_io::SceneReader` when the `scene` feature is enabled. Flat OBJ and PLY
readers do not implement this trait; use `flatten_to_scene` to wrap their mesh
lists explicitly.

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

Scene types live in `draco_io::scene` and are re-exported from `draco_io` when
the `scene` feature is enabled.

### Scene

Container for a complete scene graph.

```rust
pub struct Scene {
    /// Optional scene name.
    pub name: Option<String>,

    /// Root nodes forming the hierarchy.
    pub root_nodes: Vec<SceneNode>,
}

impl Scene {
    pub fn new(name: Option<String>) -> Self;
    pub fn from_mesh_instances(name: Option<String>, mesh_instances: Vec<MeshInstance>) -> Self;
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

    /// Mesh instances attached to this node.
    pub mesh_instances: Vec<MeshInstance>,

    /// Child nodes.
    pub children: Vec<SceneNode>,
}

impl SceneNode {
    pub fn new(name: Option<String>) -> Self;
    pub fn with_mesh_instances(name: Option<String>, mesh_instances: Vec<MeshInstance>) -> Self;
}
```

---

### MeshInstance

A mesh attached to a scene node with optional metadata.

```rust
pub struct MeshInstance {
    /// Optional instance name.
    pub name: Option<String>,

    /// The mesh data.
    pub mesh: Mesh,

    /// Optional local transform for this mesh instance.
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
- Triangle faces (`f`); polygon faces are triangulated
- Object groups (`o`, `g`)
- Point clouds via position-only OBJ files

**Additional Methods:** `from_bytes`, `read_from_bytes`, `read_positions`, and
the convenience free function `read_obj_positions`.

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

**Implements:** `Writer`, `WriteToBytes`, `PointCloudWriter`

**Attribute support:** writes positions, triangle faces, normals, and
texcoords. Materials, MTL files, and vertex colors are not emitted.

**API Surface:**

Construction and input: `new`, `add_mesh`, `add_points`, and `add_point`.

Output: `write`, `write_to_vec`, and `write_to`.

Counters: `vertex_count` and `face_count`.

---

## PLY Format

### PlyReader

Reads Stanford PLY files (ASCII, binary little-endian, and binary big-endian).

```rust
use draco_io::{PlyReader, Reader};

let mut reader = PlyReader::open("model.ply")?;
let mesh = reader.read_mesh()?;
```

**Implements:** `Reader`, `PointCloudReader`

**Supported Features:**
- Vertex positions (x, y, z)
- Vertex normals (nx, ny, nz)
- Vertex colors (red, green, blue, alpha)
- Per-vertex texture coordinates (`texture_u`/`texture_v`, `u`/`v`, or `s`/`t`)
- Triangle or polygon faces (`vertex_indices`); polygons are triangulated
- ASCII format
- Binary little-endian format
- Binary big-endian format

**Additional Methods:** `from_bytes`, `read_from_bytes`, `read_positions`, and
the convenience free function `read_ply_positions`.

---

### PlyWriter

Writes Stanford PLY files (ASCII by default, or binary little/big-endian when enabled).

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

**Implements:** `Writer`, `WriteToBytes`, `PointCloudWriter`

**Attribute support:** writes positions, normals, colors, per-vertex texcoords,
and triangle faces. Arbitrary custom PLY properties are not generated.

**API Surface:**

Construction and format: `new`, `with_format`, `set_format`, `format`,
`with_binary_little_endian`, `set_binary_little_endian`, and
`is_binary_little_endian`.

Input: `add_mesh`, `add_points`, `add_point`, and `add_points_with_colors`.

Inspection and output: `has_normals`, `has_colors`, `write`, `write_to_vec`,
and `write_to`.

---

## FBX Format

### FbxReader

Reads Autodesk binary FBX files (7.x).

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
- Vertex positions
- Triangle/polygon indices with fan triangulation
- Binary FBX 7.x

Normals, colors, UVs, materials, animation, and skinning are not mapped yet.

**Additional Methods:** `from_bytes`, `read_from_bytes`, and `read_nodes`.

---

### FbxWriter

Writes Autodesk binary FBX files (7.5).

```rust
use draco_io::{FbxWriter, Writer};

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

**Implements:** `Writer`, `WriteToBytes`

**Attribute support:** writes position-only triangle meshes. If a mesh contains
normals, colors, texcoords, or generic attributes, `add_mesh()` returns
`InvalidInput` instead of silently dropping them.

**API Surface:**

Construction and compression: `new`, `with_compression`,
`with_compression_threshold`, and `is_compression_enabled`.

Input and output: `add_mesh`, `write`, `write_to_vec`, and `write_to`.

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

**API Surface:**

Construction: `open`, `from_glb`, `from_gltf`, `from_bytes`, and
`from_bytes_with_base_path`.

Draco payload access: `has_draco_extension`, `draco_primitives`,
`get_draco_data`, `decode_draco_mesh`, `decode_draco_point_cloud`
(`point_cloud_decode`), and `decode_all_draco_meshes`.

General mesh and scene decode: `read_mesh`, `read_meshes`, `read_scene`,
`read_scenes`, and `decode_all_meshes`.

Metadata inspection: `num_meshes`, `num_buffers`, `extensions_used`, and
`extensions_required`.

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

**Implements:** `Writer`, `WriteToBytes`, `SceneWriter`

`GltfWriter` emits current Draco bitstreams and uses encoder-side mesh summary
metadata to build the `KHR_draco_mesh_compression` accessors. The writer
feature depends on `draco-core/encoder` only; it does not decode its own output
at runtime. Use `encode_draco_mesh()` when callers need the raw `.drc` payload
without a glTF/GLB container.

**API Surface:**

Construction and input: `new`, `add_mesh`, `add_draco_mesh`, and `add_scene`.
The trait `SceneWriter::add_scene` uses default quantization; the inherent
`GltfWriter::add_scene` also accepts optional custom quantization.

File output: `write`, `write_glb`, `write_gltf`, and `write_gltf_embedded`.

In-memory output: `to_gltf_embedded`, `to_glb`, `write_to_vec`, and `write_to`.

**Free Functions:**

| Function | Description |
|----------|-------------|
| `encode_draco_mesh(mesh, quantization) -> Result<Vec<u8>>` | Encode raw Draco bytes with the same validation and quantization defaults as `GltfWriter` |

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

    /// Mesh name (if available).
    pub mesh_name: Option<String>,

    /// Index of the primitive within the mesh.
    pub primitive_index: usize,

    /// Buffer view containing the Draco data.
    pub buffer_view: usize,

    /// Attribute mappings from glTF semantic to Draco attribute ID.
    pub attributes: HashMap<String, usize>,
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

    #[error("Unsupported feature: {0}")]
    Unsupported(String),
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
use draco_io::{MeshInstance, Scene, SceneNode, Transform};
use draco_core::mesh::Mesh;

fn export_scene(meshes: Vec<Mesh>) -> Result<(), draco_io::GltfWriteError> {
    // Build scene graph
    let mut root = SceneNode::new(Some("Root".to_string()));
    
    for (i, mesh) in meshes.into_iter().enumerate() {
        let mut child = SceneNode::new(Some(format!("Object_{}", i)));
        child.mesh_instances.push(MeshInstance {
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
        root_nodes: vec![root],
    };

    // Export
    let mut writer = GltfWriter::new();
    writer.add_scene(&scene, None)?;
    writer.write_glb("scene.glb")
}
```
