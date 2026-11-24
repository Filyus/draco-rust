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
  { positions, indices, normals, uvs, colors },
  { position_bits: 14, normal_bits: 10, texcoord_bits: 12, color_bits: 10, encoding_speed: 5, decoding_speed: 5 },
);
if (!result.success) throw new Error(result.error);
const drc = result.bytes; // Uint8Array
```

`positions` is a `Float32Array` of xyz triples. `indices` is a required
`Uint32Array` of triangle corners, though it can be empty. `normals`, `uvs`,
and `colors` are optional; set `include_normals`, `include_uvs`, or
`include_colors` to `false` to omit a supplied channel. The three include
options default to `true`.

`encoding_speed` ranges from 0 (smallest output) to 10 (fastest encode).
`decoding_speed` defaults to the encoding speed. For meshes,
`encoding_method: 0` lets the encoder choose (default), `1` forces sequential,
and `2` forces EdgeBreaker.

To write a point cloud, pass an empty `indices` array and set
`point_cloud: true`. This opt-in selects Draco's point-cloud coder; without it,
an input with no faces is still encoded as a faceless mesh. Three options act
on point clouds only:

- `prediction_search` chooses each attribute's prediction by its estimated cost.
- `spatial_point_order` orders the points along a Hilbert curve.
- `point_order_search` runs that curve through a refinement and keeps it only
  when the coder prices it under the order the points came in, so a
  scan-ordered capture keeps its order and a scrambled one is reordered.

These options can change the output bytes from Google's encoder. Prediction
search and point ordering can reduce size; spatial ordering can also make a
file larger. `point_order_search` spends encode time measuring the candidate
and only keeps it when it is smaller. `spatial_point_order` always reorders the
decoded points; `point_order_search` reorders them when its candidate wins.

## Quantization grids and metadata

The named channels take `position_bits`, `normal_bits`, `texcoord_bits`, and
`color_bits`. Positions can use a shared grid with `position_origin` (three
numbers) and `position_range`; provide both together and with `position_bits`.
Values outside the grid are clamped to its edges.

Additional attributes go in `extras`. Each item requires `type`, `components`,
`dataType`, `uniqueId`, and a flat `values` array with enough components for
every point. `type` is one of `POSITION`, `NORMAL`, `COLOR`, `TEX_COORD`, or
`GENERIC`; `dataType` is a Draco scalar name such as `float32` or `uint8`;
`values` can be a `Float32Array`, `Float64Array`, or plain numeric array.
`normalized` defaults to `false`. For example, a one-component float attribute
can be written as:

```js
extras: [{
  type: "GENERIC",
  components: 1,
  dataType: "float32",
  uniqueId: 7,
  name: "opacity",
  values: opacities,
  quantizationBits: 12,
}]
```

An extra attribute can also set `normalized` and `metadata`. Its
`quantizationBits` sets the bit budget; `quantizationOrigin` and
`quantizationRange` name an optional grid and must be supplied together. A grid
must match the component count, and quantization grids require float data.
Transform parameters travel in the stream, so decoders read the same grid.

When `color_bits` is between 1 and 30, the byte colors are normalized to
`[0, 1]` and encoded as a quantized float attribute. Zero leaves the colors in
their original byte representation. `parse_drc_bytes` still returns RGBA bytes;
`decode_draco` without a requested output type reflects the stream's float
attribute when color quantization is enabled.

The geometry and each extra attribute can carry `metadata` in the same shape
`parse_drc_bytes` returns: `{ entries, subMetadata? }`, with byte arrays keyed
by entry name. A `.drc` recode can pass this metadata back unchanged. An extra's
`name` is stored in its metadata under `"name"`.

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
