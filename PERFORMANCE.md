# Performance

Decode and encode benchmarks, profiling tests, and the Rust-vs-C++ speed
snapshot. Use `--release` for timing runs; add `-- --nocapture` to see the
printed comparison output. Correctness and parity tests live in
[`TESTING.md`](TESTING.md).

## Speed Snapshot

**The reference these tables measure against is not stock Draco.** The C++
checkout they link (`DRACO_CPP_BUILD_DIR`) carries a local debug patch:
`std::getenv("DRACO_VERBOSE")` inside `mesh_edgebreaker_decoder_impl.cc`'s
per-face loop and inside the traversal observer's per-vertex callback. `getenv`
scans the environment block, so that side's time scales with the environment --
which is also the whole of the launch sensitivity described below. Padding the
environment with a dummy variable takes the C++ decode from `1,690` to `12,355
us/1k faces` while the Rust side stays at `51.6` to `52.7`.

Against a pristine upstream 1.5.7 build of the same version, the ratios are a
different story -- `3` runs, medians, `us/1k faces`:

| Speed | Encode C++ / Rust | Encode | Decode C++ / Rust | Decode |
| ---: | ---: | ---: | ---: | ---: |
| 0 | `602` / `542` | `1.11x` | `67.3` / `83.5` | `0.81x` |
| 5 | `353` / `391` | `0.90x` | `36.3` / `55.7` | `0.65x` |
| 9 | `346` / `385` | `0.90x` | `31.6` / `50.8` | `0.62x` |
| 10 | `38.8` / `32.0` | `1.21x` | `16.6` / `11.5` | `1.44x` |

That sweep is synthetic and position-only. On the Stanford Bunny -- 69k faces,
one decoder per side, same payload, whole-decode milliseconds -- the same
comparison after the 2026-08-18 decode work reads:

| Asset | Speed | C++ | Rust | |
| --- | ---: | ---: | ---: | ---: |
| with normals | 1 | `14.30` | `12.96` | `1.10x` |
| with normals | 5 | `8.10` | `8.36` | `0.97x` |
| with normals | 9 | `4.44` | `5.16` | `0.86x` |
| position only | 5 | `3.31` | `4.76` | `0.70x` |
| position only | 9 | `2.98` | `4.30` | `0.69x` |

So the port is ahead on a real mesh at speed 1, at parity at speed 5, and
`1.2x` to `1.45x` behind where connectivity dominates -- not the `1.6x` the
synthetic sweep alone suggested. Encode is at parity to `10%` behind, and the
sequential path at speed `10` is `1.2x` to `1.44x` ahead.

Where the remaining gap is, measured rather than guessed: a stage comparison
against a `RelWithDebInfo` build of the same upstream source puts this port
*ahead* on entropy decoding (`0.25` ms against `0.53`) and on prediction
(`0.20` against `0.41`), and behind on two things -- the corner-table accessors
with the generic machinery around them (`2.4` ms against `0.38`), and memory
traffic (about `2` ms against `0.5`). The accessors are asked roughly twelve
questions per corner: `909,552` calls to `vertex` and `482,165` to `opposite`
in one 69k-face decode.

The tables below are kept as they were measured, and are only meaningful
against the patched reference they name; replacing them needs a decision about
which C++ build is the reference, which is the maintainer's call.

Measured 2026-08-17 on the seeded and real-corpus profiles below, against the
C++ reference build, `3` runs each, every cell the median across runs of the
harness's own `avg [p10..p90]`. Absolute microsecond figures are comparable
only within a run and only against a snapshot taken the same way.

**How the process is started changes the C++ side by up to `45%`.** The same
binary, the same workload, `us/1k faces` on decode at speed `9`: `2,428` run
through `cargo test` from a bash shell, `2,101` through `cargo test` from
PowerShell, `1,671` launched directly. The Rust side reads `51.9`, `52.3` and
`52.3` across those three -- it does not move at all. Nothing about the code
explains this and the cause is not established; what it means in practice is
that a C++ figure carries its launch context with it, and comparing two
snapshots taken differently produces a difference that is entirely the
harness. This snapshot is therefore taken the way the commands in this
document are written -- `cargo test ... -- --nocapture` from a shell -- which
is also how the figures it replaces were taken.

Run-to-run agreement within one launch method is `1.2%` to `5.1%` on the
speedup column, so treat a difference under `5%` between snapshots as not
measured.

Seeded mesh sweep encode, `avg [p10..p90]` across `12` stratified samples:

Samples: `3` grid, `3` fan, `3` boundary ribbon, `3` torus.

Sampled points: avg `12,156`, p50 `9,664`, p10..p90 `[7,578..21,435]`.

Sampled faces: avg `18,641`, p50 `19,327`, p10..p90 `[15,151..21,487]`.

| Speed | C++ raw us | Rust raw us | C++ us/1k faces | Rust us/1k faces | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `30,027 [21,276..42,714]` | `9,822 [4,711..22,019]` | `1,591 [1,178..2,064]` | `545 [256..1,324]` | `4.25x [1.54..6.55]` |
| 1 | `29,292 [20,601..41,232]` | `9,526 [4,345..21,018]` | `1,549 [1,152..1,955]` | `527 [239..1,264]` | `4.23x [1.54..6.16]` |
| 2 | `25,418 [16,726..38,541]` | `7,311 [2,076..19,843]` | `1,346 [912..1,851]` | `410 [114..1,193]` | `6.58x [1.55..9.75]` |
| 3 | `25,162 [16,221..38,102]` | `7,274 [2,105..19,819]` | `1,331 [901..1,812]` | `408 [115..1,191]` | `6.56x [1.52..9.88]` |
| 4 | `25,238 [16,242..38,148]` | `7,250 [2,027..19,874]` | `1,336 [903..1,845]` | `407 [111..1,195]` | `6.65x [1.53..10.16]` |
| 5 | `24,789 [15,833..37,018]` | `6,975 [1,771..19,651]` | `1,313 [878..1,814]` | `392 [98..1,181]` | `7.33x [1.54..10.95]` |
| 6 | `24,933 [15,760..38,143]` | `6,978 [1,808..19,615]` | `1,318 [878..1,828]` | `392 [100..1,179]` | `7.30x [1.56..10.95]` |
| 7 | `24,991 [15,888..37,792]` | `6,964 [1,802..19,686]` | `1,323 [881..1,839]` | `392 [99..1,183]` | `7.38x [1.55..11.11]` |
| 8 | `24,678 [15,899..37,611]` | `6,866 [1,711..19,516]` | `1,307 [874..1,801]` | `387 [94..1,173]` | `7.57x [1.54..11.46]` |
| 9 | `24,635 [15,739..37,968]` | `6,861 [1,723..19,535]` | `1,303 [866..1,823]` | `386 [94..1,174]` | `7.65x [1.56..11.64]` |
| 10 | `779 [566..1,219]` | `625 [400..1,061]` | `41 [34..56]` | `32 [26..50]` | `1.30x [1.13..1.41]` |

Seeded mesh sweep decode, `avg [p10..p90]` across `12` stratified samples:

| Speed | C++ raw us | Rust raw us | C++ us/1k faces | Rust us/1k faces | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `47,083 [34,591..64,127]` | `1,625 [1,019..2,251]` | `2,492 [2,284..2,989]` | `85 [69..104]` | `29.65x [24.39..33.21]` |
| 1 | `46,634 [34,093..64,213]` | `1,528 [782..2,382]` | `2,466 [2,264..2,996]` | `79 [53..111]` | `33.03x [26.46..43.08]` |
| 2 | `46,553 [34,837..64,214]` | `1,293 [809..2,187]` | `2,463 [2,262..2,989]` | `67 [45..103]` | `39.15x [29.11..51.10]` |
| 3 | `46,226 [34,124..63,722]` | `1,218 [687..2,188]` | `2,443 [2,239..2,991]` | `62 [44..102]` | `42.12x [29.22..51.06]` |
| 4 | `46,276 [33,986..64,336]` | `1,235 [690..2,186]` | `2,446 [2,251..2,982]` | `63 [44..102]` | `42.04x [29.06..51.52]` |
| 5 | `46,284 [33,798..63,896]` | `1,108 [583..2,034]` | `2,447 [2,232..2,975]` | `57 [38..95]` | `47.57x [31.36..58.96]` |
| 6 | `46,193 [34,024..64,080]` | `1,104 [598..2,044]` | `2,445 [2,236..2,986]` | `56 [39..96]` | `47.41x [31.55..57.91]` |
| 7 | `46,124 [34,209..64,148]` | `1,110 [598..2,043]` | `2,439 [2,238..2,989]` | `57 [38..96]` | `47.51x [31.14..58.96]` |
| 8 | `46,074 [33,660..63,539]` | `1,022 [546..1,909]` | `2,436 [2,224..2,982]` | `52 [34..89]` | `52.31x [33.38..66.02]` |
| 9 | `45,940 [33,556..64,039]` | `1,023 [520..1,860]` | `2,428 [2,214..2,969]` | `52 [34..89]` | `52.43x [33.36..64.39]` |
| 10 | `296 [196..472]` | `223 [140..377]` | `16 [13..22]` | `12 [9..18]` | `1.37x [1.25..1.44]` |

Real `.drc` corpus normal-distribution sample:

Samples: `24` draws from `16` compatible real mesh fixtures, seed
`0x6a7573737265616c`.

Sampled points: avg `2,096`, p50 `97`, p10..p90 `[9..4,959]`, min..max
`[8..34,834]`.

Sampled faces: avg `4,001`, p50 `170`, p10..p90 `[9..8,525]`, min..max
`[8..69,451]`.

This sample is mostly small: half its meshes are under `170` faces, so it
measures per-call cost where the seeded sweep measures throughput.

Source `.drc` decode speedup: avg `14.71x`, p50 `13.02x`, p10..p90 `[4.22..25.11]`,
decoded size match `24/24`.

Real corpus re-encode/decode speedup:

| Speed | Encode avg | Encode p50 | Encode p10..p90 | Decode avg | Decode p50 | Decode p10..p90 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `5.07x` | `5.19x` | `[3.13..6.39]` | `15.16x` | `13.97x` | `[8.90..23.42]` |
| 1 | `5.29x` | `5.19x` | `[3.22..7.25]` | `16.59x` | `16.08x` | `[8.97..27.31]` |
| 2 | `7.62x` | `7.69x` | `[3.61..10.93]` | `19.05x` | `17.49x` | `[9.24..31.64]` |
| 3 | `7.46x` | `7.65x` | `[3.43..10.53]` | `19.10x` | `17.74x` | `[9.15..32.22]` |
| 4 | `7.60x` | `7.66x` | `[3.98..9.86]` | `19.15x` | `17.85x` | `[9.30..31.00]` |
| 5 | `7.33x` | `7.80x` | `[3.58..9.61]` | `19.74x` | `18.33x` | `[10.24..32.74]` |
| 6 | `5.22x` | `5.49x` | `[2.80..7.00]` | `25.73x` | `29.83x` | `[8.73..41.89]` |
| 7 | `5.49x` | `5.71x` | `[2.99..7.25]` | `26.20x` | `30.49x` | `[9.55..42.59]` |
| 8 | `5.42x` | `5.76x` | `[3.11..7.45]` | `27.29x` | `32.22x` | `[9.72..45.06]` |
| 9 | `5.30x` | `5.71x` | `[3.04..7.52]` | `27.78x` | `33.62x` | `[9.49..44.90]` |
| 10 | `1.40x` | `1.44x` | `[1.03..1.88]` | `1.68x` | `1.56x` | `[1.42..2.13]` |

Every speed decodes the whole sample without failures, in all `3` runs. Speed
`0` once reported `3/24`, which was the separate-connectivity seam ordering
defect rather than a timing result.

### Against The 2026-07-31 Figures

The figures this replaces read `3.49x` to `7.43x` on seeded encode and
`28.98x` to `49.76x` on decode. Taken the same way, today's are higher on
every speed of both, and the C++ side -- unchanged code, and the thing that
cannot have improved -- lands within `3%` to `8%` of what it read then, which
is what a session-to-session difference on one machine looks like:

| | C++ then | C++ now | Rust then | Rust now | Speedup then | Speedup now |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| encode sp0 | `1,680` | `1,591` | `660` | `545` | `3.49x` | `4.25x` |
| encode sp9 | `1,410` | `1,303` | `438` | `386` | `7.43x` | `7.65x` |
| decode sp0 | `2,662` | `2,492` | `93` | `85` | `28.98x` | `29.65x` |
| decode sp9 | `2,644` | `2,428` | `59` | `52` | `49.76x` | `52.43x` |

Rust is `7%` to `18%` faster across the sweep and the C++ side moved `3%` to
`8%`, so the ratio rises everywhere: most on encode at speeds `0` and `1`
(`3.49x` to `4.25x`) and on decode at speed `10` (`1.19x` to `1.37x`), which
are the two places the optimization series paid the most. The real corpus
agrees -- encode `4.81-7.85x` to `5.07-7.62x`, decode `14.58-26.03x` to
`15.16-27.78x`, source `.drc` decode `13.76x` to `14.71x`.

An earlier version of this section reported the opposite, from snapshots taken
by launching the binary directly and compared against figures taken through
`cargo test`. That difference was the launch method described above, not the
machine and not the code, and it is the reason the method is now stated in the
first paragraph rather than left implicit.

### What The 2.0 Optimization Series Changed

The series between `7a277ff` and `f18f3e1` -- kd-tree walk, parallelogram
prediction bounds, corner-table construction, the entropy and normal memos --
measured as a paired A/B rather than against the snapshot above, since the
snapshot's own denominator moves. Both revisions were built with the same
toolchain and profile, and each binary carries its own copy of the unchanged
C++ side, which is the control: it agrees between the two to within `0.9%` on
the seeded sweep, so the Rust-side differences are the change.

These pairs were run by launching the binaries directly, which is the one
comparison the launch sensitivity above does not touch: it moves the C++ side
and leaves the Rust side alone, both revisions were launched the same way, and
what is being compared here is Rust against Rust.

Seeded sweep, `4` interleaved pairs, `us/1k faces` avg over the 12 samples.
The newer revision was faster in all `88` comparisons -- `4` pairs times `11`
speeds times encode and decode -- so the direction is not in question. Speeds
`3`, `4`, `6` and `7` track their neighbours and are left out of the table:

| Speed | Encode before | Encode after | Gain | Decode before | Decode after | Gain |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | `601` | `542` | `1.11x` | `86.8` | `84.2` | `1.03x` |
| 1 | `584` | `525` | `1.11x` | `81.2` | `78.3` | `1.04x` |
| 2 | `416` | `409` | `1.02x` | `68.8` | `65.8` | `1.05x` |
| 5 | `398` | `392` | `1.01x` | `59.5` | `56.7` | `1.05x` |
| 8 | `389` | `385` | `1.01x` | `55.5` | `51.8` | `1.07x` |
| 9 | `390` | `386` | `1.01x` | `55.3` | `51.9` | `1.07x` |
| 10 | `32.9` | `32.1` | `1.03x` | `13.8` | `11.5` | `1.20x` |

So on meshes of this size the series bought `1%` to `2%` on encode at speeds
`2` and above, `11%` at speeds `0` and `1`, and `3%` to `7%` on decode, with
`20%` at speed `10`.

The real corpus disagrees on encode. Over `5` interleaved pairs the newer
revision was faster in `11/55` encode comparisons against `41/55` decode ones:
encode is `3%` to `13%` slower across speeds `1` to `9` and unchanged at speed
`0`, while decode runs from `2%` slower to `5%` faster, most speeds landing
between `1%` and `3%` faster. Speeds `6` and `10` are left out of both ranges
because the C++ control moved `8%` between the two binaries' runs there, which
is larger than the effect being read.

The difference between the two profiles is mesh size, so the reading that fits
is per-call setup: a memo table earns its keep across thousands of faces and
is pure overhead on a mesh of eight, and half this corpus is under `170`
faces. That is a hypothesis this snapshot does not settle -- what it measures
is the sign and the size on both profiles, with the same control. Anyone
taking it further should measure the memo allocations directly rather than
inferring them from these two tables.

### The 2026-08-18 Round, Including What Did Not Work

Measured with `dev/profiling/abn.sh` on the Bunny, `2` seconds per run, `24`
to `40` interleaved runs per binary, and -- for anything claimed as a result --
two builds per condition, perturbed by a never-called function so their code
layout differs. That last part is not ceremony: on this workload two builds of
one condition sat `0.8%` apart, and a single-build reading of the encoder
change below said `2.6%` where the two-build reading says `1.9%`.

Landed:

- The corner table reserved once from the header's face count, capped by what
  the input could describe: `2.2%` at speed 1 with normals, `3.2%` on the
  position-only mesh at speed 5. A counting allocator found it -- ten
  reallocations totalling 3.9 MB to reach a 1.7 MB table.
- The sentinel test dropped from `CornerTable::vertex` and `opposite`, where
  the sentinel already indexes past the map: `2.3%` and `2.7%` on the
  position-only mesh.
- The traversal-order record reserved the same way as the corner table: `0.7%`
  on the position-only mesh, with both builds of each condition on the same
  median. Allocation per decode is now `98` calls and `7.8` MB, from `145` and
  `10.9` MB.
- The seed-free depth-first attribute traversal walked once per decode instead
  of once per attribute decoder, worth `8.7%` of decode at speed 5 and `3.7%`
  at speed 1 on a mesh with two attributes. A probe timed the walk at `1,153`
  ms of an `8,007` ms decode budget across two decoders before the change.
  Meshes with a single attribute decoder have no duplicate to remove and do not
  move.
- Parallelogram prediction in wrapping `i32` instead of widened `i64`, worth
  `0.65%` of decode at speed 5.
- The encoder's split-symbol lookup as a `Vec` indexed by face instead of a
  `HashMap` hashed by it, worth `1.9%` of encode.

Measured and rejected, so the next attempt can start elsewhere:

- Fetching the opposite face's three vertices in one bounds-checked slice
  rather than three accessor calls: `1.4%` **slower**.
- `#[inline]` on the ten hottest corner-table and mesh accessors, on the
  theory that they were not being inlined across codegen units: `0.2%` on
  decode and `0.5%` on encode, both inside the build-to-build spread. Retried
  on the position-only mesh, where those accessors are `17%` of self time
  rather than a diluted share: `0.35%`, still inside the spread. They are
  already inlined where it matters.
- Padding the corner maps with a trailing sentinel so `vertex` and `opposite`
  could clamp instead of checking: not implemented, because it does not pay.
  Clamping needs `min(index, len - 1)`, which is a comparison of its own, and
  the indexed load still carries a bounds check the compiler cannot elide
  without knowing the map is non-empty. What is left in those accessors after
  the sentinel test came out is one compare, one conditional move and one load
  -- about a cycle per call, which is what `get_unchecked` would also cost
  minus the compare.
- Rewriting `AnsDecoder::read_normalize` to pop from a prefix slice instead of
  indexing at an offset: not detectable.
- Skipping `CornerTable::is_index_consistent` entirely (a probe, not a
  proposal): not detectable, so the check is not what it costs.
- Reading the quantizer's three floats through one twelve-byte slice instead of
  twelve indexed byte loads: not detectable, so the compiler was already
  merging them.
- Reusing the `data_to_corner_map` that `assign_points_to_corners` already
  builds, instead of the attribute traversal building its own: the two are
  different walks, `34,834` entries against `35,924` on the Bunny, so there is
  nothing to reuse.
- Reserving the corner table's capacity up front from the symbol count instead
  of growing it a face at a time: `2.2%` **slower** as capacity-only and `2.5%`
  slower when the length was grown too. One large allocation takes its pages
  from the operating system, while incremental growth reuses pages the heap
  already holds -- and `14.7%` of this decode's time is already kernel-side
  page work, so adding to it is the wrong direction.
- Skipping the traversal cache's copy when a mesh has only one attribute
  decoder, on the theory that it copies three per-vertex vectors for nothing:
  `2.1%` and `2.2%` **slower** on a position-only Bunny, both builds agreeing.
  The cache is left copying unconditionally. The same pair of binaries also
  settles the question the cache raised: on that position-only mesh, where
  there is no second decoder to serve, the caching revision is `0.5%` to `0.6%`
  faster than the one before it, so nothing regressed for single-attribute
  meshes.
- Resolving the three corner vertices once in the valence traversal instead of
  through the per-delta helpers, which re-resolved `next` up to three times:
  not detectable. It removes real work and is not slower, but two builds per
  condition disagreed by `1.6%` and `2.1%` in the session that measured it, so
  it is not landed on evidence this weak.

What the profile says is left: on decode, roughly a fifth of self time sits in
the corner-table accessors and the generic machinery around them -- bounds
checks, `Option`, range iterators -- and `corner_table.rs` already records that
removing the bounds check there with `get_unchecked` is worth `2.0%`, which is
the standing price of the no-`unsafe` promise. Nothing else in either profile
is both this large and reachable without `unsafe` or a restructuring bigger
than a micro-optimization.

## Main C++ vs Rust Benchmarks

### Decode Through The C++ Bridge

File: `crates/draco-cpp-test-bridge/tests/bench_decode_cpp_vs_rust.rs`

Package: `draco-cpp-test-bridge`

Purpose: in-process decode benchmark, C++ bridge vs Rust. The timed region is
matched between C++ and Rust, and the reported result uses median batches.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_decode_cpp_vs_rust --release -- --nocapture
```

### Encode Through The C++ Bridge

File: `crates/draco-cpp-test-bridge/tests/bench_encode_cpp_vs_rust.rs`

Package: `draco-cpp-test-bridge`

Purpose: in-process encode benchmark, C++ bridge vs Rust, without external
process startup cost.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_encode_cpp_vs_rust --release -- --nocapture
```

### Encode/Decode Matrix

File: `crates/draco-cpp-test-bridge/tests/bench_encode_decode_matrix.rs`

Package: `draco-cpp-test-bridge`

Purpose: encode/decode performance and correctness across multiple speeds and
mesh sizes.

```sh
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test bench_encode_decode_matrix --release -- --nocapture
```

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

