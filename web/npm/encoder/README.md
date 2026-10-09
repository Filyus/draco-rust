# @draco-rust/encoder

Encodes meshes and point clouds to [Draco](https://google.github.io/draco/)
(`.drc`) in WebAssembly. It is part of [draco-rust](https://github.com/Filyus/draco-rust),
a safe Rust port of Google's Draco, and with default options writes the same
bytes as Google's encoder. It is not affiliated with Google. To decode, use
[`@draco-rust/decoder`](https://www.npmjs.com/package/@draco-rust/decoder).

~133 KiB gzip.

## Usage

```js
import init, { create_drc } from "@draco-rust/encoder";

await init();
const result = create_drc(
  { positions, indices, normals, uvs, colors },  // typed arrays; all but positions optional
  { position_bits: 14, normal_bits: 10, texcoord_bits: 12, encoding_speed: 5, decoding_speed: 5 },
);
if (!result.success) throw new Error(result.error);
const drc = result.binary_data; // Uint8Array
```

`positions` is a `Float32Array` of xyz triples and `indices` a `Uint32Array` of
triangle corners. Leave `indices` out, or set `point_cloud: true`, to write a
point cloud. Two options act on point clouds only:
- `prediction_search` chooses each attribute's prediction by its estimated cost.
- `spatial_point_order` orders the points along a Hilbert curve.

Both write smaller files, at the cost of output no longer byte-identical to
Google's encoder.

For loading from a CDN or in Node, see the
[decoder's README](https://www.npmjs.com/package/@draco-rust/decoder); the same
`init` options apply, with `@draco-rust/encoder/index_bg.wasm` as the file.

Licensed under Apache-2.0, like Draco.
