# basisu-probe

Two questions about [`basisu`](https://crates.io/crates/basisu), asked once and
answered with numbers. It is a pure-Rust Basis Universal transcoder published
2026-07-18, Apache-2.0, covering strictly more than `draco-texture`: ETC1S,
UASTC LDR, UASTC HDR 4×4 and 6×6, XUASTC, raw ASTC, the `.basis` container with
video and global codebooks, and about twenty transcode targets against this
crate's six.

Deliberately outside both workspaces. `basisu` carries roughly twelve megabytes
of lookup tables as Rust source; nothing in the ordinary build or test cycle
should wait for it.

## Could it replace `draco-texture` in the browser?

No, by a factor of two and a half.

| module | gzip |
|---|--:|
| `ktx2-wasm`, six targets, with Zstd | **166 KiB** |
| this probe on `basisu`, seven targets, no Zstd | 437 KiB |
| this probe asking only for RGBA8, no Zstd | 390 KiB |

The measurement is generous to `basisu` twice over: the probe leaves its `zstd`
feature off while the shipped module carries `ruzstd`, and both were optimised
the same way — `opt-level = "z"`, fat LTO, then `wasm-opt -Oz --converge`.

The third row is the answer to the obvious objection. Narrowing to one target
saved 47 KiB of 437, so the tables are not reachability-trimmed: they are
dispatched over and stay linked whatever the caller asks for. No feature trims
them either — the crate's flags are `std`, `zstd` and `libm`. For a module
fetched only when a KTX2 texture turns up, that settles it.

Reproducing:

```sh
wasm-pack build tools/basisu-probe --release --target web --out-dir /tmp/probe
wasm-opt /tmp/probe/probe_bg.wasm -o /tmp/probe_opt.wasm -Oz --converge \
  --low-memory-unused --enable-bulk-memory --enable-nontrapping-float-to-int \
  --enable-sign-ext --enable-mutable-globals
gzip -9 -c /tmp/probe_opt.wasm | wc -c
```

## Is it useful as a second oracle?

Yes, and that is why this stays.

```sh
cargo test --manifest-path tools/basisu-probe/Cargo.toml
```

160 images across five fixtures, every level, six targets, byte-identical. The
node gates already compare against Binomial's prebuilt WASM, but that build is
dated 2024-11-29 — older than the source `draco-texture` was ported from — and
fed a malformed file it can be left reporting success while writing nothing.
`basisu` is a third implementation, itself verified against a vendored C++
oracle compiled in its own tree, so agreeing with it means agreeing with the
C++ transcoder at two removes and two dates through code that shares nothing
with ours.

### The one place the two disagree

ETC1S to ASTC, and it is neither one's defect. The reference C++ has two extra
branches behind `BASISD_SUPPORT_ASTC_HIGHER_OPAQUE_QUALITY`, trading a second
64 KiB table for better opaque blocks. Emscripten builds compile them out;
`draco-texture` is gated against one of those and so does not have them.
`basisu` is ported from the native configuration and does — it carries
`G_ETC1_TO_ASTC_0_255` and the CEM 4 and CEM 8 packers those branches need.

This is the first independent confirmation of that split. Until now it was read
off an `#ifdef` in the vendored source, and the table those branches need was
not even present there to check against.

Adopting them would cost about 60 KiB against a 175 KiB budget, for the pair
with the narrowest audience there is — a machine with ASTC and no ETC — and
would end byte-exactness against the browser oracle in exchange. Declined, on
those grounds rather than by omission.
