# Draco and Meshoptimizer benchmark

Raw per-case measurements are in `raw-results.jsonl`; this file is the compact index.

## Codec curves: identical quantized point records

| Dataset | Points | Bits | Meshopt L2 B | Meshopt L3 B | Draco sequential B | Meshopt prep ms | Meshopt encode ms | Draco encode ms | Fidelity |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| drjohnson_30000 | 3177554 | 8 | 185943320 | 185943320 | 149592094 | 54.503 | 500.434 | 1310.8515 | pos RMSE 0.07565 |
| drjohnson_30000 | 3177554 | 12 | 269814197 | 269814197 | 243062639 | 58.6776 | 1375.34 | 1429.2174 | pos RMSE 0.00469 |
| drjohnson_30000 | 3177554 | 16 | 374283550 | 374283550 | 338417341 | 58.3725 | 1060.96 | 1656.5684 | pos RMSE 0.00029 |
| playroom_30000 | 1916379 | 8 | 110419342 | 110419342 | 89477236 | 30.5156 | 329.05 | 806.5704000000001 | pos RMSE 0.06699 |
| playroom_30000 | 1916379 | 12 | 161297539 | 161297539 | 144336639 | 35.1204 | 863.448 | 876.9808999999999 | pos RMSE 0.00417 |
| playroom_30000 | 1916379 | 16 | 222663797 | 222663797 | 201018599 | 39.3215 | 685.745 | 1036.4975000000002 | pos RMSE 0.00026 |
| train_30000 | 1074761 | 8 | 61081827 | 61081445 | 48535505 | 16.417 | 192.718 | 436.4148 | pos RMSE 0.44935 |
| train_30000 | 1074761 | 12 | 88507496 | 88507496 | 80189945 | 22.5338 | 462.233 | 485.2119 | pos RMSE 0.02873 |
| train_30000 | 1074761 | 16 | 124769062 | 124768607 | 113690711 | 21.8561 | 386.23 | 605.1159 | pos RMSE 0.00179 |
| train_7000 | 741883 | 8 | 42570700 | 42570700 | 34911032 | 11.695 | 117.287 | 299.16330000000005 | pos RMSE 0.47125 |
| train_7000 | 741883 | 12 | 62599111 | 62594568 | 56807946 | 18.9995 | 318.708 | 343.44809999999995 | pos RMSE 0.02866 |
| train_7000 | 741883 | 16 | 86534545 | 86534545 | 80808579 | 15.3308 | 248.763 | 426.0893 | pos RMSE 0.00179 |
| xyz | 1000000 | 8 | 3003932 | 3003932 | 3000300 | 16.1902 | 6.677 | 16.5658 | pos RMSE 0.19620 |
| xyz | 1000000 | 12 | 5390503 | 4742295 | 4506316 | 13.536 | 20.1222 | 17.659100000000002 | pos RMSE 0.01221 |
| xyz | 1000000 | 16 | 6007839 | 6007839 | 6024271 | 12.6589 | 13.849 | 20.7466 | pos RMSE 0.00076 |
| xyz_normal_rgb | 1000000 | 8 | 2913648 | 2913648 | 2326005 | 13.702 | 32.7688 | 47.3995 | pos RMSE 0.01990 |
| xyz_normal_rgb | 1000000 | 12 | 5543260 | 5543260 | 3807640 | 13.2913 | 52.7253 | 49.3556 | pos RMSE 0.00123 |
| xyz_normal_rgb | 1000000 | 16 | 8574190 | 8574190 | 5271169 | 14.3613 | 49.6034 | 54.717099999999995 | pos RMSE 0.00008 |
| xyz_rgb | 1000000 | 8 | 2838416 | 2838416 | 2251186 | 14.0579 | 24.0063 | 33.848499999999994 | pos RMSE 0.01990 |
| xyz_rgb | 1000000 | 12 | 5403880 | 5403880 | 3727306 | 13.5819 | 35.3906 | 35.826499999999996 | pos RMSE 0.00123 |
| xyz_rgb | 1000000 | 16 | 8426906 | 8426906 | 5185409 | 14.0192 | 34.657 | 39.546699999999994 | pos RMSE 0.00008 |

## Draco point-cloud coding choices

| Dataset | Points | Mode | Bits | Bytes | Encode ms | Decode ms | Exact point set |
| --- | ---: | --- | ---: | ---: | ---: | ---: | --- |
| train_7000 | 741883 | sequential_input_order | 12 | 56807946 | 337.31 | 265.67 | true |
| train_7000 | 741883 | kd_tree | 12 | 63300159 | 496.54 | 484.89 | true |
| train_7000 | 741883 | point_order_search | 12 | 46862947 | 1215.07 | 268.38 | true |
| train_30000 | 1074761 | sequential_input_order | 12 | 80189945 | 491.07 | 390.69 | true |
| train_30000 | 1074761 | kd_tree | 12 | 90348929 | 765.01 | 711.32 | true |
| train_30000 | 1074761 | point_order_search | 12 | 67940570 | 1753.31 | 377.14 | true |
| playroom_30000 | 1916379 | sequential_input_order | 12 | 144336639 | 875.09 | 755.90 | true |
| playroom_30000 | 1916379 | kd_tree | 12 | 167770703 | 1249.69 | 1192.46 | true |
| playroom_30000 | 1916379 | point_order_search | 12 | 122156244 | 2969.40 | 668.67 | true |
| drjohnson_30000 | 3177554 | sequential_input_order | 12 | 243062639 | 1531.73 | 1217.18 | true |
| drjohnson_30000 | 3177554 | kd_tree | 12 | 271570925 | 2457.33 | 2267.01 | true |
| drjohnson_30000 | 3177554 | point_order_search | 12 | 210816569 | 4990.53 | 1119.61 | true |
| xyz | 1000000 | sequential_input_order | 12 | 4506316 | 17.48 | 26.99 | true |
| xyz | 1000000 | kd_tree | 12 | 2254956 | 123.72 | 31.14 | true |
| xyz | 1000000 | point_order_search | 12 | 2562134 | 137.63 | 19.77 | true |
| xyz_rgb | 1000000 | sequential_input_order | 12 | 3727306 | 34.34 | 32.07 | true |
| xyz_rgb | 1000000 | kd_tree | 12 | 4710145 | 128.42 | 48.90 | true |
| xyz_rgb | 1000000 | point_order_search | 12 | 3362287 | 325.67 | 30.98 | true |
| xyz_normal_rgb | 1000000 | sequential_input_order | 12 | 3807640 | 49.26 | 51.17 | true |
| xyz_normal_rgb | 1000000 | kd_tree | 12 | 7035326 | 162.05 | 68.91 | true |
| xyz_normal_rgb | 1000000 | point_order_search | 12 | 3454707 | 410.42 | 49.43 | true |

## Gaussian splat glTF integration

| Dataset | Codec | Mode | Package B | gzip B | Brotli B | Points | Time ms |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |

## Triangle mesh glTF integration

| Variant | Codec | Package B | gzip B | Brotli B | Vertices | Indices |
| --- | --- | ---: | ---: | ---: | ---: | ---: |

Gaussian point-cloud bytes in KHR_draco_mesh_compression are experimental and outside the extension's standard TRIANGLES/TRIANGLE_STRIP mode table.
