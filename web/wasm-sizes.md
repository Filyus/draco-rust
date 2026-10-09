# Optimized WASM module sizes

Written by `cargo run --manifest-path web/build-tool/Cargo.toml -- --record-sizes`, one row per module and feature profile, in bytes. `gzip` is what a browser downloads. Nothing enforces a ceiling: the point of committing these is that a module's weight moves in the diff of the change that moves it.

Recorded with wasm-opt from Binaryen `version_133`, the release the workflows install; the build tool refuses `--record-sizes` with any other, and `WASM_OPT` points it at one that is not on `PATH`.

Sizes are toolchain- and platform-dependent. Two builds of one commit on the same machine differ by a byte, but Windows and Linux differ by about 875, so re-record on one machine before reading a difference smaller than that as a change in the code.

| module | profile | raw | gzip |
| --- | --- | ---: | ---: |
| drc-wasm | release | 616292 | 220864 |
| fbx-wasm | release | 584493 | 239787 |
| gltf-wasm | accessors,draco-encode,point-cloud-decode,raw-resources,strict-validation | 728808 | 282734 |
| gltf-wasm | release | 309765 | 124570 |
| ktx2-wasm | release | 365621 | 174986 |
| obj-wasm | release | 76561 | 42577 |
| ply-wasm | release | 200363 | 90829 |
| stl-wasm | release | 90588 | 48855 |
