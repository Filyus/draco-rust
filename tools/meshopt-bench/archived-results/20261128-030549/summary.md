# Draco and Meshoptimizer benchmark

Raw per-case measurements are in `raw-results.jsonl`; this file is the compact index.

## Codec curves: identical quantized point records

| Dataset | Points | Bits | Meshopt L2 B | Meshopt L3 B | Draco sequential B | Meshopt prep ms | Meshopt encode ms | Draco encode ms | Fidelity |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |

## Draco point-cloud coding choices

| Dataset | Points | Mode | Bits | Bytes | Encode ms | Decode ms | Exact point set |
| --- | ---: | --- | ---: | ---: | ---: | ---: | --- |

## Gaussian splat glTF integration

| Dataset | Codec | Mode | Package B | gzip B | Brotli B | Points | Time ms |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| train_7000 | Meshopt | c | 21327144 | 17539533 | 16732102 | 741883 | 897.4819000000001 |
| train_7000 | Meshopt | cc | 12351744 | 9581113 | 9240392 | 717324 | 589.4972 |
| train_7000 | draco-gltf | point cloud (experimental) | 19579344 | 19518848 | 19509041 | 741883 | 816.7309 |
| train_30000 | Meshopt | c | 36567048 | 31520979 | 30371315 | 1074761 | 1326.6165 |
| train_30000 | Meshopt | cc | 29192108 | 19991154 | 18783127 | 1032012 | 908.0361 |
| train_30000 | draco-gltf | point cloud (experimental) | 36465872 | 36333791 | 36294290 | 1074761 | 1198.9243 |
| playroom_30000 | Meshopt | c | 61348852 | 52692988 | 50997026 | 1916379 | 2309.3123 |
| playroom_30000 | Meshopt | cc | 43368152 | 31359601 | 29935531 | 1852807 | 1658.0397 |
| playroom_30000 | draco-gltf | point cloud (experimental) | 60709620 | 60581993 | 60664648 | 1916379 | 2135.0432 |
| drjohnson_30000 | Meshopt | c | 103971304 | 88851518 | 85951083 | 3177554 | 3822.7462 |
| drjohnson_30000 | Meshopt | cc | 72861688 | 51572850 | 48992611 | 3001178 | 2688.4454 |
| drjohnson_30000 | draco-gltf | point cloud (experimental) | 102497820 | 102005761 | 101981027 | 3177554 | 3486.5143000000003 |

## Triangle mesh glTF integration

| Variant | Codec | Package B | gzip B | Brotli B | Vertices | Indices |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| gltfpack c | Meshopt | 218240 | 175955 | 163631 | 34834 | 208353 |
| gltfpack cc | Meshopt | 196656 | 137121 | 127292 | 34834 | 208353 |
| gltfpack cf | Meshopt | 913636 | 615142 | 482027 | 34834 | 208353 |
| draco-gltf draco_only | Draco | 95420 | 89580 | 86763 | 34834 | 208353 |
| draco-gltf fallback | Draco | 2598788 | 1375995 | 1148846 | 34834 | 208353 |

Gaussian point-cloud bytes in KHR_draco_mesh_compression are experimental and outside the extension's standard TRIANGLES/TRIANGLE_STRIP mode table.
