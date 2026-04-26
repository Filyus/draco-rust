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
    
    # Build with wasm-pack.
    # Convert module name from kebab-case to snake_case for output, dropping the
    # crate-only "-wasm" suffix so generated files match the web app imports.
    base_name="${module%-wasm}"
    out_name="${base_name//-/_}"
    wasm-pack build --release --no-opt --target web --out-dir "$OUTPUT_DIR" --out-name "$out_name"

    wasm_file="$OUTPUT_DIR/${out_name}_bg.wasm"
    clean_wasm_file="$OUTPUT_DIR/${out_name}.wasm"
    js_file="$OUTPUT_DIR/${out_name}.js"

    if [ -f "$wasm_file" ]; then
        if command -v wasm-opt >/dev/null 2>&1; then
            wasm-opt "$wasm_file" \
                -Oz \
                --enable-bulk-memory \
                --enable-nontrapping-float-to-int \
                --enable-sign-ext \
                --enable-mutable-globals \
                -o "$wasm_file"
        else
            echo "  wasm-opt not found; leaving ${out_name}_bg.wasm unoptimized"
        fi

        mv "$wasm_file" "$clean_wasm_file"
        sed -i 's/_bg\.wasm/.wasm/g' "$js_file"
        rm -f "$OUTPUT_DIR/${out_name}"*_bg.wasm
    fi
    
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
