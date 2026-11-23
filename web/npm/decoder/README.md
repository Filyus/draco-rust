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

### Metadata

A `.drc` can carry metadata: unnamed byte entries by key, nested blocks, a
block for the whole geometry and one per attribute. Both `parse_drc_bytes` and
`decode_draco` hand it back unchanged.

```js
const metadata = result.metadata;                    // the geometry's block, when it has one
metadata.entries.sh_degree;                          // the raw bytes, as written
metadata.strings.producer;                           // the same entry as text, when it is valid UTF-8
metadata.subMetadata?.node;                          // a nested block, same shape again

result.meshes[0].extras[0].metadata;                 // an attribute's own block
result.attributes[2].metadata?.name;                 // what a generic is called, in `decode_draco`
```

An entry is whatever bytes the producer wrote -- the bitstream carries no type
tag -- so `entries` holds them raw and `strings` holds the ones that are valid
UTF-8 as text. Reading a typed entry back is a `DataView` over the raw array:
`new DataView(entry.buffer, entry.byteOffset).getInt32(0, true)` for an `i32`,
for instance.

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

Without the second argument every attribute comes back in its own type. The
index is a `Uint32Array`; a third argument `{ index: "auto" }` makes it a
`Uint16Array` wherever the points number at most 65536, half the bytes.

The same options object accepts output ceilings and point-cloud decoder
threads:

```js
const result = decode_draco(bytes, requests, {
  index: "auto",
  max_points: 4_000_000, // point-cloud streams
  max_faces: 8_000_000,
  max_decoded_bytes: 256 * 1024 * 1024,
});
```

Defaults are 256,000,000 points for point-cloud streams, 512,000,000 faces,
and 2 GiB of decoded attribute bytes. The `threads` field is accepted for API
parity, but these WebAssembly packages decode on one thread. These ceilings
also work with `parse_drc_bytes`, `pool.decode`, and the three.js loader's
`decode_options` constructor option. Pool settings such as `transfer`,
`signal`, and `workers` remain separate.

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
`pool.decode` also takes the decode options above and an `AbortSignal` as
`signal`: aborting rejects the promise at once. A decode a worker has
already started still runs to its end and its result is dropped; one not yet
sent is never sent.
A bundler that follows `new URL("./worker.js", import.meta.url)`, as Vite and
webpack do, emits the worker on its own.

### With three.js

`@draco-rust/decoder/three` exports `DRACOLoader`: three.js's own
`DRACOLoader`, decoding on this package's workers instead of the Draco decoder
three fetches with `setDecoderPath`. Everything else is three's class, so it
goes wherever one is expected, TypeScript included:

```js
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { DRACOLoader } from "@draco-rust/decoder/mesh/three";

const gltfLoader = new GLTFLoader().setDRACOLoader(new DRACOLoader());
const gltf = await gltfLoader.loadAsync("model.glb");
```

`load`, `parse`, `setWorkerLimit`, `preload` and `dispose` work as three's do,
and so does the geometry: three builds it, so vertex colours, normalization and
the padding r181 added follow the three you have. Byte for byte it is the
geometry three's own loader builds, with one difference: an attribute that
does not convert to the type asked for -- a negative value into a
`Uint16Array`, say -- is an error here, where three's loader fills the
attribute with whatever its failed conversion left in memory.

The constructor takes three's `LoadingManager` and then the pool's options,
`new DRACOLoader(manager, { workers: 2 })`, or `{ pool }` to share one pool
between loaders. `three` is a peer dependency, from r150 (`0.150.0`); the
package's other entries do not need it.

### From a CDN, without a bundler

```js
import init, { parse_drc_bytes } from "https://cdn.jsdelivr.net/npm/@draco-rust/decoder@0.3/mesh/index.js";
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
