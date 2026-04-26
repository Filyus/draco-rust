#!/bin/bash
# Cross-platform wrapper for the Draco Web WASM build tool.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cargo run --manifest-path "$SCRIPT_DIR/build-tool/Cargo.toml" -- "$@"
