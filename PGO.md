# PGO (Profile-Guided Optimization)

Profile-guided optimization feeds runtime profile data back into a second
compilation so the compiler can lay out hot code together, inline the right
generic monomorphizations, and push cold paths out of line. For a
decode/encode codec like Draco this targets the branchy paths -- rANS
decoding, edgebreaker traversal, prediction-scheme dispatch, scheme selection
-- that ordinary `-Copt-level=3` leaves flat.

## How PGO relates to a source library

PGO is a flag on whoever compiles the final binary. A library published as
source is compiled by its consumers, each under their own toolchain and build
settings, so whether PGO applies to code from these crates is a property of
the consuming build, not of the crates. Nothing has to be done *inside* the
library to "enable" PGO: set the flags (below) on the build that produces the
final binary, and every dependency -- `draco-core` included -- is compiled
under them.

## Applying PGO to a binary

Two-pass build: compile instrumented, run a representative workload to collect
a profile, merge it, recompile with the profile.

```sh
PROFDIR=/path/to/prof

# 1. instrumented build (applies to the whole dependency graph)
RUSTFLAGS="-Cprofile-generate=$PROFDIR" cargo build --release

# 2. run a representative workload; the instrumented binary writes .profraw
./target/release/<binary>

# 3. merge the raw profiles
llvm-profdata merge -o $PROFDIR/merged.profdata $PROFDIR/*.profraw

# 4. rebuild with the profile
RUSTFLAGS="-Cprofile-use=$PROFDIR/merged.profdata" cargo build --release
```

Use an `llvm-profdata` whose LLVM version matches `rustc`'s, or the merge
rejects the raw profiles. The rustup `llvm-tools` component ships a matching
one:

```sh
find "$(rustc --print sysroot)" -name llvm-profdata
```

`cargo clean` before steps 1 and 4 removes any doubt that dependencies were
rebuilt with the intended flags.

### The profile is only as good as the training corpus

Train on inputs that exercise the paths you care about: meshes *and* point
clouds, the attribute types you actually use (positions, normals, UVs), and a
spread of encoder `speed` settings -- each selects different prediction
schemes and entropy coders. A profile trained on one kind of file is tuned to
that file and may regress on others.

### Expected benefit

On a native build (`-Copt-level=3`, single codegen unit) PGO gives roughly
**9% on decode and 12% on encode** for a representative mesh (constrained
multi-parallelogram, position attribute, quantization 10). It is
workload-dependent: branchy, dispatch-heavy workloads gain the most, tight
arithmetic loops gain little, and the figure is additive to whatever the
application itself does. The PGO binary is usually smaller too, because cold
paths get outlined.

## PGO on a WASM target

**Step 1 above cannot be done at all on `wasm32`, and step 4 does not need to
be.** `-Cprofile-generate` injects a dependency on `profiler_builtins`, and the
Rust toolchain ships that crate for the native targets and for neither wasm
one:

```sh
ls "$(rustc --print sysroot)/lib/rustlib/<target>/lib/" | grep profiler_builtins
```

`x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` return two files each;
`wasm32-unknown-unknown` and `wasm32-wasip1` return nothing, and an
instrumented build for either fails with `can't find crate for
profiler_builtins`. `-Zbuild-std=std,panic_abort,profiler_builtins` on nightly
does not rescue it: that crate's `build.rs` compiles the LLVM profile runtime
from `src/llvm-project/compiler-rt`, which the `rust-src` component does not
carry, so it panics with `profiler runtime source directory not found`. A full
Rust checkout with submodules would, which is a different undertaking.

`-Cprofile-use` needs no runtime, and that is the way through: **train on a
native build and spend the profile on the wasm one.** It works today with the
stock toolchain --

```sh
# 1. instrument and train natively, where the runtime exists
RUSTFLAGS="-Cprofile-generate=$PROFDIR" cargo build --release --example encode_loop
./encode_loop <mesh.obj> rust 5 30      # and decode_loop, and the payloads you care about
llvm-profdata merge -o $PROFDIR/merged.profdata $PROFDIR/*.profraw

# 2. spend it on the wasm build -- no profiler runtime involved
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="-Cprofile-use=$PROFDIR/merged.profdata" \
  cargo build --release --target wasm32-unknown-unknown -p drc-wasm
```

-- and it demonstrably attaches: the module comes out byte-different from the
same source built without it, in a clean target directory.

The remaining costs are the ones that make it a poor deal. The profile is a
native build's, so the `968` functions the wasm module has and the native run
never executed -- the bindings, the allocator shims, the panic paths -- carry
no data (`-Cllvm-args=-pgo-warn-missing-function` counts them). And the browser
JIT does its own dynamic profiling on top, absorbing part of whatever is left.

### Measured here: nothing, for `2.8%` of the download

On `drc-wasm`, against the shipped module and with `draco-core` already at
`opt-level = 2`, eleven interleaved rounds in one process: encode `0.989x`
with `6/11` rounds faster, decode `1.028x` with `2/11`. Both are inside their
own spread and their signs are scattered, which is what samples of zero look
like. The module grows `2.8%` of its gzip.

So the answer for this repository is that PGO is *wireable* and is not worth
wiring: the level (below) took `1.74-1.78x` from the same build for a manifest
change, and a profile on top of it adds nothing measurable. Anyone revisiting
this should train on a corpus this one did not -- five `testdata` meshes at
speed 5, encode and decode -- before concluding it can never pay.

**And for this repository, WASM is the whole of the question.** These crates
are published as source and nothing here links them into a native binary --
the only `main.rs` files are `web/build-tool` and `web/dev-server`, neither of
which is a codec, and the benchmark harnesses are dev tooling. The binaries
this repository ships are the `*-wasm` modules, which consume `draco-core`,
`draco-io` and `draco-gltf` directly. So "apply PGO here" means applying it to
those, and the paragraph above is the applicable one.

The figures above are also measured at `-Copt-level=3`, and `web/Cargo.toml`
builds release at `opt-level = "z"` with `panic = "abort"` and `strip` -- for
everything except `draco-core`, which is at `2`. That size choice is
deliberate for a module downloaded per page load, and it overlaps part of what
PGO would buy: cold-path outlining is what `"z"` already does. Neither figure
carries over to this target without being re-taken on it.

### The level itself was worth more than PGO claims to be

Before reaching for a profile, check the level. `draco-core` was on `"z"` with
the rest of the module until it was measured, and moving that one package to
`opt-level = 2` is worth **`1.78x` on encode and `1.74x` on decode** in
`drc-wasm` -- larger than the `9%`/`12%` PGO offers natively, from a
three-line manifest change and no profile to collect. It costs `24.7%` of that
module's gzip and between nothing and `6.9%` of the others'.

The knob is per package on purpose. `"z"` remains right for the bindings and
the glue, which are cold and mostly size; it is wrong for a codec whose loops
unroll and vectorise. A crate *written* for `"z"` behaves the other way and
gets slower as the level rises, so nothing here transfers by assumption --
`[profile.release.package.<crate>]` plus a measurement is the pattern, not the
particular level.

## See also

[`PERFORMANCE.md`](PERFORMANCE.md) covers how to benchmark and profile
decode/encode -- useful both for picking PGO training inputs and for measuring
the win.
