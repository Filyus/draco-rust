# Draco and Meshoptimizer benchmark

Raw per-case measurements are in `raw-results.jsonl`; this file is the compact index.

## Codec curves: identical quantized point records

| Dataset | Points | Bits | Meshopt L2 B | Meshopt L3 B | Draco sequential B | Meshopt prep ms | Meshopt encode ms | Draco encode ms | Fidelity |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |

## Draco point-cloud coding choices

| Dataset | Points | Mode | Bits | Bytes | Encode ms | Decode ms | Exact point set |
| --- | ---: | --- | ---: | ---: | ---: | ---: | --- |

## Gaussian splat glTF: shared source and matched quantization budget

Both outputs start from the same PLY. The lossless row uses no quantization. The q12 row matches nominal bits per semantic; the quantization grids and filters differ, so the quality table reports measured error. Each complete GLB was compressed with identical gzip and Brotli settings.

| Dataset | Profile | Meshopt GLB B | Meshopt gzip B | Meshopt Brotli B | Draco GLB B | Draco gzip B | Draco Brotli B |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| train_7000 | lossless | 151640332 | 148117918 | 148536436 | 175088060 | 148804235 | 148711659 |
| train_7000 | q12_matched_bits | 49210620 | 42225816 | 42092076 | 36670084 | 36608070 | 36670212 |
| train_30000 | lossless | 219932940 | 214911555 | 215412198 | 253647280 | 218031107 | 217723281 |
| train_30000 | q12_matched_bits | 73416520 | 64964192 | 64698582 | 50033436 | 49921930 | 50014074 |
| playroom_30000 | lossless | 371816616 | 362706322 | 362897145 | 452269148 | 377645320 | 373175422 |
| playroom_30000 | q12_matched_bits | 125806580 | 109963834 | 109429687 | 97280608 | 97032177 | 97280933 |
| drjohnson_30000 | lossless | 648041116 | 631796234 | 631983336 | 749906440 | 649994623 | 648860377 |
| drjohnson_30000 | q12_matched_bits | 217901524 | 189619546 | 188927534 | 161286336 | 160406484 | 160745652 |

Decode timings are absolute medians in milliseconds over three fresh imports. `import_file` eagerly expands KHR_meshopt_compression bufferViews, while Draco payload decoding occurs in `read_primitives`; the stage columns expose that boundary and are not directly comparable codec-only timings. Compare the full GLB→geometry column for end-to-end latency.

| Dataset | Profile | Meshopt import + KHR buffer decode ms | Meshopt primitive read ms | Meshopt full GLB→geometry ms | Draco import/parse ms | Draco decode + primitive read ms | Draco full GLB→geometry ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| train_7000 | lossless | 308.77 | 116.08 | 423.82 | 51.05 | 96.86 | 148.47 |
| train_7000 | q12_matched_bits | 288.93 | 114.80 | 403.55 | 11.57 | 312.93 | 324.51 |
| train_30000 | lossless | 476.29 | 169.81 | 644.92 | 82.96 | 153.33 | 236.28 |
| train_30000 | q12_matched_bits | 360.83 | 166.10 | 526.89 | 16.07 | 448.67 | 464.97 |
| playroom_30000 | lossless | 873.28 | 302.42 | 1175.70 | 142.49 | 267.27 | 409.76 |
| playroom_30000 | q12_matched_bits | 654.52 | 299.67 | 954.19 | 32.03 | 861.14 | 893.18 |
| drjohnson_30000 | lossless | 1412.98 | 504.11 | 1914.37 | 235.68 | 430.10 | 670.67 |
| drjohnson_30000 | q12_matched_bits | 1077.04 | 493.12 | 1566.52 | 51.92 | 1351.23 | 1403.15 |

Quality against the unquantized base GLB; pairs are `Meshopt / Draco`. RMSE fields are absolute attribute-space errors.

| Dataset | Profile | POSITION Euclidean RMSE | ROTATION angular RMSE° | SCALE accessor RMSE | OPACITY accessor RMSE | SH coefficient scalar RMSE | SH-view RGB RMSE |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| train_7000 | lossless | 0.000000 / 0.000000 | 0.000001 / 0.000001 | 0.000000 / 0.000000 | 0.000000 / 0.000000 | 0.000000 / 0.000000 | 0.000000 / 0.000000 |
| train_7000 | q12_matched_bits | 0.022111 / 0.028670 | 0.020453 / 0.027745 | 0.000100 / 0.000087 | 0.000070 / 0.000070 | 0.000482 / 0.000957 | 0.000476 / 0.000901 |
| train_30000 | lossless | 0.000000 / 0.000000 | 0.000001 / 0.000001 | 0.000000 / 0.000000 | 0.000000 / 0.000000 | 0.000000 / 0.000000 | 0.000000 / 0.000000 |
| train_30000 | q12_matched_bits | 0.022089 / 0.028742 | 0.020799 / 0.027960 | 0.002137 / 0.002473 | 0.000070 / 0.000070 | 0.000488 / 0.001418 | 0.000466 / 0.001308 |
| playroom_30000 | lossless | 0.000000 / 0.000000 | 0.000001 / 0.000001 | 0.000000 / 0.000000 | 0.000000 / 0.000000 | 0.000000 / 0.000000 | 0.000000 / 0.000000 |
| playroom_30000 | q12_matched_bits | 0.003382 / 0.004174 | 0.020614 / 0.027935 | 0.000245 / 0.000166 | 0.000070 / 0.000071 | 0.000469 / 0.000785 | 0.000504 / 0.000773 |
| drjohnson_30000 | lossless | 0.000000 / 0.000000 | 0.000001 / 0.000001 | 0.000000 / 0.000000 | 0.000000 / 0.000000 | 0.000000 / 0.000000 | 0.000000 / 0.000000 |
| drjohnson_30000 | q12_matched_bits | 0.003382 / 0.004692 | 0.020718 / 0.027965 | 0.000141 / 0.000095 | 0.000070 / 0.000071 | 0.000488 / 0.000897 | 0.000528 / 0.000929 |

The Draco `POINTS` stream remains outside the primitive-mode table in `KHR_draco_mesh_compression`; this is an experimental container path.

## Triangle mesh glTF integration

| Variant | Codec | Package B | gzip B | Brotli B | Vertices | Indices |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| gltfpack c | Meshopt | 218240 | 175955 | 173622 | 34834 | 208353 |
| gltfpack cc | Meshopt | 196656 | 137121 | 137537 | 34834 | 208353 |
| gltfpack cf | Meshopt | 913636 | 615142 | 634791 | 34834 | 208353 |
| draco-gltf draco_only | Draco | 95420 | 89580 | 87674 | 34834 | 208353 |
| draco-gltf fallback | Draco | 2598788 | 1375995 | 1241164 | 34834 | 208353 |

Gaussian point-cloud bytes in KHR_draco_mesh_compression are experimental and outside the extension's standard TRIANGLES/TRIANGLE_STRIP mode table.
