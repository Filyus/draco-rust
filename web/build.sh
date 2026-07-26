#!/bin/bash
# Cross-platform wrapper for the Draco Web WASM build tool.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# The front-end is TypeScript under web/src; www/ holds only its build output
# alongside the WASM packages, so compile it before anything serves that
# directory. Use `npm run watch:ts` while developing the front-end itself.
(cd "$SCRIPT_DIR" && npm run build:ts)

cargo run --manifest-path "$SCRIPT_DIR/build-tool/Cargo.toml" -- "$@"
