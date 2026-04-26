# draco-core API Reference

Complete API documentation for the draco-core crate.

## Table of Contents

- [Geometry Types](#geometry-types)
  - [Mesh](#mesh)
  - [PointCloud](#pointcloud)
  - [PointAttribute](#pointattribute)
  - [GeometryAttributeType](#geometryattributetype)
  - [DataType](#datatype)
  - [Index Types](#index-types)
- [Encoding API](#encoding-api)
  - [MeshEncoder](#meshencoder)
  - [PointCloudEncoder](#pointcloudencoder)
  - [EncoderBuffer](#encoderbuffer)
  - [EncoderOptions](#encoderoptions)
- [Decoding API](#decoding-api)
  - [MeshDecoder](#meshdecoder)
  - [PointCloudDecoder](#pointclouddecoder)
  - [DecoderBuffer](#decoderbuffer)
- [Error Handling](#error-handling)
  - [Status](#status)
  - [DracoError](#dracoerror)
- [Transforms](#transforms)
  - [AttributeQuantizationTransform](#attributequantizationtransform)
  - [AttributeOctahedronTransform](#attributeoctahedrontransform)
- [Low-Level Components](#low-level-components)

---

## Geometry Types

### Mesh

A triangle mesh containing faces and point attributes.

```rust
use draco_core::{Mesh, FaceIndex, PointIndex};

// Create a new mesh
let mut mesh = Mesh::new();

// Add a face (triangle)
mesh.set_num_faces(1);
mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);

// Query mesh properties
let num_faces = mesh.num_faces();
let face = mesh.face(FaceIndex(0));  // Returns [PointIndex; 3]
```

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> Mesh` | Create an empty mesh |
| `add_face(face: Face)` | Append a face |
| `set_face(id: FaceIndex, face: Face)` | Set face at index |
| `face(id: FaceIndex) -> Face` | Get face at index |
| `num_faces() -> usize` | Number of faces |
| `set_num_faces(n: usize)` | Resize face array |

**Inherited from PointCloud** (via `Deref`):

| Method | Description |
|--------|-------------|
| `add_attribute(attr) -> i32` | Add attribute, returns ID |
| `attribute(id: i32) -> &PointAttribute` | Get attribute by ID |
| `attribute_mut(id: i32) -> &mut PointAttribute` | Get mutable attribute |
| `named_attribute(type) -> Option<&PointAttribute>` | Get by semantic type |
| `num_attributes() -> i32` | Number of attributes |
| `num_points() -> usize` | Number of points |

---

### PointCloud

A collection of points with associated attributes.

```rust
use draco_core::{PointCloud, PointAttribute, GeometryAttributeType};

let mut pc = PointCloud::new();
pc.set_num_points(100);

// Add position attribute
let pos_att = PointAttribute::new();
// ... initialize attribute ...
let att_id = pc.add_attribute(pos_att);

// Access attributes
let pos = pc.named_attribute(GeometryAttributeType::Position);
```

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> PointCloud` | Create empty point cloud |
| `set_num_points(n: usize)` | Set number of points |
| `num_points() -> usize` | Get number of points |
| `add_attribute(attr) -> i32` | Add attribute |
| `attribute(id: i32) -> &PointAttribute` | Get attribute by ID |
| `attribute_mut(id: i32) -> &mut PointAttribute` | Mutable access |
| `named_attribute_id(type) -> i32` | Find attribute by type (-1 if not found) |
| `named_attribute(type) -> Option<&PointAttribute>` | Get attribute by type |
| `num_attributes() -> i32` | Attribute count |

---

### PointAttribute

Per-point attribute data (positions, normals, colors, etc.).

```rust
use draco_core::{PointAttribute, GeometryAttributeType, DataType};

let mut attr = PointAttribute::new();

// Initialize: type, components, data type, normalized, num_values
attr.init(GeometryAttributeType::Position, 3, DataType::Float32, false, 100);

// Write data
let position: [f32; 3] = [1.0, 2.0, 3.0];
let bytes: Vec<u8> = position.iter().flat_map(|v| v.to_le_bytes()).collect();
attr.buffer_mut().write(0, &bytes);

// Read data
let data = attr.buffer().data();
```

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> PointAttribute` | Create uninitialized attribute |
| `init(type, components, data_type, normalized, num_values)` | Initialize attribute |
| `attribute_type() -> GeometryAttributeType` | Get semantic type |
| `data_type() -> DataType` | Get data type |
| `num_components() -> u8` | Components per value (e.g., 3 for vec3) |
| `normalized() -> bool` | Is normalized integer data |
| `size() -> usize` | Number of attribute values |
| `buffer() -> &DataBuffer` | Raw data buffer |
| `buffer_mut() -> &mut DataBuffer` | Mutable data buffer |
| `unique_id() -> u32` | Unique identifier |
| `set_unique_id(id: u32)` | Set unique identifier |

---

### GeometryAttributeType

Semantic type of an attribute.

```rust
use draco_core::GeometryAttributeType;

let attr_type = GeometryAttributeType::Position;
```

**Variants:**

| Variant | Value | Description |
|---------|-------|-------------|
| `Invalid` | -1 | Invalid/uninitialized |
| `Position` | 0 | Vertex positions |
| `Normal` | 1 | Vertex normals |
| `Color` | 2 | Vertex colors |
| `TexCoord` | 3 | Texture coordinates |
| `Generic` | 4 | Custom/generic attribute |

---

### DataType

Primitive data types for attribute values.

```rust
use draco_core::DataType;

let dt = DataType::Float32;
```

**Variants:**

| Variant | Description |
|---------|-------------|
| `Invalid` | Invalid/uninitialized |
| `Int8` | Signed 8-bit integer |
| `UInt8` | Unsigned 8-bit integer |
| `Int16` | Signed 16-bit integer |
| `UInt16` | Unsigned 16-bit integer |
| `Int32` | Signed 32-bit integer |
| `UInt32` | Unsigned 32-bit integer |
| `Int64` | Signed 64-bit integer |
| `UInt64` | Unsigned 64-bit integer |
| `Float32` | 32-bit float |
| `Float64` | 64-bit float |
| `Bool` | Boolean |

---

### Index Types

Type-safe index wrappers for geometry elements.

```rust
use draco_core::{PointIndex, FaceIndex, AttributeValueIndex, CornerIndex, VertexIndex};

let point = PointIndex(0);
let face = FaceIndex(0);
let attr_value = AttributeValueIndex(0);
```

| Type | Description |
|------|-------------|
| `PointIndex` | Index into point array |
| `FaceIndex` | Index into face array |
| `AttributeValueIndex` | Index into attribute value array |
| `CornerIndex` | Index into corner table |
| `VertexIndex` | Index into vertex array |

---

## Encoding API

### MeshEncoder

Encodes meshes to Draco format.

```rust
use draco_core::{MeshEncoder, EncoderOptions, EncoderBuffer, Mesh};

let mesh: Mesh = /* ... */;

let mut encoder = MeshEncoder::new();
encoder.set_mesh(mesh);

let mut options = EncoderOptions::new();
let mut buffer = EncoderBuffer::new();

encoder.encode(&options, &mut buffer)?;

let compressed_data = buffer.data();
```

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> MeshEncoder` | Create new encoder |
| `set_mesh(mesh: Mesh)` | Set mesh to encode |
| `mesh() -> Option<&Mesh>` | Get reference to mesh |
| `encode(options, buffer) -> Status` | Encode mesh |
| `num_encoded_faces() -> usize` | Faces encoded |
| `corner_table() -> Option<&CornerTable>` | Access corner table |

---

### PointCloudEncoder

Encodes point clouds to Draco format.

```rust
use draco_core::{PointCloudEncoder, EncoderOptions, EncoderBuffer, PointCloud};

let pc: PointCloud = /* ... */;

let mut encoder = PointCloudEncoder::new();
encoder.set_point_cloud(pc);

let mut options = EncoderOptions::new();
let mut buffer = EncoderBuffer::new();

encoder.encode(&options, &mut buffer)?;
```

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> PointCloudEncoder` | Create new encoder |
| `set_point_cloud(pc: PointCloud)` | Set point cloud to encode |
| `encode(options, buffer) -> Status` | Encode point cloud |

---

### EncoderBuffer

Output buffer for compressed data.

```rust
use draco_core::EncoderBuffer;

let mut buffer = EncoderBuffer::new();
// ... encoding ...
let data: &[u8] = buffer.data();
let size = buffer.size();
```

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> EncoderBuffer` | Create empty buffer |
| `data() -> &[u8]` | Get encoded data |
| `size() -> usize` | Size in bytes |
| `clear()` | Clear buffer |

---

### EncoderOptions

Configuration for encoding.

```rust
use draco_core::EncoderOptions;

let mut options = EncoderOptions::new();

// Global options
options.set_global_int("encoding_speed", 7);
options.set_global_int("decoding_speed", 7);

// Per-attribute options
options.set_attribute_int(0, "quantization_bits", 14);  // Position
options.set_attribute_int(1, "quantization_bits", 10);  // Normal

// Prediction scheme
options.set_prediction_scheme(1);  // Parallelogram
```

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> EncoderOptions` | Create with defaults |
| `set_global_int(key, value)` | Set global option |
| `get_global_int(key, default) -> i32` | Get global option |
| `set_attribute_int(att_id, key, value)` | Set per-attribute option |
| `get_attribute_int(att_id, key, default) -> i32` | Get per-attribute option |
| `get_encoding_speed() -> i32` | Get encoding speed (0-10) |
| `get_decoding_speed() -> i32` | Get decoding speed (0-10) |
| `set_encoding_method(method: i32)` | Set encoding method |
| `get_encoding_method() -> Option<i32>` | Get encoding method |
| `set_prediction_scheme(scheme: i32)` | Set prediction scheme |
| `set_version(major, minor)` | Force specific version |

**Common Options:**

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `encoding_speed` | i32 | 5 | 0=best compression, 10=fastest |
| `decoding_speed` | i32 | 5 | 0=best compression, 10=fastest |
| `quantization_bits` | i32 | varies | Bits per component |

---

## Decoding API

### MeshDecoder

Decodes Draco data to meshes.

```rust
use draco_core::{MeshDecoder, DecoderBuffer, Mesh};

let data: &[u8] = /* compressed data */;

let mut buffer = DecoderBuffer::new(data);
let mut decoder = MeshDecoder::new();
let mut mesh = Mesh::new();

decoder.decode(&mut buffer, &mut mesh)?;

println!("Faces: {}", mesh.num_faces());
println!("Points: {}", mesh.num_points());
```

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> MeshDecoder` | Create new decoder |
| `decode(buffer, mesh) -> Status` | Decode to mesh |

---

### PointCloudDecoder

Decodes Draco data to point clouds.

```rust
use draco_core::{PointCloudDecoder, DecoderBuffer, PointCloud};

let data: &[u8] = /* compressed data */;

let mut buffer = DecoderBuffer::new(data);
let mut decoder = PointCloudDecoder::new();
let mut pc = PointCloud::new();

decoder.decode(&mut buffer, &mut pc)?;
```

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> PointCloudDecoder` | Create new decoder |
| `decode(buffer, point_cloud) -> Status` | Decode to point cloud |

---

### DecoderBuffer

Input buffer for compressed data. Provides sequential byte and bit-level access to compressed data.

```rust
use draco_core::{DecoderBuffer, DracoError};

let data: &[u8] = /* ... */;
let mut buffer = DecoderBuffer::new(data);

// Low-level access - all methods return Result<T, DracoError>
let byte = buffer.decode_u8()?;
let value = buffer.decode_varint()?;
let remaining = buffer.remaining_size();
```

**Methods:**

| Method | Description |
|--------|-------------|
| `new(data: &[u8]) -> DecoderBuffer` | Create from data |
| `remaining_size() -> usize` | Bytes remaining |
| `decode_u8() -> Result<u8, DracoError>` | Read u8 |
| `decode_u16() -> Result<u16, DracoError>` | Read little-endian u16 |
| `decode_u32() -> Result<u32, DracoError>` | Read little-endian u32 |
| `decode_u64() -> Result<u64, DracoError>` | Read little-endian u64 |
| `decode_f32() -> Result<f32, DracoError>` | Read little-endian f32 |
| `decode_f64() -> Result<f64, DracoError>` | Read little-endian f64 |
| `decode_varint() -> Result<u64, DracoError>` | Read variable-length int |
| `decode_string() -> Result<String, DracoError>` | Read null-terminated string |
| `decode_bytes(&mut [u8]) -> Result<(), DracoError>` | Read bytes into buffer |
| `decode_slice(size) -> Result<&[u8], DracoError>` | Read and return slice |
| `set_position(pos) -> Result<(), DracoError>` | Set read position |
| `advance(bytes: usize)` | Skip bytes |

---

## Error Handling

### Status

Result type alias for Draco operations.

```rust
pub type Status = Result<(), DracoError>;
```

### DracoError

Error type for all Draco operations.

```rust
use draco_core::DracoError;

match result {
    Ok(()) => println!("Success"),
    Err(DracoError::DracoError(msg)) => println!("Error: {}", msg),
    Err(DracoError::IoError(msg)) => println!("IO Error: {}", msg),
    Err(DracoError::BufferError(msg)) => println!("Buffer Error: {}", msg),
    Err(e) => println!("Other error: {}", e),
}
```

**Variants:**

| Variant | Description |
|---------|-------------|
| `DracoError(String)` | General error |
| `IoError(String)` | I/O error |
| `InvalidParameter(String)` | Invalid parameter |
| `UnsupportedVersion(String)` | Version not supported |
| `UnknownVersion(String)` | Unknown version |
| `UnsupportedFeature(String)` | Feature not supported |
| `BitstreamVersionUnsupported` | Bitstream version issue |
| `BufferError(String)` | Buffer read/decode error |

---

## Transforms

### AttributeQuantizationTransform

Quantizes floating-point attributes to integers for compression.

```rust
use draco_core::AttributeQuantizationTransform;

let transform = AttributeQuantizationTransform::new();

// Get quantization parameters after decoding
let min_values = transform.min_value();
let range = transform.range();
let quantization_bits = transform.quantization_bits();
```

**Methods:**

| Method | Description |
|--------|-------------|
| `new() -> Self` | Create new transform |
| `quantization_bits() -> i32` | Bits per component |
| `min_value() -> &[f32]` | Minimum values |
| `range() -> f32` | Value range |
| `is_initialized() -> bool` | Is transform initialized |

---

### AttributeOctahedronTransform

Encodes unit normals using octahedron projection.

```rust
use draco_core::AttributeOctahedronTransform;

let transform = AttributeOctahedronTransform::new();
let quantization_bits = transform.quantization_bits();
```

---

## Low-Level Components

### CornerTable

Half-edge data structure for mesh connectivity.

```rust
use draco_core::CornerTable;

let corner_table: &CornerTable = /* from encoder/decoder */;

let opposite = corner_table.opposite(corner_index);
let next = corner_table.next(corner_index);
let prev = corner_table.previous(corner_index);
let vertex = corner_table.vertex(corner_index);
let face = corner_table.face(corner_index);
```

### Entropy Coders

Low-level entropy coding for advanced use cases.

| Type | Description |
|------|-------------|
| `AnsCoder` / `AnsDecoder` | Asymmetric Numeral Systems |
| `RAnsBitEncoder` / `RAnsBitDecoder` | rANS bit-level coding |
| `DirectBitEncoder` / `DirectBitDecoder` | Direct bit I/O |
| `FoldedBit32Encoder` / `FoldedBit32Decoder` | Folded 32-bit coding |

### Prediction Schemes

| Type | Description |
|------|-------------|
| `PredictionSchemeMethod` | Enum of prediction methods |
| `PredictionSchemeTransformType` | Enum of transform types |

**Prediction Methods:**

| Method | Value | Description |
|--------|-------|-------------|
| `None` | 0 | No prediction |
| `Difference` | 1 | Delta from previous |
| `Parallelogram` | 2 | Parallelogram prediction |
| `MultiParallelogram` | 3 | Constrained multi-parallelogram |
| `TexCoordsDeprecated` | 4 | Deprecated texture coords |
| `TexCoordsPortable` | 5 | Portable texture coords |
| `GeometricNormal` | 6 | Geometric normal prediction |

---

## Version Information

```rust
use draco_core::version::{DEFAULT_MESH_VERSION, VERSION_FLAGS_INTRODUCED};

// Current default version for encoding
let (major, minor) = DEFAULT_MESH_VERSION;
```

---

## Complete Example

```rust
use draco_core::{
    Mesh, MeshEncoder, MeshDecoder,
    EncoderBuffer, DecoderBuffer, EncoderOptions,
    PointAttribute, GeometryAttributeType, DataType,
    PointIndex, FaceIndex, DracoError,
};

fn round_trip_mesh() -> Result<(), DracoError> {
    // Create a simple triangle mesh
    let mut mesh = Mesh::new();
    
    // Add position attribute
    let mut pos_att = PointAttribute::new();
    pos_att.init(GeometryAttributeType::Position, 3, DataType::Float32, false, 3);
    
    let positions: [[f32; 3]; 3] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.5, 1.0, 0.0],
    ];
    for (i, pos) in positions.iter().enumerate() {
        let bytes: Vec<u8> = pos.iter().flat_map(|v| v.to_le_bytes()).collect();
        pos_att.buffer_mut().write(i * 12, &bytes);
    }
    mesh.add_attribute(pos_att);
    
    // Add face
    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    
    // Encode
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    
    let options = EncoderOptions::new();
    let mut encode_buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut encode_buffer)?;
    
    println!("Encoded size: {} bytes", encode_buffer.size());
    
    // Decode
    let mut decode_buffer = DecoderBuffer::new(encode_buffer.data());
    let mut decoder = MeshDecoder::new();
    let mut decoded_mesh = Mesh::new();
    decoder.decode(&mut decode_buffer, &mut decoded_mesh)?;
    
    println!("Decoded mesh: {} faces, {} points",
        decoded_mesh.num_faces(),
        decoded_mesh.num_points());
    
    Ok(())
}
```
