# @draco-rust/stl

Reads and writes binary and ASCII STL in WebAssembly. Which kind a
file is follows from its length, not from a leading `solid`, which binary
exporters often write too. It is part of
[draco-rust](https://github.com/Filyus/draco-rust) and drives the
[converter](https://filyus.github.io/draco-rust/).

~48 KiB gzip.

## Usage

```js
import init, { parse_stl_bytes, create_stl } from "@draco-rust/stl";

await init();
const result = parse_stl_bytes(stlBytes);
if (!result.success) throw new Error(result.error);
const { positions, indices, normals } = result.meshes[0];

const written = create_stl({ positions, indices }, {});   // binary by default
const stl = written.binary_data; // Uint8Array; with { format: "ascii" }, text in written.data
```

Meshes keep STL's own shape: three vertices per triangle, each with the facet
normal, so a cube reads as 36 vertices rather than 24.

For loading from a CDN or in Node, see the
[decoder's README](https://www.npmjs.com/package/@draco-rust/decoder); the same
`init` options apply, with `@draco-rust/stl/index_bg.wasm` as the file.

Licensed under Apache-2.0.
