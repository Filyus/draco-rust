# @draco-rust/decoder

Decodes [Draco](https://google.github.io/draco/) geometry (`.drc`) in WebAssembly.
It is part of [draco-rust](https://github.com/Filyus/draco-rust), a safe Rust port of
Google's Draco, and is byte-for-byte compatible with its bitstream. It is not
affiliated with Google.

## Entries

Each entry is a separate module. A bundler includes only the one you import.

| import | decodes | gzip |
|---|---|---|
| `@draco-rust/decoder` | meshes and point clouds | ~87 KiB |
| `@draco-rust/decoder/mesh` | meshes only, which is all glTF's `KHR_draco_mesh_compression` allows | ~72 KiB |
| `@draco-rust/decoder/point-cloud` | point clouds only (scans, splats) | ~58 KiB |
| `@draco-rust/decoder/legacy` | meshes and point clouds, plus bitstreams older than 2.2 | ~97 KiB |

## Usage

The function names match the converter that drives this module, so they are
`snake_case`.

```js
import init, { parse_drc_bytes } from "@draco-rust/decoder";

await init();
const result = parse_drc_bytes(new Uint8Array(await (await fetch("bunny.drc")).arrayBuffer()));
if (!result.success) throw new Error(result.error);
const { positions, indices, normals, uvs, colors } = result.meshes[0];
```

`positions` is a `Float32Array` of xyz triples, and `indices` a `Uint32Array` of
triangle corners, which is empty for a point cloud. `normals` and `uvs` are
`Float32Array`s and `colors` a `Uint8Array`, each empty when the stream has no
such attribute. Attributes the stream carries beyond those are in
`meshes[0].extras`, and `result.warnings` says what was not interpreted.

### From a CDN, without a bundler

```js
import init, { parse_drc_bytes } from "https://cdn.jsdelivr.net/npm/@draco-rust/decoder@0.1/mesh/index.js";
await init(); // finds index_bg.wasm next to the script
```

### In Node

Node's `fetch` does not read files, so pass the module's bytes yourself:

```js
import { readFile } from "node:fs/promises";
import init, { parse_drc_bytes } from "@draco-rust/decoder";

await init({ module_or_path: await readFile(new URL(import.meta.resolve("@draco-rust/decoder/index_bg.wasm"))) });
```

For a subpath entry, resolve its own file, for example
`@draco-rust/decoder/mesh/index_bg.wasm`.

## Notes

- Runs in every browser with WebAssembly, in Node 20.6 and later, in Deno and
  in Bun.
- Decoding happens on the calling thread. Call it from a Web Worker to keep a
  page responsive.
- Licensed under Apache-2.0, like Draco.
