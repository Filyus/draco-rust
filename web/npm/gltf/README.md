# @draco-rust/gltf

Reads and writes glTF 2.0 and GLB in WebAssembly, including Draco geometry
(`KHR_draco_mesh_compression`), with a lossless document model: JSON the code
does not understand survives a round trip unchanged. It is part of
[draco-rust](https://github.com/Filyus/draco-rust) and drives the
[converter](https://filyus.github.io/draco-rust/).

## Entries

Each entry is a separate module. A bundler includes only the one you import.

| import | what it adds | gzip |
|---|---|---|
| `@draco-rust/gltf` | read glTF and GLB, decode Draco, read any accessor (animations, skins, morph targets), raw buffers | ~124 KiB |
| `@draco-rust/gltf/validate` | as above, plus strict validation of references, the node tree and `POSITION` bounds, for files you do not trust | ~152 KiB |
| `@draco-rust/gltf/writer` | as above, plus writing geometry, GLB output and Draco compression | ~261 KiB |

## Usage

```js
import init, { GltfAsset } from "@draco-rust/gltf";

await init();
const asset = new GltfAsset(glbBytes, "2.0");      // or "2.1" for the draft profile
for (let mesh = 0; mesh < asset.meshCount(); mesh++) {
  for (let p = 0; p < asset.primitiveCount(mesh); p++) {
    const geometry = asset.readPrimitive(mesh, p);  // Draco or not, the same result
    // geometry.attributeSemantic(i), attributeBytes(i), indexBytes(), ...
  }
}
```

The reader uses Draco decode ceilings well above the largest checked assets.
Pass a third argument to set the per-stream ceilings:

```js
const asset = new GltfAsset(glbBytes, "2.0", {
  max_points: 4_000_000,
  max_faces: 8_000_000,
  max_decoded_bytes: 256 * 1024 * 1024,
});
```

The same object is the fourth argument to `GltfAsset.withResources`. Defaults
are 256,000,000 points for point-cloud streams, 512,000,000 faces, and 2 GiB of
decoded attribute bytes. `threads` is accepted for API parity, but this
WebAssembly build decodes on one thread. The release reader omits the optional
point-cloud decoder; the writer build and converter include it.

For a `.gltf` with external files, pass the files by URI:
`GltfAsset.withResources(jsonBytes, { "scene.bin": binBytes }, "2.0")`.
`readPrimitives(pairs)` reads many primitives in one call, and
`asset.json()` and `asset.glb(2)` write the document back out.
`asset.jsonAt("/nodes/0/translation")` reads one value by JSON Pointer, the
form `KHR_animation_pointer` uses, as JSON bytes, or `undefined` when nothing
is there.

## On workers

`GltfAsset` and the geometry it reads are wasm objects, which stay on the
thread that made them. The pool reads a whole file on a worker instead and
returns plain arrays: the document's JSON and every primitive of every mesh,
Draco-compressed or not, decoded.

```js
import { createPool } from "@draco-rust/gltf/pool"; // or /validate/pool, /writer/pool

const pool = createPool();
const { json, meshes } = await pool.read(glbBytes);
// meshes[m][p] = { mode, attributes: [{ semantic, count, components, componentType, normalized, bytes }], index }
const summary = await pool.inspect(glbBytes); // what inspect_gltf returns
pool.dispose();
```

`pool.read` accepts the same settings as `decode_options` alongside its
`resources` and `profile`:

```js
const { json, meshes } = await pool.read(glbBytes, {
  decode_options: { max_points: 4_000_000 },
});
```

A `.gltf` with external files takes them as `pool.read(jsonBytes, { resources })`,
the way `GltfAsset.withResources` does, and `profile` picks `"2.0"` or `"2.1"`.

The workers start as work arrives, and the wasm is compiled once for all of
them. Inputs are copied to the worker, or handed over with `{ transfer: true }`;
`{ signal }` takes an `AbortSignal`, which rejects the promise at once. A task
a worker has started still runs to its end and its result is dropped. With
`workers: 0`, or where there are no workers, the pool runs on the calling
thread. It works on Web Workers, Node's worker threads (Node 20.16 and later)
and from a CDN, as the decoder's does.

For loading from a CDN or in Node, see the
[decoder's README](https://www.npmjs.com/package/@draco-rust/decoder); the same
`init` options apply, with this package's `index_bg.wasm` files.

Licensed under Apache-2.0.

## Writing Draco

Import the writer entry for geometry writes and Draco encoding:

```js
import init, { GltfAsset } from "@draco-rust/gltf/writer";

await init();
const asset = new GltfAsset(glbBytes, "2.0");
asset.compressPrimitive(0, 0, 5, 5, 14, 10, 12, 10, 12, 0, {
  position_quantization_grid: { origin: [0, 0, 0], range: 100 },
  max_output_bytes: 64 * 1024 * 1024,
});
const compressed = asset.glb(2);
```

`compressPrimitive` keeps its positional arguments: mesh, primitive, encoding
speed, decoding speed, position/normal/texture-coordinate/color/generic bits,
and method (`0` auto, `1` sequential, `2` EdgeBreaker for meshes or KD-tree for
point clouds). The final options object
is optional. It accepts `encoding_speed`, `decoding_speed`, `position_bits`,
`normal_bits`, `texcoord_bits`, `color_bits`, `generic_bits`,
`encoding_method`, `fallback`, `max_output_bytes`,
`position_quantization_grid`, `allow_points`, `spatial_point_order`,
`point_order_search`, `prediction_search`, and `gaussian_splat_quantization`.
When an option also has a
positional argument, the object value wins. `fallback: true` retains ordinary
geometry alongside Draco. `max_output_bytes` caps the final resolved buffers.

`GeometryWriteOptions.useDraco(encodingSpeed, decodingSpeed, fallback, options)`
accepts the other settings above; its positional `fallback` argument selects
the storage mode, so do not pass `fallback` in that options object.
`position_quantization_grid` takes a three-number
`origin` and finite positive `range`, and requires position bits. Point-cloud
compression is opt-in with `allow_points`; it writes `POINTS` under an
extension whose specification does not define that mode.
`spatial_point_order` and `point_order_search` can change the decoded point
order. Point-cloud search options have no effect on triangle meshes.

Both encoding entry points accept Gaussian splat settings in the options
object:

```js
const splatOptions = {
  allow_points: true,
  position_bits: 14,
  gaussian_splat_quantization: {
    scale_bits: 10,
    opacity_bits: 8,
    sh_degrees: [{ bits: 10, max_abs: 0.25 }, 8, 7, 6],
    sh_coefficients: {
      'KHR_gaussian_splatting:SH_DEGREE_2_COEF_3': 'preserve',
    },
  },
};
```

An SH degree rule can be a bit count, `'preserve'`, `{ bits, max_abs }` for a
grid that decodes zero exactly, or `{ bits, origin, range }` for an explicit
RGB grid. Values outside the grid are clamped. Coefficient overrides accept a
bit count or `'preserve'`. An omitted or `null` rule inherits the degree
setting or `generic_bits`. Rotation and integer splat attributes
retain their precision. A cloud with Float32 rotation uses sequential in
auto mode and rejects an explicit KD-tree request. Float32 opacity uses a
fixed `[0, 1]` grid when quantized.
