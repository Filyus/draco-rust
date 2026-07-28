# basis-cpp-oracle

The reference Basis transcoder, vendored and compiled here, so that
`draco-texture` can be checked against it anywhere rather than on one machine.

```sh
cargo test --manifest-path tools/basis-cpp-oracle/Cargo.toml
```

247 images: five fixtures, every level, seven targets, byte for byte.

## Why it exists

The node gates in `web/tests` compare against Binomial's prebuilt WASM at a
path inside a three.js checkout. Three things follow from that, and this undoes
all three.

**They skip on any machine without it**, which is every CI runner: the gate
prints `SKIPPED` and exits 0. Until this crate, nothing byte-exact was verified
anywhere except one developer machine.

**That build is dated 2024-11-29**, older than the source `draco-texture` was
ported from. The original plan for this work named oracle-versus-source skew as
its second risk and mitigated it by measurement — the two agree, so nothing
changed. This removes the question instead: the vendored source under `csrc/`
is revision `9bebe16`, the one the port was made from.

**It degrades.** Fed a malformed file, the WASM module can be left reporting
success from `transcodeImage` having written nothing, which the differential
gate works around with a canary. A fresh `ktx2_transcoder` per call has no such
state.

## The configuration is the point

`build.rs` compiles with `BASISD_SUPPORT_ASTC_HIGHER_OPAQUE_QUALITY=0`. That is
the profile every emscripten build gets — and so what a browser runs, and what
`draco-texture` implements. The reference's other profile adds two branches to
ETC1S-to-ASTC and a second 64 KiB table.

This is the only check that can reach that pair at all. The `basisu` crate is
ported from the native profile, so `tools/basisu-probe` has to skip it. Here
the switch is ours: set it to `1` and the same test measures what those two
branches are worth, rather than leaving it an estimate.

Formats nothing here targets — ATC, PVRTC, FXT1 — are switched off rather than
vendored, which keeps about 1.2 MB of `.inc` tables out of the repository.

Zstd is not linked in. It is undone on the Rust side before the bytes reach the
oracle, using the decompressor `draco-texture` already carries; linking a
second one in to test a transcoder would be answering a question nobody asked.

## What it found

The C++ asserts that a level's stored and uncompressed lengths agree when there
is no supercompression. This reader did not check it, and the seed generator
was writing zero there — so the repository was producing files it would read
happily and the reference would refuse. Both fixed, with a regression test.

## Vendoring

`csrc/` is `transcoder/` from `BinomialLLC/basis_universal` at `9bebe16`,
Apache-2.0, with only the files this configuration compiles. Its `LICENSE` is
kept beside it. To move to a later revision, copy the same file list across and
run the test: a difference in output is either a change in the reference or a
defect here, and the test says which files to look at.
