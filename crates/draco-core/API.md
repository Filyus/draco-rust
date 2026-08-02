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
- [Feature Flags](#feature-flags)
- [Encoding API](#encoding-api)
  - [MeshEncoder](#meshencoder)
  - [EncodedMeshInfo](#encodedmeshinfo)
  - [EncodedAttributeInfo](#encodedattributeinfo)
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

**API Surface:**

Face editing: `new`, `add_face`, `set_face`, `face`, `num_faces`,
`set_num_faces`, `try_set_num_faces`, and `set_face_from_indices`.

Bulk index import: `set_faces_from_flat_indices`, `set_faces_from_u8_indices`,
`set_faces_from_le_u16_indices`, and `set_faces_from_le_u32_indices`.

Point/attribute access comes from `PointCloud` via `Deref`: `num_points`,
`num_attributes`, `add_attribute`, `attribute`, `attribute_mut`,
`named_attribute`, and the checked `try_attribute` variants.

Topology cleanup: `deduplicate_point_ids` rebuilds point and attribute maps
with duplicate point IDs collapsed.

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

**API Surface:**

Point count: `new`, `set_num_points`, and `num_points`.

Attributes: `add_attribute`, `add_attribute_preserve_unique_id`,
`num_attributes`, `attribute`, `attribute_mut`, `named_attribute_id`, and
`named_attribute`. Checked lookups are available as `try_attribute` and
`try_attribute_mut`.

Metadata: `metadata`, `metadata_mut`, `metadata_or_insert`, `set_metadata`,
`attribute_metadata_by_unique_id`, `attribute_metadata_by_string_entry`, and
`set_attribute_metadata`.

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

**API Surface:**

Initialization and shape: `new`, `init`, `try_init`, `size`,
`resize_unique_entries`, `num_components`, `data_type`, `attribute_type`,
`normalized`, `byte_stride`, and `byte_offset`.

Storage: `buffer` and `buffer_mut` expose the raw `DataBuffer`.

Point-to-value mapping: `mapped_index`, `set_identity_mapping`,
`set_explicit_mapping`, `set_point_map_entry`, and `try_set_point_map_entry`.

Metadata and mutators: `unique_id`, `set_unique_id`, `set_attribute_type`,
`set_data_type`, `set_num_components`, `set_attribute_transform_data`, and
`attribute_transform_data`.

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
| `Uint8` | Unsigned 8-bit integer |
| `Int16` | Signed 16-bit integer |
| `Uint16` | Unsigned 16-bit integer |
| `Int32` | Signed 32-bit integer |
| `Uint32` | Unsigned 32-bit integer |
| `Int64` | Signed 64-bit integer |
| `Uint64` | Unsigned 64-bit integer |
| `Float32` | 32-bit float |
| `Float64` | 64-bit float |
| `Bool` | Boolean |

---

### Index Types

Type-safe index wrappers for geometry elements.

```rust
use draco_core::{AttributeValueIndex, FaceIndex, PointIndex};
use draco_core::geometry_indices::{CornerIndex, VertexIndex};

let point = PointIndex(0);
let face = FaceIndex(0);
let attr_value = AttributeValueIndex(0);
let corner = CornerIndex(0);
let vertex = VertexIndex(0);
```

| Type | Description |
|------|-------------|
| `PointIndex` | Index into point array |
| `FaceIndex` | Index into face array |
| `AttributeValueIndex` | Index into attribute value array |
| `CornerIndex` | Index into corner table |
| `VertexIndex` | Index into vertex array |

---

## Feature Flags

`draco-core` gates codec directions and optional compatibility paths so embedders
can keep builds small.

| Feature | Default | Description |
|---------|---------|-------------|
| `encoder` | yes | Mesh and point-cloud encoding APIs |
| `decoder` | yes | Mesh and point-cloud decoding APIs |
| `point_cloud_decode` | yes | Point-cloud decoder path |
| `edgebreaker_valence_encode` | yes | Modern EdgeBreaker valence traversal for high-compression mesh encoding |
| `edgebreaker_valence_decode` | yes | Decode EdgeBreaker valence traversal streams |
| `legacy_bitstream_encode` | yes | Compatibility support for writing older Draco bitstream layouts and deprecated prediction schemes |
| `legacy_bitstream_decode` | yes | Decode older Draco bitstreams and deprecated prediction schemes |
| `debug_logs` | no | Diagnostic logging |
| `force_sequential_seeds` | no | Test/debug knob for deterministic traversal experiments |

Public encoders write the current Draco bitstream by default. Legacy encode
support is for compatibility and conformance tests, not for normal writer
output.

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
let encoded_info = encoder.encoded_mesh_info();
```

**API Surface:**

Construction and input: `new`, `set_mesh`, and `mesh`.

Encoding and results: `encode`, `num_encoded_faces`, `corner_table`, and
`encoded_mesh_info`.

---

### EncodedMeshInfo

Summary produced by `MeshEncoder` after a successful `encode()`. This describes
the encoded mesh shape and attribute metadata without requiring a decoder pass.
It is primarily useful for container writers such as glTF
`KHR_draco_mesh_compression`.

```rust
pub struct EncodedMeshInfo {
    pub encoding_method: i32,
    pub num_encoded_faces: usize,
    pub num_encoded_points: usize,
    pub attributes: Vec<EncodedAttributeInfo>,
}
```

`num_encoded_points` is the global encoded point count used by compressed
attribute accessors. Individual attributes also report their own
`num_encoded_values`, which can differ when split attribute connectivity or
seams are involved.

---

### EncodedAttributeInfo

Attribute summary produced by `MeshEncoder`.

```rust
pub struct EncodedAttributeInfo {
    pub source_attribute_id: i32,
    pub attribute_type: GeometryAttributeType,
    pub data_type: DataType,
    pub num_components: u8,
    pub normalized: bool,
    pub unique_id: u32,
    pub num_encoded_values: usize,
    pub position_min: Option<Vec<f64>>,
    pub position_max: Option<Vec<f64>>,
}
```

`unique_id` is the Draco attribute id that container formats should reference.
`position_min` and `position_max` are populated only for position attributes.

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

**API Surface:** `new`, `set_point_cloud`, `point_cloud`, and `encode`.

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

**API Surface:**

Lifecycle and versioning: `new`, `clear`, `resize`, `set_version`,
`version_major`, and `version_minor`.

Inspection: `data` and `size`.

Byte-aligned writes: `encode`, `encode_data`, `encode_u8`, `encode_u16`,
`encode_u32`, `encode_u64`, `encode_varint`, and `encode_varint_signed_i32`.

Bit-level writes: `start_bit_encoding`, `encode_least_significant_bits32`, and
`end_bit_encoding`.

---

### EncoderOptions

Configuration for encoding. Keys and semantics mirror the C++ library's
`EncoderOptions`/`ExpertEncoder` layer (`encoding_speed`, `decoding_speed`,
`quantization_bits` keyed by attribute id, `prediction_scheme`,
`encoding_method`) — not the `draco_encoder` CLI, whose `-cl` and
`-qp`/`-qt`/`-qn`/`-qg` flags are a thin, CLI-only layer on top that resolves
an attribute *type* to an attribute *id* and renames/inverts a couple of
knobs on the way in. `set_compression_level` and `set_attribute_quantization`
below exist so a caller porting CLI settings does not have to do that
translation by hand.

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

**API Surface:**

Generic options: `set_global_int`, `get_global_int`, `set_attribute_int`, and
`get_attribute_int`.

Common knobs: `get_encoding_speed`, `get_decoding_speed`, `get_speed`,
`set_encoding_method`, `get_encoding_method`, `set_prediction_scheme`,
`get_prediction_scheme`, `set_version`, and `get_version`.

CLI-equivalent convenience: `set_compression_level`, `get_compression_level`,
`set_attribute_quantization`, and `get_attribute_quantization`.

**Common Options:**

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `encoding_speed` | i32 | 5 | 0=best compression, 10=fastest |
| `decoding_speed` | i32 | 5 | 0=best compression, 10=fastest |
| `quantization_bits` | i32 | unset (no quantization) | Bits per component, 1..=30 for position/generic attributes, 2..=30 for normals |

#### Relationship to the `draco_encoder` CLI

The CLI's `-cl <0..10>` is compression level — 0 least, 10 most — the reverse
sense of `encoding_speed`/`decoding_speed`, where 10 is fastest. The CLI
converts once, in its own `main()`, before calling the same `SetSpeedOptions`
this crate's `set_global_int("encoding_speed"/"decoding_speed", _)` writes to:

```text
speed = 10 - compression_level
```

`set_compression_level`/`get_compression_level` apply exactly that conversion
and nothing else; they do not add a `compression_level` key, since none
exists at this layer in the C++ library either. Confirmed byte-identical
against C++ Draco 1.5.7 for every integer speed/level pair in the documented
0..=10 range.

The two tools default to different speeds: `EncoderOptions::new()` is speed 5
(`get_compression_level()` reports 5), while the CLI defaults to `-cl 7`
(speed 3). `EncoderOptions::new()` with no changes is byte-identical to
`draco_encoder -cl 5 -qp 0` (no position quantization) — not to the CLI run
with no flags at all, which is `-cl 7 -qp 11`.

`-qp`/`-qt`/`-qn`/`-qg` all set the same `quantization_bits` key, once the CLI
has resolved POSITION/TEX_COORD/NORMAL/GENERIC to the attribute id this
library's options are keyed by; `set_attribute_quantization(att_id, bits)` is
that same call one layer down, and `-1` on `get_attribute_quantization`
means unset, same as an omitted CLI flag (which quantizes nothing, not zero
bits).

Neither `set_global_int("encoding_speed", _)` nor `set_compression_level`
range-checks its input, matching the CLI, which passes `-cl` through the same
subtraction unchecked. Both sides agree on every integer in 0..=10; outside
it, this crate's `10 - speed` is unclamped everywhere, while upstream clamps
one specific internal knob derived from it (the entropy coder's own
compression-level input) back to its documented default of 7 before use, so
a speed more than a few units outside 0..=10 can diverge from the CLI's
output by a few bytes in the entropy-coded section. No supported input is
affected; this is stated for completeness, not as a caveat callers need to
plan around.

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

**API Surface:** `new` and `decode`.

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

**API Surface:** `new` and `decode`.

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

**API Surface:**

Lifecycle, position, and versioning: `new`, `set_version`, `version_major`,
`version_minor`, `position`, `set_position`, `advance`, and `try_advance`.

Inspection: `remaining_size`, `remaining_data`, and `peek_bytes`.

Byte-aligned reads: `decode`, `decode_u8`, `decode_u16`, `decode_u32`,
`decode_u64`, `decode_f32`, `decode_f64`, `decode_varint`,
`decode_varint_signed_i32`, `decode_string`, `decode_bytes`, and
`decode_slice`.

Bit-level reads: `start_bit_decoding`, `decode_least_significant_bits32`, and
`end_bit_decoding`.

---

## Error Handling

### Status

Result type alias for Draco operations.

```rust
pub type Status = Result<(), DracoError>;
```

### DracoError

Error type for all Draco operations. An opaque struct one pointer wide, in the
shape of `std::io::Error`: ask it for its `kind()` rather than matching it.

```rust
use draco_core::{DracoError, ErrorKind};

match result {
    Ok(()) => println!("Success"),
    Err(error) => match error.kind() {
        ErrorKind::General => println!("Error: {}", error.message()),
        ErrorKind::Io => println!("IO Error: {}", error.message()),
        ErrorKind::Buffer => println!("Buffer Error: {}", error.message()),
        _ => println!("Other error: {}", error),
    },
}
```

`Display` prints the kind followed by the message; `message()` returns the
message alone.

**Constructors and kinds:**

| Constructor | `ErrorKind` | Description |
|-------------|-------------|-------------|
| `DracoError::general(msg)` | `General` | General error |
| `DracoError::io(msg)` | `Io` | I/O error |
| `DracoError::invalid_parameter(msg)` | `InvalidParameter` | Invalid parameter |
| `DracoError::unsupported_version(msg)` | `UnsupportedVersion` | Version not supported |
| `DracoError::unknown_version(msg)` | `UnknownVersion` | Unknown version |
| `DracoError::unsupported_feature(msg)` | `UnsupportedFeature` | Feature not supported |
| `DracoError::bitstream_version_unsupported()` | `BitstreamVersionUnsupported` | Bitstream version issue |
| `DracoError::buffer(msg)` | `Buffer` | Buffer read/decode error |
| `DracoError::allocation_exceeds_input(requested, stream)` | `AllocationExceedsInput` | Decode would allocate more than the stream could describe |

`DracoError::new(kind, msg)` builds any of them directly.

`ErrorKind` is `#[non_exhaustive]`: match with a `_` arm so a later release can
tell one refusal from another without a major bump.

Why a struct and not an enum: every fallible function in this crate returns
`Status`, so the size of the failure case is paid by the success case at every
call site. With the message stored inline, `Result<(), DracoError>` was 32 bytes
and needed dropping. Boxed, it is a pointer returned in a register.

---

## Transforms

### AttributeQuantizationTransform

Quantizes floating-point attributes to integers for compression.

```rust
use draco_core::AttributeQuantizationTransform;

let attribute = /* float PointAttribute */;
let mut transform = AttributeQuantizationTransform::new();

// Compute quantization parameters for an attribute.
let ok = transform.compute_parameters(&attribute, 14);
```

**API Surface:**

Own methods: `new`, `set_parameters`, and `compute_parameters`.

The `AttributeTransform` trait provides metadata initialization, forward and
inverse attribute transforms, parameter encode/decode, and transformed
data-shape queries.

---

### AttributeOctahedronTransform

Encodes unit normals using octahedron projection.

```rust
use draco_core::AttributeOctahedronTransform;

let transform = AttributeOctahedronTransform::new(10);
let quantization_bits = transform.quantization_bits();
```

**API Surface:**

Own methods: `new`, `is_valid_quantization_bits`, `set_parameters`,
`is_initialized`, `quantization_bits`, and `generate_portable_attribute`.

The `AttributeTransform` trait provides metadata initialization, forward and
inverse attribute transforms, parameter encode/decode, and transformed
data-shape queries.

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
| `None` | -2 | No prediction |
| `Undefined` | -1 | Let encoder choose |
| `Difference` | 0 | Delta from previous |
| `MeshPredictionParallelogram` | 1 | Parallelogram prediction |
| `MeshPredictionMultiParallelogram` | 2 | Legacy multi-parallelogram prediction |
| `MeshPredictionTexCoordsDeprecated` | 3 | Deprecated texture coords predictor |
| `MeshPredictionConstrainedMultiParallelogram` | 4 | Constrained multi-parallelogram prediction |
| `MeshPredictionTexCoordsPortable` | 5 | Portable texture coords predictor |
| `MeshPredictionGeometricNormal` | 6 | Geometric normal prediction |

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
