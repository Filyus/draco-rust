# draco-core

A pure Rust implementation of Google's Draco 3D mesh compression library. This crate provides core compression and decompression functionality for 3D geometric meshes and point clouds.

For the project overview, compatibility notes, and benchmarks, see the
[Draco Rust workspace README](https://github.com/Filyus/draco-rust).

## Features

- **Mesh Compression**: Encode 3D meshes with configurable quantization levels
- **Mesh Decompression**: Decode Draco-compressed mesh data
- **Point Cloud Support**: Compress and decompress point cloud data
- **EdgeBreaker Encoding**: Industry-standard mesh connectivity compression
- **Attribute Prediction**: Advanced prediction schemes for positions, normals, texture coordinates
- **rANS Entropy Coding**: High-performance entropy coding for attribute data
- **Feature Flags**: Separate `encoder` and `decoder` features for minimal builds

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
draco-core = { version = "0.1.0", path = "../draco-core" }
```

### Feature Flags

| Feature   | Default | Description                           |
|-----------|---------|---------------------------------------|
| `encoder` | ✓       | Mesh/point cloud encoding support     |
| `decoder` | ✓       | Mesh/point cloud decoding support     |

To use only decoding (smaller binary):

```toml
[dependencies]
draco-core = { version = "0.1.0", default-features = false, features = ["decoder"] }
```

## Quick Start

### Decoding a Draco File

```rust
use draco_core::{DecoderBuffer, MeshDecoder, Mesh};

fn decode_mesh(data: &[u8]) -> Result<Mesh, draco_core::DracoError> {
    let mut buffer = DecoderBuffer::new(data);
    let mut decoder = MeshDecoder::new();
    let mut mesh = Mesh::new();
    
    decoder.decode(&mut buffer, &mut mesh)?;
    
    println!("Decoded mesh with {} faces and {} points", 
        mesh.num_faces(), 
        mesh.num_points());
    Ok(mesh)
}
```

### Encoding a Mesh

```rust
use draco_core::{
    Mesh, MeshEncoder, EncoderBuffer, EncoderOptions,
    PointAttribute, GeometryAttributeType, DataType,
    PointIndex, FaceIndex,
};

fn encode_mesh(mesh: &Mesh) -> Result<Vec<u8>, draco_core::DracoError> {
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    
    let mut options = EncoderOptions::new();
    // Higher speed = faster encoding, lower compression
    options.set_global_int("encoding_speed", 5);
    options.set_global_int("decoding_speed", 5);
    
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer)?;
    
    Ok(buffer.data().to_vec())
}
```

### Creating a Mesh Programmatically

```rust
use draco_core::{
    Mesh, PointAttribute, GeometryAttributeType, DataType,
    PointIndex, FaceIndex,
};

fn create_triangle() -> Mesh {
    let mut mesh = Mesh::new();
    
    // Create position attribute
    let mut pos_att = PointAttribute::new();
    pos_att.init(GeometryAttributeType::Position, 3, DataType::Float32, false, 3);
    
    // Set vertex positions
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
    
    // Add a face
    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    
    mesh
}
```

## Core Types

### Geometry

| Type | Description |
|------|-------------|
| `Mesh` | Triangle mesh with faces and attributes |
| `PointCloud` | Collection of points with attributes |
| `PointAttribute` | Per-point attribute data (positions, normals, etc.) |
| `GeometryAttributeType` | Attribute semantics (Position, Normal, Color, TexCoord, Generic) |
| `DataType` | Data types (Float32, Int32, UInt8, etc.) |

### Encoding

| Type | Description |
|------|-------------|
| `MeshEncoder` | Encodes meshes to Draco format |
| `PointCloudEncoder` | Encodes point clouds to Draco format |
| `EncoderBuffer` | Output buffer for encoded data |
| `EncoderOptions` | Configuration for encoding (speed, quantization) |

### Decoding

| Type | Description |
|------|-------------|
| `MeshDecoder` | Decodes Draco data to meshes |
| `PointCloudDecoder` | Decodes Draco data to point clouds |
| `DecoderBuffer` | Input buffer for compressed data |

### Error Handling

| Type | Description |
|------|-------------|
| `Status` | Result type alias for Draco operations |
| `DracoError` | Error type with variants for different failure modes |

## Compression Settings

### Quantization Bits

Control precision vs. compression ratio:

```rust
let mut options = EncoderOptions::new();

// Position quantization (default: 14 bits)
options.set_attribute_int(0, "quantization_bits", 14);

// Normal quantization (default: 10 bits)  
options.set_attribute_int(1, "quantization_bits", 10);

// Texture coordinate quantization (default: 12 bits)
options.set_attribute_int(2, "quantization_bits", 12);
```

### Speed Settings

Balance speed vs. compression:

```rust
let mut options = EncoderOptions::new();

// 0 = best compression, 10 = fastest encoding
options.set_global_int("encoding_speed", 5);
options.set_global_int("decoding_speed", 5);
```

## Module Structure

```
draco-core
├── Core Types
│   ├── mesh              - Mesh data structure
│   ├── point_cloud       - Point cloud data structure  
│   ├── geometry_attribute- Attribute definitions
│   ├── geometry_indices  - Index types (PointIndex, FaceIndex, etc.)
│   ├── data_buffer       - Raw data storage
│   └── draco_types       - DataType enum
│
├── Encoding (feature = "encoder")
│   ├── mesh_encoder      - Main mesh encoder
│   ├── point_cloud_encoder- Point cloud encoder
│   ├── encoder_buffer    - Output buffer
│   ├── encoder_options   - Encoding configuration
│   └── mesh_edgebreaker_encoder - EdgeBreaker connectivity encoding
│
├── Decoding (feature = "decoder")
│   ├── mesh_decoder      - Main mesh decoder
│   ├── point_cloud_decoder- Point cloud decoder
│   ├── decoder_buffer    - Input buffer
│   └── mesh_edgebreaker_decoder - EdgeBreaker connectivity decoding
│
├── Prediction Schemes
│   ├── prediction_scheme_delta
│   ├── prediction_scheme_parallelogram
│   ├── prediction_scheme_constrained_multi_parallelogram
│   ├── prediction_scheme_tex_coords_portable
│   └── prediction_scheme_geometric_normal
│
└── Entropy Coding
    ├── ans               - Asymmetric Numeral Systems
    ├── rans_bit_encoder/decoder - rANS bit-level coding
    ├── rans_symbol_encoder/decoder - rANS symbol coding
    └── symbol_encoding   - Symbol encoding utilities
```

## Compatibility

This crate aims for bit-exact compatibility with the official C++ Draco library. Files encoded with this crate can be decoded by the C++ implementation and vice versa.

## License

Apache-2.0 (same as the original Draco library)

## See Also

- [draco-io](../draco-io) - File format I/O with Draco compression support
- [API Reference](API.md) - Detailed API documentation
- [Google Draco](https://github.com/google/draco) - Original C++ implementation
