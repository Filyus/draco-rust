# ClusterFuzzLite build environment for the cargo-fuzz `decode_drc` target.
# Uses the OSS-Fuzz Rust base builder (nightly + cargo-fuzz preinstalled).
FROM gcr.io/oss-fuzz-base/base-builder-rust

COPY . $SRC/draco-rust
WORKDIR $SRC/draco-rust
COPY .clusterfuzzlite/build.sh $SRC/
