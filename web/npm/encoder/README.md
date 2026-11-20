# @draco-rust/encoder

Encodes meshes and point clouds to [Draco](https://google.github.io/draco/)
(`.drc`) in WebAssembly. It is part of [draco-rust](https://github.com/Filyus/draco-rust),
a safe Rust port of Google's Draco, and with default options writes the same
bytes as Google's encoder. It is not affiliated with Google. To decode, use
[`@draco-rust/decoder`](https://www.npmjs.com/package/@draco-rust/decoder).

~133 KiB gzip.

## Usage

```js
import init, { encode_draco } from "@draco-rust/encoder";

await init();
const result = encode_draco(
  { positions, indices, normals, uvs, colors },  // typed arrays; all but positions optional
  { position_bits: 14, normal_bits: 10, texcoord_bits: 12, encoding_speed: 5, decoding_speed: 5 },
);
if (!result.success) throw new Error(result.error);
const drc = result.bytes; // Uint8Array
```

`positions` is a `Float32Array` of xyz triples and `indices` a `Uint32Array` of
triangle corners. Leave `indices` out, or set `point_cloud: true`, to write a
point cloud. Three options act on point clouds only:
- `prediction_search` chooses each attribute's prediction by its estimated cost.
- `spatial_point_order` orders the points along a Hilbert curve.
- `point_order_search` runs that curve through a refinement and keeps it only
  when the coder prices it under the order the points came in, so a
  scan-ordered capture keeps its order and a scrambled one is reordered.

They write smaller files, at the cost of output no longer byte-identical to
Google's encoder. `point_order_search` also costs encode time to measure, and
`spatial_point_order` reorders the decoded points whatever the price.

## Quantization grids and metadata

Per attribute, `quantizationBits` names a bit budget and, with it,
`quantizationOrigin` and `quantizationRange` name the grid the values are
quantized on -- the same three options `ExpertEncoder::SetAttributeExplicitQuantization`
takes. Positions take `position_origin` and `position_range` beside
`position_bits`. The grid goes into the stream as the transform's own
parameters, so any decoder reads it the same way; values outside it come back
on its edges rather than off it.

The mesh can carry `metadata` -- and each extra attribute its own -- in the
same shape `parse_drc_bytes` reports it: `{ entries, subMetadata? }`, entries
byte arrays keyed by name. A `.drc -> .drc` recode passes them back and keeps
what the file said about itself; a `name` on an extra still lands in its
metadata as `"name"`.

## On workers

`@draco-rust/encoder/pool` encodes on workers, so a large mesh does not block
the calling thread:

```js
import { createPool } from "@draco-rust/encoder/pool";

const pool = createPool({ workers: 2 });
const result = await pool.encode(mesh, { position_bits: 14 }); // what encode_draco returns
pool.dispose();
```

The workers start as work arrives, and the wasm is compiled once for all of
them. Inputs are copied to the worker, or handed over with `{ transfer: true }`;
`{ signal }` takes an `AbortSignal`, which rejects the promise at once. A task
a worker has started still runs to its end and its result is dropped. With
`workers: 0`, or where there are no workers, the pool runs on the calling
thread. It works on Web Workers, Node's worker threads (Node 20.16 and later)
and from a CDN, as the decoder's does.

For loading from a CDN or in Node, see the
[decoder's README](https://www.npmjs.com/package/@draco-rust/decoder); the same
`init` options apply, with `@draco-rust/encoder/index_bg.wasm` as the file.

Licensed under Apache-2.0, like Draco.
