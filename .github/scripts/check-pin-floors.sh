#!/usr/bin/env bash
# Builds a crate as it would be published against the lowest version of each
# internal dependency its manifest admits, taken from crates.io.
#
# usage: check-pin-floors.sh <crate>
#
# A dependent's pin is a floor: `draco-core = "2.2.1"` admits 2.2.1 and every
# later 2.x. The workspace and `test-packaged-crates.ps1` build it against the
# draco-core in this tree, and `cargo publish --dry-run` against the newest
# compatible one on crates.io, so a pin left below the API the code calls
# passes them all and breaks for a user whose lockfile holds the floor. This
# packages the crate, holds each internal dependency at its pinned version and
# checks the result with every feature on.
#
# Every pinned version must already be on crates.io, which the publish
# preflight checks before it calls this. Set PIN_CHECK_ALLOW_DIRTY=1 to package
# a tree with uncommitted changes when running it by hand.
set -euo pipefail

crate="${1:?usage: check-pin-floors.sh <crate>}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
manifest="$root/crates/$crate/Cargo.toml"
python="${PYTHON:-python3}"

read_manifest() {
  "$python" - "$manifest" "$1" <<'EOF'
import sys
import tomllib

manifest = tomllib.load(open(sys.argv[1], "rb"))
if sys.argv[2] == "version":
    print(manifest["package"]["version"])
else:
    internal = {"draco-core", "draco-io"}
    for name, spec in manifest.get("dependencies", {}).items():
        if name in internal and isinstance(spec, dict) and "version" in spec:
            print(name, spec["version"])
EOF
}

deps="$(read_manifest deps)"
if [ -z "$deps" ]; then
  echo "$crate has no internal dependencies to hold at their floor."
  exit 0
fi
version="$(read_manifest version)"

package_args=(--manifest-path "$manifest" --no-verify)
if [ "${PIN_CHECK_ALLOW_DIRTY:-0}" = "1" ]; then
  package_args+=(--allow-dirty)
fi
cargo package "${package_args[@]}"

target="$(cargo metadata --manifest-path "$manifest" --format-version 1 --no-deps \
  | "$python" -c "import json, sys; print(json.load(sys.stdin)['target_directory'])")"
archive="$target/package/$crate-$version.crate"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
# Through stdin: GNU tar reads a `D:` path on Windows as a remote host.
tar -xzf - -C "$work" <"$archive"
cd "$work/$crate-$version"
export CARGO_TARGET_DIR="$work/target"

cargo generate-lockfile
while read -r name pin; do
  cargo update --package "$name" --precise "$pin"
  echo "Holding $name at $pin."
done <<<"$deps"
cargo check --all-features
echo "$crate $version builds against the floor of every internal pin."
