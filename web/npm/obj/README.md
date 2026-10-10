# @draco-rust/obj

Reads and writes Wavefront OBJ in WebAssembly: positions, normals, texture
coordinates, groups and material references. It is part of
[draco-rust](https://github.com/Filyus/draco-rust) and drives the
[converter](https://filyus.github.io/draco-rust/).

~42 KiB gzip.

## Usage

```js
import init, { parse_obj_bytes, create_obj } from "@draco-rust/obj";

await init();
const result = parse_obj_bytes(objBytes);
if (!result.success) throw new Error(result.error);
const { positions, indices, normals, uvs } = result.meshes[0];

const written = create_obj({ positions, indices, normals, uvs }, {});
const obj = written.data; // the OBJ text
```

Polygons are triangulated. `create_obj_multi` writes several meshes into one
file.

For loading from a CDN or in Node, see the
[decoder's README](https://www.npmjs.com/package/@draco-rust/decoder); the same
`init` options apply, with `@draco-rust/obj/index_bg.wasm` as the file.

Licensed under Apache-2.0.
