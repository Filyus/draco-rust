# Draco and Meshoptimizer: Gaussian Splats, Point Clouds and bun_zipper

**Verdict: diagnostic. Keep Draco.** Meshoptimizer decoded identical quantized
point records faster, while Draco produced smaller payloads. For q12 splat
GLBs, Draco also gave smaller files and shorter load times in `draco-gltf`.
Meshopt recovered position, rotation and SH more accurately at the same nominal
bit budget. Draco splat GLBs use experimental `POINTS` support outside
`KHR_draco_mesh_compression`, which specifies triangle modes.

Measurements used native Release builds on one thread with a Ryzen AI 7 350,
32 GiB RAM, Windows 11 `10.0.26300.0`, rustc 1.100.0 and MSVC 14.44.35207.
The Draco crates were `draco-core 2.5.1` and `draco-gltf 0.11.1`, from
`c58482654622e5dc1d16ee3e6e010cd886e2f3fa`. Meshoptimizer v1.3 supplied
the codec and triangle tools. Gaussian PLY and `-gsd 3` used the experimental
revision. Both commits are pinned in the
[benchmark revision manifest](meshoptimizer.json).

**Point codecs.** Both codecs encoded the same integer records quantized at
8, 12 and 16 bits. Meshopt levels 2 and 3 and Draco sequential shared the same
spatial order. Separate 12-bit rows tested raw input order, KD-tree and
`point_order_search`. All variants restored the quantized point records exactly.

At 12 bits, the four SH degree 3 scenes gave these payloads in decimal MB:

| Scene | Splats | Meshopt L2 / L3 | Draco sequential | Draco order search | Position RMSE |
| --- | ---: | ---: | ---: | ---: | ---: |
| `train_7000` | 741,883 | 62.60 / 62.59 | 56.81 | 46.86 | 0.02866 |
| `train_30000` | 1,074,761 | 88.51 / 88.51 | 80.19 | 67.94 | 0.02873 |
| `playroom_30000` | 1,916,379 | 161.30 / 161.30 | 144.34 | 122.16 | 0.00417 |
| `drjohnson_30000` | 3,177,554 | 269.81 / 269.81 | 243.06 | 210.82 | 0.00469 |

Draco sequential was smaller across all three bit depths. Order search cut
the 12-bit payload by 21.9–25.1% relative to Meshopt L2, with roughly
3.4–3.8 times the encode time. Meshopt L2 took 317–1,393 ms to encode after
14–64 ms of spatial prep. Draco sequential took 342–1,432 ms. Order search
took 1.22–4.94 s in a single encode per scene.

The three synthetic clouds used one million points each. KD-tree won on XYZ,
while order search won on both colored profiles.

| Profile, 1M points | Meshopt L2 / L3 | Draco sequential | Draco KD-tree | Draco order search |
| --- | ---: | ---: | ---: | ---: |
| XYZ | 5.39 / 4.74 MB | 4.51 MB | 2.25 MB | 2.56 MB |
| XYZ+RGB | 5.40 / 5.40 MB | 3.73 MB | 4.71 MB | 3.36 MB |
| XYZ+normal+RGB | 5.54 / 5.54 MB | 3.81 MB | 7.04 MB | 3.45 MB |

**Absolute codec decode time.** At 12 bits, Meshopt decoded the splat streams
in 17.24–75.90 ms and Draco sequential took 269.58–1,194.98 ms. On synthetic
clouds, the respective ranges were 1.34–3.40 ms and 27.39–52.52 ms. These are
three-run medians for decoding the streams without glTF loading.

Both codecs share the same error from the input quantizer. At 12 bits,
rotation RMSE was 0.0359–0.0524°, activated scale RMSE was
0.000027–0.000098, and activated opacity RMSE was 0.000186–0.000236. The
worst SH scalar RMSE was 0.000641–0.001165. Sampled RGB error over eight
directions was 0.000160–0.000273 RMSE.

**Splat GLB size and quality.** Both outputs start from the same PLY.
Meshopt encodes it directly, and Draco sequential reads a `gltfpack -gsd 3
-noq` GLB. Both restore all decoded attribute values exactly in the lossless
profile. The q12 profile allocates 12 nominal bits to position, rotation,
scale and opacity, and 10/10/9/9 to SH degrees 0–3. Meshopt uses
`-vp 12 -vn 11 -vt 12 -vc 12 -gsd 3 -c -ce khr`.

Both complete GLBs use the same gzip best and Brotli quality 5, window 22
settings. At q12, Meshopt's Brotli files were 12.5–29.4% larger relative to
Draco. With lossless data, the Meshopt Brotli files were slightly smaller.
All sizes are in bytes.

| Scene | Profile | Meshopt GLB | Meshopt Brotli | Draco GLB | Draco Brotli |
| --- | --- | ---: | ---: | ---: | ---: |
| `train_7000` | lossless | 151,640,332 | 148,536,436 | 175,088,060 | 148,711,659 |
| `train_7000` | q12 | 49,210,620 | 42,092,076 | 36,670,084 | 36,670,212 |
| `train_30000` | lossless | 219,932,940 | 215,412,198 | 253,647,280 | 217,723,281 |
| `train_30000` | q12 | 73,416,520 | 64,698,582 | 50,033,436 | 50,014,074 |
| `playroom_30000` | lossless | 371,816,616 | 362,897,145 | 452,269,148 | 373,175,422 |
| `playroom_30000` | q12 | 125,806,580 | 109,429,687 | 97,280,608 | 97,280,933 |
| `drjohnson_30000` | lossless | 648,041,116 | 631,983,336 | 749,906,440 | 648,860,377 |
| `drjohnson_30000` | q12 | 217,901,524 | 188,927,534 | 161,286,336 | 160,745,652 |

The two quantizers use different grids and filters. At the same nominal q12
budget, Meshopt had lower position, rotation and SH errors on all four scenes.
Scale results varied and opacity errors were close. The pairs below are
Meshopt / Draco. RMSE measures deviation from the shared unquantized input.
SH-view RGB measures clamped color per splat at eight fixed directions.
The benchmark establishes neither a visibility threshold nor visual quality
at equal file size.

| Scene | Position RMSE | Rotation RMSE° | Scale RMSE | Opacity RMSE | SH scalar RMSE | SH-view RGB RMSE |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `train_7000` | 0.022111 / 0.028670 | 0.020453 / 0.027745 | 0.000100 / 0.000087 | 0.000070 / 0.000070 | 0.000482 / 0.000957 | 0.000476 / 0.000901 |
| `train_30000` | 0.022089 / 0.028742 | 0.020799 / 0.027960 | 0.002137 / 0.002473 | 0.000070 / 0.000070 | 0.000488 / 0.001418 | 0.000466 / 0.001308 |
| `playroom_30000` | 0.003382 / 0.004174 | 0.020614 / 0.027935 | 0.000245 / 0.000166 | 0.000070 / 0.000071 | 0.000469 / 0.000785 | 0.000504 / 0.000773 |
| `drjohnson_30000` | 0.003382 / 0.004692 | 0.020718 / 0.027965 | 0.000141 / 0.000095 | 0.000070 / 0.000071 | 0.000488 / 0.000897 | 0.000528 / 0.000929 |

The full GLB timer starts before the local file read and ends at
`PackedGeometry`. It includes Meshopt buffer expansion and filters during
import, and Draco stream decode during primitive reading.

Run `sweep-20261128-101850` completed four scenes at q8, q10, q12, q14, q16
and lossless. Each load time is the median of three local reads after one
warm-up. Both codecs used Brotli quality 5, window 22 for size measurements.

At q12, Draco's Brotli files were smaller on all four scenes. Draco also
loaded faster in `draco-gltf` by 54–163 ms at q12. Meshopt retained lower
position and directional RGB RMSE across those same q12 rows.

**Size under two numerical error ceilings.** For each scene, use Meshopt
q12's position and directional RGB RMSE as upper limits. Pick the smallest
measured Brotli file per codec meeting both limits. MB are decimal.

| Scene | Meshopt q12 MB / load ms | Draco q14 MB / load ms | Position RMSE ceiling | Directional RGB RMSE ceiling |
| --- | ---: | ---: | ---: | ---: |
| `train_7000` | 42.09 / 416.17 | 47.06 / 347.36 | 0.022111 | 0.000476 |
| `train_30000` | 64.70 / 527.75 | 64.96 / 508.25 | 0.022089 | 0.000466 |
| `playroom_30000` | 109.43 / 952.07 | 122.55 / 1,044.64 | 0.003382 | 0.000504 |
| `drjohnson_30000` | 188.93 / 1,551.59 | 205.89 / 1,519.86 | 0.003382 | 0.000528 |

Meshopt q12 was 0.4–10.7% smaller than Draco q14. The smaller Draco q12 files
exceeded both ceilings on all four scenes. Draco q14 reduced both errors
further than required. These are discrete
operating points under two ceilings. They leave q13, separate budgets for
individual attributes and rendered image quality open.

**Default splat settings.** The historical GLB size rows below use
`gltfpack -c` defaults and a Draco re-encode of that output, with 16-bit
position and 12 bits for other attributes. Use the shared-source sweep above to compare the codecs. The default `-cc` removes 3.3–5.6% of splats
by dropping points below the opacity threshold of `2 / 256`. The pinned
writer rejects `-cf` for splats.

| Scene | Meshopt `-c` GLB | Draco point GLB | Meshopt Brotli | Draco Brotli | `-cc` kept |
| --- | ---: | ---: | ---: | ---: | ---: |
| `train_7000` | 21.33 MB | 19.58 MB | 16.73 MB | 19.51 MB | 96.7% |
| `train_30000` | 36.57 MB | 36.47 MB | 30.37 MB | 36.29 MB | 96.0% |
| `playroom_30000` | 61.35 MB | 60.71 MB | 51.00 MB | 60.66 MB | 96.7% |
| `drjohnson_30000` | 103.97 MB | 102.50 MB | 85.95 MB | 101.98 MB | 94.4% |

**Triangle control.** The tracked `bun_zipper.ply` was converted to a
plain GLB with 34,834 vertices and 208,353 indices. Meshopt `-c` and `-cc`
gave 218,240 B and 196,656 B. Draco-only gave 95,420 B. With fallback,
Meshopt totaled 913,636 B including the external buffer, and Draco totaled
2,598,788 B. Package compression applies the same settings to each asset.
Local file-to-geometry loading took 2.49–2.50 ms with Meshopt and
4.06–4.70 ms with Draco using the same timer.

**Validation and limits.** Upstream Meshopt recovered the prepared codec
records byte for byte. `draco-gltf` reread both sets of GLBs and verified
counts and attributes. The C++ Draco 1.5.9 probe checked 6 streams in 18 GLBs
from the default run with matching point fingerprints. The shared-source
batch was checked through `draco-gltf`. Experimental upstream `gltfpack`
opened one q12 GLB but skipped the Gaussian attributes during readback.

The codec process peaked at 1.17–4.94 GiB across the four scenes. The
separate Meshopt bridge peaked at 1.53 GiB. The table records historical
per-process GLB working sets. Browser, WASM, SIMD and parallel performance
remain outside this comparison.

| Scene | Harness peak | `gltfpack -c` / `-cc` peak |
| --- | ---: | ---: |
| `train_7000` | 580 MiB | 582 MiB / 582 MiB |
| `train_30000` | 856 MiB | 841 MiB / 841 MiB |
| `playroom_30000` | 1,508 MiB | 1,496 MiB / 1,496 MiB |
| `drjohnson_30000` | 2,502 MiB | 2,477 MiB / 2,477 MiB |
| `bun_zipper` | 76 MiB | 8 MiB for each Meshopt mode |

Local run IDs are `20261128-023421` for the original matrix,
`20261128-030549` for memory, `20261128-032508` for codec decode and
`20261128-044357` for shared-source GLBs. The raw JSONL retains per-attribute
errors and individual measurements.
