#!/bin/bash
# Build script for Draco Web WASM modules
# Requires wasm-pack to be installed: cargo install wasm-pack

set -e

echo "Building Draco Web WASM Modules"
echo "================================"

modules=(
    "obj-reader-wasm"
    "obj-writer-wasm"
    "ply-reader-wasm"
    "ply-writer-wasm"
    "gltf-reader-wasm"
    "gltf-writer-wasm"
    "fbx-reader-wasm"
    "fbx-writer-wasm"
)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WEB_DIR="$SCRIPT_DIR"
OUTPUT_DIR="$WEB_DIR/www/pkg"

# Create output directory
mkdir -p "$OUTPUT_DIR"

echo ""
echo "Output directory: $OUTPUT_DIR"

for module in "${modules[@]}"; do
    echo ""
    echo "Building $module..."
    
    MODULE_PATH="$WEB_DIR/$module"
    
    if [ ! -d "$MODULE_PATH" ]; then
        echo "  Module not found: $MODULE_PATH"
        continue
    fi
    
    cd "$MODULE_PATH"
    
    # Build with wasm-pack
    # Convert module name from kebab-case to snake_case for output
    out_name="${module//-/_}"
    wasm-pack build --target web --out-dir "$OUTPUT_DIR" --out-name "$out_name"
    
    if [ $? -eq 0 ]; then
        echo "  Success!"
    else
        echo "  Build failed!"
    fi
    
    cd "$WEB_DIR"
done

echo ""
echo "================================"
echo "Build complete!"
echo ""
echo "To serve the web app, run:"
echo "  cd www"
echo "  python -m http.server 8080"
echo "  # Or use any static file server"
echo ""
echo "Then open http://localhost:8080 in your browser"
