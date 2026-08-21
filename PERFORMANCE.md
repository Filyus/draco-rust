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

