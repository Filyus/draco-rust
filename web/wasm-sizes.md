# Optimized WASM module sizes

Written by `cargo run --manifest-path web/build-tool/Cargo.toml -- --record-sizes`, one row per module and feature profile, in bytes. `gzip` is what a browser downloads. Nothing enforces a ceiling: the point of committing these is that a module's weight moves in the diff of the change that moves it.

Sizes are toolchain- and platform-dependent. Two builds of one commit on the same machine differ by a byte, but Windows and Linux differ by about 875, so re-record on one machine before reading a difference smaller than that as a change in the code.

| module | profile | raw | gzip |
| --- | --- | ---: | ---: |
| drc-wasm | release | 336127 | 134071 |
| fbx-wasm | release | 449607 | 200400 |
| gltf-wasm | accessors,draco-encode,raw-resources,strict-validation | 554845 | 231456 |
| gltf-wasm | release | 314692 | 132190 |
| ktx2-wasm | release | 370034 | 179174 |
| obj-wasm | release | 87925 | 46399 |
| ply-wasm | release | 152604 | 71885 |
| stl-wasm | release | 99415 | 51843 |
