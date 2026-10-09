# Optimized WASM module sizes

Written by `cargo run --manifest-path web/build-tool/Cargo.toml -- --record-sizes`, one row per module and feature profile, in bytes. `gzip` is what a browser downloads. Nothing enforces a ceiling: the point of committing these is that a module's weight moves in the diff of the change that moves it.

Sizes are toolchain- and platform-dependent. Two builds of one commit on the same machine differ by a byte, but Windows and Linux differ by about 875, so re-record on one machine before reading a difference smaller than that as a change in the code.

| module | profile | raw | gzip |
| --- | --- | ---: | ---: |
| drc-wasm | release | 622150 | 222337 |
| fbx-wasm | release | 584480 | 239744 |
| gltf-wasm | accessors,draco-encode,raw-resources,strict-validation | 729404 | 280025 |
| gltf-wasm | release | 357096 | 142548 |
| ktx2-wasm | release | 366749 | 175075 |
| obj-wasm | release | 77149 | 42741 |
| ply-wasm | release | 202301 | 91286 |
| stl-wasm | release | 91155 | 49026 |
