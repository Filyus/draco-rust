# draco-texture: where this stands

Written 2026-07-28, at the point the KTX2 work was paused. It records what is
finished, what was deliberately left out and why, and what the next step would
be — so that resuming does not start by re-deciding what was already decided.

The operational detail lives elsewhere and is not repeated here:
[`web/README.md`](../../web/README.md) for the target table, the survey figures
and the module sizes; [`FUZZING.md`](../../FUZZING.md) for the campaign;
[`hardening_status.yaml`](../../hardening_status.yaml) for the malformed-input
posture and its open risks.

## Finished

Both codecs `KHR_texture_basisu` permits are read and transcoded, and every
pair either can reach is implemented. Nothing in the extension's scope is
missing.

| source | targets |
|---|---|
| ETC1S | RGBA8, BC1, BC3, ETC1, ETC2, ASTC 4×4 |
| UASTC LDR 4×4 | RGBA8, BC7, ASTC 4×4, ETC1, ETC2 |

The container reads `none`, `Zstd` and `BasisLZ` supercompression, and names
plain `vkFormat` files rather than refusing them — a deliberate divergence from
the reference reader, which insists on a 44- or 60-byte DFD and so cannot say
what an ordinary R8G8B8A8 file is.

Build slices are by hardware family — `bc`, `etc`, `astc` — because that is the
axis along which a target goes out of date. There is no `legacy` flag: one was
built and removed, see below.

### What backs that up

Everything is gated byte for byte against Binomial's own build, the one three.js
ships. Current counts:

| gate | measures |
|---|---|
| `ktx2-block-formats` | 204 mip levels across nine source/target pairs |
| `ktx2-etc1s`, `ktx2-uastc` | 32 and 11 mip levels into pixels |
| `ktx2-uastc-modes` | all 19 UASTC modes; five appear in no fixture and are built |
| `ktx2-format-choice` | 17 ranking cases, desktop and phone |
| `ktx2-differential` | 2400 mutants, 2505 images identical to the reference |
| `ktx2_malformed.rs` | six sweeps over headers, long fields, truncation, payloads |
| `ktx2_transcode` (libFuzzer) | 13521 executions, 5149 edges, no finding |
| `basisu` cross-check | 215 images against a third implementation, in CI |
| C++ parity | 247 images against the vendored reference at the ported revision, in CI |

## Left out on purpose

**UASTC to BC1 and BC3.** The reference reaches them through an approximation
with its own tables. On a machine with neither BC7 nor ASTC, RGBA8 is the
honest answer rather than a reproduction of somebody else's heuristic.

**The HDR codecs** — `UASTC HDR 4x4`, `ASTC HDR 6x6`, `UASTC HDR 6x6
intermediate`. Not variants of what is implemented: separate codecs, separate
KTX2 colour models, separate bitstreams, decoding to half-float rather than to
bytes. `KHR_texture_basisu` is ratified and pins `colorModel` to
`KHR_DF_MODEL_UASTC` and the transfer function to sRGB or linear, so an HDR
payload is doubly outside it. There is no public sign of that changing: the only
issue in the glTF tracker titled for HDR textures was closed in 2019, and the
real blocker is glTF's own material model rather than the codec.

Worth knowing when it does become relevant: HDR's benefit is not confined to
content with bright highlights. The bitrate is identical — both UASTC 4×4
variants are 128 bits a block — and what differs is that the endpoints are half
floats. On an ordinary panorama, which is stored sRGB and then linearised for
lighting, the quantisation error an 8-bit encoding leaves near black is what
shows, and that is where an HDR codec wins even with nothing above 1.0.

**XUASTC LDR.** Six months old at time of writing, and its KTX2 colour models
(169, 170) are still marked `TODO - coordinate with Khronos` in Binomial's own
source. A port would be to a moving target.

**PVRTC1/2, ATC, FXT1.** The hardware is gone. The reference calls two of them
"niche" and "super obscure" itself.

## What is worth doing next, and is not done

**BC5 / EAC RG11 and BC4 / EAC R11.** The one remaining group where the format
is frozen, the hardware is current, and the gain is measurable: a tangent-space
normal map through BC1 or ETC1 falls apart, while BC5 keeps two channels at
eight bits per pair. Single-channel maps — roughness, occlusion, metalness —
want BC4 or EAC R11 for the same reason. This is the next thing to build.

## Known limits

**The node gates still run on one machine**, but nothing depends on that any
more: `tools/basis-cpp-oracle` vendors the reference at the revision this was
ported from and compares 247 images on any runner, and `tools/basisu-probe`
compares 215 against a third implementation. What the node gates add on top is
the browser-side ranking and upload, which needs a browser anyway.

**(superseded)** They compare against Binomial's prebuilt
WASM at a path inside a three.js checkout, so on a runner they print SKIPPED and
exit 0 — which was always the stated intent of that step, but left CI proving
only that the module builds and fits its budget. What proves the bytes on a
runner is the `basis-crosscheck` job: 215 images against the `basisu` crate,
which comes from crates.io and needs nothing external. The proper fix is K17, a
C++ oracle built in tree, after which the node gates run everywhere too.

**The ETC and ASTC uploads are unexercised.** Their transcoding is checked byte
for byte in Node, but no desktop offers either extension, so the
`compressedTexImage2D` call itself is only covered where a browser has them.

**The 2026-02-24 upstream change to the alpha ETC1S DFD is untested here.** It
fixed what `basisu` writes so KTX-Software validates it. This reader decides
ETC1S alpha from the DFD's length — 44 against 60 — so a file from a newer
encoder could be read differently. Every fixture predates the change and there
is nothing to test against until a file written by `basisu` v2.1 or later turns
up.

**Differential checking excludes the key/value section.** This reader reports it;
the reference acts on it. With only that section's length changed, the
reference's block output stayed byte-identical while its RGBA output did not, so
mutating it makes both readers right and different. Its range is checked — that
part was a real defect, found by the gate and fixed.

**The oracle degrades.** Fed a malformed file, Binomial's module can be left
returning success from `transcodeImage` having written nothing. An all-zero
image then looks exactly like this reader inventing pixels. The differential
gate transcodes the pristine seed after every mutant and discards a comparison
that came from a degraded instance; anyone else using that transcoder as an
oracle needs the same guard.

## Two decisions that were made and then unmade

Both are recorded because the reasoning is easy to repeat and wrong.

**`modern` and `legacy` policy flags.** `legacy = ["etc"]` read straight off the
survey — every machine with ASTC also has ETC, so the family looks like it
serves only the difference. It was backwards: ETC1S had no ASTC target at the
time, so `etc` was the only compressed path the common codec had on any phone,
and retiring it would have sent that codec to RGBA8 on the machines least able
to afford it. The layer was removed rather than relabelled. Since ETC1S now
reaches ASTC too, the flag could be reinstated honestly — but it would cost the
ETC1S half quality, and that is a decision rather than a formality.

**"ETC1S to ASTC is pointless."** Argued from the survey: every ASTC device has
ETC, so the pair serves nobody. True as far as it went, and it missed that the
pair is the prerequisite for ever retiring the ETC family at all. The better
reason to build something is sometimes not the one it looks like it has.

## The `basisu` crate, measured

[`basisu`](https://crates.io/crates/basisu) 0.1.0 appeared on 2026-07-18 — a
pure-Rust transcoder covering strictly more than this: every codec including
HDR and XUASTC, the `.basis` container, about twenty targets. It was measured
rather than argued about; see [`tools/basisu-probe`](../../tools/basisu-probe).

It cannot replace this crate in the browser: the same job built on it is
437 KiB gzip against 166 here, and 390 even when asked for one target only, so
its tables are not trimmable. It is kept as a **second oracle** instead, which
is worth more than it sounds — the node gates compare against a build dated
2024-11-29 that degrades on malformed input, and this is a third implementation
verified against a vendored C++ oracle of its own. 160 images agree byte for
byte.

The one disagreement is ETC1S to ASTC, and it confirmed something previously
only read off an `#ifdef`: `basisu` implements the two
`BASISD_SUPPORT_ASTC_HIGHER_OPAQUE_QUALITY` branches that every emscripten
build compiles out. Each of us matches a different build of the same reference.

## How upstream stands

Dates from the repository's own history, for judging what is stable rather than
what is new.

| codec | appeared | last functional decoder change |
|---|---|---|
| ETC1S | 2019 | none found; output unchanged for ≥ 20 months, measured |
| UASTC LDR 4×4 | 2020 | same |
| UASTC HDR 4×4 | 2024-09-10 | 2025-01-21 |
| ASTC HDR 6×6 | 2025-01-21 | 2026-03-06 (encoder) |
| XUASTC LDR | 2026-01-19 | 2026-07-03 |

"Measured" is the strongest evidence available and it is our own: this crate is
ported from source dated 2026-07-22 and gated against a prebuilt transcoder
dated 2024-11-29. They agree byte for byte over 204 mip levels. Whatever changed
in between did not change the output.

The container is a different story from the codecs. Between 2026-03-01 and
2026-07-19 the reference reader was hardened six times against malformed KTX2 —
an overflow in the header parser, two cases from fuzzing, a maximum texture
size, slice-descriptor validation, overflow-safe range checks, and an oversized
level length. Four of those this reader already handled. The two it did not are
[`ktx2.rs`](src/ktx2.rs)'s dimension limit and level-length ceiling, added after
reading them. That is the surface where upstream still moves, and the one worth
re-reading when it does.
