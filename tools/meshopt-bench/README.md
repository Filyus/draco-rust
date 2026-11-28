# Draco and Meshoptimizer benchmark

## What the four-scene sweep shows

**Draco q12 is smaller than Meshopt q12 on all four scenes.** At q12, Draco
also has higher position and directional RGB RMSE on all four. Taking the
two Meshopt q12 errors as ceilings, Draco q14 is the smallest tested Draco
profile that meets both. The table shows both comparisons side by side.

| Scene | Draco q12, exceeds ceilings | Meshopt q12, sets ceilings | Draco q14, meets ceilings |
| --- | ---: | ---: | ---: |
| `train_7000` | 36.67 MB / 321.17 ms | 42.09 MB / 416.17 ms | 47.06 MB / 347.36 ms |
| `train_30000` | 50.01 MB / 462.66 ms | 64.70 MB / 527.75 ms | 64.96 MB / 508.25 ms |
| `playroom_30000` | 97.28 MB / 898.07 ms | 109.43 MB / 952.07 ms | 122.55 MB / 1,044.64 ms |
| `drjohnson_30000` | 160.75 MB / 1,388.56 ms | 188.93 MB / 1,551.59 ms | 205.89 MB / 1,519.86 ms |

Cells show Brotli size and local GLB load time through CPU geometry, including
Meshopt buffer processing. Draco q14 achieves lower error than both ceilings.
The sweep skipped q13 and independent attribute tuning, so the smaller
format at closely matched error remains unresolved. Directional RGB RMSE
evaluates individual splats. Rendered images were outside this sweep.

The [benchmark report](RESULTS.md) has the codec-only results, per-attribute
errors and validation details. The [archived runs](archived-results) contain
the raw JSONL and generated summaries.

## Run

On Windows, install Git, Rust and the Visual Studio 2022 C++ tools. Run
the synthetic point clouds and the tracked bunny mesh with no corpus download.

```powershell
.\tools\meshopt-bench\run.ps1
```

## Reproduce the splat measurements

Run the full benchmark with `curl.exe` and the public
[Voxel51 Gaussian Splatting dataset](https://huggingface.co/datasets/Voxel51/gaussian_splatting).

```powershell
.\tools\meshopt-bench\run.ps1 -DownloadCorpus
```

This downloads about 1.7 GB into the cache's `corpus` folder. The script
checks each file against [corpus.json](corpus.json) and reuses verified files
on subsequent runs. If you already have the four PLYs, point the script at
their directory with `-CorpusPath`.

```powershell
.\tools\meshopt-bench\run.ps1 -CorpusPath D:\data\splat-corpus
```

Use `-OnlyGltfIntegration` for GLB results, `-SkipGltfIntegration` for
codec results or `-SkipSynthetic` to omit the synthetic point clouds.

The script builds in Release mode and writes a new timestamped folder under
`runs` in the cache. The default cache is
`%LOCALAPPDATA%\draco-rust\meshopt-bench`. Set `-CacheRoot` to use another
location. The input corpus is read in place.

Each run saves `summary.md`, `raw-results.jsonl` and the generated GLBs.
The JSONL includes settings, timings, sizes, working set measurements and
attribute errors. The run's `work` directory holds intermediate buffers.

## Meshoptimizer revisions

- v1.3 supplies the vertex codec and triangle glTF tools.
- The experimental revision supplies Gaussian PLY import and `-gsd 3`.

Both commit IDs are pinned in [meshoptimizer.json](meshoptimizer.json).
The build script fetches them into a shared Git cache on demand and checks
out each commit in a separate worktree.

## Reading the results

**Point codec.** Both encoders receive the same 8-, 12- or 16-bit integer
records. Meshopt levels 2 and 3 and Draco sequential share the spatial order.
Separate rows cover Draco raw input order, KD-tree and order search. The
synthetic inputs contain one million points each with XYZ, XYZ+RGB or
XYZ+normal+RGB. Both decoders must recover the prepared records exactly.

**Splat GLB.** Both paths start from the same PLY and preserve all SH degrees
through 3. Meshopt encodes the PLY directly. Draco sequential reads the
unquantized `gltfpack -gsd 3 -noq` GLB. The lossless profile requires exact
decoded attribute values. At q12, both use a nominal 12 bits for position,
rotation, scale and opacity and 10/10/9/9 bits for SH degrees 0–3. Meshopt
uses `-vp 12 -vn 11 -vt 12 -vc 12 -gsd 3 -c -ce khr`.

The quantization grids and filters differ. Use the measured errors to compare
output quality. The SH color metric samples eight directions per splat.
Draco splat GLBs use experimental `allow_points` support outside the
primitive modes defined by `KHR_draco_mesh_compression`.

**Triangle GLB.** The tracked `bun_zipper.ply` supplies the control mesh.
Meshopt uses `-c`, `-cc` and `-cf` with `-ce khr`. Draco is measured
with and without fallback. `draco-gltf` rereads both formats and checks
vertex and index counts.

**Sizes and times.** The same gzip best and Brotli quality 5, window 22
settings apply to both outputs. New GLB timings use one untimed warm-up
and the median of three local reads. Meshopt buffers expand during import and Draco decodes during
primitive reading. Compare the full GLB-to-geometry totals. Codec decode
times are measured separately on one native thread.

## C++ Draco compatibility

Point the corpus probe at the run's `glb` directory and provide a build
of upstream Draco.

```powershell
$env:DRACO_CPP_SOURCE_DIR = 'D:\path\to\draco-1.5.9'
$env:DRACO_CPP_BUILD_DIR = 'D:\path\to\draco-1.5.9\build'
$env:DRACO_REQUIRE_CPP_BRIDGE = '1'
$env:DRACO_CORPUS_DIR = Join-Path $env:LOCALAPPDATA 'draco-rust\meshopt-bench\runs\<run>\glb'
cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge --test external_draco_corpus_probe --release -- --ignored --nocapture
```
