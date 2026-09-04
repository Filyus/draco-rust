# Performance

What this port currently measures against C++ Draco, and the harnesses that
produce those numbers. Use `--release` for timing runs; add `-- --nocapture` to
see the printed comparison output. Correctness and parity tests live in
[`TESTING.md`](TESTING.md).

Two companion documents carry what this one deliberately does not. The
reusable optimization patterns -- what to reach for and what measured to
nothing -- are in [`TRICKS.md`](TRICKS.md). The round-by-round record of how
each number was arrived at, including everything that was tried and did not
work, is in [`PERFORMANCE-LOG.md`](PERFORMANCE-LOG.md); consult it before
re-running an experiment, and append to it rather than to this file.

## Pin the reference build

**Not every C++ Draco checkout on a machine is stock Draco, and an unpinned
comparison silently picks one.** The bridge links whichever build the last
`cargo` invocation resolved, so this is the first thing to get right and the
easiest to get wrong.

One checkout here carries a local debug patch: `std::getenv("DRACO_VERBOSE")`
inside `mesh_edgebreaker_decoder_impl.cc`'s per-face loop and inside
`mesh_attribute_indices_encoding_observer.h`. The second file is not a decoder
file -- it is the traversal observer the *encoder's* `MeshTraversalSequencer`
drives once per vertex -- so the patch sits on both paths, which earlier
readings of this warning missed. `getenv` scans the environment block, so that
side's time scales with the environment: padding it with a dummy variable takes
the C++ decode from `1,690` to `12,355 us/1k faces` while the Rust side stays
at `51.6` to `52.7`.

Measured directly -- one Rust binary, one payload, one timed region, the linked
C++ library the only variable -- the Bunny at speed 5 encodes in `60,950 us`
against the patched checkout and `12,621 us` against pristine 1.5.7, a factor
of `4.8`. Both builds carry identical Release flags (`/MD /O2 /Ob2 /DNDEBUG`)
and libraries within `3%` of each other in size, so nothing about the build
configuration reveals which one is linked.

So: set `DRACO_CPP_BUILD_DIR`/`DRACO_CPP_SOURCE_DIR` explicitly for every
comparison, and say in the write-up which build a figure is against. Every
number in this document is against pristine upstream 1.5.7.

## Speed Snapshot

Seeded synthetic sweep, position-only -- `3` runs, medians, `us/1k faces`:

| Speed | Encode C++ / Rust | Encode | Decode C++ / Rust | Decode |
| ---: | ---: | ---: | ---: | ---: |
| 0 | `602` / `542` | `1.11x` | `67.3` / `83.5` | `0.81x` |
| 5 | `353` / `391` | `0.90x` | `36.3` / `55.7` | `0.65x` |
| 9 | `346` / `385` | `0.90x` | `31.6` / `50.8` | `0.62x` |
| 10 | `38.8` / `32.0` | `1.21x` | `16.6` / `11.5` | `1.44x` |

That sweep is synthetic and position-only. On the Stanford Bunny -- 69k faces,
one decoder per side, same payload, whole-decode milliseconds -- the same
comparison after the corner-table access round (`decode_loop`, 300 iterations,
pristine 1.5.7) reads:

| Asset | Speed | C++ | Rust | |
| --- | ---: | ---: | ---: | ---: |
| with normals | 1 | `13.94` | `11.80` | `1.18x` |
| with normals | 5 | `7.76` | `7.66` | `1.01x` |
| with normals | 9 | `4.46` | `4.80` | `0.93x` |
| position only | 1 | `5.80` | `6.77` | `0.86x` |
| position only | 5 | `3.36` | `4.29` | `0.78x` |
| position only | 9 | `3.01` | `3.97` | `0.76x` |

So the port is ahead on a real mesh at speed 1, at or past parity at speed 5
on both payloads, and `1.3x` behind at worst -- speed 9, position only, where
connectivity dominates most -- not the `1.6x` the synthetic sweep alone
suggests. Encode stays at parity to `10%` behind; the sequential path at speed
`10` is `1.2x` to `1.44x` ahead.

**These two tables are the oldest current figures here, and they disagree with
the newer ones below.** Both predate the release-profile change and the rounds
after it; the per-harness tables in the next section were taken at `0667cfe1`
and put the port ahead on every cell of every payload they cover. Where the two
disagree, the per-harness tables are the later measurement. Re-taking these two
against a pristine build is the outstanding item -- see
[`PERFORMANCE-LOG.md`](PERFORMANCE-LOG.md) for what the reference build was
when they were taken.

## Benchmarks

Every harness in the workspace that produces a performance number, what it is
for, how to run it, and its most recent reading. Point the C++ side at a
reference build with `DRACO_CPP_BUILD_DIR`/`DRACO_CPP_SOURCE_DIR` and pin it
explicitly -- see [Pin the reference build](#pin-the-reference-build) for what
an unpinned one costs.

Results carry the date and commit they were taken at. A table without one
predates this convention and should be re-taken before it is quoted.

### Encode Matrix, One Process

File: `crates/draco-cpp-test-bridge/examples/encode_matrix.rs`

Package: `draco-cpp-test-bridge`

Purpose: every payload against every speed, both sides interleaved in one
process, reported as medians with their spread and with the two output sizes
compared per cell. The harness to reach for when a table is wanted rather than
a single number -- it costs one build and one run instead of a process per
cell, and the spread column says which cells resolved anything.

```sh
ITERS=80 cargo run --release --manifest-path crates/Cargo.toml   -p draco-cpp-test-bridge --example encode_matrix -- 5 0,3,5,8,10 <mesh.obj>...
```

### Named Models, Both Operations, One Process

File: `crates/draco-cpp-test-bridge/examples/model_matrix.rs`

Package: `draco-cpp-test-bridge`

Purpose: compress and decompress named models on both sides in one process,
for a comparison meant to be quoted rather than acted on. The sibling matrices
sweep speeds over generated payloads; this one answers "how do the two compare
on this actual model, at these settings", and prints the two output sizes so a
cell that is not comparing like with like says so.

The timed regions are matched to the C++ shim on purpose, because that is where
a cross-implementation comparison usually goes wrong. `EncodeMeshToBuffer` and
`DecodeMeshFromBuffer` alone are inside the clock; the encoder, the buffer and
the `Mesh` clone that `set_mesh` needs are outside it, because C++ hands its
encoder a `const Mesh&` and never copies -- timing that copy would charge one
side for work the other does not do.

Quantization targets the *position* attribute by name. Attribute 0 is not
always position -- in `car.drc` it is the normal -- and quantizing the wrong one
is a difference between the two sides rather than a setting. The output sizes
are what catch it: they came out 42,688 against 41,718 until this was fixed,
and byte-identical after.

```sh
DRACO_CPP_BUILD_DIR=... DRACO_CPP_SOURCE_DIR=.../src cargo run --release   --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --example model_matrix   -- 9 20 4 10 bunny=testdata/bunny_cpp_standard.drc lamp=testdata/lamp_cpp_std.drc   car=testdata/car.drc
```

A model that exists only as a `.drc` reaches the sibling matrices, which take
`.obj`, through `examples/drc_to_obj.rs`.

Ryzen AI 7 350, one thread, C++ Draco 1.5.7 release, speed 4, 10-bit positions,
medians of nine rounds of twenty. All three wrote byte-identical output.

Measured 2026-09-04 at `0667cfe1`:

| model | faces | C++ encode | Rust encode | | C++ decode | Rust decode | |
|---|---|---|---|---|---|---|---|
| bunny | 69,451 | `19,196.0 [18,739.0..22,145.0]` | `11,812.0 [11,609.8..13,298.7]` | `1.63x` | `6,181.0 [6,100.0..7,251.0]` | `4,453.3 [4,395.5..5,700.4]` | `1.39x` |
| lamp | 12,082 | `2,753.0 [2,728.0..2,803.0]` | `1,913.3 [1,874.2..1,957.5]` | `1.44x` | `1,293.0 [1,258.0..1,322.0]` | `986.0 [959.3..1,004.1]` | `1.31x` |
| car | 1,744 | `649.0 [638.0..662.0]` | `376.7 [369.6..391.5]` | `1.72x` | `263.0 [259.0..268.0]` | `152.4 [149.4..164.2]` | `1.73x` |

Microseconds. A same-session repeat of the run agreed to within `1-2%` on
every ratio (bunny `1.61x`/`1.38x`, lamp `1.45x`/`1.32x`, car `1.72x`/`1.75x`).
The ratios are stable across runs to within 0.02x; the absolute figures are
not, and are comparable only inside their own run. Against the previous
snapshot on this machine (`1.60x`/`1.36x`, `1.42x`/`1.36x`, `1.80x`/`1.78x`),
every ratio moved by `0.03x` or less -- inside run-to-run noise, not a
regression or an improvement. An earlier run of the same command on a quieter
machine read `1.62x / 1.34x`, `1.40x / 1.37x` and `1.79x / 1.80x` while its
absolute encode times were some 20% lower -- which is the reason ratios are
quoted from a run rather than carried between them.

### Real Models, Compress Then Decompress, Every Speed

File: `crates/draco-cpp-test-bridge/tests/bench_real_models.rs`

Package: `draco-cpp-test-bridge`

Purpose: unlike `model_matrix` (one fixed speed, `.drc` fixtures written by
whichever encoder version produced them), this compresses each real asset with
the Rust encoder at every speed `0..=10` first, then decodes that
freshly-written stream on both sides -- so the stream both decoders read is
pinned to the settings being swept rather than inherited from the fixture. Not
previously catalogued in this document.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_real_models --release -- --nocapture
```

Ryzen AI 7 350, one thread, C++ Draco 1.5.7 release, 10-bit positions, median
of 21 runs (5 for the two bunny meshes, over 30k faces). Measured 2026-09-04 at
`0667cfe1`; C++/Rust ratio, `>1x` favors Rust:

| model | faces | enc x @0 | @5 (default) | @9 | @10 | dec x @0 | @5 (default) | @9 | @10 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Stanford bunny (.ply) | 69,451 | `1.3x` | `1.1x` | `1.1x` | `2.4x` | `1.1x` | `1.0x` | `1.3x` | `2.9x` |
| bunny (.drc) | 69,451 | `1.3x` | `1.3x` | `1.2x` | `2.4x` | `1.1x` | `1.3x` | `1.3x` | `2.9x` |
| car | 1,744 | `3.2x` | `1.9x` | `1.8x` | `2.2x` | `1.7x` | `1.8x` | `2.2x` | `3.3x` |
| lamp | 12,082 | `1.6x` | `1.5x` | `1.8x` | `1.9x` | `1.2x` | `1.3x` | `1.7x` | `1.9x` |

At speed 10 (sequential, no edgebreaker) the port is `1.9x`-`2.9x` ahead on
every model and every operation -- the largest, most consistent lead in this
document, and the full 11-speed run (this table shows four columns of it) has
never previously appeared here: the harness existed (one commit,
`c61a14fd`) but nothing had run it through into `PERFORMANCE.md`. Speeds 6-9
show a size jump on three of the four models (e.g. lamp `143,412B` to
`151,701B` between speed 5 and 6) worth noting for whoever next reads the
compression-ratio side of this sweep, though it is not this table's subject.
Full per-speed output, including the smaller car and lamp assets which sit
inside a few percent of each other at the small end, is in the harness's own
`--nocapture` output rather than reproduced here in full.

### Decode Through The C++ Bridge

File: `crates/draco-cpp-test-bridge/tests/bench_decode_cpp_vs_rust.rs`

Package: `draco-cpp-test-bridge`

Purpose: in-process decode benchmark, C++ bridge vs Rust. The timed region is
matched between C++ and Rust, and the reported result uses median batches. The
mesh is a synthetic position-only grid (a regular triangulated plane), swept
over three sizes and every C++-encoded speed -- distinct from the seeded
mesh sweep further up, which mixes grid, fan, ribbon and torus topologies, and
from `encode_matrix`/`decode_matrix`'s interleaved-in-one-process design.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_decode_cpp_vs_rust --release -- --nocapture
```

Ryzen AI 7 350, pristine C++ Draco 1.5.7, median per-iteration over 9 batches.
Measured 2026-09-04 at `0667cfe1`, C++/Rust speedup (`>1x` favors Rust):

| grid | speed 0 | speed 1 | speed 5 | speed 10 | overall |
| --- | ---: | ---: | ---: | ---: | ---: |
| 20x20 (722 faces) | `1.04x` | `1.22x` | `1.32x` | `1.80x` | `1.20x` |
| 50x50 (4,802 faces) | `1.25x` | `1.24x` | `1.32x` | `1.72x` | `1.29x` |
| 100x100 (19,602 faces) | `1.25x` | `1.26x` | `1.32x` | `1.70x` | `1.30x` |

Every cell favors Rust, the lead widens with mesh size at every non-10 speed,
and speed 10 (sequential, no edgebreaker) is the largest margin on all three
sizes -- the same shape the real-model and seeded sweeps above show.

### Encode Through The C++ Bridge

File: `crates/draco-cpp-test-bridge/tests/bench_encode_cpp_vs_rust.rs`

Package: `draco-cpp-test-bridge`

Purpose: in-process encode benchmark, C++ bridge vs Rust, without external
process startup cost. Same synthetic grid family as the decode test above, at
two sizes.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_encode_cpp_vs_rust --release -- --nocapture
```

Same machine and reference build, averaged over 5 iterations. Measured
2026-09-04 at `0667cfe1`, byte-identical output on every cell (`MATCH`):

| grid | speed 0 | speed 1 | speed 5 | speed 10 |
| --- | ---: | ---: | ---: | ---: |
| 50x50 (4,802 faces) | `1.44x` | `1.57x` | `1.51x` | `1.68x` |
| 100x100 (19,602 faces) | `1.54x` | `1.55x` | `1.40x` | `1.77x` |

### Decode Matrix, One Process

File: `crates/draco-cpp-test-bridge/examples/decode_matrix.rs`

Package: `draco-cpp-test-bridge`

Purpose: the decode side of `encode_matrix` -- every payload against every
speed, both sides interleaved in one process. Each cell encodes once with the
Rust encoder at that speed, then decodes the same bytes on both sides; point
and face counts are compared per cell. `ALLOC=1` adds allocations and bytes
per decode, `SAMPLE_ALLOC=1` adds backtraces for the first payload's decode.
`--features mimalloc` swaps the global allocator to ask how much of a gap is
the allocator rather than the decode.

```sh
ITERS=40 cargo run --release --manifest-path crates/Cargo.toml   -p draco-cpp-test-bridge --example decode_matrix -- 5 0,5,10 <mesh.obj>...
```

### Corner-Table Construction, One Stage, Either Side

File: `crates/draco-cpp-test-bridge/examples/corner_table_loop.rs`

Package: `draco-cpp-test-bridge`

Purpose: `CornerTable::init`/`Create` alone, C++ against Rust, on an identical
face array built once outside the timed loop -- for isolating one stage a
whole-encode benchmark would otherwise fold into "a few percent of the
total". Vertex and degenerated-face counts are printed so a run that built two
different tables is visible.

```sh
cargo run --release --manifest-path crates/Cargo.toml   -p draco-cpp-test-bridge --example corner_table_loop -- <mesh.obj> [iters]
```

### One Decoder, One Payload, One Loop

File: `crates/draco-cpp-test-bridge/examples/decode_loop.rs`

Package: `draco-cpp-test-bridge`

Purpose: exactly one side per process, one payload, one speed -- for a
profiler or counting allocator that cannot separate C++ from Rust when both
run in the same process, unlike `bench_decode_cpp_vs_rust`. Reports
allocations and bytes per decode on the Rust side. `SAMPLE_ALLOC=1` backtraces
allocations of 64 KB or more; `REUSE_DECODE=1` decodes into one `Mesh` through
one `MeshDecoder` for the whole loop instead of rebuilding both per iteration.

```sh
cargo run --release --manifest-path crates/Cargo.toml   -p draco-cpp-test-bridge --example decode_loop -- <mesh.obj> cpp|rust <speed> <iters>
```

### One Encoder, One Payload, One Loop

File: `crates/draco-cpp-test-bridge/examples/encode_loop.rs`

Package: `draco-cpp-test-bridge`

Purpose: the encode-side sibling of `decode_loop`, same one-side-per-process
shape. The C++ side goes through `profile_cpp_encode`, which is
position-only, so pass a position-only mesh when comparing sides.
`REUSE_ENCODER=1` keeps one `MeshEncoder` across the loop instead of building
one per iteration -- a converter walking many primitives against a caller
encoding one mesh.

```sh
cargo run --release --manifest-path crates/Cargo.toml   -p draco-cpp-test-bridge --example encode_loop -- <mesh.obj> cpp|rust <speed> <iters>
```

### Encode/Decode Matrix

File: `crates/draco-cpp-test-bridge/tests/bench_encode_decode_matrix.rs`

Package: `draco-cpp-test-bridge`

Purpose: encode/decode performance and correctness across multiple speeds and
mesh sizes. Two tests: `bench_generated_encode_decode_matrix` covers a sphere,
a subdivided cube and a 100x100 grid; `bench_encode_decode_matrix` is the
100x100 grid alone, full encode-then-decode, every speed.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_encode_decode_matrix --release -- --nocapture
```

Same machine and reference build. Measured 2026-09-04 at `0667cfe1`, byte
size and decoded point/face counts matched on every cell (`OK`); `bench_encode_decode_matrix`,
the 100x100 grid (19,602 faces), C++/Rust speedup:

| speed | encode | decode |
| ---: | ---: | ---: |
| 0 | `1.56x` | `1.23x` |
| 1 | `1.51x` | `1.20x` |
| 5 | `1.65x` | `1.27x` |
| 9 | `1.63x` | `1.31x` |
| 10 | `1.80x` | `1.81x` |

`bench_generated_encode_decode_matrix`'s cube subdiv20 (4,800 faces) reads
similarly: encode `1.40x`-`1.80x`, decode `1.23x`-`1.85x` across speeds
0-10, both ends anchored by the same speed-10 sequential-path lead the other
tables in this document show.

### Decode Real Files

File: `crates/draco-cpp-test-bridge/tests/bench_decode_real_files.rs`

Package: `draco-cpp-test-bridge`

Purpose: decode timing on real `.drc` files from testdata, C++ bridge vs Rust.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_decode_real_files --release -- --nocapture
```

### Rust vs External C++ Tools

File: `crates/draco-core/tests/bench_external_cpp_encode.rs`

Package: `draco-core`

Purpose: Rust encode/decode compared with external C++ encoder/decoder tools.
Note that C++ runs here include process startup overhead.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-core --test bench_external_cpp_encode --release -- --nocapture
```

### Point Cloud Smoke Benchmark

File: `crates/draco-core/tests/bench_point_cloud.rs`

Package: `draco-core`

Purpose: point cloud encode/decode performance smoke test.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-core --test bench_point_cloud --release -- --nocapture
```

### One Point-Cloud Operation, Either Side

Files: `crates/draco-cpp-test-bridge/examples/pointcloud_drc.rs` and
`crates/draco-cpp-test-bridge/cpp/pointcloud_drc.cpp`

Purpose: the point-cloud shape of `encode_drc`/`decode_drc` -- one operation,
an iteration count to subtract against under callgrind, and the only harness
that reaches the KD-tree encoder at all. The cloud is generated rather than
read (there is no point-cloud corpus here), deterministically from the point
count, by the same generator on both sides; generation happens outside the
loop, so the subtraction cancels it. Check the printed byte count matches
across the two before reading any figure.

```sh
./pointcloud_drc <encode|decode> <sequential|kdtree> <points> [iters]
```

### What A Decode Actually Produced

File: `crates/draco-cpp-test-bridge/examples/dump_decoded.rs`

Package: `draco-cpp-test-bridge`

Purpose: decode a `.drc` and write every face and every attribute value, in
decode order, as bytes -- so "the output did not change" is one `cmp` between
two builds rather than an argument about which tests would have caught it. The
counterpart of `decode_drc.rs`, which reports only a point and face count, and a
count is not the output: a prediction round that got the arithmetic wrong on
one component of one entry still decodes the same number of points. Every
optimization round should run it over the seeded payloads and `testdata/*.drc`
against its parent commit.

```sh
cargo run --release --manifest-path crates/Cargo.toml   -p draco-cpp-test-bridge --example dump_decoded -- grid_s5.drc out.bin
```

## Profiling And Micro-Benchmarks

### Sequential Pipeline Profile

File: `crates/draco-cpp-test-bridge/tests/profile_sequential_pipeline.rs`

Package: `draco-cpp-test-bridge`

Purpose: detailed sequential encoder/decoder stage profiling, rANS loop
micro-profile, clean and seeded topology cases, clone/setup overhead, and Rust
vs C++ breakdowns.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test profile_sequential_pipeline --release -- --nocapture
```

Useful test functions in this file:

- `profile_sequential_pipeline`
- `profile_detailed_breakdown`
- `profile_encoding_stages`
- `profile_symbol_encoding_details`
- `profile_rans_loop_micro`
- `profile_full_encode_breakdown`
- `profile_clean_topologies`
- `profile_seeded_mesh_sweep`
- `profile_real_corpus_gaussian_sweep`
- `profile_mesh_clone_overhead`
- `profile_point_ids_creation`
- `profile_rust_vs_cpp_breakdown`
- `profile_decode_rust_vs_cpp`
- `profile_decode_sequential_breakdown`

To turn profile data into a faster binary (a separate, build-time step rather
than a test), see [`PGO.md`](PGO.md).

