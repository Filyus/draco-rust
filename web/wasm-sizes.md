# Optimized WASM module sizes

Written by `cargo run --manifest-path web/build-tool/Cargo.toml -- --record-sizes`, one row per module and feature profile, in bytes. `gzip` is what a browser downloads. Nothing enforces a ceiling: the point of committing these is that a module's weight moves in the diff of the change that moves it.

Sizes are toolchain- and platform-dependent. Two builds of one commit on the same machine differ by a byte, but Windows and Linux differ by about 875, so re-record on one machine before reading a difference smaller than that as a change in the code.

| module | profile | raw | gzip |
| --- | --- | ---: | ---: |
| drc-wasm | release | 620499 | 222313 |
| fbx-wasm | release | 587401 | 240997 |
| gltf-wasm | accessors,draco-encode,point-cloud-decode,raw-resources,strict-validation | 732329 | 283625 |
| gltf-wasm | release | 311254 | 125154 |
| ktx2-wasm | release | 365619 | 174975 |
| obj-wasm | release | 76973 | 42671 |
| ply-wasm | release | 201731 | 91175 |
| stl-wasm | release | 90869 | 48945 |
