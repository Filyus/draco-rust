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

**This affects the encode tables too, which earlier readings of this warning
missed.** `mesh_attribute_indices_encoding_observer.h` is not a decoder file:
it is the traversal observer the *encoder's* `MeshTraversalSequencer` drives
once per vertex, so the patch sits on both paths. Measured directly -- one
Rust binary, one payload, one timed region, the linked C++ library the only
variable: the Bunny at speed 5 encodes in `60,950 us` against the patched
checkout and `12,621 us` against pristine 1.5.7, a factor of `4.8`. Both
builds carry identical Release flags (`/MD /O2 /Ob2 /DNDEBUG`) and libraries
within `3%` of each other in size, so this is the source patch, not an
optimization setting. Every bridge-measured C++ figure below -- seeded sweep
and real corpus, encode as well as decode -- carries that factor.
`corner_table_loop` and `encode_loop` pin `DRACO_CPP_BUILD_DIR` explicitly for
this reason, and anything else comparing against C++ should.

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

Against the 2026-08-18 numbers this table replaces: position-only moved from
`0.70x`/`0.69x` to `0.78x`/`0.76x` at speeds 5 and 9, and the normal-carrying
mesh from `0.97x`/`0.86x` to `1.01x`/`0.93x` at speeds 5 and 9 -- consistent
with the `11-13%` and `2.7-2.9%` the two payloads showed in the interleaved
`abn.sh` measurement of the same round, which is a different tool measuring
the same change and landing on the same number twice.

So the port is ahead on a real mesh at speed 1, at or past parity at speed 5
on both payloads now, and `1.3x` behind at worst -- speed 9, position only,
where connectivity dominates most -- not the `1.6x` the synthetic sweep alone
suggested, and closer than the earlier snapshot on every row. Encode is
unaffected by this round (nothing in the corner-table access work touched the
encode path) and stays at parity to `10%` behind; the sequential path at speed
`10` is still `1.2x` to `1.44x` ahead.

Where the remaining gap was, measured rather than guessed, before the
corner-table access round documented below: a stage comparison against a
`RelWithDebInfo` build of the same upstream source put this port *ahead* on
entropy decoding (`0.25` ms against `0.53`) and on prediction (`0.20` against
`0.41`), and behind on two things -- the corner-table accessors with the
generic machinery around them (`2.4` ms against `0.38`), and memory traffic
(about `2` ms against `0.5`). The accessors were asked roughly twelve questions
per corner: `909,552` calls to `vertex` and `482,165` to `opposite` in one
69k-face decode. **These numbers predate the corner-table access round**
(accessor fusion, then the removal of a dead depth-first traversal that alone
accounted for `11-13%` of position-only decode time) and the call counts in
particular are now smaller by construction -- a fresh stage-by-stage profile
has not been re-run since, so treat this paragraph as history, not current
state, until it is.

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
  proposal): not detectable, so the check is not what it costs. **Superseded,
  see "Round Three" below** -- that probe measured skipping the check on
  whichever payload diluted it enough to hide in noise; disassembling the
  function found it genuinely vectorized, and the actual redundant call (the
  main table scanned twice, not once) measures `1.8-2.1%` on the payload where
  it is not diluted.
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

### The Corner-Table Access Refactor

Stage profiling against pristine upstream 1.5.7 attributed `59%` of the
decode-time gap to corner-table access -- `2.04` ms of self time on
`draco_core::corner_table` plus the generic bounds-check/`Option` machinery it
inlines into, against `1.19` ms for the equivalent work in C++. Per-call cost
had already been measured to a floor (`get_unchecked` worth `2.0%`, the
standing price of `#![forbid(unsafe_code)]`; the `%3` wrap branch and a
face-triple slice read both measured slower than what they replaced), so this
round targeted call *count* and structure instead: fusing compositions of
total accessors into one bounds-checked lookup, and removing one duplicated
traversal. Landing was gated on behavioural identity and no regression, not on
a speed threshold -- the goal was fewer places the access pattern lives, and a
speedup was a secondary question.

Measured with the same `abn.sh` protocol as above, position-only Bunny at
speed 5 and the normal-carrying Bunny at speed 1, two builds per condition:

- `vertex_after`/`vertex_before` replacing the `vertex(next(c))` /
  `vertex(previous(c))` idiom at ~35 call sites (upstream's own long-standing
  TODO in `mesh_prediction_scheme_parallelogram_shared.h`), plus fusing
  `swing_left`/`swing_right`/`left_corner`/`right_corner` into one lookup each:
  within the `~1%` build-to-build spread on the position-only mesh, `4-5%`
  faster on the normal-carrying mesh at speed 1 (constrained multi-parallelogram,
  which walks these swings the most).
- `#[inline]` on the accessors, re-tested this time under a temporary
  `codegen-units = 16` bench profile (the shipped `crates/Cargo.toml` has no
  `[profile.release]`, so a consumer's build gets that instead of the `1` this
  harness normally pins) to rule out the earlier null result being an artifact
  of the harness: still within the build-to-build spread. **Not applied a
  second time** -- the earlier rejection stands under the regime it was
  originally worried about not covering.
- Caching the previous fan step's `vertex()` answer per attribute table in the
  seam-detection walk (`assign_edgebreaker_points_to_corners`), replacing a
  short-circuiting `.any()` that re-asked a question the walk had already
  answered one step earlier: `0.4%` on the normal-carrying mesh, both builds
  agreeing to `0.04%`. No effect on position-only meshes, which walk no
  attribute tables.
- Unifying the two textually duplicated depth-first traversals (the generic
  mesh decoder's and EdgeBreaker's point-assignment copy) into one
  `corner_traversal::traverse_from_corner`, generic over an observer closure so
  monomorphisation still inlines it rather than a vtable call landing in the
  middle of a 200k-vertex walk: no measurable change on either payload, as
  expected -- the point was removing the duplicate, not speed.

Net: a few percent on the payloads that exercise the swing-heavy predictors,
nothing measurable on the plainest one, and one fewer place a future change to
traversal or fan-walking has to be made twice. The corner-table access layer
is now one lookup per composed question instead of a chain of total accessors
re-checking a sentinel each already answered.

### Round Two: A Traversal Nobody Read

A call-count comparison against pristine C++ (instrumenting every `CornerTable`
method, one cold decode each side, counts are exact rather than sampled) found
that despite the round above, Rust still made `25%` more table loads per
corner than C++ -- and, unlike per-call cost, that gap was concentrated: the
`opposite`-family calls were `49%` over C++'s count, `left_most_corner` was
`2.0x` over. That is a call-count anomaly, not a per-call one, so it called for
reading code next to its C++ counterpart rather than another accessor
micro-benchmark.

`MeshEdgebreakerDecoder::assign_points_to_corners` ran a full depth-first
traversal -- `is_vertex_on_boundary`/`swing_*` on every vertex -- to fill
`point_ids` and `data_to_corner_map`. Grepping every read of both arrays found
none: the function's actual output, `mesh.set_face`, already built every face
straight from `corner_table.vertex`/`vertex_after`/`vertex_before`, and
`data_to_corner_map`'s one external consumer (`take_data_to_corner_map`) wrote
it to a field nothing ever read again. C++'s own source for the equivalent
function confirms the shortcut is exact, not an approximation: when there are
no attribute seams, `AssignPointsToCorners` skips the traversal entirely and
reads vertex indices straight off the corner table, verbatim the same
operation Rust's dead-code path was gated behind an unused DFS to reach. This
also retracts an earlier explanation in this document's history: the
"mesh assembly" profiling bucket that showed `0.23` ms in Rust against `0.00`
in C++ was attributed at the time to profiler inlining hiding C++'s work
elsewhere. It was not an attribution artifact -- it was this traversal, doing
real, measurable, entirely unread work.

A smaller instance of the same shape sat one function up: `vertex_to_corner_map`
was built with a per-vertex `left_most_corner()` call, on a table just
truncated to exactly that length -- so every call was answering a question a
direct transform of the vec already in hand could answer without a bounds
check.

Deleting the dead traversal and simplifying the vec build (`dev/profiling/abn.sh`,
two builds per condition): `11-13%` faster on the position-only Bunny at
speeds `5` and `9`, `2.7-2.9%` faster on the normal-carrying Bunny at speed
`1` -- all several times the `~1%` build-to-build spread. A `decode_loop` run
confirms the mechanism rather than just the direction: `137` to `129`
allocations per decode, `13.18` to `12.80` MB, matching the four `Vec`s
(`point_ids`, `data_to_corner_map`, `visited_vertices`, `visited_faces`) the
deleted traversal no longer allocates.

The lesson for the next round: the accessor-fusion work above was aimed at
per-call cost, which was already close to its floor, so it found a few
percent. This found over ten, and it was sitting in plain sight the whole
time -- a traversal whose output had no reader was never going to show up in
an accessor-level profile as anything other than "more calls", and the fix was
proven safe by reading two call graphs side by side, not by guessing and
measuring.

### Round Three: The Same Table, Checked Twice

A call-count instrumentation pass confirmed round two's arithmetic exactly:
table loads on the position-only Bunny went from `1,403,540` to `1,125,237`
against a pristine-C++ count of `1,125,393` -- parity to `156` loads, and
`left_most_corner` matched C++'s `72,869` to the call. The load-count lever is
now fully spent; nothing more is reachable that way.

A fresh samply profile of the resulting build (position-only, speed 5) found
`Ord::max` and `wrapping_add` sitting at `3.49%` and `2.01%` of self time --
line-table attribution pointed at `is_index_consistent`'s branchless-max fold.
Disassembling it settled whether that was a scalar bug before spending a
single benchmark run on it: it is not -- SSE2, 8 `u32` per iteration, a
sign-flip trick that keeps the max reduction branch-free. The cost is
bandwidth, not instructions: the fold streams the full `1.67` MB
corner-to-vertex and opposite-corner arrays, and `MeshEdgebreakerDecoder::
assign_points_to_corners` and `MeshDecoder`'s attribute-traversal generators
each ran it once on the exact same, by-then-unmodified table -- one check
doing the other's job over again. C++ carries no equivalent check on this path
at all.

Landed: an `already_validated` flag threaded through the shared DFS helper so
the main-table callers skip the second scan while the seam-broken clone (a
different table, checked nowhere else) keeps its own. `1.8-2.1%` on
position-only at speeds 5 and 9, `1.0-1.2%` at speed 0 (the prediction-degree
traversal, a second call site with the same shape), no detectable change on
the normal-carrying mesh at speed 1 where the same absolute saving is a
smaller share of a larger decode. This also resolves a standing contradiction:
an earlier round's probe recorded skipping the check entirely as "not
detectable" -- that was true on whatever payload diluted a `2%`-ish saving
below the noise floor, not evidence the check was free.

**Resolved by disassembly.** `next_in_face`'s `4.26%` self time is real work,
not an attribution artifact, and not a bug. Extracting the `.o` from the
`codegen-units = 1` build and listing its symbol table found no standalone
symbol for `next_in_face`, `prev_in_face`, or any of `vertex`, `opposite`,
`next`, `previous`, `vertex_after`, `vertex_before`, `swing_left`,
`swing_right`, `left_corner`, `right_corner`, `face`, `first_corner`, or
`left_most_corner` -- every one of them is inlined at every call site, crate
wide, with no exception. Samply's `4.26%` is the sum of dozens of inlined
copies of the same six-instruction wrap (`lea`/`imul 0xaaaaaaab`/`cmp`/branch,
matching the standard unsigned-`%3` magic multiply) correctly attributed back
to one source line by the compiler's debug info, not one hot function paying
call overhead. One instance, read straight out of the disassembly of
`assign_points_to_corners`'s inlined tail loop:

```
1c3c: lea  rax, [rdi + 2*rdi]                  ; c0 = f * 3
1c56: mov  edx, [corner_to_vertex_map + 4*c0]  ; vertex(c0) -- plain load
1ca2: lea  r8d, [eax + 1]                      ; next_in_face: c+1, then
1ca6: imul ecx, r8d, 0xaaaaaaab                ;   the %3 test, then the
1cb5: add  eax, -2                             ;   c-2 branch -> vertex_after
1cbf: imul ecx, eax, 0xaaaaaaab                ; prev_in_face on the same c0,
1ccd: dec  eax                                 ;   mirrored              -> vertex_before
```

Both C++'s `Next`/`Previous` and Rust's `next_in_face`/`prev_in_face` run the
identical `%3` test the identical number of times for the identical
compositions (`SwingLeft`/`SwingRight`/`Vertex(Next(c))` and their Rust
mirrors) -- and round three's call-count instrumentation already put the two
sides at load parity to `156` calls out of `1.1` million. So this is not extra
work Rust does that C++ skips, the way the dead traversal and the doubled
consistency scan were; it is the same work both sides do, made visible here
only because this build's debug info resolves it down to source lines. There
is nothing to fix -- the arithmetic-vs-branch form was already decided (branch
wins by `8.8%`), and the only way to do less of it is fewer swing/vertex_after
compositions, which is exactly what the load-count parity says is not
available anymore.

### Round Four: The Allocator, Not The Bytes

The fresh profile that closed round three's open question also surfaced a
bucket nobody had attributed yet: `memset` (`5.07%`), three separate
`Vec::push_mut` leaves (`3.51%` combined), `ptr::write` (`3.41%`), and
`intrinsics::unlikely` (`2.49%`) -- roughly `14%` of self time with no single
source line to point at. `SAMPLE_ALLOC=1` on `decode_loop` (already built for
exactly this) gave the concrete shape instead of a guess: one position-only
decode makes 27 allocations at or above 64 KB, and one size --
`139,336` bytes, `num_vertices * 4` -- accounts for **twelve** of them. Some
are genuinely necessary, separate buffers (`vertex_to_data_map` in the
attribute-traversal generator, two `Vec<CornerIndex>::resize` calls inside
`SequentialIntegerAttributeDecoder::decode_values`, `vertex_to_corner_map`'s
own build); at least one is a leftover of round two's own fusion, which
replaced a per-vertex `left_most_corner()` loop with `.collect()` and removed
the function calls but not the allocation. A separate three-step growth
(`65,536` -> `131,072` -> `262,144` bytes) showed an unreserved
`Vec<PointIndex>` inside `EdgebreakerConnectivityDecoder::decode_connectivity`
paying for its own capacity doubling mid-decode.

Before restructuring any of that, the cheaper question: is this bucket
allocator overhead or data-volume bandwidth? Swapping the global allocator to
`mimalloc` (dev-only, `#[global_allocator]` in the harness binaries, never in
the library -- allocator choice is the consumer's call, same as PGO) answers
it without touching a line of `draco-core`. `dev/profiling/abn.sh`, two builds
per condition:

| payload | speed | system allocator | mimalloc | win |
| --- | ---: | ---: | ---: | ---: |
| position only | 1 | `6,530` | `5,067` | `22.4%` |
| position only | 5 | `4,191` | `2,946` | `29.7%` |
| position only | 9 | `3,691` | `2,576` | `30.2%` |
| with normals | 1 | `12,333` | `9,919` | `19.6%` |
| with normals | 5 | `7,282` | `5,395` | `25.9%` |
| with normals | 9 | `4,579` | `3,332` | `27.2%` |

(microseconds per decode, median of two builds each side.) Every row's
build-to-build spread was under `0.3%`; these wins are `60`-`100x` that.
Correctness checked directly, not assumed: both binaries decode to the
identical `34,834`-point, `69,451`-face mesh from the identical `58,893`-byte
stream.

This is by a wide margin the largest single lever this session found -- bigger
than the dead traversal, the doubled consistency scan, and the accessor fusion
combined. And it answers its own question: the bucket was allocator overhead,
not bytes touched, since nothing about the *data volume* changed, only which
code manages the heap.

Re-run through `decode_loop` itself (not the separate harness, so C++ and
Rust+mimalloc come from one tool, one build of each, one payload load) against
pristine 1.5.7:

| payload | speed | C++ | Rust (mimalloc) | ratio |
| --- | ---: | ---: | ---: | ---: |
| position only | 1 | `6,688` | `5,536` | `1.21x` |
| position only | 5 | `3,596` | `3,243` | `1.11x` |
| position only | 9 | `3,096` | `2,677` | `1.16x` |
| with normals | 1 | `14,338` | `11,046` | `1.30x` |
| with normals | 5 | `8,174` | `5,738` | `1.42x` |
| with normals | 9 | `4,486` | `3,464` | `1.30x` |

Every row is now Rust ahead, several by a wide margin -- a reversal from the
system-allocator table above (worst case `0.76x`, position only speed 9)
without a single line of `draco-core` changing. The library itself cannot make
this choice for its consumer, so it stays out of the `[Speed Snapshot]`
headline numbers above until this document's maintainer decides whether and
how to recommend it downstream (a `README` note, a Cargo feature on a
consuming binary, nothing at all). But for anyone choosing an allocator for a
binary that links this crate, the number to know is `20-30%` on decode alone.

What is still unmeasured: this session did not verify what allocator the C++
reference itself effectively uses (plain MSVC `operator new`/Windows heap, by
everything read of its build so far) -- if it is the same default heap Rust
was fighting, the fair comparison for *both* sides swapping allocators remains
open, and the `1.1x`-`1.4x` numbers above should be read as "Rust with a
better allocator against C++ with the default one," not as a claim about
which language's default is faster in general.

### Source-Level Follow-Up: Audited, Not A Win

The twelve `139,336`-byte sites and the unreserved connectivity-decode `Vec`
looked like a natural next step once mimalloc named the shape of the problem
(call count, not bytes moved) -- reduce the count at the source instead of
paying it faster. Two lines of follow-up, both closed without a fix:

**The growing `Vec`.** `SAMPLE_ALLOC=1` traced three reallocations
(`65,536` -> `131,072` -> `262,144` bytes) inside
`EdgebreakerConnectivityDecoder::decode_connectivity`'s call tree, debug-info
labelled `RawVec<PointIndex>`. Every `.push()` reachable from that function --
including through the `EdgebreakerTraversalDecoder` trait methods -- was
checked directly: `active_corner_stack` capacity `16`, `invalid_vertices`
capacity `2,048`, `processed_connectivity_corners` capacity `69,451` (exactly
its `reserve()`, no overshoot). None match. A crate-wide grep for
`Vec<PointIndex>` outside encode-only files turns up nothing reachable from
decode at all -- the label really was a debug-info artifact of LLVM folding
identical-layout generic instances (`PointIndex`, `VertexIndex`, `CornerIndex`
and bare `u32` are all 4-byte Copy newtypes with identical codegen). Both
guesses about *which* field the folded name stood in for were wrong, though --
resolved in round five below by a debugger, not by more reading.

**The twelve `139,336`-byte sites.** Traced as far as backtraces allow: one is
round two's `vertex_to_corner_map` build in
`MeshEdgebreakerDecoder::decode_connectivity`; two are `Vec<CornerIndex>::resize`
inside `SequentialIntegerAttributeDecoder::decode_values`, sized for the
parallelogram predictor's own `data_to_corner_map`; the largest identified
group -- three of the twelve -- are `point_ids`, `data_to_corner_map`, and
`vertex_to_data_map` inside `generate_point_ids_and_corners_dfs_for_table`.
Reading that function settles the question a lifetime audit exists to answer:
all three are written in the same closure on every iteration of the same
traversal, live simultaneously, and are returned to the caller as
`AttributeTraversalArrays` -- cached and read again by every later attribute
decoder. Nothing here is scratch space with a lifetime that ends before the
next allocation starts; every buffer found is a real, load-bearing, distinct
output, not a duplicate of work already done (the shape that made the dead
traversal and the doubled consistency scan free to delete). A shared
scratch-buffer pool does not apply. The only route left to fewer allocations
here is a structural one -- folding the three parallel `Vec`s into one `Vec`
of a small struct, cutting three allocations to one at the cost of touching
`AttributeTraversalArrays`'s definition and every destructuring call site --
which is a real refactor with its own correctness surface, not a quick
follow-up to a benchmark. Left undone: the allocator question is answered and
cheap; this one is neither, so it stays optional rather than becoming the next
step by default.

### Round Five: The Table Nobody Reserved

Source reading and static disassembly closed round four's growing-`Vec`
question as far as they could and left it unresolved. A debugger picked up
where they stopped. `cdbX64.exe` (the classic console engine bundled with
WinDbg Preview; `kernelbase!HeapReAlloc` never resolved in this environment --
no route to the public symbol server -- so the breakpoint went on the
crate-generated `__rust_realloc` shim directly, found by wildcard symbol
search once `.sympath+` pointed at the harness's own build directory)
conditioned on the exact byte size (`0x40000` = `262,144`) and read the call
stack past the one-time encode `decode_loop` also runs before its timed
decode loop -- the first hits at that exact size were
`MeshEdgebreakerEncoder::encode_connectivity`, not decode at all, and cost a
round of confusion before the harness's own `encoded: ...` line printing
between hits made the boundary obvious. The hit that mattered landed inside
`EdgebreakerConnectivityDecoder::decode_connectivity`, confirming the
attribution round four already had, and disassembling the caller found the
real field: offset `self+0x30`, three `Vec`s in from the start of
`CornerTable` -- `vertex_corners`. `VertexIndex`, `CornerIndex` and
`PointIndex` are all 4-byte Copy newtypes with identical codegen, so LLVM had
folded `vertex_corners: Vec<CornerIndex>`'s `RawVec` instantiation into
`PointIndex`'s; the debug-info label was accurate about *a* type, just not
about which field carried it.

`vertex_corners` grows through `set_left_most_corner`'s `resize`, one vertex
at a time, called from five different symbol arms across the whole
connectivity decode -- the one table of `CornerTable`'s three that
`try_reserve_faces` never touched, because it is indexed by vertex rather than
by face or corner, and nothing had sized it against the vertex count already
being tracked for exactly this purpose
(`EdgebreakerConnectivityDecoder::max_num_vertices`). Landed as
`try_reserve_vertices`, mirroring `try_reserve_faces` down to why it is
capacity-only, reserved against `max_num_vertices.min(input_face_bound)` --
not `max_num_vertices` alone, which traces back to a header count checked only
against `3 * num_faces`, not against the stream size; `input_face_bound` is
what already keeps that count from being honoured past what the buffer could
describe, for the other two tables, and this reuses that guarantee rather than
placing new trust in an unvalidated count.

`dev/profiling/abn.sh`, two builds per condition: `3.0-3.4%` on position-only
at speed 5, `1.2-1.4%` at speed 9, `1.8-1.9%` on the normal-carrying mesh at
speed 1 -- several times the `0.0-0.4%` build-to-build spread on these runs.
`decode_loop`'s own count: `91` to `77` allocations per decode, `7.39` to
`7.02` MB -- a drop in count and bytes together, the signature of removing a
reallocation chain rather than changing what gets computed. This is the first
fix this session found by disassembling a specific live allocation rather than
by reading source or a static disassembly first; both had already been tried
on this exact question and both stopped short of the field.

### Round Six: A Corner Nobody Wanted

A fresh profile after round five showed the same shape as before it -- the
same accessor and entropy machinery at the top, nothing newly hot -- on the
position-only Bunny, which this session had already spent five rounds on. The
`SAMPLE_ALLOC` histogram was similarly quiet: no more repeated-size families
past the twelve already audited and closed. Profiling a payload nobody had
looked at yet this round -- the normal-carrying Bunny at speed 1, which
exercises the geometric-normal predictor and constrained multi-parallelogram
that position-only never touches -- surfaced `CornerPositions::get` at four
separate source lines, `~3.6%` combined.

Reading it found the same shape as `vertex_after`/`vertex_before` were built
to fix, one level up: `compute_predicted_value`'s fan walk computed a
neighbour corner with `next`/`previous` for no reason but to hand it to `get`,
which immediately turned it back into a vertex to key its cache.
`vertex_after`/`vertex_before` already fold that round trip into one lookup;
the fan walk just wasn't using them, because the cache's key was a vertex and
`get`'s parameter was a corner. `get` now delegates to a new
`get_by_vertex`, and the fan walk calls that directly, checked to answer the
same `[0, 0, 0]` for an invalid vertex that the old corner-checked early
return did -- not by adding an equivalent check, but because
`position_for_vertex`'s own bounds check already lands there by construction.

`2.5%` on the normal-carrying Bunny at speed 1, two builds per condition, no
detectable change at speeds 5 or 9 (`0.0-0.4%`, inside the build-to-build
spread) -- a different normal-prediction mode there runs this loop less. The
lesson for the next round: the two most recent finds came from switching what
gets profiled, not from digging deeper into the same payload after its easy
answers ran out.

### A Source-Level Follow-Up, Audited -- Not A Win

The same normal-carrying profile put `compute_original_values` at three
source lines, `~2-3%` combined, in the constrained multi-parallelogram
predictor -- the scheme speeds `0` and `1` use. Reading it found a genuine
redundancy: the pass that decides which of a vertex's fan corners qualify as
parallelograms already asks the corner table and `vertex_to_data_map` for a
corner's opposite and that opposite's three neighbour vertices' data ids, to
check the qualifying condition -- then keeps only the corner index. The
second pass, for every corner that qualified, asked the exact same four
questions again to get the exact same three answers. Neither table changes
between the passes, so the second answer is provably identical to the first,
not just usually so. Carrying the three already-resolved data ids alongside
the corner removes the second pass's redundant calls entirely, along with an
`Option`-unwrapping triple and its error closure that existed for a failure
which -- given the first pass already succeeded on the same lookup -- could
not occur.

Landed anyway, but not for a measured win: `dev/profiling/abn.sh` at speeds 0
and 1, two builds per condition, found `0.0-0.4%`, inside the build-to-build
spread. Re-reading which lines the profile actually named explains why: `1066`
and `1070` are the *qualifying* pass's own lookups, run once per swing step
across the whole fan; `1206` is the tail's averaging division. None of the
three are the redundant second pass this fix removes, which only runs for the
up-to-four corners that qualify per entry -- evidently too small a volume to
clear noise, unlike the profile line count made it look. The fix stands on its
own terms regardless: `cargo test`'s byte-exact corpus is unchanged, the
removed code was provably dead work, not a guess that didn't pay off, and this
session's standing rule is architecture and correctness first -- a clean
simplification that measures to zero is not a reason to keep the redundant
version.

### The Same Fusion, Two More Predictors

Grepping the rest of `prediction_scheme_*` for `corner_table.next`/`.previous`
found the geometric-normal predictor's shape twice more: both texture-
coordinate predictors' `compute_predicted_value` (the current portable one and
the pre-2.2-bitstream legacy one) computed a neighbour corner for no reason but
to hand it to `vertex` -- checked in both, same as before, that the corner had
no other use in the function. Fixed on the decode side of both;
`legacy_bitstream_decode` is a default feature, so the deprecated file's fix
runs under the same `cargo test` as everything else. The encode side of each
carries the identical shape and is left alone, matching every other case of
this fusion this session.

No tracked asset carries texture coordinates at Bunny scale, so measuring
needed a synthetic UV-carrying variant built from the normal-carrying mesh.
`dev/profiling/abn.sh`, two builds per condition: no detectable change at
speeds 1 and 5, ambiguous at speed 9 where the two baseline builds disagreed
with each other by more than the apparent head/base gap. The geometric-normal
fix's `2.5%` came from a call inside a fan walk, repeated once per neighbour
across every step; this call runs once per data entry with no loop to
compound it in, the same shape that made the constrained-multi-parallelogram
fix two commits back measure to zero. Landed for the same reason that one
was: correct, provably equivalent, `cargo test` unchanged.

### The Same Bug, A Smaller Table

The redirect back to memory allocation -- the session's own earlier finding
that the dominant remaining gap was allocator-shaped, not per-call accessor
cost -- prompted a grep for every other `.resize()` call across the crate,
looking for more of round five's shape: a `Vec` grown one element at a time
with no upfront reservation. `EdgebreakerConnectivityDecoder::is_vert_hole`
matched exactly: `mark_vert_not_hole` grows it via `resize(index + 1, true)`,
called from five symbol arms during connectivity decode, same as
`vertex_corners` before round five's fix, and for the same reason -- nothing
had sized it against `max_num_vertices`, already tracked for exactly this
purpose. Reserved the same way, against `max_num_vertices.min(input_face_bound)`.

`decode_loop`'s allocation count: `77` to `65` allocations per decode, `7.02`
to `6.95` MB -- the same drop-in-count-and-bytes signature as round five,
confirming a reallocation chain was removed rather than the amount of work
changing. `dev/profiling/abn.sh`, two builds per condition, position-only at
speeds 5 and 9 and the normal-carrying mesh at speed 1: no change past the
build-to-build spread at any of the three. `is_vert_hole` is `Vec<bool>`,
one byte per vertex against `vertex_corners`' four, so the reallocation
chain it was making moved far fewer bytes -- round five's win was mostly
about what was being copied, not how many times. Landed for the fix itself,
matching this session's standing rule that a correct, provable simplification
lands even where it doesn't clear the noise floor.

Two other `.resize()` call sites share the identical shape --
`vertex_valences` in `mesh_edgebreaker_traversal_predictive_decoder.rs` and
`mesh_edgebreaker_traversal_valence_decoder.rs` -- but belong to the
predictive and valence traversal decoder types, not the Standard type any
benchmark payload in this session's corpus exercises. Left open rather than
fixed blind.

### The Traversal Nobody Thought Was The Default

The previous round's "not the Standard type any benchmark exercises" was
wrong. `select_edgebreaker_traversal` picks the valence traversal for any
mesh with 1000+ faces below speed 5 -- exactly what this session's own
speed-1 measurements have been decoding through since round one, on every
Bunny variant. `vertex_valences` in both `mesh_edgebreaker_traversal_valence_decoder.rs`
and its type-1 legacy sibling `mesh_edgebreaker_traversal_predictive_decoder.rs`
grow one vertex at a time from `on_vertex_created`, the same shape as
`vertex_corners` and `is_vert_hole` before them -- deliberately left unsized
against the bitstream's own vertex count, since that count is an unvalidated
header claim.

Fixed with a new trait method rather than another one-off: `EdgebreakerTraversalDecoder::reserve_vertices`,
a sibling to the existing `reserve_traversal_order`, called once from
`decode_connectivity` with the same already-validated bound the corner table
and `is_vert_hole` already trust
(`max_num_vertices.min(input_face_bound)`) -- honouring it costs nothing a
hostile stream could not already cost through normal decode. A decoder with
no such table inherits the trait's no-op default.

`decode_loop`'s allocation count at speed 1: `105` to `91` allocations per
decode, `7.61` to `7.25` MB -- the same signature as the two fixes before it.
`dev/profiling/abn.sh`, two builds per condition, speed 1: `~0.5-0.8%` on
`bunny_pos` and the normal-carrying Bunny, both past the `~0.4%`
build-to-build spread; no change past the spread on the UV variant. The
type-1 predictive decoder has no benchmark payload -- it is reachable only
under `force_predictive_traversal`, a pre-0.10.0 compatibility path -- so it
is fixed on the strength of being the identical shape as its valence
sibling, unmeasured; `cargo test`'s legacy round-trip fixtures cover it for
correctness, not speed.

### The Encoder's Turn: Dead State And Unreserved Symbols

Every allocation round so far had audited decode; the encoder -- the side the
synthetic sweep puts at `0.90x` of C++ at speeds 5-9 -- had never had the same
treatment. A new `encode_loop` example (the encode-side sibling of
`decode_loop`: one side per process, counting allocator, `SAMPLE_ALLOC`
backtraces) gave the encoder its first allocation histogram, and it held the
same shapes decode's five rounds had already named:

- `encoded_faces`, a `Vec<(FaceIndex, CornerIndex)>` pushed once per face and
  read by nothing -- the decoder's dead-traversal shape again, paying a
  65 KB -> 1 MB reallocation chain per encode. Deleted.
- The per-symbol vectors (`symbols`, `symbol_to_encoder_corner`,
  `processed_connectivity_corners`) growing unreserved through capacity
  doubling. Upstream reserves `processed_connectivity_corners_` against
  `num_faces` (`mesh_edgebreaker_encoder_impl.cc`); all three are now
  reserved the same way.
- `generate_attribute_traversal`'s `corner_order` collected to exact capacity
  and then pushed the init-face corners past it, doubling a 278 KB buffer to
  add a handful of entries. Sized for both parts up front.

Two source-reading finds against upstream `ComputeOppositeCorners` landed in
the same commit: the half-edge removal shifted the whole remaining bucket
with `copy_within` (a `memmove` call per matched half-edge, ~100k per Bunny
encode) where C++ shifts one entry at a time and stops at the first unused
slot -- now the same early-stopping loop; and the degenerated-face count ran
as a separate whole-table pass after `compute_vertex_corners`, where C++
counts it inside the half-edge loop against the pre-split vertex map -- now
counted there, which also pins the count to the same table state C++ reads
(the late pass could in principle disagree after non-manifold vertex
splitting).

`encode_loop` on the position-only Bunny at speed 5: `163` to `101`
allocations per encode, `17.2` to `12.5` MB, output byte-identical
(`58,893` bytes; `83,754` at speed 9, also matching the C++ bridge's output
exactly). `dev/profiling/abn.sh`, two builds per condition:

| payload | speed | encode |
| --- | ---: | ---: |
| position only | 5 | `6.1-6.5%` |
| position only | 9 | `4.2-4.9%` |
| position only | 1 | `1.9-2.5%` |
| with normals | 5 | `3.6-4.3%` |

Decode as the control: the `real_bench` pair read the head `3-5%` slower on
decode, which none of the changed functions can reach (`CornerTable::init`
and everything under it is encoder-only). Cross-checked in a different
binary: `decode_loop` built at both commits shows mixed-sign differences
under `1%` and identical allocation counts (`65` allocations, `6.95` MB both
sides), so the `real_bench` decode delta is that binary's link layout, not a
regression -- and a reminder that the harness-side `black_box` pad perturbs
the harness binary's own code, not the layout of the `draco_core` rlib it
links, so two "independent" builds of one condition share more layout than
the protocol assumes.

Against the standing `0.90x` encode gap at speeds 5-9, this round closes
roughly half to two-thirds of it on the payloads measured. What the fresh
profile says is left on encode: the corner-table complex (`~23%` self time,
the same half-edge search C++ runs), `geometry_indices::eq` inside those
linear searches, and the generic bounds-check/`Option` machinery -- the same
floor decode reached, with the same `2.0%`-with-`unsafe` standing price. The
kd-tree encoder's six-copies-per-node pattern (the decode side's `edaba23`)
remains open and unmeasured; nothing here benchmarks a kd-tree encode.

### What The Encoder Actually Spends Its Time On

The round above left "the corner-table complex, `~23%` self time" as the
encoder's remaining lead, read off a whole-encode profile. That reading was
wrong by a factor of two, and finding out changed what the next round should
be.

`log2` looked like the obvious next target -- `8.3%` self time at speed 1 on
the normal-carrying Bunny, with `shannon_entropy` another `7.0%` beside it.
It is not a target at all: upstream computes `frequency * log2(frequency)`
for the old and new frequency of every symbol on every peek, with no memo,
and this port has memoised it since `f9a331d`. On that axis the port is ahead
of C++ by construction, and the remaining calls are the cost model itself,
which byte parity pins. A new `dev/profiling/callers.py` (which call sites
put time in a hot leaf, as opposed to `resolve.py`'s "where does time go")
settled that in one command instead of a benchmark round.

The real shape came from a harness the tooling did not have: `ct_bench`,
which loops `CornerTable::init` alone. **Corner-table construction is `5.4`
ms of an `11.4` ms position-only speed-5 encode -- about `45%`.** The
whole-encode profile had attributed `10%` to `CornerTable::init`, because
inlining splits the function across source lines and an inclusive-by-name
rollup counts only the frames that survive as symbols. One source line's
inclusive share is not a stage's cost, and this document had been reading it
as one.

Two changes went in on the strength of that, both provably identical and
neither a speedup:

- `init` filled `corner_to_vertex_map` with a bounds-checked store per corner
  over a `resize` fill that every one of those stores overwrote. Corner
  `3f + i` is face `f`'s vertex `i`, so the map is the face array read as one
  flat run -- `clear` plus `extend_from_slice`, one copy, no fill.
- `compute_opposite_corners` indexed `vertex_edges` per entry while searching
  a vertex's half-edge bucket and again while inserting into it, re-reading
  `vertex_offset` inside the search loop. A vertex's half-edges are one
  contiguous run, so both now slice that run once and iterate it.

Measured on the changed path rather than through the encode that dilutes it:
two builds per condition, 14 interleaved rounds each, `5373.5` to `5375.9`
us/init -- **`+0.05%`**, with the two head builds straddling the two base
builds. Real work came off and no time did. That is the third independent
bounds-check null in this codebase (the rANS LUT elisions cost `+0.7%` and
`+1.4%`, the face-triple slice read `1.4%` slower), and the reason is the one
the earlier nulls also had: a predicted bounds check off the dependency path
is absorbed by an out-of-order core. Treat the class as unpaid here until a
profile says otherwise.

The same session also corrects the previous round's control reading. A
`black_box`-guarded pad in `real_bench.rs` perturbs the *harness* binary's
layout and leaves the `draco-core` rlib's alone, which is why two "builds per
condition" agreed to `0.1-0.4%` there. Putting the pad inside `draco-core`
instead puts the floor at `~1.4%` on this workload -- an order of magnitude
larger, and the floor a library change actually has to clear. Every figure in
the previous round is well above `1.4%` and stands; the point is that the
floor quoted beside them was measured on the wrong binary.

### The Biggest Stage Is Not The Gap

That left the question worth asking: `45%` of encode is one algorithm C++ runs
too, at per-corner costs already measured to their floor -- is the port behind
*there*? Nothing had ever compared the stage on its own, because the bridge
exposed no corner-table entry point and a whole-encode benchmark times this
stage together with everything around it.

Added: `draco_profile_corner_table` in the bridge, and a
`corner_table_loop` example that runs both sides on the identical flat face
array, each building it once outside its own timed loop. It prints the vertex
and degenerated-face counts from both so a run that built two different tables
is visible rather than quietly comparable.

Stanford Bunny, `69,451` faces, position only, against the pristine 1.5.7
`Release` build pinned explicitly through `DRACO_CPP_BUILD_DIR` -- three runs
of `120` iterations each:

| side | us per build |
| --- | ---: |
| C++ `CornerTable::Create` | `6,462` / `6,555` / `6,805` |
| Rust `CornerTable::init` | `5,390` / `5,393` / `5,453` |

Both sides build the same table (`34,834` vertices, `0` degenerated faces).
**The port is `1.2x` ahead on the stage that is `45%` of its encode**, and the
gap is a fifth of the measurement, far above anything a floor could explain.
The unpinned run -- which had picked up the other locally available optimized
build -- gave the same answer, so the direction is not an artifact of which
reference was linked.

That is a redirect, not a result to celebrate. If the port is ahead on `45%`
of encode and still behind overall at speeds 5-9, then the entire remaining
gap lives in the other `55%` -- the traversal, the predictors, the entropy
stage -- and every round that has gone into the corner-table complex has been
spending effort on the half that was already winning. The next round should
start by splitting *that* half the same way this one split the table: a
per-stage comparison against C++ on identical input, before any profile gets
read for hot lines.

The caveat on this number, stated so it is not read as more than it is: it
compares one function against one function on a manifold mesh with no
degenerate faces and no attribute seams. It says nothing about how the two
sides behave on the inputs where their non-manifold handling diverges.

### The Sweep For What The Lint Cannot See

`encoded_faces` was the second write-only structure found by hand (the
decoder's dead traversal was the first), which raised the right question:
why does nothing catch these? Because nothing can -- rustc's `dead_code`
lint counts a write as a use (`.push()` is a method call on the field), so
state that is written and never read is outside its reach by construction,
and in a profile a dead write is indistinguishable from a live one. The only
detector is grepping for readers.

So that grep ran as a sweep: every struct field in `draco-core` with writes
and no reads anywhere in the crate. Three candidates; one
(`attribute_predictions`) was a false positive, read across a line break the
single-line pattern missed. The two real ones are gone: `point_to_vertex_map`
on `MeshEncoder`, which on the sequential path allocated an identity
`(0..num_points).collect()` -- `139` KB on the Bunny -- solely to fill a field
nothing read; and `init_face_input_indices`, whose own comment said "for our
own debugging". Output byte-identical, no speed claim made or needed. The
sweep is cheap to re-run and worth re-running after any porting burst, since
ported-but-unread state is exactly what a behaviour-first port produces.

### And The Whole Encode, Against A Reference Pinned On Purpose

Pinning `DRACO_CPP_BUILD_DIR` for the stage comparison also made the
denominator cheap to take, and it does not say what this document's headline
says. `encode_loop`, Bunny, position only, `40` iterations a side, timed
region matched, pristine 1.5.7:

| Speed | C++ | Rust | | bytes |
| ---: | ---: | ---: | ---: | ---: |
| 1 | `32,843` | `23,807` | `1.38x` | `45,135` |
| 5 | `12,621` | `11,259` | `1.12x` | `58,893` |
| 9 | `11,961` | `11,369` | `1.05x` | `83,754` |

Both sides emit byte-identical payloads at every speed, which is what makes
the comparison a comparison. **On this asset the encoder is ahead at all three
speeds**, where the `[Speed Snapshot]` table above -- taken on the synthetic
seeded sweep -- reads `0.90x` at speeds 5 and 9.

Decode, taken the same way in the same session as a check on the pinning
rather than as news, reproduces the Bunny table above: `5,777` / `3,279` /
`2,950` us for C++ against `6,575` / `4,252` / `3,785` for Rust at speeds
`1` / `5` / `9` -- `0.88x`, `0.77x`, `0.78x`, against the `0.86x`, `0.78x`,
`0.76x` already recorded there. So the decode figures were taken against a
pristine reference and stand as written; it is the *bridge* tables, and the
encode half of the warning above, that needed correcting.

Both encode readings are real; they are different assets, and the split is the
same one decode already showed, where the synthetic sweep said `1.6x` behind
and the Bunny said `1.3x` at worst. The synthetic position-only meshes are roughly `19k` faces
against the Bunny's `69k` and cost about twice as much per face on both sides,
so they weight per-call setup far more heavily. Neither table should be quoted
as "the" encode ratio without naming its asset; this session's work makes the
Bunny row move and leaves the question of which asset a consumer resembles
exactly where it was.

### Splitting The Other Half, And Where The Sweep's Gap Actually Was

The previous round ended by saying the remaining encode gap must live outside
the corner table, and that the next step was to split that half the same way.
Doing it moved the question twice before it produced an answer.

**First: the harness was comparing unequal regions.** `draco_profile_encode`
builds its `draco::Mesh` under a separate timer and times `EncodeMeshToBuffer`
alone; `encode_loop`'s Rust side timed `set_mesh(mesh.clone())` together with
the encode. Every figure it had produced compared a Rust encode plus a 1.2 MB
mesh copy against a C++ encode without one. Charged separately, the clone is
`~160` us on the Bunny and speed 5 reads `10,590` us where the mixed region
read `11,259` -- so the Bunny table above understates the port by about `6%`.

**Second: the gap is not spread across the sweep, it is one family.** The
sweep's twelve meshes are four families, and its aggregate `0.90x` hid the
fact that three of them were already ahead. Dumped to `.obj` (via
`dump_seeded_meshes_as_obj`) and run through the pinned tools at speed 5,
before this round's fix:

| family | C++ | Rust | |
| --- | ---: | ---: | ---: |
| grid | `2,638` | `2,077` | `1.27x` |
| torus | `2,630` | `2,279` | `1.15x` |
| fan | `17,631` | `16,242` | `1.09x` |
| **boundary ribbon** | `3,152` | `3,516` | **`0.90x`** |

All three ribbon seeds were behind (`0.90x`, `0.96x`, `0.85x`); nothing else
was. So "the encoder is `10%` behind at speeds 5-9" was never a property of
the encoder -- it was one topology, averaged into a sweep.

(The deficit itself did not survive a better harness: see [One Run For The
Whole Matrix](#one-run-for-the-whole-matrix), where the ribbon interleaved in
a single process reads `0.98-1.04x` rather than `0.90x`. What this round found
inside it -- three passes over the positions where one does -- was measured
against paired Rust builds and stands on its own.)

**Third: splitting the ribbon.** `corner_table_loop` puts its table at C++
`1,031` against Rust `794` -- `1.30x` ahead, the same direction as the Bunny.
Subtracting leaves everything-else at C++ `2,121` against Rust `2,722`, so the
whole `364` us deficit and more sat outside the table, exactly where the last
round predicted.

**What was there.** Attributing every sample to its nearest `draco_core`
caller rather than reading a self-time table found `position_bounds` at
`10.9%` of the ribbon encode and `5.6%` of the Bunny's -- larger, on the
ribbon, than the entire gap. `build_encoded_mesh_info` runs unconditionally on
every encode, and on the quantized position path it built the portable
attribute, built the dequantized attribute from that, and folded the result,
to report six numbers. `EncodeMeshToBuffer` does none of this, so part of the
"gap" was work the two sides were never both doing.

The fix is not to skip the reporting but to stop paying three passes for it:
the quantize/dequantize round trip is monotonic non-decreasing per component,
so the extremes map to the extremes and folding the *original* attribute then
round-tripping two scalars is the same answer. One pass, six scalar
operations, no attributes materialized.
`quantization_round_trip_monotonic_test` pins the monotonicity itself, and
fails when the round trip is deliberately broken.

`dev/profiling/abn.sh`, two builds per condition perturbed inside
`draco-core`, 24 interleaved rounds: **`12.2-12.8%` on the ribbon, `6.0-6.4%`
on the Bunny**, paired builds agreeing to `0.6%`. Against C++ at speed 5,
medians of three rounds, the ribbon family goes from `0.85-0.96x` to
`0.98-0.99x` -- parity, within the C++ side's own `10%` run-to-run drift on
this payload, against the Rust side's `1.7%`.

Method notes worth carrying, both of which cost time this round:

- **Quote the C++ side as a median of several runs, or not at all.** On the
  ribbon it moved `3,236` to `3,632` between runs of one binary on one
  payload -- `10%`, enough on its own to invent or erase a gap this size. The
  Rust side held to `1.7%` across the same runs.
- **A sweep's average is not a workload.** Four families were averaged into
  one ratio that described none of them, and the round that chased it spent
  its effort on the corner table, which was ahead on every family. Split a
  composite benchmark before optimizing against it.

After this, the port is ahead or at parity on every seeded family and every
Bunny speed measured. The open leads it leaves are listed under
[Unexplored](#unexplored), of which the `fan` family is by far the largest:
splitting it the same way found `15,679` us of its `16,465` us encode inside
`CornerTable::init` -- `95%`, against `802-818` us for the ribbon and grid
tables at comparable face counts.

### The Fan's Quadratic Stage Was Not The One Predicted

The previous round handed this one the largest measured lead in the encoder:
`15,679` us of the fan family's `16,465` us encode inside `CornerTable::init`,
`19x` per face what the same table costs on a grid, with the hypothesis that
`compute_opposite_corners` searches a high-valence vertex's half-edge bucket
linearly and is therefore `O(valence^2)` on the hub.

**The hypothesis was wrong, and the counter that killed it is the cheapest
thing in this round.** A valence histogram confirmed the shape of the input --
the bipyramid fan has two hub vertices of valence `8,401` against a median of
`4`, where grid and ribbon peak at `6` and `3`. But counting the actual trip
counts of the bucket search, the insertion scan and the removal shift found
all three linear, and the fan doing *fewer* search steps than the grid at
comparable face count:

| family | faces | max valence | search steps | insert | shift | `init` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| grid | `18,050` | `6` | `72,294` | `45,220` | `44,839` | `938` |
| ribbon | `19,458` | `3` | `87,560` | `48,646` | `29,185` | `1,022` |
| fan | `16,802` | `8,401` | `67,207` | `33,603` | `33,603` | `24,866` |

`compute_opposite_corners` searches the *sink* vertex's bucket, and on a fan
the sink of a hub-incident corner is a ring vertex of valence four. The hub's
own bucket is written, not searched.

**Timing the three stages of `init` separately put `18,847` us of the fan's
`19,827` in `break_non_manifold_edges`** -- `95%` of the build, against
`237-355` us for the same stage on grid and ribbon. That function walks the
1-ring around each pivot and, for every edge, scans the list of edges already
seen around that pivot looking for a repeated sink vertex. The list is the
pivot's valence, so the walk is `O(valence^2)` -- on the hub, `8,401^2` per
hub, twice.

**The fix is an index over that list, and it needs only two entries per key.**
The scan takes the first entry whose corner differs from `opp_edge_corner`,
skipping the one that merely closes the 1-ring; the corners stored are
distinct, one per corner visited around the pivot, so at most the first
candidate can ever be the skipped one. A per-sink-vertex slot holding the
first two corners answers the scan in `O(1)`, and a generation stamp scopes
the table to one pivot walk without clearing it.

Rust matches upstream here line for line, including the asymmetry where the
pushed key is `vertex(Previous(current_c))` while the lookup key is
`vertex(Next(current_c))`. That oddity is preserved -- the index is keyed on
what is stored, not on what the comment says is stored.

`corner_table_loop`, pinned 1.5.7, 100 iterations:

| family | C++ table | Rust table before | Rust table after |
| --- | ---: | ---: | ---: |
| grid | `1,082` | `818` | `926` |
| ribbon | `1,059` | `802` | `925` |
| **fan** | `16,968` | `15,679` | **`886`** |

Whole encode at speed 5, `encode_loop`, three interleaved rounds per side,
medians:

| seed | C++ | Rust before | Rust after | |
| --- | ---: | ---: | ---: | ---: |
| fan 0 | `18,340` | `16,242` | `1,783` | `10.3x` |
| fan 1 | `23,776` | -- | `2,070` | `11.5x` |
| fan 2 | `14,838` | -- | `1,588` | `9.3x` |

Grid, ribbon and torus were run in the same session as controls and did not
move: `2,050`, `3,370` and `2,290` us against C++ `2,330`, `3,240` and
`2,690`.

So the fan family goes from `1.09x` to `9-11x` ahead, and the cost that made
it the largest lead in the document is gone from the Rust side while C++ still
pays it in full. This is the first place the port beats the reference by an
order of magnitude, and it does so because upstream's `TODO(ostava)` on this
loop was never taken up.

**`break_non_manifold_edges_matches_the_list_form`** pins the change: random
triangle soups over four to eight vertices produce the folded 1-rings the
function exists for, and every one is built twice -- once through the index,
once through upstream's list scan kept as `break_non_manifold_edges_by_list`
-- with the two `opposite_corners` arrays compared. The test asserts that the
breaking path fired on more than a hundred of the thousand soups, so it cannot
pass by never exercising what it covers, and disabling the index's lookup
makes it fail.

One thing the test does *not* cover: across those thousand soups the list form
never once skipped a match and then took a later one, so the second-slot
fallback is unexercised. It is kept because it is what upstream's loop does,
not because a case for it was found.

Method note: **a hypothesis with arithmetic behind it is still a hypothesis.**
The `O(valence^2)` reasoning was right about the shape of the cost and wrong
about which loop paid it, and the twenty-line counter that settled it ran
before any code was changed. Stage timings inside a function that is already
known to be the hot one cost almost nothing and would have pointed straight at
the answer.

### The Restart Upstream's TODO Points At, Tried And Rejected

`break_non_manifold_edges` has a second quadratic shape, and it is the one
upstream marked: `TODO(ostava): This can be optimized as we don't really need
to iterate over all corners.` Breaking an edge sets `mesh_connectivity_updated`
and the whole corner sweep runs again from zero. The bound is real -- a sweep
that updates connectivity does at least one break, breaks are bounded by
edges, so the worst case is `O(corners * edges)`.

**The bound is not the behaviour.** A sweep does not stop at the first break;
it continues, breaking everything it meets, and only the corners a broken walk
left unvisited need another sweep. Counting sweeps over 140 random triangle
soups -- 50 to 7,000 faces over 4 to 60 vertices, the densest folding this
document has been able to construct -- the count saturates at **three**, and
does not grow with size:

| soups | sweeps |
| ---: | ---: |
| 6 | `1` |
| 16 | `2` |
| 58 | `3` |

A clean manifold mesh does one sweep, so the restart costs nothing at all
there. Two deliberately constructed shapes -- a cluster of folded 1-rings
placed ahead of a large clean grid, and a 200,000-face soup -- both did one
sweep and never triggered it.

The optimization was written anyway: corners never become unvisited, so a
repeated sweep can start at a cursor advanced past the visited prefix instead
of at zero. It is four lines, preserves the visit order exactly, and it was
**not kept**. What it saves is bounded by construction at two linear scans of a
`bool` array over `num_corners` -- about `0.3%` of a table build on the only
inputs where it happens at all -- and the A/B says so: `34,064` to `36,361` us
on the worst soup, `5,120` to `4,408` on another, which is a change reading as
noise in both directions. The function's subtlety is what made the previous
round misdiagnose it; adding state to it for an effect below the measurement
floor is the wrong trade.

Worth carrying: **a complexity bound and a measured trip count are different
claims.** This loop's worst case is quadratic and its observed cost is a
constant three sweeps, and only one of those two facts tells you whether to
spend code on it.

### One Run For The Whole Matrix

Every per-family figure above cost a process per cell: `encode_loop` takes one
mesh, one side and one speed, so a four-family table at five speeds is forty
processes, the two sides never see the same machine conditions, and the rounds
needed to beat the C++ side's drift multiply all of it. `encode_matrix` runs
the matrix in one process, interleaving the sides within each round, and
prints the median with **the spread beside it** -- a cell whose spread is wider
than the gap it claims resolved nothing, and that should be visible rather
than assumed.

Five rounds, 80 iterations per cell, pinned 1.5.7:

| payload | speed | C++ | spread | Rust | spread | |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| grid | 0 | `6,138` | `11.3%` | `3,997` | `9.4%` | `1.54x` |
| grid | 3 | `2,240` | `12.9%` | `1,692` | `3.5%` | `1.32x` |
| grid | 5 | `1,935` | `2.9%` | `1,446` | `2.6%` | `1.34x` |
| grid | 8 | `1,802` | `2.8%` | `1,358` | `2.3%` | `1.33x` |
| grid | 10 | `584` | `3.4%` | `379` | `3.8%` | `1.54x` |
| ribbon | 0 | `7,142` | `2.8%` | `5,084` | `7.3%` | `1.40x` |
| ribbon | 3 | `2,863` | `7.4%` | `2,913` | `7.4%` | `0.98x` |
| ribbon | 5 | `2,652` | `2.5%` | `2,577` | `2.0%` | `1.03x` |
| ribbon | 8 | `2,510` | `2.5%` | `2,411` | `2.0%` | `1.04x` |
| ribbon | 10 | `1,082` | `1.0%` | `824` | `3.7%` | `1.31x` |
| torus | 0 | `7,165` | `1.9%` | `4,737` | `1.2%` | `1.51x` |
| torus | 3 | `2,750` | `3.8%` | `2,073` | `3.8%` | `1.33x` |
| torus | 5 | `2,260` | `3.7%` | `1,752` | `3.0%` | `1.29x` |
| torus | 8 | `2,096` | `4.9%` | `1,626` | `3.4%` | `1.29x` |
| torus | 10 | `593` | `3.4%` | `398` | `6.2%` | `1.49x` |
| fan | 0 | `21,248` | `2.8%` | `3,421` | `1.6%` | `6.21x` |
| fan | 3 | `18,067` | `6.1%` | `1,523` | `3.3%` | `11.86x` |
| fan | 5 | `17,810` | `5.4%` | `1,299` | `2.9%` | `13.71x` |
| fan | 8 | `17,957` | `3.9%` | `1,232` | `1.9%` | `14.58x` |
| fan | 10 | `583` | `2.7%` | `366` | `1.3%` | `1.59x` |

Every cell's two output sizes matched, which the harness checks and flags, so
this is a correctness sweep as well as a timing one.

What it changes:

- **The ribbon was never behind.** The per-process runs put it at `0.90-0.96x`
  and this document has carried that as the one family the port had not won.
  Interleaved in one process it is `0.98x` to `1.04x` at speeds 3-8 and
  `1.31-1.40x` at the ends, with the only sub-parity cell carrying a `7.4%`
  spread on both sides -- unresolved, not lost. What the earlier figure
  measured was two processes, not two encoders.
- **Speed 10 is a different encoder.** It drops both sides by roughly `25x` on
  every family and puts the fan back at `1.59x`: the sequential encoder builds
  no corner table, so the topology that dominates every other speed does not
  exist there. Any figure quoted as "the encoder" that averages speed 10 with
  the rest is averaging two different programs.
- **Speed 0 is the widest margin on the manifold families** -- `1.40-1.54x`,
  against `1.29-1.34x` in the middle -- and it is the only speed range this
  document had never split per family.
- **The absolute numbers are lower than the per-process ones** (grid speed 5
  reads `1,446` here against `2,050` there), on both sides by a similar
  factor. Ratios within one harness are comparable; absolute figures across
  the two are not, and the tables above were taken with the older one.

### The Diagnostic Pass, Run Once For Everything

Four of the open encode leads needed a number before they could be ordered,
and run separately each would have cost its own sweep. `encode_matrix` gained
three opt-in emitters instead -- `STAGES`, `ALLOC`, `SAMPLE_ALLOC` -- and
`draco-core` an off-by-default `count_table_loads` feature, so one run answers
all of them. Installing the counting allocator did not move the timings: grid
speed 5 reads `1,447` us against `1,446` in the run before it.

**`CornerTable::init` is no longer one stage's problem.** Medians over 80
builds per payload:

| payload | total | opposite_corners | break_non_manifold | vertex_corners |
| --- | ---: | ---: | ---: | ---: |
| grid | `648` | `144` `22%` | `318` `49%` | `187` `29%` |
| ribbon | `565` | `154` `27%` | `245` `43%` | `165` `29%` |
| torus | `821` | `174` `21%` | `360` `44%` | `287` `35%` |
| fan | `621` | `131` `21%` | `302` `49%` | `188` `30%` |

`break_non_manifold_edges` is `43-49%` of the build on **every** family,
including three that have no non-manifold edge at all. On those it walks every
corner's 1-ring, twice -- once swinging left to the boundary, once swinging
right recording sink vertices -- and concludes nothing. That walk is the same
one `compute_vertex_corners` makes immediately afterwards. Nothing has been
tried here yet; it is the largest thing this pass found.

**The ribbon allocates 19,541 times per encode.** Allocations per encode, from
the same run:

| payload | speed 0 | speed 3 | speed 5 | speed 8 | speed 10 |
| --- | ---: | ---: | ---: | ---: | ---: |
| grid | `9,460` | `162` | `87` | `78` | `50` |
| ribbon | `19,634` | `19,582` | `19,541` | `19,531` | `49` |
| torus | `9,236` | `189` | `89` | `81` | `51` |
| fan | `8,612` | `143` | `85` | `76` | `51` |

Two shapes, both new. The ribbon's is roughly one allocation per vertex
(`19,460` of them) at every EdgeBreaker speed and gone at speed 10 -- so it is
in the EdgeBreaker connectivity path on a mesh where every vertex is on a
boundary, which is where the `7.9%` memory-shaped leaf was seen and never
chased. The other is speed 0's `8,600-9,500` on the families that otherwise
allocate under 200: the constrained multi-parallelogram predictor, allocating
about one per vertex of its own.

**The load counts put the two sides at exact parity on the traversal.**
Instrumented C++ (a *copy* of the reference, never the pinned build) against
the Rust feature, one encode of the grid at speed 5:

| accessor | C++ | Rust |
| --- | ---: | ---: |
| `swing_left` | `116,760` | `116,760` |
| `swing_right` | `54,908` | `54,908` |
| `left_corner` / `right_corner` | `27,266` | `0` |
| `opposite`, called directly | `121,670` | `175,822` |
| **`opposite_corners` loads, total** | **`320,604`** | **`347,490`** |
| `corner_to_vertex_map` loads | `543,028` | `308,378` |
| `vertex_corners` loads | `9,214` | `9,214` |

The swing counts match **to the call**, and so does `vertex_corners`. What
does not: the port makes `26,886` more `opposite_corners` loads -- `8.4%`, and
`8-9%` on all four families -- and `43%` *fewer* `corner_to_vertex_map` loads.

The first explanation to go was the obvious one. This port answers the
sentinel with the lookup's own bounds check where C++ branches first, so the
surplus should have been loads at out-of-range indices; a counter for exactly
those reported **zero** on all four payloads. The surplus is real loads at
valid indices, about `1.5` per face, and the port reaches them through
`opposite` directly where C++ routes `27,266` of its own through
`GetLeftCorner`/`GetRightCorner` -- accessors this port has and its encoder
never calls. Since both forms are one load, that routing is not the surplus
either; it only accounts for where the calls are spelled.

So the encoder has a call-count anomaly of the same kind as decode's round
two, an order of magnitude smaller: `8-9%`, uniform across topologies, not
explained by any fusion or sentinel difference, and not yet localized to a
call site. Reading the two call graphs side by side is what localized decode's,
and it is what this needs.

Method note: **the metric has to survive the port's own optimizations.** A
per-method comparison would have shown `next` and `previous` far below C++'s
counts and `swing_left` at parity, all three of which are artifacts of fusing
`Opposite(Next(c))` into one lookup. Counting loads -- the same event on both
sides regardless of how many function calls wrap it -- is what makes "exact
parity on the traversal" a statement about work rather than about spelling.

### One Allocation Per Vertex, On The One Family That Was Behind

The diagnostic pass put the boundary ribbon at `19,541` allocations per encode
against a grid's `87`, at every EdgeBreaker speed and none at speed 10. Three
instruments in sequence turned that into a call site, and the order matters
because the first one was the wrong tool:

- **Backtraces first, and they were useless.** `SAMPLE_ALLOC` captures a stack
  per allocation, so a budget of 64 stacks is spent on whatever the encode
  allocates first -- buffers, attribute clones, the corner table -- none of
  which is the thing repeating twenty thousand times.
- **A size histogram named it in one run.** `19,460` allocations of exactly
  `16` bytes, against a `19,460`-vertex mesh: one per vertex, and `16` bytes is
  what a `Vec` of a four-byte element asks for on its first growth.
- **Narrowing the sampler to that one size** then gave 64 stacks that all say
  the same thing.

`dfs_visit_from_corner_cpp` opened with `let mut corner_stack = Vec::new()`,
and it is called once per seed corner. On a mesh where every vertex sits on a
boundary, that is once per vertex -- a fresh four-element allocation for a
stack that never holds much, twenty thousand times, thrown away each time.
The stack now lives in the caller and is cleared rather than allocated. Same
traversal, same order, same bytes out.

`encode_matrix`, five rounds of 80 iterations, pinned 1.5.7:

| payload | speed | allocations | Rust before | Rust after | ratio before | ratio after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ribbon | 3 | `19,582` -> `125` | `2,913` | `2,471` | `0.98x` | `1.14x` |
| ribbon | 5 | `19,541` -> `84` | `2,577` | `2,156` | `1.03x` | `1.22x` |
| ribbon | 8 | `19,531` -> `74` | `2,411` | `2,032` | `1.04x` | `1.23x` |

`-14%` to `-16%` on the ribbon, against a `2.4-6.3%` spread, with grid, torus
and fan measured in the same run as controls and unmoved (`1.31-1.32x` at
speeds 3-8, within their own spread of before). Every cell's two output sizes
still match.

This closes the last cell in the matrix that was not clearly ahead: **no
family is at or below parity at any speed now.** It also retires the `7.9%`
"memory-shaped leaves on the ribbon" entry that had been carried unexplained
since the encoder's first profile -- it was one `Vec::new`.

What it does not close: **speed 0 still allocates `8,600-19,600` times on
every family**, unchanged by this and untouched. That is the constrained
multi-parallelogram predictor, allocating about one per vertex of its own, and
it is now the largest allocation count left.

Method note: **match the instrument to the shape of the count, not to the
question.** "Where does this allocation come from" reads like a job for a
backtrace, and a backtrace budget is exactly the wrong shape for a count that
scales with the input -- every stack it can afford is spent before the
repeating one starts. The histogram costs one counter per size and answered it
outright.

### Four Bools On The Heap, Once Per Vertex

The previous round left speed 0 allocating `8,600-19,600` times per encode on
every family, against under `200` everywhere else, and named the method that
had just worked: histogram first, then narrow the sampler to the size the
histogram reports. Run on the grid at speed 0 it took two commands.

**`8,836` allocations of exactly `2` bytes**, plus `378` of `1` byte -- against
`9,216` vertices. Narrowing the sampler to two bytes put all of them in
`MeshPredictionSchemeConstrainedMultiParallelogramEncoder`, at

```rust
let mut excluded = vec![true; num_parallelograms];
```

`num_parallelograms` is bounded by `MAX_NUM_PARALLELOGRAMS`, which is `4`, and
the line runs once per vertex. So the encoder was going to the heap for at
most four bools, per vertex, and the two sizes in the histogram are simply how
many parallelograms that vertex had. It is now a fixed array sliced to length,
and `next_permutation` takes the slice.

`encode_matrix`, five rounds of 80 iterations, pinned 1.5.7:

| payload | speed | allocations | Rust before | Rust after | spread | ratio before | ratio after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| grid | 0 | `9,460` -> `247` | `4,046` | `3,716` | `9.8%` | `1.51x` | `1.64x` |
| ribbon | 0 | `19,634` -> `177` | `5,422` | `4,674` | `6.2%` | `1.32x` | `1.53x` |
| torus | 0 | `9,236` -> `312` | `4,728` | `4,415` | `2.1%` | `1.51x` | `1.62x` |
| fan | 0 | `8,612` -> `212` | `3,459` | `3,186` | `1.2%` | `5.96x` | `6.66x` |

`-6.6%` to `-13.8%`, with speeds 3 and 5 measured in the same run as controls
and unmoved. Read the spread column honestly: torus, fan and ribbon are
resolved several times over, **grid's `8.2%` is not** -- its own spread that
round was `9.8%`, so the grid row is consistent with the others rather than
independent evidence. The allocation counts are exact, and they are the same
number on all four.

Speed 1 gains the same way (`1.66x`, `1.57x`, `1.69x`, `8.06x`), which is the
expected shape: 0 and 1 are the only speeds running this predictor.

Taken with the ribbon's `Vec::new` one round earlier, the pattern is worth
naming: **both were a container allocated inside a per-element loop for a
payload that never grows** -- a four-element stack and a four-bool mask. Neither
is visible in a profile, which reports them as `memset` and `RawVec::grow_one`
with no source line, and neither is visible in a total allocation figure until
something else on the same run allocates two orders of magnitude less. The
size histogram finds both in one command because a per-element allocation is
one size repeated, and that is a shape nothing else in an encode has.

### The Ring Walked Twice, And A Ceiling Estimated Wrong

The diagnostic pass put `break_non_manifold_edges` at `43-49%` of the table
build on **every** family, including three with no non-manifold edge at all.
The mechanism was visible in which family was cheapest: the ribbon, whose
vertices have valence 3 and hit the boundary on the first step, was the
cheapest of the four despite having the most corners, while grid, torus and
fan -- full interior rings -- were the expensive ones. So the cost was the
walk, and each pivot's ring is walked about twice: once swinging left to find
the leftmost corner, once swinging right to record.

**The two passes visit the same corners in opposite orders**, so the first can
be recorded and replayed instead of swung back. That is exact rather than
approximate: `swing_right(swing_left(x)) == x` because `previous(next(o))` is
`o`, `previous(next(x))` is `x`, and `opposite` is an involution --
established by `compute_opposite_corners` and preserved here, which only ever
clears **both** directions of an edge. The recording order the "first match
wins" rule depends on is untouched, because the replay produces the same
corners in the same order the right-swinging pass would have.

Stage timings, medians of 200 builds, two runs per condition:

| stage | before | after | |
| --- | ---: | ---: | ---: |
| grid `break_non_manifold` | `318`, `317` | `275`, `276` | `-13.2%` |
| torus `break_non_manifold` | `359`, `359` | `323`, `324` | `-9.8%` |
| grid `init`, whole | `656`, `653` | `602`, `603` | `-8.0%` |
| torus `init`, whole | `830`, `824` | `787`, `786` | `-5.2%` |

The other two stages sat still across the same runs (`compute_opposite_corners`
`145`->`141`, `compute_vertex_corners` `193`->`186` on the grid), which is the
control this measurement needed and got for free from the split.

**The ceiling was estimated at `2x` and came in at `1.13x`, and the reason is
worth keeping.** Halving the walks does not halve the function, because the two
passes are not the same price: the left pass does one swing per corner and
nothing else, while the right pass does a swing *plus* a `next`, a
`corner_to_vertex_map` read, a `previous`, a sink-slot lookup and a slot
write. Removing the cheap pass removes about a sixth of the per-corner work,
which is what `-13%` is. The estimate came from counting *passes* when the
thing that costs is *work per corner* -- a mistake with the same shape as
counting method calls when the thing that costs is array loads, two rounds
earlier.

At `~45%` of `init` and `init` at `~45%` of a position-only encode, this is
`2-3%` of an encode, which the whole-encode harness cannot resolve -- its
spread on these payloads that round was `7-22%`. The stage split is the
measurement here, and it is the reason the change is reported at all rather
than lost in a whole-encode run that would have said nothing either way.
`break_non_manifold_edges_matches_the_list_form` covers the traversal change
too, since the list form it compares against keeps the original double swing;
breaking the replay's index deliberately makes it fail.

### The Surplus Was Mine, And Splitting The Counter Found It

The diagnostic pass left the port making `26,886` more `opposite_corners`
loads than C++ on the grid -- `8-9%` on all four families -- with the swing
counts at exact parity and no explanation. Two candidate mechanisms had
already been ruled out by measurement. What found it was one more counter:
splitting `opposite` into loads made **inside the table build** and loads made
**by the encoder**.

| grid, speed 5 | C++ | Rust |
| --- | ---: | ---: |
| `opposite`, from the encoder | `121,670` | `121,672` |
| `opposite`, building the table | -- | `54,150` |

The encoder halves agree **to two calls out of 121,670**. The entire surplus
was in `break_non_manifold_edges`, at exactly one load per corner of the mesh
-- `18,050` faces, `54,150` corners -- and it was introduced by this document's
own fan round: moving the sink-vertex scan to an index hoisted

```rust
let opp_edge_corner = self.opposite(edge_corner);
```

out of the branch C++ computes it in. Upstream evaluates it only after a
matching sink vertex is found; the index version evaluated it before deciding
whether there was one. Restored to its branch, the count goes from `54,150` to
`8,836` -- the number of interior vertices, where the ring closes and the sink
vertex genuinely repeats.

With this and the replay one round earlier, the port now makes **fewer**
`opposite_corners` loads than C++ on every family: `257,619` against `320,604`
on the grid, where the pass started at `347,490`.

Stage timings, medians of 200 builds, two runs per condition: grid
`break_non_manifold` `275`,`276` -> `263`,`263`, torus `323`,`324` ->
`311`,`311`. Read that against the noise floor the same runs show -- the
untouched `compute_vertex_corners` moved `186`->`191` across them, `2.7%` --
and the honest statement is that the `-4%` is consistent and reproducible but
only modestly clear of it. **The load count is the evidence here**: `45,314`
loads removed per encode is exact and does not have a spread.

Two things worth carrying:

- **A call-count anomaly can be the port's own last change.** This one was
  eight rounds old, uniform across topologies, and looked like a structural
  difference between two encoders. It was a lookup that moved three lines.
  Nothing about `8-9%, uniform` distinguished "upstream does less" from "we
  regressed"; only splitting the counter did.
- **Where a counter is read matters as much as what it counts.** One number
  for `opposite` said the port did `8%` more work and pointed nowhere. The same
  number split by caller said the encoder was at parity to two calls and the
  table build carried all of it, which named the function, the line, and the
  round that introduced it.

### The Fold, Sized First And Rewritten Twice

The backlog carried `position_bounds_from_attribute`'s inner fold as a
call-per-scalar shape worth revisiting, with the note that its share after the
previous round's fix was unmeasured. Sizing it first was the whole point of
that note, and it changed what the round was worth doing:

| payload | fold | `build_encoded_mesh_info`, whole | share of encode |
| --- | ---: | ---: | ---: |
| grid | `20-22` | `56-67` | `1.4%` |
| ribbon | `43-44` | `106-112` | `2.0%` |

So the fold is `1.4-2.0%` of an encode, not the `10.9%` the ribbon showed
before the round that removed the two extra passes around it. Anything found
here is worth a fifth of a percent of an encode, and that is the number the
round has to be read against.

**The first rewrite made it `65%` slower.** Hoisting the per-point work into a
closure -- one that captures `min` and `max` mutably and is called from two
loops split on `point_ids.is_empty()` -- took the grid from `20` to `33` us and
the ribbon from `43` to `70`. The intended savings were real and the
restructuring around them cost more than they were worth; the accumulators no
longer stay where the compiler wants them once a closure owns them.

**The second kept only the part that was actually the idea.** A point's three
components are twelve contiguous bytes, so one fixed-size slice replaces three
offset computations and three bounds checks per point. Same loop shape as
before, same `is_empty` test per iteration, nothing restructured:

| payload | before | after | |
| --- | ---: | ---: | ---: |
| grid | `22`, `20` | `18`, `18` | `-12%` |
| ribbon | `44`, `43` | `38`, `38` | `-12%` |

Two runs per condition, same probe placement on both sides, and the numbers
repeat exactly. `-12%` of `1.4-2.0%` is about `0.2%` of an encode, which no
whole-encode run here will ever show; the function-level probe is the
measurement, and the honest claim is that **the function got cheaper and the
encode did not measurably.**

Worth carrying: **sizing first is what keeps a rewrite proportionate.** The
`65%` regression came from treating a `1.4%` line as worth restructuring
around, which is how a loop acquires a closure it did not need. Knowing the
share beforehand would not have prevented writing it, but it does say
immediately that the answer to a `65%` regression is to take the
restructuring back out rather than to tune it.

### Decode, Re-Taken Against A Pinned Reference -- And It Is Behind

`decode_matrix` is the decode side of `encode_matrix`, sharing its payload
loader, its median-and-spread reporting and its encoder options. Each cell
encodes once with the Rust encoder, then decodes the same bytes on both sides
interleaved, and compares the point and face counts per cell. It exists
because decode's stage attribution has been marked history in this document
since three rounds landed on it, and re-taking it used to mean a process per
cell.

The first run corrects the record in a direction this document has not had to
report before. Five rounds of 40 iterations, pinned 1.5.7:

| payload | speed 0 | speed 3 | speed 5 | speed 8 | speed 10 |
| --- | ---: | ---: | ---: | ---: | ---: |
| grid | `0.92x` | `0.92x` | `0.85x` | `0.89x` | `1.55x` |
| ribbon | `0.78x` | `0.58x` | `0.55x` | `0.54x` | `1.32x` |
| torus | `1.00x` | `0.95x` | `0.84x` | `0.85x` | `1.57x` |
| fan | `0.91x` | `0.95x` | `0.97x` | `0.91x` | `1.53x` |

**Decode is behind C++ on every EdgeBreaker cell of this matrix**, and ahead
only at speed 10, where the sequential decoder runs. That is not what the
opening section of this document says, and the reason is the warning at the
top of it: every decode figure predating the pinning warning was taken against
the patched reference that is `4.8x` slower. Those numbers were never
retracted for decode the way they were for encode, because nothing had
re-taken them. This does.

**The ribbon's cell was the worst in either matrix, and it was the same bug the
encoder had.** `19,515` allocations per decode against a torus's `57`, at
`0.55x`. The size histogram said `19,459` allocations of `4` bytes, and
narrowing to that size named
`corner_traversal::traverse_from_corner`, which opened with
`let mut corner_stack = vec![start_corner];` and is called once per seed
corner. On a mesh where every vertex is on a boundary, that is once per
vertex.

This is the third instance of one shape -- the encoder's depth-first walk, the
constrained predictor's exclusion mask, and now the decoder's traversal -- and
all three were a container allocated inside a per-element loop for a payload
that never grows. The stack moved to the caller:

| payload | speed | allocations | Rust before | Rust after | ratio before | ratio after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ribbon | 5 | `19,515` -> `58` | `1,662` | `971` | `0.55x` | `0.94x` |
| ribbon | 8 | `19,517` -> `60` | `1,501` | `871` | `0.54x` | `0.93x` |

`-42%`, with torus and grid measured in the same runs as controls and unmoved
(`0.84x`->`0.85x`, `0.85x`->`0.89x`, both within their spread). The ribbon's
own spread stays wide -- `44-55%` even at 200 iterations, which is worth
noting rather than smoothing over -- but the move is an order of magnitude
larger than that, and the allocation count is exact.

What this leaves is the real finding: **decode is `0.84x` to `0.94x` on every
EdgeBreaker cell**, uniformly, after the one outlier is fixed. Uniform is a
different problem from one topology being slow, and none of the encode-side
work applies to it. That is now the largest open item in this document, and
it is stated here as measured rather than as explained.

### Decode's Gap Is Not Work: The Load Counts Are At Parity

The decode matrix left `0.84-0.94x` on every EdgeBreaker cell, uniform across
four topologies, with nothing from the encode campaign pointing at it. The
first step was the metric that produced decode's two largest historical wins:
exact load counts against the instrumented C++ copy, split by caller the way
the encoder's surplus needed.

One decode of the grid at speed 5:

| accessor | C++ | Rust |
| --- | ---: | ---: |
| `opposite`, called directly | `36,100` | `36,100` |
| `opposite`, building the table | -- | `0` |
| `left_corner` | `9,216` | `9,216` |
| `right_corner` | `18,050` | `18,050` |
| `swing_right` | `280` | `280` |
| `swing_left` | `9,402` | `9,308` |
| `left_most_corner` | `18,426` | `18,426` |
| **`corner_to_vertex_map` loads** | **`199,211`** | **`199,211`** |
| **`opposite_corners` loads, total** | **`73,048`** | **`72,954`** |

Five of the eight are exact. The vertex map is exact to the load out of
`199,211`. The one difference is `swing_left`, at `94` calls out of `9,402`,
and the port comes out `94` loads **under** C++ overall.

**So the two decoders do the same table work, and the gap is not work done.**
That rules out the entire class of explanation that produced rounds two and
three -- a traversal nobody read, a scan done twice -- and it rules it out by
measurement rather than by inspection. Whatever the `10-15%` is, it is per-call
cost, memory behaviour, or something outside the corner table.

The allocation figures point at the second of those, and this document already
sized it. A grid decode allocates `2.4` MB for a mesh whose decoded form is
about `330` KB, and the size histogram shows the shape round four described:
twelve allocations of `num_vertices * 4` (`36,864` bytes here), four of
`num_vertices * 12`, and a handful of face-sized buffers. Round four then
measured what that costs by swapping the global allocator in the harness:
`19.6%` to `30.2%` on decode, at `60-100x` the build-to-build spread.

That is the same order as the gap, from the same side of it, which makes the
one-sided allocator comparison this document has carried as an open decision
the thing that now blocks the answer:

- If the C++ reference's CRT allocator is materially better than Rust's
  default on Windows, `0.84-0.94x` on loads-identical code is what that looks
  like, and the port's answer is an allocator choice its consumers make -- not
  a code change.
- If it is not, then `19.6-30.2%` of the Rust decode is recoverable by
  allocating less, and the twelve `num_vertices * 4` buffers are where to
  start.

Nothing here distinguishes those, and no amount of Rust-side profiling will:
the measurement that separates them is **both sides swapping allocators**,
which needs the C++ build to link one too. That is the next step, and it is a
build question rather than a code one.

Method note, and it is the same one twice now: **a metric that comes back at
parity is a result, not a failed measurement.** The load comparison cost one
patched header and two runs, and what it bought was the elimination of every
hypothesis of the form "the port does more work here" -- which is where the
previous decode rounds all landed, and which would otherwise have been the
obvious place to spend the next week.

### Allocation Pressure In The Core, Independent Of Who Wins The Allocator

The allocator question above decides how to *read* the decode ratio; it does
not decide whether the core should allocate less. Less pressure wins under
either allocator and for every consumer, so it is worth doing while that
comparison stays open. What follows is the first pass, and it is reported with
its result rather than its intent.

**A dead traversal was looked for and is not there.** Twelve allocations of
`num_vertices * 4` per decode had the shape of round two's finding -- two
traversal generators both running -- so the generators were counted directly:
one per decode, `generate_point_ids_and_corners_dfs` into
`..._dfs_for_table`, with `per_decoder_traversal = 0`. The prediction-degree
generator does not run at speed 5 at all. Hypothesis closed for the cost of
one probe, before any code moved.

**What was there was six copies of one block.** Each attribute predictor's
setup did

```rust
data_to_corner_map.resize(num_points, 0);
...
data_to_corner_map.copy_from_slice(map);
vertex_to_data_map.resize(map.len(), 0);
vertex_to_data_map.copy_from_slice(map);
```

on arrays the mesh decoder's own traversal had already built and handed down
as overrides, and which nothing writes to afterwards -- so two allocations the
size of the point and vertex counts, plus two `memcpy`s, per attribute, to own
a copy of something read-only. The same block stood **six times**, once per
predictor method, differing only in comments. It is now one helper that
borrows the overrides and fills the owned vectors only when there is nothing
to borrow.

`decode_matrix`, grid at speed 5: `57` allocations and `2,419` KB per decode
become `54` and `2,347` -- exactly the two `36,864`-byte buffers, gone. The
file loses `158` lines net.

**The clock did not move.** `0.85x` to `0.88x` on the grid and `0.84x` to
`0.85x` on the torus, both inside their spread. That is the honest result and
it is worth stating plainly rather than quoting the allocation count as though
it were a speedup: three allocations out of fifty-seven is not where a
`10-15%` gap lives. The change stands on the code it deletes and the pressure
it removes, not on a measurement it did not produce.

Worth carrying: **"fewer allocations" and "faster" are separate claims, and a
round that delivers the first should not be written up as the second.** The
three per-element allocations earlier in this document each moved the clock by
`12-42%` because they scaled with the mesh; this one is a constant two per
attribute, and constants do not show up next to a per-vertex loop no matter how
satisfying the diff is.

### How Much Of Decode's Gap Is The Allocator: About Half, And All Of The Variance

With the load counts at parity, memory behaviour was the remaining candidate,
and the way to test it without touching `draco-core` is to swap the allocator
under the harness. `Counting` is now generic over what it wraps, so the
example decides -- `--features mimalloc` against the same pinned C++ in the
same process, which is what the older one-sided figure could not do.

`decode_matrix`, five rounds of 200 iterations:

| payload | speed | C++ | Rust, platform | Rust, mimalloc | ratio, platform | ratio, mimalloc |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| grid | 5 | `540` | `614` | `580` | `0.88x` | `0.93x` |
| grid | 8 | `489` | `561` | `521` | `0.87x` | `0.94x` |
| torus | 5 | `630` | `748` | `716` | `0.85x` | `0.88x` |
| torus | 8 | `556` | `641` | `612` | `0.87x` | `0.90x` |
| fan | 5 | `537` | `555` | `528` | `0.97x` | `1.02x` |
| fan | 8 | `467` | `508` | `479` | `0.92x` | `0.98x` |

The C++ column is the control and it does not move -- same binary, same
allocator, both runs.

**Two findings, and the second is the more useful one.**

`4.3%` to `7.1%` of the Rust decode is the platform allocator, which closes
roughly half of a `13%` gap and leaves the rest somewhere else. So the answer
to "is decode behind because of the allocator" is *partly*, and the honest
form of the earlier open question is now: `6-10%` remains after the allocator
is taken out of the picture.

And **the run-to-run spread collapses**: the grid's Rust side goes from
`47.5%` to `1.1%` at speed 5, `7.0%` to `1.7%` at speed 8, the torus from
`6.8%` to `0.6%`. Every wide spread this document has reported on the decode
side was the platform allocator's variance, not the decoder's. That is worth
more than the `5%`: it means a decode measurement taken under mimalloc can
resolve changes an order of magnitude smaller than one taken without it.

**This also retires a number.** Round four measured the same swap at `19.6%`
to `30.2%`. It is not reproducible now and should not be quoted: since then
this document removed a per-vertex allocation from the decoder's traversal and
two per-attribute copies from the predictors' setup, taking a position-only
decode from `137` allocations to `54`. The allocator's leverage fell because
there is less for it to do -- which is the intended outcome of those rounds,
stated as a number for once rather than as a hope.

Method note: **a swap that isolates one variable is worth building properly
once.** The earlier figure came from an ad-hoc binary that is not in the
repository, could not be re-run, and compared against a differently-built C++.
This one is a feature flag on a committed harness, runs against the pinned
reference in the same process, and carries its own control column.

### Where Decode's Time Actually Is, And Two Symbols That Lie

The allocator took `5%` and most of the variance; the rest needed a profile
rather than another ratio. Phase timings first, grid at speed 5 under
mimalloc, so the shares below are of something known:

| phase | grid | torus |
| --- | ---: | ---: |
| connectivity | `250` | `310` |
| attributes | `350` | `430` |
| -- of which the DFS traversal generation | `68` | `102` |
| -- of which `decode_values` | `182` | `243` |
| ---- of which entropy symbols | `92` | `127` |
| -- unaccounted setup | `~100` | `~85` |

Then `samply` on `decode_loop`, 3,000 decodes, self time:

| % | function |
| ---: | --- |
| `14.6` | `EdgebreakerConnectivityDecoder::decode_connectivity` |
| `10.0` | `symbol_encoding::decode_raw_symbols` |
| `9.4` | drop glue (two merged symbols) |
| `5.6` | `InternalTraversalDecoder::decode_symbol` |
| `5.3` | `MeshPredictionSchemeParallelogramDecoder::compute_original_values` |
| `5.1` | `corner_traversal::traverse_from_corner` |
| `3.6` | `CornerTable::try_grow_to_face` |
| `3.4` | `memset` |
| `2.9` | `AttributeQuantizationTransform::inverse_transform_attribute` |

**About `15%` of a decode is memory management** -- drop glue, `memset` and
`RtlCopyMemory` together -- against `54` allocations and `2.29` MB. That is
the concrete form of the pressure argument, and it is larger than the whole
remaining gap against C++.

**Two of the entries above are lies, and the reason generalises.** This build
merges identical LLVM functions, so a merged symbol carries *one*
instantiation's name for code reached from many places:

- `quicksort::<..., RAnsSymbolEncoder::create::{closure#0}>` at `2.2%` reads
  as an encoder building a symbol table during a decode. There is no such
  call. `callers.py` puts the samples under `traverse_from_corner`, and
  neither that function nor the generator it serves contains a sort at all --
  the name belongs to some other `sort_by::<[usize]>` the linker folded it
  into.
- `drop_glue::<[Vec<bool>; 4]>` at `4.7%` reads as the constrained
  multi-parallelogram predictor's crease state being freed, which cannot
  happen at speed 5 where the parallelogram predictor runs. Same cause.

So in this build a symbol *name* is evidence of a type's layout, not of a call
site. Both were chased to the source before being dropped, at two greps each --
cheaper than the alternative, which is optimizing a function that never runs.

**One change tried and rejected.** `try_grow_to_face` runs once per face and
calls `Vec::try_reserve` on both tables, though `decode_connectivity` reserves
the whole table up front from a bound it already trusts, so the reserve is
redundant in the common case. Guarding it behind a capacity check measured
`584` -> `593` us on the grid and `723` -> `735` on the torus, paired in one
session: no gain, possibly a small loss, and that round's spreads were `9-29%`
so neither reading is resolved. `try_reserve` against sufficient capacity
already compiles to the comparison the guard adds. Not kept.

Worth recording about the measurement itself: **the `1%` spreads mimalloc gave
one session did not reproduce the next.** Same command, same payloads, and the
C++ side came back at `50%` spread with the Rust side at `9-29%`. Whatever
quietened the machine earlier was not the allocator alone, and a change worth
`2%` needs a session that can show `2%` -- checked at the time, not inherited
from a previous run.

### The Setup Nobody Had Split, Split

Three questions the previous round left open, closed in one session
(`68eb30b`, `e53cee6`, `5fded47`). Rust-side spreads under mimalloc were
`0.3-4%` throughout; every paired A/B below is two binaries interleaved
`A B A B` in one session with the pinned C++ as control.

**The unidentified large buffers all have names now.** One
`SAMPLE_ALLOC_MIN=100000` run resolved every size the last round listed:
`149,784` x2 is the corner table's two arrays at their input-bounded initial
reservation (`try_reserve_faces`), `299,568` x2 is the same two arrays after
Vec doubling grew them past it mid-decode, `216,600` is the mesh's face array,
and the four `110,592`s are the symbol buffer, the integer values, the
portable attribute and the dequantized target -- all legitimate. The
`36,864`-class ten are the traversal work arrays plus the mapping
intermediate; backtraces there resolve to inlined mush, but the sizes match
the known `num_vertices * 4` locals line by line.

**The doubling overshoot is gone** (`68eb30b`): when growth passes the
reservation, `try_grow_to_face` now targets the smaller of doubling and the
header's declared face count -- exact for a truthful header, no worse than
doubling for a lying one. Grid alloc traffic `2290 -> 2185` KB per decode
(counted). The clock could not resolve it: the paired A/B read `+1%` with the
control steady, which is inside this project's measured `1.4-3.1%`
two-binary layout term, and a near-identical micro-change in the same
function was already measured unresolvable last round. Kept for the memory
shape, claimed for nothing else.

**The `~100` us of unattributed attribute-phase setup is attributed.** A
five-block phase probe (grid+torus average, speed 5, mimalloc): parse `0.3`,
per-decoder setup `90` (which is the DFS traversal generation itself, already
in the table above), `decode_values` `238`, the sequencer's mapping fix `42`,
portable-pending copies `4`. So the "unaccounted" block was two things
hiding in plain sight: the traversal was being counted twice under different
names, and the mapping fix was real and unlisted.

Two changes came out of that split:

- `e53cee6` removes the identity map fill EdgeBreaker setup wrote one
  fallible call per point per attribute, which the sequencer's mapping fix
  then overwrote entirely -- work upstream never does. Torus `-0.7%`, grid
  noise, same sign in most pairs.
- `5fded47` rebuilds the mapping fix itself: the map is assembled once as
  the attribute's own representation (a `Vec<AttributeValueIndex>` with
  INVALID for unreached points, half the size of the old
  `Vec<Option<...>>` intermediate) and handed to each attribute as one slice
  copy, instead of one fallible call per point per attribute. Torus
  `744 -> 729` us, `-1.9%`, same sign in all four pairs; grid `-0.8%`.

After the round, the mimalloc matrix reads grid `0.91-0.92x`, torus
`0.86-0.88x`, ribbon `0.97-0.99x`, fan `0.96-1.01x` -- but the C++ side's
spread was `13-49%` in that run, so those ratios are directional, not
settled. The Rust-side absolutes are the numbers to compare next session:
grid `584`/`524` us at speeds 5/8, torus `730`/`625`, ribbon `899`/`810`,
fan `526`/`481`.

### Bytes, Not Count: What The C++ Allocator Actually Pays For

The mimalloc harness collapses spread, and it also *masks the cost under
study*: the same two copy removals that read `0-2%` under mimalloc read
`3-4.6%` under the System allocator. So this round measured under System
(spreads `2-15%` on the quiet payloads; the grid stayed noisy) and used the
allocation counters -- which are deterministic -- as the primary metric.
Five changes landed (`64680ea`, `5fcf9e4`, `6d2a2cf`, `e4e0063`, `ac7ca7f`),
one instrument (`d7225e1`).

**The decisive number came from instrumenting the C++ side.** The bridge now
counts C++ allocations behind `DRACO_BRIDGE_COUNT_ALLOCS=1` (a counting build
is never a timing build): C++ 1.5.7 makes **more** allocations than the port
-- grid `78` vs `46`, torus `85` vs `50` -- and moves **fewer bytes**: grid
`1261` vs `1566` KB, torus `1241` vs `1536`, ribbon `1779` vs `3208`. The
"fewer allocations" direction is exhausted; the open question is bytes, and
on the ribbon the surplus is `1.4` MB.

What landed, System-allocator paired A/Bs, sign-consistent unless noted:

- `64680ea` -- the DFS traversal cache was cloned even when no later decoder
  would read it (three point-sized arrays per single-attribute decode), and
  `copy_point_mapping` rebuilt the portable map per point instead of one
  slice copy. Grid `897 -> 860` us (`-4.6%`), torus `-4%`, 11/12 pairs.
- `5fcf9e4` -- the edgebreaker processed-corner list and vertex-to-corner map
  were `to_vec()`d out of a decoder that was dropped on the next line;
  `mem::take` instead. Torus `-4%`, grid `-3%`, 11/12 pairs.
- `6d2a2cf` -- `decode_raw_symbols` grew past its input-bounded reserve one
  push at a time; growth now capped at the declared count (reached only
  after a capacity of real symbols, so a lie buys nothing beyond doubling).
  Ribbon `-2.5%`; torus untouched, as its symbols fit the reserve.
- `e4e0063` -- the corner-table reserve ratio went `2 -> 4` faces per byte:
  the `min` against the declared count means the ratio never inflates a
  truthful file's reservation, and at `2` the grid (2.89 faces/byte) paid a
  whole-table reallocation. Grid traffic `1935 -> 1566` KB, allocations
  `49 -> 46`; clock unresolved that session.
- `ac7ca7f` -- the seam corner table is borrowed, not cloned, per decoder.
  No seeded payload exercises the path; landed on the borrow checker's
  evidence alone, no number claimed.

Standing after the round, System allocator, seven rounds
(`grid`/`ribbon`/`torus`/`fan`): `0.92/0.98/0.87/1.00x` at speed 5,
`0.91/0.95/0.90/0.94x` at speed 8, `0.92/0.88/1.00/0.90x` at speed 0 --
from `0.85/0.75/0.81/0.93x` (speed 5) at the start of the session.

### In-Place Prediction: The Values Buffer Folds Into The Corrections

The largest byte surplus the counting round named was the separate `values`
vector: C++'s sequential integer decoder runs inverse prediction on one
buffer (its `in_corr` and `out_data` are the same pointer), while the port
allocated `corrections`, then a zeroed `values`, and wrote across. `bbbe34e`
changes the decode contract to single-buffer: `compute_original_values(data,
..)` takes the corrections and leaves the values, and every scheme qualifies
because each entry reads its correction only at the offset it is about to
write, with predictions drawn from entries already reconstructed -- the same
invariant C++'s aliasing already relied on. The vestigial `CorrType`
parameter left the decode traits with it (breaking change to `draco-core`'s
public trait surface; the workspace is ~150 commits unpushed ahead anyway).

Counters (deterministic, `ALLOC=1`, per decode at speed 5): grid `1566 ->
1458` KB, ribbon `3208 -> 2980`, torus `1536 -> 1431`; allocation counts
`46 -> 44` / `45` / `50 -> 48`. Exactly the predicted one allocation plus
one memset per predicted attribute.

Clocks (System allocator, 4 interleaved pairs, paired per-round differences,
C++ column as control): ribbon speed 5 `-99/-82/-111` us in the three pairs
after warm-up (about `-8%`, control moved < 2%) -- the ribbon cell reads
`1.01x` after the change. Torus speed 5 `-1/-19/-16/-44` us, small but
sign-consistent. Grid speed 5 mixed-sign, lean win inside the spread. All
speed-0 cells and the fan: null, as expected -- the fold removes setup
cost, not per-value work, and speed 0's time is dominated elsewhere.

Standing after the round (System, spot check): speed 5
`0.95/1.01/0.93/1.01x` (`grid`/`ribbon`/`torus`/`fan`), speed 0 unchanged
within noise.

### The dhat Round: Reserves Undershooting The Entropy Floor

The `decode_dhat` example (dhat as the global allocator, one decode inside
the profiled region -- never a timing build) gave the ribbon's surplus
per-site names. Every remaining removable byte was the same shape: an
input-derived initial reserve undershooting, then a whole-buffer
reallocation whose copy dhat counts and C++ never pays. The ribbon is the
worst case by construction: a near-pure strip entropy-codes its
connectivity at `6.0` faces per byte (the corner-table ratio sat at `4`)
and its corrections at `2.7` symbols per *bit* (the symbol reserve sat at
one per bit).

`77d2c4c` raises both dials: corner table `4 -> 8` faces per byte, symbol
reserve `8 -> 32` per byte. The `min` against the declared count still
means a truthful file never over-reserves; the hostile budgets are `288`
and `128` bytes per input byte, linear in the input. No ratio covers every
stream -- a degenerate symbol distribution makes both densities unbounded
-- so these stay measured dials, moved only when a payload shows the
undershoot.

Counters (`ALLOC=1`, per decode): ribbon speed 5 `2980 -> 2393` KB and
`45 -> 40` allocations (surplus over C++ `~1.2 -> ~0.6` MB), ribbon speed
0 `3587 -> 3206`, fan speed 0 `2129 -> 1621`. Clocks (System, 4
interleaved pairs, C++ column as control): ribbon speed 5 negative in all
four pairs (`-17..-101` us, `~-4%`), fan speed 0 `~-4%`, grid and torus
speed 0 sign-consistent small wins against a control drifting the other
way; grid/torus speed 5 null, as their reserves never undershot.

The two-block site dhat left in the connectivity decoder turned out to be
the vertex-table reserve capping the declared vertex count at the face
bound directly: a strip has `V = F + 2`, the clip cost a doubling
reallocation of the whole vertex table for two vertices. `984ec8a` bounds
it by `min(declared, 3 * face_bound)` -- three vertices per face is the
geometric ceiling, proven rather than a dial. Ribbon speed 5 `2393 ->
2203` KB, allocations `40 -> 38`.

What dhat says is left on the ribbon after all three fixes (`~420` KB
over C++): the three parallel `77.8` KB traversal vectors -- the
already-catalogued B5 -- and buffers with C++ equivalents. The per-site
map is one `decode_dhat` run away at any time.

Speed 0 then repeated the whole shape one rung denser (`bb6159b`): the
valence coder pushes the ribbon's connectivity to `11.4` faces per byte
and its corrections to `4.4` symbols per bit, so the freshly raised dials
undershot again. Now `16` faces per byte and `64` symbols per byte --
speed 0 is the densest coding a Draco encoder produces, so this raise
should be the last the corpus can force; hostile budgets `576`/`256`
bytes per input byte. Ribbon speed 0 `3017 -> 2406` KB and `54 -> 50`
allocations (surplus over C++ `~360` KB), clocks 4/4 pairs negative
(`-14..-142` us, `~-6%`) with a flat control. The C++ counters at speed 0
put the other three families within `110-120` KB of C++, so their
remaining speed-0 gap (`0.89-0.92x`) is not memory -- the next instrument
there is the phase probe, not the allocator.

### Speed 0, Decomposed At Last

The phase probe is committed now (`19fd437`, `DECODE_PHASES=1` -- its
third insertion, so it stays), with the prediction-degree traverser under
the `setup` phase alongside the DFS. First speed-0 split (System, one
process, us/decode): grid `conn 522 / setup 301 / values 561 / mapfix
52`, ribbon `612/266/653/79`, fan `488/314/447/39` -- against speed 5's
grid `439/82/206/53`. Speed 0's own costs are the valence connectivity
(`+80..100` us), the constrained multi-parallelogram values pass
(`~2.7x` the parallelogram's) and the prediction-degree traversal
(`3.7-4.8x` the DFS).

But the C++ column pays those same algorithms, and the delta-of-deltas
says how much of the speed-0 gap is speed-0's own: C++ grid goes
`740 -> 1376` us from speed 5 to 0 (`+636`) while Rust goes `829 ->
1497` (`+668`), so grid's speed-0-specific shortfall is `~30` us -- the
rest is the same gap the speed-5 cell already carries. Fan's is `~70`,
ribbon's `~120` (before this session's dial fixes, which took ribbon's
back down). Closing the rest of speed 0 therefore mostly means closing
speed 5, and the instrument for what remains is per-stage comparison
against instrumented C++ or callgrind under WSL -- the same next step
the torus cell already waits on.

### Callgrind, Both Sides: The Gap Is Connectivity, Not Attributes

Every previous decode round measured a total and guessed at its parts.
This one lays the two decoders side by side per stage, and the answer
reverses a working assumption: **the attribute path is at parity or
better, and the whole instruction gap is in connectivity.**

**The instrument, and why it exists here.** `valgrind` has no Windows
port, so this ran under WSL2 Ubuntu (callgrind 3.26, rustc 1.98 -- the
same compiler as the Windows side). Callgrind attributes instructions to
functions by symbol, so no source patching is needed on either side: the
C++ stage names and the Rust stage names are simply read off. Draco 1.5.7
builds there in five minutes (`cmake -DCMAKE_BUILD_TYPE=Release
-DDRACO_TESTS=OFF`, `make`), and both drivers are committed --
`examples/dump_drc.rs` writes the exact bytes the matrices decode,
`examples/decode_drc.rs` and `cpp/decode_drc.cpp` each decode that file
and nothing else. Both were checked to decode to identical point and face
counts before a single number was read, and the wall-clock gap reproduces
under Linux (torus `0.85x`, grid `0.90x`, fan `0.86x`), so the phenomenon
being profiled is the one the Windows tables measure.

**The decomposition** (seeded grid, speed 5, one decode, C++ totals with
its ~1.6 MB of dynamic-linker startup removed):

| stage | C++ | Rust | |
| --- | ---: | ---: | --- |
| symbol loop, table sizing, points↔corners, raw symbols | `3.55M` | `8.53M` | **`2.4x`** |
| DFS traversal, mapping fix, prediction, dequantisation | `5.54M` | `6.22M` | `1.12x` |

Inside the attribute half the port is *ahead* on the traversal itself
(`1.93M` against C++'s `2.36M`) and behind on prediction (`2.11M` against
`1.72M`) and dequantisation (`811K` against `641K`). Multiplying the
instruction split by the phase probe's wall clock puts C++'s connectivity
near `289` us against the port's `439`, and C++'s attributes near `451`
against the port's `390` -- a net `+89` us, which is exactly the measured
grid gap. Two independent instruments agreeing to the microsecond is the
strongest attribution this campaign has produced.

**A caution that came with it.** The port executes about `1.5x` C++'s
instructions while running only `1.1x` slower, so instruction counts here
convert to time at roughly half their face value -- the landed fix below
measured `-8.4%` instructions and `-4.8%` clock. Quote both.

**What landed** (`2b94943`): `try_grow_to_face` grew the corner table
three corners at a time through `Vec::resize`, whose `extend_with` walks
the fill element-at-a-time behind a set-length-on-drop guard: `130`
instructions per face, `13.8%` of a grid decode, where upstream's
`CornerTable::Reset` fills both tables once and pays nothing per face.
The capacity is already reserved before the symbol loop, so the one-face
step became a fixed-size `extend_from_slice`. Callgrind: grid `17.24M ->
15.79M` (`-8.4%`), torus `-8.3%`. Paired System clocks, four interleaved
rounds, **all eight seeded cells negative** against a flat C++ control:
grid s5 `-4.8%`, fan s5 `-6.8%`, fan s0 `-5.5%`, the rest `-1.1` to
`-3.4%`.

**What did not** (`c2f2178`): the corner table's consistency scan carried
a comment claiming its branch-free max reduction vectorises. Outlining
the call under callgrind priced it at `338,620` instructions -- `3.1` per
element, which is scalar. Unsigned 32-bit max is `pmaxud`, SSE4.1, and
the baseline `x86-64` target has SSE2; rebuilt for `x86-64-v2` the
identical source costs `210,103`. So the loop shape was never the
problem, and two rewrites aimed at the shape (eight accumulators to break
the reduction chain, then constant indices to keep them in registers)
each measured within `30` instructions of the original. Both reverted;
the comment now records the measurement and points at the target flag,
which belongs to whoever builds the crate, as the allocator does.

**Still open in connectivity, by size of the excess:** the symbol loop
itself with its un-inlined helpers (`4.13M` against `2.25M`),
`assign_points_to_corners` (`921K` against `415K`, `41` instructions per
face against `23` for the same three-vertex read),
`decode_raw_symbols` (`1.26M` against `620K`), and `try_grow_to_face`
even after the fix (`1.08M` against `Reset`'s `271K`).

## Unexplored

Leads this document has evidence for and has not followed, roughly by size of
what is known to be behind them. Each says what was measured, what was not,
and the smallest next step -- a fresh session should be able to start from any
one line.

### What the fan round left behind

The fan's `O(valence^2)` stage is fixed; what it touched on the way is not.

- **C++ still pays `16,968` us for the fan table against Rust's `886`.** That
  is upstream's cost, not the port's, and it means every fan-family ratio in
  this document is now measuring how slow the reference is on this topology
  rather than how fast the port is. A comparison that says something about the
  port needs a payload where both sides are linear.
- **The second-slot fallback in the sink index is unexercised.** A thousand
  random soups never produced a walk where the first matching entry is the one
  that closes the 1-ring and a later one is the real non-manifold edge.
  Either a construction for it exists and should become a test, or the case is
  unreachable and the code can say so -- neither has been established.
- **Whether the sweep count can be forced past three.** Bounded above by the
  number of breaks, observed at three across every soup tried, and the gap
  between those two is the whole question of whether the restart is ever worth
  code. A mesh that forces a fourth sweep would settle it; none was found.
- **`compute_opposite_corners` and `compute_vertex_corners` are now the whole
  of `init`,** at roughly `800` and `210` us on a 18k-face mesh, and neither
  has been split further. The table is `45%` of a position-only encode, so
  they are the next-largest known block in the encoder.

### Not yet split, though the tools now exist

- **The seeded families at speeds other than 5.** Speed 5 is the only one
  decomposed per family. Speeds 0-1 run constrained multi-parallelogram over
  the valence traversal and speeds 8-10 run difference prediction over single
  connectivity -- different code, and the per-family ratios there are
  unknown. `dump_seeded_meshes_as_obj` plus `encode_loop` covers it in one
  pass.
- **`6-10%` of decode is unexplained**, and the profile says it is spread
  across the real decoders rather than pooled anywhere: connectivity `14.6%`,
  raw symbols `10.0%`, traversal `5.1%`, parallelogram prediction `5.3%`.
  Nothing in that list does work C++ does not -- the load counts already said
  so. The one bucket that is not decode work is memory management at `~15%`,
  which is where an attack should go.
- ~~The `~100` us of unattributed attribute-phase setup~~ -- split and largely
  spent; see "The Setup Nobody Had Split, Split" above. What remains of the
  mapping fix is the corner walk itself, which upstream's
  `UpdatePointToAttributeIndexMapping` also pays.
- ~~The unidentified large decode buffers~~ -- all named in the same section;
  the doubling overshoot among them is fixed (`68eb30b`). What remains is
  `~2.1` MB across `~52` allocations for a `330` KB mesh, now all accounted
  for as either upstream-equivalent work buffers or the four legitimate
  value-pipeline buffers.
- ~~The separate `values` buffer~~ -- folded into the corrections in
  `bbbe34e`; see "In-Place Prediction" above.
- ~~Per-site byte totals for the ribbon surplus~~ -- taken; see "The dhat
  Round" above. Surplus now `~0.6` MB: one connectivity-decoder
  reallocation and the B5 traversal vectors are what remains named.
- **The mimalloc matrix must not be the only decode reading.** The same two
  copy removals read `0-2%` under mimalloc and `3-4.6%` under System -- the
  allocator swap masks exactly the cost the memory rounds are removing.
  Take decode clocks under System (accepting its spread), and treat the
  allocation counters as the primary metric for memory changes.
- **An encode-side call-count comparison against C++.** Instrumenting every
  `CornerTable` method and comparing exact counts is what produced decode's
  rounds two and three -- the two largest decode wins of the campaign. It has
  never been run on the encoder. The callgrind stand now makes this cheap:
  an `encode_drc` driver beside the two decode ones, and the same
  align-by-stage read.
- **The four named connectivity excesses.** See "Callgrind, Both Sides"
  above -- the symbol loop with its un-inlined helpers,
  `assign_points_to_corners` at `41` instructions per face against C++'s
  `23`, `decode_raw_symbols` at `2.0x`, and what is left of
  `try_grow_to_face`. This is where the remaining decode gap lives, it is
  now located rather than suspected, and the instrument to work against
  it is committed. Read the conversion caution first: instructions here
  are worth about half their face value in time.

### Known shapes, unmeasured

- **`build_encoded_mesh_info` is `4-5%` of an encode**, of which the position
  fold is now `1.2-1.8%`. The rest has never been split. This is work the C++
  side does not do at all, which makes it a question about the API surface
  before it is one about speed -- see the entry under Decisions below.
- **What is left of speed 0's allocations.** The exclusion mask is gone, but
  speed 0 still allocates `177-312` times per encode against `74-89` at speeds
  5-8. Smaller than what was just removed and no longer one size repeated, so
  the histogram will not answer it as cleanly; the remaining sizes are `32`,
  `16` and `36,864` bytes, at 21, 18 and 16 allocations each on the grid.
- **The kd-tree encoder still carries the six-copies-per-node pattern** that
  `edaba23` removed from the kd-tree *decoder* for `-23.4%`. Unchanged since,
  and still unmeasured, because nothing benchmarks a kd-tree encode -- that
  harness has to exist first.
- **The three parallel `Vec`s in
  `generate_point_ids_and_corners_dfs_for_table`.** Audited in round four and
  left deliberately; demoted further by the allocator round's decisive fact.
  Folding them cuts three allocations to one and moves not a single byte --
  the data is the same size either way -- and "fewer allocations" is the
  direction the C++ counter comparison closed (the port already allocates
  fewer times than C++; the open cost is bytes). Two of the three are also
  different index spaces (`vertex_to_data_map` is per-vertex, the others
  per-entry), so the fold is smaller than the catalogue entry assumed. Do
  not spend the refactor without a measurement naming these allocations.

### Decisions, not measurements

- **`build_encoded_mesh_info` runs on every encode.** `EncodeMeshToBuffer`
  produces nothing equivalent, so part of any encode comparison is work only
  one side does. Whether it should become opt-in the way
  `store_number_of_encoded_faces` already is, is an API call for this
  document's maintainer, not a performance fix.
- **The allocator comparison is still one-sided, but the question is
  smaller.** `decode_matrix --features mimalloc` now measures the swap against
  the pinned reference in one process, with a control column: it is worth
  `4.3-7.1%` and most of the variance. What is still unrun is the C++ side
  swapping too, which would say whether the reference's own allocator is
  better than the platform's -- interesting, but no longer load-bearing for
  any decision here.

### Owed to the document itself

- **Every bridge-measured C++ figure carries the `4.8x` patched-reference
  factor** described at the top: the seeded sweep, the real corpus, encode and
  decode alike. They need re-taking with `DRACO_CPP_BUILD_DIR` pinned to
  pristine 1.5.7. Until then those tables are internally consistent and
  externally meaningless.
- **The `[Speed Snapshot]` headline is an average over the seeded sweep**,
  which this session showed is a composite whose members disagree in sign. It
  should be re-taken per family, or stop being quoted as one ratio.

### Sweeps worth widening

- The write-only-field sweep that found `point_to_vertex_map` and
  `init_face_input_indices` was a single-line regex over `self.<field>` in
  `draco-core` alone; it produced one false positive from a read that spanned
  a line break. A multi-line-aware version, run over the other crates and over
  locals rather than only fields, has not been run.
- The dead diagnostic block in the constrained multi-parallelogram encoder was
  found by grepping debug *comments*, not systematically. "Computation whose
  result is never observed" has the same blind spot as write-only state --
  neither the `dead_code` lint nor a profile can see it -- and no general
  detector for it exists here.

## Benchmarks

Where each measurement in this document comes from. Point the C++ side at a
reference build with `DRACO_CPP_BUILD_DIR`/`DRACO_CPP_SOURCE_DIR` and pin it
explicitly -- see the warning at the top for what an unpinned one costs.

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

