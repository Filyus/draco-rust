# Optimized WASM module sizes

Written by `cargo run --manifest-path web/build-tool/Cargo.toml -- --record-sizes`, one row per module and feature profile, in bytes. `gzip` is what a browser downloads. Nothing enforces a ceiling: the point of committing these is that a module's weight moves in the diff of the change that moves it.

Sizes are toolchain- and platform-dependent. Two builds of one commit on the same machine differ by a byte, but Windows and Linux differ by about 875, so re-record on one machine before reading a difference smaller than that as a change in the code.

| module | profile | raw | gzip |
| --- | --- | ---: | ---: |
| drc-wasm | release | 350249 | 142609 |
| fbx-wasm | release | 492073 | 216702 |
| gltf-wasm | accessors,draco-encode,raw-resources,strict-validation | 557940 | 234959 |
| gltf-wasm | release | 311179 | 131918 |
| ktx2-wasm | release | 370663 | 179074 |
| obj-wasm | release | 87903 | 46402 |
| ply-wasm | release | 159655 | 75072 |
| stl-wasm | release | 100584 | 53022 |
