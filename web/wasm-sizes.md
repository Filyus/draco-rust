# Optimized WASM module sizes

Written by `cargo run --manifest-path web/build-tool/Cargo.toml -- --record-sizes`, one row per module and feature profile, in bytes. `gzip` is what a browser downloads. Nothing enforces a ceiling: the point of committing these is that a module's weight moves in the diff of the change that moves it.

Sizes are toolchain- and platform-dependent. Two builds of one commit on the same machine differ by a byte, but Windows and Linux differ by about 875, so re-record on one machine before reading a difference smaller than that as a change in the code.

| module | profile | raw | gzip |
| --- | --- | ---: | ---: |
| drc-wasm | release | 501568 | 177847 |
| fbx-wasm | release | 594823 | 249148 |
| gltf-wasm | accessors,draco-encode,raw-resources,strict-validation | 693197 | 267708 |
| gltf-wasm | release | 353555 | 143579 |
| ktx2-wasm | release | 370663 | 179074 |
| obj-wasm | release | 87903 | 46402 |
| ply-wasm | release | 179003 | 81493 |
| stl-wasm | release | 103632 | 54081 |
