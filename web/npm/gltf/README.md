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
| `@draco-rust/gltf` | read glTF and GLB, decode Draco, read any accessor (animations, skins, morph targets), raw buffers | ~123 KiB |
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

For a `.gltf` with external files, pass the files by URI:
`GltfAsset.withResources(jsonBytes, { "scene.bin": binBytes }, "2.0")`.
`readPrimitives(pairs)` reads many primitives in one call, and
`asset.json()` and `asset.glb(2)` write the document back out.

For loading from a CDN or in Node, see the
[decoder's README](https://www.npmjs.com/package/@draco-rust/decoder); the same
`init` options apply, with this package's `index_bg.wasm` files.

Licensed under Apache-2.0.
