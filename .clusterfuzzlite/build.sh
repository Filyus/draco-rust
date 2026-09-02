#!/bin/bash
set -euo pipefail
# ClusterFuzzLite / OSS-Fuzz build script for every cargo-fuzz target in
# fuzz/Cargo.toml. The `decode_drc` target is built with legacy decode features
# enabled there.
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
#
# Every [[bin]] in fuzz/Cargo.toml, and the list is checked against it below:
# a target added there and not here would build and then never be fuzzed by
# this layer, which is how `mesh_text_roundtrip` went missing once.
fuzz_targets=(decode_drc compress_gltf draco_gltf_import fbx_read_scene fbx_roundtrip ktx2_transcode encode_drc mesh_text_readers mesh_text_roundtrip)
declared=$(grep -E '^name = "' fuzz/Cargo.toml | grep -v draco-fuzz | sed -E 's/name = "(.*)"/\1/' | sort)
listed=$(printf '%s\n' "${fuzz_targets[@]}" | sort)
if [ "$declared" != "$listed" ]; then
  echo "fuzz targets in fuzz/Cargo.toml and in this script differ:" >&2
  diff <(echo "$declared") <(echo "$listed") >&2 || true
  exit 1
fi

# The same libFuzzer dictionaries the in-repo workflow passes with -dict=,
# handed over the OSS-Fuzz way: a <target>.options file next to the binary.
declare -A dicts=(
  [mesh_text_readers]=mesh_text
  [mesh_text_roundtrip]=mesh_text
  [fbx_read_scene]=fbx
  [fbx_roundtrip]=fbx
  [compress_gltf]=gltf
  [draco_gltf_import]=gltf
  [ktx2_transcode]=ktx2
)
cp fuzz/dict/*.dict "$OUT/"
for target in "${!dicts[@]}"; do
  printf '[libfuzzer]\ndict = %s.dict\n' "${dicts[$target]}" > "$OUT/$target.options"
done

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
