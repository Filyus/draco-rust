# Draco and Meshoptimizer benchmark

Raw per-case measurements are in `raw-results.jsonl`; this file is the compact index.

## Codec curves: identical quantized point records

| Dataset | Points | Bits | Meshopt L2 B | Meshopt L3 B | Draco sequential B | Meshopt prep ms | Meshopt encode ms | Draco encode ms | Fidelity |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |

## Draco point-cloud coding choices

| Dataset | Points | Mode | Bits | Bytes | Encode ms | Decode ms | Exact point set |
| --- | ---: | --- | ---: | ---: | ---: | ---: | --- |

## Gaussian splat GLB: size, error and loading time

Both codecs start from the same PLY. Compare size at similar measured error across budgets. Equal nominal bits do not imply equal precision. Directional RGB measures sampled per-splat error without rasterization.

GLB→geometry includes reading the local file, Meshopt buffer decoding and filters during import, and Draco decoding during primitive reads. The stage timings and per-attribute measurements remain in JSONL. External gzip and Brotli apply to size only.

| Dataset | Codec | Profile | GLB B | gzip B | Brotli B | GLB→geometry ms | Position RMSE | Rotation RMSE° | Scale RMSE | Opacity RMSE | SH RMSE | Directional RGB RMSE |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| train_7000 | Meshopt | lossless | 151640332 | 148117918 | 148536436 | 465.45 | 0.000000 | 0.000001 | 0.000000 | 0.000000 | 0.000000 | 0.000000 |
| train_7000 | Draco | lossless | 175088060 | 148804235 | 148711659 | 160.63 | 0.000000 | 0.000001 | 0.000000 | 0.000000 | 0.000000 | 0.000000 |
| train_7000 | Meshopt | q8_matched_bits | 18865772 | 15105238 | 14925942 | 342.56 | 0.348642 | 0.328258 | 0.001649 | 0.001133 | 0.007701 | 0.007603 |
| train_7000 | Draco | q8_matched_bits | 16593224 | 16374621 | 16396272 | 288.66 | 0.455956 | 0.445847 | 0.001452 | 0.001128 | 0.015992 | 0.015151 |
| train_7000 | Meshopt | q10_matched_bits | 30011104 | 26203361 | 26214888 | 366.82 | 0.088454 | 0.081609 | 0.000400 | 0.000282 | 0.001927 | 0.001903 |
| train_7000 | Draco | q10_matched_bits | 26301912 | 26228556 | 26290224 | 292.99 | 0.116541 | 0.111128 | 0.000348 | 0.000281 | 0.003890 | 0.003670 |
| train_7000 | Meshopt | q12_matched_bits | 49210620 | 42225816 | 42092076 | 416.17 | 0.022111 | 0.020453 | 0.000100 | 0.000070 | 0.000482 | 0.000476 |
| train_7000 | Draco | q12_matched_bits | 36670084 | 36608070 | 36670212 | 321.17 | 0.028670 | 0.027745 | 0.000087 | 0.000070 | 0.000957 | 0.000899 |
| train_7000 | Meshopt | q14_matched_bits | 54324048 | 50011090 | 49908259 | 368.07 | 0.005525 | 0.005386 | 0.000025 | 0.000018 | 0.000120 | 0.000119 |
| train_7000 | Draco | q14_matched_bits | 47068836 | 46984518 | 47059810 | 347.36 | 0.007163 | 0.006939 | 0.000022 | 0.000018 | 0.000239 | 0.000225 |
| train_7000 | Meshopt | q16_matched_bits | 60212764 | 58045002 | 58300847 | 373.77 | 0.001381 | 0.002163 | 0.000006 | 0.000004 | 0.000030 | 0.000030 |
| train_7000 | Draco | q16_matched_bits | 57719884 | 57603541 | 57668727 | 320.38 | 0.001791 | 0.001735 | 0.000005 | 0.000004 | 0.000060 | 0.000056 |
| train_30000 | Meshopt | lossless | 219932940 | 214911555 | 215412198 | 653.60 | 0.000000 | 0.000001 | 0.000000 | 0.000000 | 0.000000 | 0.000000 |
| train_30000 | Draco | lossless | 253647280 | 218031107 | 217723281 | 229.35 | 0.000000 | 0.000001 | 0.000000 | 0.000000 | 0.000000 | 0.000000 |
| train_30000 | Meshopt | q8_matched_bits | 33509496 | 28597584 | 28482831 | 498.03 | 0.346321 | 0.334127 | 0.017384 | 0.001108 | 0.007808 | 0.007452 |
| train_30000 | Draco | q8_matched_bits | 21820888 | 21617032 | 21659346 | 428.50 | 0.466184 | 0.449042 | 0.021639 | 0.001154 | 0.023338 | 0.022247 |
| train_30000 | Meshopt | q10_matched_bits | 53133300 | 46828866 | 46596186 | 560.88 | 0.088138 | 0.083069 | 0.006505 | 0.000278 | 0.001953 | 0.001865 |
| train_30000 | Draco | q10_matched_bits | 35289552 | 35126009 | 35243997 | 426.08 | 0.113915 | 0.111941 | 0.008129 | 0.000278 | 0.005656 | 0.005285 |
| train_30000 | Meshopt | q12_matched_bits | 73416520 | 64964192 | 64698582 | 527.75 | 0.022089 | 0.020799 | 0.002137 | 0.000070 | 0.000488 | 0.000466 |
| train_30000 | Draco | q12_matched_bits | 50033436 | 49921930 | 50014074 | 462.66 | 0.028742 | 0.027960 | 0.002473 | 0.000070 | 0.001418 | 0.001308 |
| train_30000 | Meshopt | q14_matched_bits | 80053656 | 76220503 | 76477895 | 535.12 | 0.005527 | 0.005467 | 0.000631 | 0.000018 | 0.000122 | 0.000117 |
| train_30000 | Draco | q14_matched_bits | 64961896 | 64842213 | 64962120 | 508.25 | 0.007158 | 0.006988 | 0.000689 | 0.000018 | 0.000354 | 0.000327 |
| train_30000 | Meshopt | q16_matched_bits | 92280948 | 89689796 | 89895995 | 544.49 | 0.001381 | 0.002175 | 0.000165 | 0.000004 | 0.000031 | 0.000029 |
| train_30000 | Draco | q16_matched_bits | 80191432 | 80005455 | 80116525 | 454.02 | 0.001790 | 0.001748 | 0.000178 | 0.000004 | 0.000089 | 0.000082 |
| playroom_30000 | Meshopt | lossless | 371816616 | 362706322 | 362897145 | 1121.32 | 0.000000 | 0.000001 | 0.000000 | 0.000000 | 0.000000 | 0.000000 |
| playroom_30000 | Draco | lossless | 452269148 | 377645320 | 373175422 | 421.19 | 0.000000 | 0.000001 | 0.000000 | 0.000000 | 0.000000 | 0.000000 |
| playroom_30000 | Meshopt | q8_matched_bits | 54158256 | 45496146 | 44865667 | 892.39 | 0.054206 | 0.331130 | 0.003544 | 0.001117 | 0.007495 | 0.008056 |
| playroom_30000 | Draco | q8_matched_bits | 43942612 | 43808240 | 43912617 | 789.07 | 0.067092 | 0.448464 | 0.002528 | 0.001121 | 0.012606 | 0.012581 |
| playroom_30000 | Meshopt | q10_matched_bits | 83671040 | 75179528 | 74847689 | 984.90 | 0.013536 | 0.082278 | 0.000961 | 0.000280 | 0.001875 | 0.002017 |
| playroom_30000 | Draco | q10_matched_bits | 70422160 | 70231024 | 70422392 | 807.46 | 0.016712 | 0.111790 | 0.000657 | 0.000280 | 0.003156 | 0.003145 |
| playroom_30000 | Meshopt | q12_matched_bits | 125806580 | 109963834 | 109429687 | 952.07 | 0.003382 | 0.020614 | 0.000245 | 0.000070 | 0.000469 | 0.000504 |
| playroom_30000 | Draco | q12_matched_bits | 97280608 | 97032177 | 97280933 | 898.07 | 0.004174 | 0.027935 | 0.000166 | 0.000071 | 0.000785 | 0.000774 |
| playroom_30000 | Meshopt | q14_matched_bits | 134865228 | 128843129 | 128795393 | 945.25 | 0.000846 | 0.005421 | 0.000061 | 0.000018 | 0.000117 | 0.000126 |
| playroom_30000 | Draco | q14_matched_bits | 122547140 | 122272105 | 122547552 | 1044.64 | 0.001043 | 0.006978 | 0.000041 | 0.000018 | 0.000194 | 0.000193 |
| playroom_30000 | Meshopt | q16_matched_bits | 155241012 | 151062007 | 150916310 | 982.37 | 0.000211 | 0.002169 | 0.000015 | 0.000004 | 0.000029 | 0.000032 |
| playroom_30000 | Draco | q16_matched_bits | 148022020 | 147626567 | 147934228 | 1569.26 | 0.000261 | 0.001745 | 0.000010 | 0.000004 | 0.000048 | 0.000048 |
| drjohnson_30000 | Meshopt | lossless | 648041116 | 631796234 | 631983336 | 2097.55 | 0.000000 | 0.000001 | 0.000000 | 0.000000 | 0.000000 | 0.000000 |
| drjohnson_30000 | Draco | lossless | 749906440 | 649994623 | 648860377 | 706.06 | 0.000000 | 0.000001 | 0.000000 | 0.000000 | 0.000000 | 0.000000 |
| drjohnson_30000 | Meshopt | q8_matched_bits | 92496696 | 77296267 | 76466956 | 1482.92 | 0.053817 | 0.332746 | 0.002144 | 0.001108 | 0.007799 | 0.008431 |
| drjohnson_30000 | Draco | q8_matched_bits | 73317624 | 72509786 | 72632284 | 1269.63 | 0.075676 | 0.449048 | 0.001510 | 0.001163 | 0.014565 | 0.015197 |
| drjohnson_30000 | Meshopt | q10_matched_bits | 144510024 | 129399865 | 128891951 | 1628.10 | 0.013515 | 0.082707 | 0.000575 | 0.000278 | 0.001953 | 0.002112 |
| drjohnson_30000 | Draco | q10_matched_bits | 116288532 | 115500734 | 115679545 | 1262.38 | 0.018781 | 0.111947 | 0.000382 | 0.000277 | 0.003581 | 0.003731 |
| drjohnson_30000 | Meshopt | q12_matched_bits | 217901524 | 189619546 | 188927534 | 1551.59 | 0.003382 | 0.020718 | 0.000141 | 0.000070 | 0.000488 | 0.000528 |
| drjohnson_30000 | Draco | q12_matched_bits | 161286336 | 160406484 | 160745652 | 1388.56 | 0.004692 | 0.027965 | 0.000095 | 0.000071 | 0.000897 | 0.000928 |
| drjohnson_30000 | Meshopt | q14_matched_bits | 233200276 | 222496274 | 222663689 | 1532.89 | 0.000846 | 0.005448 | 0.000035 | 0.000018 | 0.000122 | 0.000132 |
| drjohnson_30000 | Draco | q14_matched_bits | 206728700 | 205641147 | 205889631 | 1519.86 | 0.001173 | 0.006989 | 0.000024 | 0.000018 | 0.000224 | 0.000232 |
| drjohnson_30000 | Meshopt | q16_matched_bits | 268201296 | 260789707 | 260817899 | 1559.65 | 0.000211 | 0.002174 | 0.000009 | 0.000004 | 0.000031 | 0.000033 |
| drjohnson_30000 | Draco | q16_matched_bits | 252399764 | 251019615 | 251248244 | 1614.87 | 0.000293 | 0.001747 | 0.000006 | 0.000004 | 0.000056 | 0.000058 |

## Triangle mesh glTF integration

| Variant | Codec | Package B | gzip B | Brotli B | Vertices | Indices |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| gltfpack c | Meshopt | 218240 | 175955 | 173622 | 34834 | 208353 |
| gltfpack cc | Meshopt | 196656 | 137121 | 137537 | 34834 | 208353 |
| gltfpack cf | Meshopt | 913636 | 615142 | 634791 | 34834 | 208353 |
| draco-gltf draco_only | Draco | 95420 | 89580 | 87674 | 34834 | 208353 |
| draco-gltf fallback | Draco | 2598788 | 1375995 | 1241164 | 34834 | 208353 |

Gaussian point-cloud bytes in KHR_draco_mesh_compression are experimental and outside the extension's standard TRIANGLES/TRIANGLE_STRIP mode table.
