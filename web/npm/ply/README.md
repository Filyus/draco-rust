# @draco-rust/ply

Reads and writes PLY in WebAssembly: meshes and point clouds, in ASCII and
binary. Every property is kept: the ones it has no attribute for come back by
name, so a Gaussian splat arrives with its scales, rotations, opacity and
spherical harmonics. It is part of
[draco-rust](https://github.com/Filyus/draco-rust) and drives the
[converter](https://filyus.github.io/draco-rust/).

~89 KiB gzip.

## Usage

```js
import init, { parse_ply_bytes, create_ply } from "@draco-rust/ply";

await init();
const result = parse_ply_bytes(plyBytes);
if (!result.success) throw new Error(result.error);
const { positions, indices, normals, colors, extras } = result.meshes[0];
// indices is empty for a point cloud; extras holds the other properties
```

`create_ply(mesh, { format })` writes one back: binary little-endian by
default, as a `Uint8Array` in `binary_data`; `format: "binary_big_endian"`
does the same, and `format: "ascii"` returns text in `data`.

For loading from a CDN or in Node, see the
[decoder's README](https://www.npmjs.com/package/@draco-rust/decoder); the same
`init` options apply, with `@draco-rust/ply/index_bg.wasm` as the file.

Licensed under Apache-2.0.
