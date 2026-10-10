# @draco-rust/decoder

Decodes [Draco](https://google.github.io/draco/) geometry (`.drc`) in WebAssembly.
It is part of [draco-rust](https://github.com/Filyus/draco-rust), a safe Rust port of
Google's Draco, and is byte-for-byte compatible with its bitstream. It is not
affiliated with Google.

## Entries

Each entry is a separate module. A bundler includes only the one you import.

| import | decodes | gzip |
|---|---|---|
| `@draco-rust/decoder` | meshes and point clouds | ~92 KiB |
| `@draco-rust/decoder/mesh` | meshes only, which is all glTF's `KHR_draco_mesh_compression` allows | ~77 KiB |
| `@draco-rust/decoder/point-cloud` | point clouds only (scans, splats) | ~63 KiB |
| `@draco-rust/decoder/legacy` | meshes and point clouds, plus bitstreams older than 2.2 | ~103 KiB |

Each entry also has a worker pool and a three.js loader of its own, which use
that entry's module: `@draco-rust/decoder/pool` and `@draco-rust/decoder/three`,
or `@draco-rust/decoder/mesh/pool`, `@draco-rust/decoder/mesh/three` and so on.

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

### Attributes the way a loader asks for them

`decode_draco` returns every attribute as a typed array, or the ones a loader
asks for, converted to the array type it needs. Upstream Draco's decoder makes
the same conversions, and this one matches it byte for byte, refusing where it
refuses.

```js
import init, { decode_draco } from "@draco-rust/decoder/mesh";

await init();
// By unique id, as glTF's KHR_draco_mesh_compression names them, or by type.
const result = decode_draco(bytes, [
  { name: "position", id: 0, type: "Float32Array" },
  { name: "color", semantic: "COLOR", type: "Uint16Array" },
]);
if (!result.success) throw new Error(result.error);
const { index, attributes } = result; // attributes[i]: { name, array, itemSize, normalized, ... }
```

Without the second argument every attribute comes back in its own type.

### On workers

Decoding runs on the calling thread. `createDecoderPool` runs it on workers
instead, so a page stays responsive while several streams decode at once:

```js
import { createDecoderPool } from "@draco-rust/decoder/mesh/pool";

const pool = createDecoderPool({ workers: 4 }); // default: cores - 1, at most 4
const result = await pool.decode(bytes, requests); // what decode_draco returns
pool.dispose();
```

The workers start as work arrives, and the wasm is compiled once for all of
them. The stream is copied to the worker; `pool.decode(bytes, requests, {
transfer: true })` hands over the buffer itself instead. The pool runs on Web
Workers, on Node's worker threads (Node 20.16 and later), and from a CDN. With
`workers: 0`, or where there are no workers, it decodes on the calling thread.
A bundler that follows `new URL("./worker.js", import.meta.url)`, as Vite and
webpack do, emits the worker on its own.

### With three.js

`createDracoLoader` stands in for three.js's `DRACOLoader`, on the pool. It
takes three's classes rather than importing three, so it builds with your own
copy:

```js
import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { createDracoLoader } from "@draco-rust/decoder/mesh/three";

const gltfLoader = new GLTFLoader();
gltfLoader.setDRACOLoader(createDracoLoader(THREE));
const gltf = await gltfLoader.loadAsync("model.glb");
```

For a `.drc` file, `await loader.parseAsync(arrayBuffer)` returns a
`BufferGeometry`, and `parse(buffer, onLoad, onError)` works as DRACOLoader's
does. Vertex colours from a `.drc` file are converted from sRGB, as
DRACOLoader converts them. `setWorkerLimit`, `preload` and `dispose` are there
too. In TypeScript, `setDRACOLoader` expects the `DRACOLoader` class, so cast
the loader: `createDracoLoader(THREE) as unknown as DRACOLoader`.

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
- `parse_drc_bytes` and `decode_draco` decode on the calling thread; the pool
  decodes on workers.
- Licensed under Apache-2.0, like Draco.
