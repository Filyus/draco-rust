#!/usr/bin/env bash
# Re-vendor the reference transcoder at a given revision.
#
#   tools/basis-cpp-oracle/vendor.sh [revision]
#
# Vendoring by hand is how a file list drifts: one header added upstream, not
# copied, and the build fails somewhere unhelpful; or one file quietly edited
# and never noticed. This copies exactly the list below, rewrites the manifest
# of hashes, and leaves the rest to `cargo test`, which will say whether the
# reference still produces what the goldens claim.
#
# The list is what this build configuration compiles and no more. ATC, PVRTC
# and FXT1 are switched off in build.rs rather than vendored, which keeps about
# 1.2 MB of tables out of the repository.
set -euo pipefail

revision="${1:-9bebe16726b3a61c8c213eeee3b7cffb462ef34e}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
csrc="$here/csrc"

files=(
  basisu.h
  basisu_astc_cfgs.inl
  basisu_astc_hdr_core.h
  basisu_astc_helpers.h
  basisu_containers.h
  basisu_containers_impl.h
  basisu_dds_transcoder.inl
  basisu_etc1_mods.inl
  basisu_file_headers.h
  basisu_idct.h
  basisu_transcoder.cpp
  basisu_transcoder.h
  basisu_transcoder_internal.h
  basisu_transcoder_uastc.h
  basisu_xbc7_decoder.h
  basisu_xbc7_decoder.inl
  basisu_transcoder_tables_astc.inc
  basisu_transcoder_tables_bc7_m5_alpha.inc
  basisu_transcoder_tables_bc7_m5_color.inc
  basisu_transcoder_tables_dxt1_5.inc
  basisu_transcoder_tables_dxt1_6.inc
)

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "Cloning basis_universal at $revision"
git clone -q --filter=blob:none --no-checkout \
  https://github.com/BinomialLLC/basis_universal.git "$work/bu"
git -C "$work/bu" fetch -q --depth 1 origin "$revision"
git -C "$work/bu" checkout -q "$revision"

for file in "${files[@]}"; do
  cp "$work/bu/transcoder/$file" "$csrc/$file"
done
cp "$work/bu/LICENSE" "$csrc/LICENSE"

date="$(git -C "$work/bu" show -s --format=%cs "$revision")"

{
  echo "# The vendored reference transcoder."
  echo "#"
  echo "# Source: https://github.com/BinomialLLC/basis_universal, transcoder/"
  echo "# Revision: $revision ($date)"
  echo "# Licence: Apache-2.0, kept as LICENSE beside these files."
  echo "#"
  echo "# Only the files this build configuration compiles are here; ATC, PVRTC and"
  echo "# FXT1 are switched off in build.rs rather than vendored, which leaves about"
  echo "# 1.2 MB of tables out."
  echo "#"
  echo "# Written by vendor.sh and verified by tests/vendoring.rs, so a local edit to"
  echo "# somebody else's source is a failing test rather than a silent divergence."
  echo "#"
  echo "# sha256  file"
  cd "$csrc"
  for file in $(printf '%s\n' "${files[@]}" LICENSE | sort); do
    printf '%s  %s\n' "$(sha256sum "$file" | cut -d' ' -f1)" "$file"
  done
} > "$csrc/UPSTREAM.txt"

echo "Vendored ${#files[@]} files plus LICENSE at $revision ($date)"
echo "Now run:"
echo "  cargo test --manifest-path tools/basis-cpp-oracle/Cargo.toml"
echo "  cargo run --manifest-path tools/basis-cpp-oracle/Cargo.toml --bin bake_goldens"
