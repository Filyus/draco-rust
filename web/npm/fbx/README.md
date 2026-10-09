# @draco-rust/fbx

Reads and writes binary FBX in WebAssembly. It keeps the model tree with its
local transforms, the materials, the skins with their weights, and the morph
targets, rather than a flat list of meshes. It is part of
[draco-rust](https://github.com/Filyus/draco-rust) and is what the
[converter](https://filyus.github.io/draco-rust/) uses to bring FBX into glTF.

~234 KiB gzip.

## Usage

```js
import init, { parse_fbx, create_fbx } from "@draco-rust/fbx";

await init();
const scene = parse_fbx(fbxBytes);
if (!scene.success) throw new Error(scene.error);
console.log(scene.meshes.length, scene.warnings);

const written = create_fbx([{ positions, indices, normals, uvs }], {});
const fbx = written.binary_data; // Uint8Array
```

Compressed FBX arrays (zlib) are read, which every mainstream exporter writes.
ASCII FBX is not supported.

For loading from a CDN or in Node, see the
[decoder's README](https://www.npmjs.com/package/@draco-rust/decoder); the same
`init` options apply, with `@draco-rust/fbx/index_bg.wasm` as the file.

Licensed under Apache-2.0.
