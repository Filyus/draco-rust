# Optimized WASM module sizes

Written by `cargo run --manifest-path web/build-tool/Cargo.toml -- --record-sizes`, one row per module and feature profile, in bytes. `gzip` is what a browser downloads. Nothing enforces a ceiling: the point of committing these is that a module's weight moves in the diff of the change that moves it.

Sizes are toolchain- and platform-dependent. Two builds of one commit on the same machine differ by a byte, but Windows and Linux differ by about 875, so re-record on one machine before reading a difference smaller than that as a change in the code.

A row whose profile ends in `glue` is the module's JavaScript glue as shipped, minified by the build. The profile before `glue` identifies the feature profile that produced that artifact.

| module | profile | raw | gzip |
| --- | --- | ---: | ---: |
| drc-wasm | release | 592860 | 206223 |
| drc-wasm | release glue | 9744 | 3203 |
| fbx-wasm | release | 545911 | 223436 |
| fbx-wasm | release glue | 8990 | 3047 |
| gltf-wasm | accessors,draco-encode,point-cloud-decode,raw-resources,strict-validation | 687053 | 261243 |
| gltf-wasm | accessors,draco-encode,point-cloud-decode,raw-resources,strict-validation glue | 21658 | 4320 |
| gltf-wasm | release | 287035 | 114867 |
| gltf-wasm | release glue | 15184 | 3665 |
| ktx2-wasm | release | 379879 | 175036 |
| ktx2-wasm | release glue | 7087 | 2340 |
| obj-wasm | release | 59067 | 35763 |
| obj-wasm | release glue | 7604 | 2764 |
| ply-wasm | release | 176790 | 81044 |
| ply-wasm | release glue | 8140 | 2870 |
| stl-wasm | release | 72263 | 42546 |
| stl-wasm | release glue | 7540 | 2724 |
