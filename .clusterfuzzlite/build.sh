#!/bin/bash
set -euo pipefail
# ClusterFuzzLite / OSS-Fuzz build script for the cargo-fuzz targets
# (`decode_drc`, `compress_gltf`, `draco_gltf_import`). The `decode_drc` target is built with legacy
# decode features enabled by fuzz/Cargo.toml.
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

# cargo-fuzz emits the binaries under the fuzz project's target dir. CFL also
# recognizes an optional <target>_seed_corpus.zip next to each binary. Build
# those archives from the same repository fixtures and committed regression
# seeds used by the lightweight fuzz workflow, so a fresh CFL cache never
# starts from an empty corpus.
fuzz_targets=(decode_drc compress_gltf draco_gltf_import)
for target in "${fuzz_targets[@]}"; do
  cp fuzz/target/*/release/"$target" "$OUT/"

  corpus_dir="fuzz/corpus/$target"
  rm -rf "$corpus_dir"
  ./fuzz/seed_corpus.sh "$target"
  archive="$OUT/${target}_seed_corpus.zip"
  python3 - "$corpus_dir" "$archive" <<'PY'
import pathlib
import sys
import zipfile

corpus = pathlib.Path(sys.argv[1])
archive = pathlib.Path(sys.argv[2])
seeds = sorted(path for path in corpus.iterdir() if path.is_file())
if not seeds:
    raise SystemExit(f"Seed corpus for {corpus.name} is empty")
with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as output:
    for seed in seeds:
        output.write(seed, seed.name)
PY
  test -s "$archive"
done
