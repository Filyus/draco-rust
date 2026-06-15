#!/bin/bash -eu
# ClusterFuzzLite / OSS-Fuzz build script for the cargo-fuzz targets
# (`decode_drc`, `compress_gltf`).
#
# This repository keeps the workspace under crates/ and has no Cargo.toml at the
# root, so cargo-fuzz must be pointed at the fuzz project with `--fuzz-dir fuzz`.
# The fuzz profile (fuzz/Cargo.toml) already builds with release semantics.

cd "$SRC/draco-rust"

# base-builder-rust requests the sanitizer through $SANITIZER. cargo-fuzz drives
# its own instrumentation via --sanitizer for address/leak/etc.; coverage uses a
# dedicated path, so fall back to the default build there.
case "${SANITIZER:-address}" in
  coverage)
    cargo fuzz build -O --fuzz-dir fuzz
    ;;
  *)
    cargo fuzz build -O --fuzz-dir fuzz --sanitizer "${SANITIZER:-address}"
    ;;
esac

# cargo-fuzz emits the binaries under the fuzz project's target dir.
cp fuzz/target/*/release/decode_drc "$OUT/"
cp fuzz/target/*/release/compress_gltf "$OUT/"
