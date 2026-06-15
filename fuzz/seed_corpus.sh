#!/usr/bin/env bash
# Seed the libFuzzer corpus for the `decode_drc` target from repository fixtures.
#
# Copies every `*.drc` fixture under `testdata/` into
# `fuzz/corpus/decode_drc/`, using a path-derived file name so fixtures in
# different directories never collide. The corpus directory is git-ignored
# (see fuzz/.gitignore); this script reconstructs it deterministically so a
# fresh checkout can start fuzzing from good coverage instead of an empty set.
#
# Usage: fuzz/seed_corpus.sh [target_name]
set -euo pipefail

target="${1:-decode_drc}"
fuzz_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$fuzz_dir/.." && pwd)"
testdata="$repo_root/testdata"
corpus="$fuzz_dir/corpus/$target"

if [ ! -d "$testdata" ]; then
    echo "testdata directory not found at $testdata" >&2
    exit 1
fi

mkdir -p "$corpus"

count=0
while IFS= read -r -d '' file; do
    relative="${file#"$testdata"/}"
    flat_name="${relative//\//__}"
    cp -f "$file" "$corpus/$flat_name"
    count=$((count + 1))
done < <(find "$testdata" -type f -name '*.drc' -print0)

echo "Seeded $count fixture(s) into $corpus"
