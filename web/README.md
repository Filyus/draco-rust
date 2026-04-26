# Draco Web 3D Format Converter

A web-based 3D format converter using WebAssembly modules built from the Draco I/O library.

## Features

- **Multiple Format Support**: Convert between OBJ, PLY, glTF/GLB, and FBX formats
- **Draco Compression**: Optional Draco mesh compression for glTF/GLB exports
- **Modular WASM**: Each reader/writer is a separate WASM module for efficient loading
- **Browser-Based**: No server required, all processing happens client-side
- **Configurable**: Adjustable quantization bits for Draco compression

## Architecture

The project is organized with each reader and writer as its own WASM module:

```
web/
├── Cargo.toml              # Workspace configuration
├── build.ps1               # Windows build script
├── build.sh                # Unix build script
├── README.md               # This file
│
├── obj-reader-wasm/        # OBJ format reader
├── obj-writer-wasm/        # OBJ format writer
├── ply-reader-wasm/        # PLY format reader
├── ply-writer-wasm/        # PLY format writer
├── gltf-reader-wasm/       # glTF/GLB format reader (with Draco support)
├── gltf-writer-wasm/       # glTF/GLB format writer (with Draco compression)
├── fbx-reader-wasm/        # FBX binary format reader
├── fbx-writer-wasm/        # FBX binary format writer
│
└── www/                    # Web application
    ├── index.html          # Main page
    ├── style.css           # Styling
    ├── app.js              # Application logic
    └── pkg/                # Built WASM modules (generated)
```

## Building

### Prerequisites

1. **Rust**: Install from https://rustup.rs/
2. **wasm-pack**: Install with `cargo install wasm-pack`

### Build Steps

#### Windows (PowerShell)

```powershell
cd web
.\build.ps1
```

#### Unix/macOS

```bash
cd web
chmod +x build.sh
./build.sh
```

### Manual Build

You can also build individual modules:

```bash
cd obj-reader-wasm
wasm-pack build --target web --out-dir ../www/pkg --out-name obj_reader_wasm
```

## Running

After building, serve the `www` directory with any static file server:

```bash
cd www
python -m http.server 8080
```

Then open http://localhost:8080 in your browser.

### Alternative Servers

```bash
# Node.js (install: npm install -g serve)
serve www

# PHP
php -S localhost:8080 -t www

# Ruby
ruby -run -e httpd www -p 8080
```

## Usage

1. **Load a File**: Drag and drop a 3D file onto the drop zone, or click to browse
2. **Review**: Check the mesh information displayed (vertex count, triangles, etc.)
3. **Configure Export**: Select output format and options
4. **Export**: Click the Export button to download the converted file

### Supported Formats

| Format | Read | Write | Notes |
|--------|------|-------|-------|
| OBJ | ✓ | ✓ | ASCII format, vertices, normals, UVs, faces |
| PLY | ✓ | ✓ | ASCII format, vertices, normals, colors |
| glTF | ✓ | ✓ | JSON format with embedded/external buffers |
| GLB | ✓ | ✓ | Binary glTF container |
| FBX | ✓ | ✓ | Binary FBX 7.x format |

### Draco Compression

When exporting to glTF/GLB, you can enable Draco compression:

- **Position Quantization**: 8-16 bits (default: 14)
- **Normal Quantization**: 6-14 bits (default: 10)
- **TexCoord Quantization**: 8-14 bits (default: 12)

Higher values = better quality, larger file size.

## API Reference

Each WASM module exposes the following functions:

### Common Functions

```javascript
// Get module version
const version = module.version();  // "0.1.0"

// Get module name
const name = module.module_name();  // e.g., "OBJ Reader"

// Get supported extensions
const exts = module.supported_extensions();  // e.g., ["obj"]
```

### Reader Functions

```javascript
// Parse file content (string for text formats)
const result = objReader.parse_obj(textContent);

// Parse file content (bytes for binary formats)
const result = fbxReader.parse_fbx(uint8Array);

// Result structure:
{
    success: boolean,
    meshes: [{
        name: string | null,
        positions: Float32Array,  // [x0, y0, z0, x1, y1, z1, ...]
        indices: Uint32Array,     // [i0, i1, i2, ...]
        normals: Float32Array,    // [nx0, ny0, nz0, ...]
        uvs: Float32Array,        // [u0, v0, u1, v1, ...]
    }],
    error: string | null,
    warnings: string[],
}
```

### Writer Functions

```javascript
// Create file content
const result = objWriter.create_obj(meshData, options);

// Mesh data structure:
{
    name: "MeshName",
    positions: [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, ...],
    indices: [0, 1, 2, ...],
    normals: [0.0, 1.0, 0.0, ...],  // optional
    uvs: [0.0, 0.0, 1.0, 0.0, ...], // optional
}

// Result structure:
{
    success: boolean,
    data: string,           // For text formats (OBJ, PLY, glTF)
    binary_data: Uint8Array, // For binary formats (GLB, FBX)
    error: string | null,
}
```

## Development

### Project Structure

Each WASM module is a separate Rust crate that depends on:
- `draco-core`: Core mesh encoding/decoding functionality
- `draco-io`: I/O traits and implementations (used for reference)
- `wasm-bindgen`: Rust/JavaScript interop
- `serde`: Data serialization

### Adding a New Format

1. Create a new module directory (e.g., `xyz-reader-wasm/`)
2. Add to workspace members in `web/Cargo.toml`
3. Implement the reader/writer with wasm-bindgen exports
4. Update the web app to load and use the new module

## License

See the main project LICENSE file.
