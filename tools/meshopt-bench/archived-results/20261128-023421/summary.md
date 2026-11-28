# Draco and Meshoptimizer benchmark

Raw per-case measurements are in `raw-results.jsonl`; this file is the compact index.

## Codec curves: identical quantized point records

| Dataset | Points | Bits | Meshopt L2 B | Meshopt L3 B | Draco sequential B | Meshopt prep ms | Meshopt encode ms | Draco encode ms | Fidelity |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| drjohnson_30000 | 3177554 | 8 | 185943320 | 185943320 | 149592094 | 54.1981 | 523.031 | 1330.2143 | pos RMSE 0.07565 |
| drjohnson_30000 | 3177554 | 12 | 269814197 | 269814197 | 243062639 | 64.114 | 1393.01 | 1431.769 | pos RMSE 0.00469 |
| drjohnson_30000 | 3177554 | 16 | 374283550 | 374283550 | 338417341 | 59.6023 | 1068.85 | 1651.0832 | pos RMSE 0.00029 |
| playroom_30000 | 1916379 | 8 | 110419342 | 110419342 | 89477236 | 32.6439 | 339.577 | 809.5211 | pos RMSE 0.06699 |
| playroom_30000 | 1916379 | 12 | 161297539 | 161297539 | 144336639 | 38.2458 | 873.117 | 940.6055 | pos RMSE 0.00417 |
| playroom_30000 | 1916379 | 16 | 222663797 | 222663797 | 201018599 | 35.1527 | 693.102 | 1035.475 | pos RMSE 0.00026 |
| train_30000 | 1074761 | 8 | 61081827 | 61081445 | 48535505 | 16.0648 | 191.487 | 436.6713 | pos RMSE 0.44935 |
| train_30000 | 1074761 | 12 | 88507496 | 88507496 | 80189945 | 21.8857 | 458.543 | 484.5549 | pos RMSE 0.02873 |
| train_30000 | 1074761 | 16 | 124769062 | 124768607 | 113690711 | 19.9606 | 381.276 | 628.4193 | pos RMSE 0.00179 |
| train_7000 | 741883 | 8 | 42570700 | 42570700 | 34911032 | 11.3419 | 118.212 | 301.18460000000005 | pos RMSE 0.47125 |
| train_7000 | 741883 | 12 | 62599111 | 62594568 | 56807946 | 13.8514 | 317.281 | 342.474 | pos RMSE 0.02866 |
| train_7000 | 741883 | 16 | 86534545 | 86534545 | 80808579 | 13.2363 | 247.488 | 430.0891 | pos RMSE 0.00179 |
| xyz | 1000000 | 8 | 3003932 | 3003932 | 3000300 | 14.509 | 7.0014 | 15.7182 | pos RMSE 0.19620 |
| xyz | 1000000 | 12 | 5390503 | 4742295 | 4506316 | 12.8952 | 19.8375 | 17.401300000000003 | pos RMSE 0.01221 |
| xyz | 1000000 | 16 | 6007839 | 6007839 | 6024271 | 13.1236 | 13.8732 | 19.8881 | pos RMSE 0.00076 |
| xyz_normal_rgb | 1000000 | 8 | 2913648 | 2913648 | 2326005 | 16.1104 | 32.1165 | 46.9173 | pos RMSE 0.01990 |
| xyz_normal_rgb | 1000000 | 12 | 5543260 | 5543260 | 3807640 | 13.2902 | 53.5544 | 49.4992 | pos RMSE 0.00123 |
| xyz_normal_rgb | 1000000 | 16 | 8574190 | 8574190 | 5271169 | 14.9199 | 50.287 | 56.4345 | pos RMSE 0.00008 |
| xyz_rgb | 1000000 | 8 | 2838416 | 2838416 | 2251186 | 13.8916 | 24.1415 | 34.4165 | pos RMSE 0.01990 |
| xyz_rgb | 1000000 | 12 | 5403880 | 5403880 | 3727306 | 13.094 | 35.639 | 35.087399999999995 | pos RMSE 0.00123 |
| xyz_rgb | 1000000 | 16 | 8426906 | 8426906 | 5185409 | 15.2882 | 34.2418 | 38.8617 | pos RMSE 0.00008 |

## Draco point-cloud coding choices

| Dataset | Points | Mode | Bits | Bytes | Encode ms | Decode ms | Exact point set |
| --- | ---: | --- | ---: | ---: | ---: | ---: | --- |
| train_7000 | 741883 | sequential_input_order | 12 | 56807946 | 383.40 | 287.50 | true |
| train_7000 | 741883 | kd_tree | 12 | 63300159 | 563.22 | 547.71 | true |
| train_7000 | 741883 | point_order_search | 12 | 46862947 | 1220.05 | 263.13 | true |
| train_30000 | 1074761 | sequential_input_order | 12 | 80189945 | 564.58 | 413.02 | true |
| train_30000 | 1074761 | kd_tree | 12 | 90348929 | 849.65 | 796.53 | true |
| train_30000 | 1074761 | point_order_search | 12 | 67940570 | 1743.96 | 377.64 | true |
| playroom_30000 | 1916379 | sequential_input_order | 12 | 144336639 | 1018.20 | 750.82 | true |
| playroom_30000 | 1916379 | kd_tree | 12 | 167770703 | 1382.42 | 1288.22 | true |
| playroom_30000 | 1916379 | point_order_search | 12 | 122156244 | 2948.60 | 674.13 | true |
| drjohnson_30000 | 3177554 | sequential_input_order | 12 | 243062639 | 1501.32 | 1215.50 | true |
| drjohnson_30000 | 3177554 | kd_tree | 12 | 271570925 | 2439.92 | 2268.48 | true |
| drjohnson_30000 | 3177554 | point_order_search | 12 | 210816569 | 4936.51 | 1095.59 | true |
| xyz | 1000000 | sequential_input_order | 12 | 4506316 | 17.62 | 28.02 | true |
| xyz | 1000000 | kd_tree | 12 | 2254956 | 123.19 | 31.07 | true |
| xyz | 1000000 | point_order_search | 12 | 2562134 | 143.66 | 18.87 | true |
| xyz_rgb | 1000000 | sequential_input_order | 12 | 3727306 | 35.20 | 29.38 | true |
| xyz_rgb | 1000000 | kd_tree | 12 | 4710145 | 131.63 | 47.30 | true |
| xyz_rgb | 1000000 | point_order_search | 12 | 3362287 | 328.56 | 29.81 | true |
| xyz_normal_rgb | 1000000 | sequential_input_order | 12 | 3807640 | 53.10 | 50.71 | true |
| xyz_normal_rgb | 1000000 | kd_tree | 12 | 7035326 | 162.28 | 65.99 | true |
| xyz_normal_rgb | 1000000 | point_order_search | 12 | 3454707 | 407.94 | 50.08 | true |

## Gaussian splat glTF integration

| Dataset | Codec | Mode | Package B | gzip B | Brotli B | Points | Time ms |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| train_7000 | Meshopt | c | 21327144 | 17539533 | 16732102 | 741883 | 904.5304 |
| train_7000 | Meshopt | cc | 12351744 | 9581113 | 9240392 | 717324 | 598.0189 |
| train_7000 | draco-gltf | point cloud (experimental) | 19579344 | 19518848 | 19509041 | 741883 | 823.0698 |
| train_30000 | Meshopt | c | 36567048 | 31520979 | 30371315 | 1074761 | 1284.2542 |
| train_30000 | Meshopt | cc | 29192108 | 19991154 | 18783127 | 1032012 | 909.6848 |
| train_30000 | draco-gltf | point cloud (experimental) | 36465872 | 36333791 | 36294290 | 1074761 | 1201.418 |
| playroom_30000 | Meshopt | c | 61348852 | 52692988 | 50997026 | 1916379 | 2329.3564 |
| playroom_30000 | Meshopt | cc | 43368152 | 31359601 | 29935531 | 1852807 | 1653.1129 |
| playroom_30000 | draco-gltf | point cloud (experimental) | 60709620 | 60581993 | 60664648 | 1916379 | 2114.4218 |
| drjohnson_30000 | Meshopt | c | 103971304 | 88851518 | 85951083 | 3177554 | 3838.9979 |
| drjohnson_30000 | Meshopt | cc | 72861688 | 51572850 | 48992611 | 3001178 | 2736.6849 |
| drjohnson_30000 | draco-gltf | point cloud (experimental) | 102497820 | 102005761 | 101981027 | 3177554 | 3470.3472 |

## Triangle mesh glTF integration

| Variant | Codec | Package B | gzip B | Brotli B | Vertices | Indices |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| gltfpack c | Meshopt | 218240 | 175955 | 163631 | 34834 | 208353 |
| gltfpack cc | Meshopt | 196656 | 137121 | 127292 | 34834 | 208353 |
| gltfpack cf | Meshopt | 913636 | 615142 | 482027 | 34834 | 208353 |
| draco-gltf draco_only | Draco | 95420 | 89580 | 86763 | 34834 | 208353 |
| draco-gltf fallback | Draco | 2598788 | 1375995 | 1148846 | 34834 | 208353 |

Gaussian point-cloud bytes in KHR_draco_mesh_compression are experimental and outside the extension's standard TRIANGLES/TRIANGLE_STRIP mode table.
