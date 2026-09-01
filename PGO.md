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

For `wasm32` builds PGO has two extra costs versus native. The profile has to
be collected by running the bundle in a browser or a WASI runner rather than
by a plain process exit, and the browser JIT already does its own dynamic
profiling, which absorbs part of the win. The native figures above are an
upper bound for the WASM case; the real yield is smaller and costs more to
set up. Whether that tradeoff is worth it is a per-target build decision.

**And for this repository, WASM is the whole of the question.** These crates
are published as source and nothing here links them into a native binary --
the only `main.rs` files are `web/build-tool` and `web/dev-server`, neither of
which is a codec, and the benchmark harnesses are dev tooling. The binaries
this repository ships are the `*-wasm` modules, which consume `draco-core`,
`draco-io` and `draco-gltf` directly. So "apply PGO here" means applying it to
those, and the paragraph above is the applicable one.

The figures above are also measured at `-Copt-level=3`, and `web/Cargo.toml`
builds release at `opt-level = "z"` with `panic = "abort"` and `strip`. That
is a deliberate size choice for a module downloaded per page load, and it
overlaps part of what PGO would buy -- cold-path outlining -- while removing
the speed baseline the `9%`/`12%` was measured against. Neither figure carries
over to this target without being re-taken on it.

## See also

[`PERFORMANCE.md`](PERFORMANCE.md) covers how to benchmark and profile
decode/encode -- useful both for picking PGO training inputs and for measuring
the win.
