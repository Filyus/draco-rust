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

## Re-Taking Every Figure

File: `tools/perf-suite`, its own workspace

Every comparison this document quotes, in one run: the seeded sweep (three
runs, their median), `model_matrix` at speeds 4 and 10, the real models, the
grid encode and decode, the encode/decode matrix, KTX2 transcoding and Zstd.
Before a release that claims a speed change, and whenever a table here is
re-taken, take it from this rather than from one harness by hand.

```sh
DRACO_CPP_BUILD_DIR=... DRACO_CPP_SOURCE_DIR=... ZSTD_SOURCE_DIR=... \
  cargo run --release --manifest-path tools/perf-suite/Cargo.toml
```

It refuses to start when the reference is unpinned, a Debug build, or carries
the `DRACO_VERBOSE` patch (looked for in the linked library and in the
headers the bridge compiles), and when the tree has uncommitted changes. It
builds every harness before timing any, with `DRACO_REQUIRE_CPP_BRIDGE=1`, so a
missing bridge fails the build instead of skipping the comparisons. The
harnesses then run one at a time. Each appends its rows to the
file `PERF_JSONL` names, and a step that writes no rows counts as failed,
whatever its exit status.

Everything lands in `.scratch/perf-suite/<date>-<commit>/`: each step's output
and rows, the machine and the reference, and `report.md` with this document's
tables and a list of every cell whose two sides wrote different output.
`--dry-run` checks the preconditions and prints the plan, `--only` runs a
subset, and `--report <dir>` rewrites a report from an earlier run's rows.
Without `ZSTD_SOURCE_DIR` the Zstd step is skipped and the report says so.

## Speed Snapshot

Seeded synthetic sweep, position-only, 12 meshes over 4 families: `us` per
1,000 faces, the median over the meshes of a run, then the median over three
runs. Measured 2026-10-10 at `57d8cb22` with `tools/perf-suite`:

| Speed | Encode C++ / Rust | Encode | Decode C++ / Rust | Decode |
| ---: | ---: | ---: | ---: | ---: |
| 0 | `420.3` / `259.1` | `1.62x` | `92.0` / `72.3` | `1.27x` |
| 1 | `613.7` / `293.5` | `2.09x` | `87.9` / `64.6` | `1.36x` |
| 2 | `229.1` / `122.9` | `1.86x` | `73.2` / `49.6` | `1.48x` |
| 3 | `227.2` / `121.0` | `1.88x` | `70.2` / `53.6` | `1.31x` |
| 4 | `232.0` / `124.7` | `1.86x` | `63.2` / `48.2` | `1.31x` |
| 5 | `202.5` / `103.3` | `1.96x` | `52.5` / `39.8` | `1.32x` |
| 6 | `205.8` / `111.1` | `1.85x` | `53.6` / `40.8` | `1.31x` |
| 7 | `200.7` / `114.4` | `1.75x` | `51.1` / `39.6` | `1.29x` |
| 8 | `190.0` / `93.0` | `2.04x` | `45.1` / `32.9` | `1.37x` |
| 9 | `201.0` / `97.9` | `2.05x` | `44.6` / `33.7` | `1.32x` |
| 10 | `52.3` / `17.0` | `3.08x` | `21.2` / `9.9` | `2.13x` |

The port is ahead at every speed on both operations: `1.6x`-`3.1x` on encode
and `1.3x`-`2.1x` on decode, widest at speed 10, the sequential coder. The
named-model and real-model tables below show the same on real meshes, with
narrower margins on decode.

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

Ryzen AI 7 350, one thread, C++ Draco 1.5.7 release, 10-bit positions,
medians of nine rounds of twenty, at speed 4 (EdgeBreaker) and speed 10
(sequential). Every cell wrote byte-identical output on both sides.

Measured 2026-10-10 at `57d8cb22`, speed 4:

| model | faces | C++ encode | Rust encode |  | C++ decode | Rust decode |  |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| bunny | 69,451 | `28,016.0 [20,733.0..32,612.0]` | `16,488.7 [12,512.9..18,283.6]` | `1.70x` | `8,903.0 [7,224.0..9,959.0]` | `6,431.8 [5,443.1..6,955.5]` | `1.38x` |
| lamp | 12,082 | `4,226.0 [4,111.0..4,426.0]` | `2,971.3 [2,688.7..3,056.7]` | `1.42x` | `1,949.0 [1,873.0..2,059.0]` | `1,488.9 [1,397.5..1,515.2]` | `1.31x` |
| car | 1,744 | `998.0 [938.0..1,034.0]` | `556.5 [501.7..576.2]` | `1.79x` | `411.0 [378.0..464.0]` | `228.7 [219.2..247.9]` | `1.80x` |

Speed 10:

| model | faces | C++ encode | Rust encode |  | C++ decode | Rust decode |  |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| bunny | 69,451 | `4,083.0 [3,696.0..4,293.0]` | `1,487.7 [1,359.8..1,677.3]` | `2.74x` | `1,767.0 [1,460.0..2,135.0]` | `638.5 [468.7..738.2]` | `2.77x` |
| lamp | 12,082 | `620.0 [558.0..650.0]` | `252.8 [229.9..270.4]` | `2.45x` | `286.0 [270.0..309.0]` | `115.7 [107.3..125.0]` | `2.47x` |
| car | 1,744 | `158.0 [153.0..160.0]` | `65.3 [59.8..72.6]` | `2.42x` | `70.0 [68.0..75.0]` | `31.5 [29.9..33.6]` | `2.22x` |

Microseconds. The ratios are stable across runs to within about 0.02x; the
absolute figures are not, and are comparable only inside their own run, which
is why ratios are quoted from a run rather than carried between them.

The run of 2026-10-04 at `99969d2d` timed the draco-core 2.2.1 release beside
that tree, the two binaries alternated (2.2.1, this, this, 2.2.1) with the C++ side as the control
that should not move. At speed 4 the Rust encode is 3-5% faster than 2.2.1's
and the decode 0-4%: the point-cloud rounds since reached the attribute coding
a mesh shares -- the rANS write and read, the symbol plan, the tagged packing,
the wrap transform's loop -- and not the connectivity, which is most of an
EdgeBreaker mesh's time. At speed 10, where attribute coding is most of the
work, the encode is 15-26% faster and the decode 7-17%.

Gains that small on the clock can be a code-layout artifact, so the same three
meshes were counted under callgrind (`encode_drc`/`decode_drc`, one iteration
minus none, both trees built in WSL). At speed 4 the encode does 1.5-2.2% fewer
instructions than 2.2.1's and the decode 1.7-3.2%; at speed 10 the encode does
8-14% fewer, the control that had to move. On the 69K-face mesh the whole
speed-4 difference is in the attribute symbols -- the plan from bit-length
counts, the symbols formed in place, the rANS write; on the decode side the
rANS run and the zero-fill it no longer does -- and the connectivity is the
same instruction for instruction. The clock's extra percent on encode is
work callgrind does not count, fewer allocations and page faults, or layout.

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
of 21 runs (5 for the two bunny meshes, over 30k faces). Measured 2026-10-10 at
`57d8cb22`; C++/Rust ratio, `>1x` favors Rust.

Encode:

| model | faces | @0 | @1 | @2 | @3 | @4 | @5 | @6 | @7 | @8 | @9 | @10 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Stanford bunny | 69,451 | `1.37x` | `1.32x` | `1.09x` | `1.10x` | `1.08x` | `1.15x` | `1.11x` | `1.17x` | `1.14x` | `1.17x` | `3.09x` |
| bunny (drc) | 69,451 | `1.41x` | `1.52x` | `1.36x` | `1.36x` | `1.30x` | `1.30x` | `1.23x` | `1.23x` | `1.31x` | `1.30x` | `3.15x` |
| car | 1,744 | `3.10x` | `3.00x` | `2.25x` | `1.94x` | `1.83x` | `1.88x` | `1.59x` | `1.48x` | `1.78x` | `1.70x` | `2.23x` |
| lamp | 12,082 | `1.55x` | `1.68x` | `1.67x` | `1.61x` | `1.61x` | `1.57x` | `1.85x` | `1.65x` | `1.81x` | `1.77x` | `2.59x` |

Decode:

| model | faces | @0 | @1 | @2 | @3 | @4 | @5 | @6 | @7 | @8 | @9 | @10 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Stanford bunny | 69,451 | `1.20x` | `1.04x` | `1.10x` | `1.21x` | `1.23x` | `1.04x` | `1.09x` | `1.17x` | `1.01x` | `1.17x` | `2.98x` |
| bunny (drc) | 69,451 | `1.29x` | `1.16x` | `1.38x` | `1.45x` | `1.45x` | `1.18x` | `1.33x` | `1.33x` | `1.41x` | `1.40x` | `3.12x` |
| car | 1,744 | `1.78x` | `1.09x` | `2.22x` | `1.63x` | `1.83x` | `1.84x` | `2.25x` | `2.23x` | `2.26x` | `2.26x` | `3.07x` |
| lamp | 12,082 | `1.23x` | `1.23x` | `1.33x` | `1.40x` | `1.34x` | `1.37x` | `1.75x` | `1.70x` | `1.77x` | `1.79x` | `2.33x` |

Every cell favors Rust. Speed 10, the sequential coder, is the widest margin on
every model, `2.2x`-`3.2x` both ways; at the EdgeBreaker speeds the Stanford
bunny decodes `1.0x`-`1.2x` faster and the smaller models mostly more. Speeds
6-9 write larger files than speed 5 on three of the four models (lamp `143,412` to
`151,701` bytes), which is the compression side of this sweep rather than its
subject. The harness's `--nocapture` output carries every model's per-speed
times and sizes.

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
Measured 2026-10-10 at `57d8cb22`, C++/Rust speedup (`>1x` favors Rust):

| grid | faces | @0 | @1 | @5 | @10 | overall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 20x20 | 722 | `1.11x` | `1.11x` | `1.33x` | `1.64x` | `1.18x` |
| 50x50 | 4,802 | `1.20x` | `1.28x` | `1.42x` | `2.24x` | `1.31x` |
| 100x100 | 19,602 | `1.36x` | `1.22x` | `1.46x` | `2.14x` | `1.37x` |

Every cell favors Rust, and speed 10 (sequential, no EdgeBreaker) is the
largest margin on all three sizes -- the same shape the real-model and seeded
sweeps above show.

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
2026-10-10 at `57d8cb22`, byte-identical output on every cell:

| grid | faces | @0 | @1 | @5 | @10 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 50x50 | 4,802 | `1.42x` | `1.56x` | `1.49x` | `2.22x` |
| 100x100 | 19,602 | `1.49x` | `1.54x` | `1.40x` | `2.48x` |

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
mesh sizes. Two tests: `bench_generated_encode_decode_matrix` covers a UV
sphere and a subdivided cube; `bench_encode_decode_matrix` is a 100x100 grid,
full encode-then-decode, every speed. Run them with `--test-threads=1`, or the
two time side by side.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_encode_decode_matrix --release -- --nocapture --test-threads=1
```

Same machine and reference build. Measured 2026-10-10 at `57d8cb22`, byte
size and decoded point and face counts matched on every cell. C++/Rust
speedup:

| mesh | operation | @0 | @1 | @2 | @3 | @4 | @5 | @6 | @7 | @8 | @9 | @10 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| grid 100x100 | encode | `1.52x` | `1.52x` | `1.50x` | `1.52x` | `1.55x` | `1.48x` | `1.54x` | `1.71x` | `1.70x` | `1.72x` | `2.64x` |
| grid 100x100 | decode | `1.37x` | `1.36x` | `1.41x` | `1.41x` | `1.47x` | `1.36x` | `1.37x` | `1.32x` | `1.41x` | `1.44x` | `2.22x` |
| sphere 24x48 | encode | `1.68x` | `1.61x` | `1.63x` | `1.60x` | `1.46x` | `1.76x` | `1.73x` | `2.03x` | `1.72x` | `1.62x` | `2.03x` |
| sphere 24x48 | decode | `1.31x` | `1.26x` | `1.42x` | `1.43x` | `1.51x` | `1.41x` | `1.33x` | `1.37x` | `1.36x` | `1.40x` | `1.88x` |
| cube subdiv20 | encode | `1.65x` | `1.59x` | `1.67x` | `1.75x` | `1.65x` | `1.83x` | `1.74x` | `1.92x` | `1.88x` | `1.67x` | `3.07x` |
| cube subdiv20 | decode | `1.32x` | `1.34x` | `1.44x` | `1.48x` | `1.47x` | `1.39x` | `1.46x` | `1.40x` | `1.45x` | `1.52x` | `2.03x` |

Encode leads by `1.5x`-`3.1x` and decode by `1.3x`-`2.2x`, widest at or near
speed 10 on every mesh.

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

### In The Browser, Against three.js's DRACOLoader

File: `web/scripts/bench-three-draco-loader.mjs`

`@draco-rust/decoder/mesh/three` against three 0.186's `DRACOLoader`, which
runs upstream's Draco 1.5.7 decoder, both on four workers in headless
Chromium. Medians, two runs; the same-source control is within 3%:

| scenario | `createDracoLoader` | `DRACOLoader` | |
| --- | ---: | ---: | ---: |
| `bun_zipper.glb` through `GLTFLoader`, warm | `19.7-20.8 ms` | `32.3 ms` | `1.55-1.64x` |
| 16 `bunny_gltf.drc` (edgebreaker) at once | `76.1-77.1 ms` | `100.9-103.8 ms` | `1.33-1.35x` |
| 16 `bunny_cpp_standard.drc` (sequential) at once | `14.0-14.7 ms` | `28.1-28.3 ms` | `1.93-2.01x` |
| a new loader to the first `bun_zipper.glb` | `63.0-71.2 ms` | `69.3-73.6 ms` | `1.03-1.10x` |

What a page downloads for it, gzip: `88.4 kB` for the `mesh` entry with its
pool, worker and loader, against `99.9 kB` for `DRACOLoader`'s default decoder
and `74.8 kB` for its glTF-only one. The round is in `PERFORMANCE-LOG.md`, as
*`createDracoLoader` Against DRACOLoader, In The Browser*.

### KTX2 Transcode Against The Reference

File: `tools/basis-cpp-oracle/examples/speed.rs`

Package: `basis-cpp-oracle`, which builds Binomial's transcoder from the
vendored source at revision `9bebe16`

Purpose: `draco-texture` against the reference it was ported from, per codec
and per target. Transcoding only -- `draco-texture` has no Basis encoder --
and without Zstd supercompression, which the reference is built without and
which is undone for both sides before timing; a Zstd file's `ruzstd` pass comes
on top of these figures and is compared with nothing. Every fixture, every
level, every target both sides reach;
each side makes a whole call per image -- parse, codebooks, decode -- because
that is what the reference does on every call, and one-time tables are warmed
on both first. Best of seven rounds. ETC1S and UASTC are reported apart: a
target is two different paths from the two codecs, and one total hid which
was slow.

```sh
cargo run --release --manifest-path tools/basis-cpp-oracle/Cargo.toml --example speed
```

Where it stands, measured 2026-10-10 at `57d8cb22` on Windows, as time against
the reference's:

| codec | all targets | slowest target |
| --- | ---: | ---: |
| ETC1S | `0.68x` | ASTC, `0.81x` |
| UASTC | `0.82x` | BC7, `0.93x` |
| both | `0.71x` | |

Every target is faster than the reference. The fixtures' UASTC blocks are 88%
solid-colour, which weights the UASTC figure toward that path; how a busy
UASTC texture stands has not been measured. The rounds behind these figures
are in `PERFORMANCE-LOG.md`, from *KTX2: Constant Tables, Built Per Decode*.

### Zstd Decompression Against C zstd

File: `tools/zstd-bench/src/main.rs`

Package: `zstd-bench`, its own workspace

Purpose: the part the transcode harness above leaves out. Every
Zstd-supercompressed KTX2 fixture, every level, through `Ktx2::level_bytes`
exactly as the transcoder calls it, against C zstd's `ZSTD_decompress` into a
buffer of the level's size; outputs are compared byte for byte first. C zstd
is compiled from the checkout `ZSTD_SOURCE_DIR` names, without its assembly
Huffman loops, as Windows builds are. Without the variable it times this
crate alone.

```sh
ZSTD_SOURCE_DIR=<a facebook/zstd checkout> cargo run --release --manifest-path tools/zstd-bench/Cargo.toml
```

Where it stands, measured 2026-10-10 at `57d8cb22` on Windows against zstd
1.5.6: about `3.2x` C's time over the four fixtures (3.2 MB out). The decoder is `ruzstd`; the
candidates measured against it, and why none was taken, are in
`PERFORMANCE-LOG.md` under *KTX2: Zstd, And Which Decoder*.

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

