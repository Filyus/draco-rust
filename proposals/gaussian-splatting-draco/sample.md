# Interoperability sample

[mixed.glb](mixed.glb) combines the repository's Draco box and synthetic
Gaussian splats. Two nodes instantiate the splat mesh with different rotations.
The local writer uses `draco-gltf` to write the point stream and its nested
extension. The fixture builder combines it with the mesh. The verifier reopens the GLB
with Google's full Draco decoder, checks geometry types and counts, resolves
attribute IDs, and compares every decoded splat component with its source.

To compress a `POINTS` primitive that declares
`KHR_gaussian_splatting`, set `CompressionOptions::compress_splats`
to `true` and call `Import::compress_primitive`. Use the default
`CompressionMode::DracoOnly`. The writer supplies the required extension
declarations and preserves the existing splat properties. Indexed input is
expanded into one splat per index, including repeated indices. The output
primitive omits `indices`.

From the repository root, install the existing web dependencies and run:

```sh
npm ci --prefix web
cargo run --manifest-path proposals/gaussian-splatting-draco/Cargo.toml
node proposals/gaussian-splatting-draco/probe.cjs
```

## KD-tree and loader conformance

Run `node proposals/gaussian-splatting-draco/conformance.cjs` from the repository
root after the Cargo build above. The script generates GLBs, tests them
through the ordinary `draco-gltf` import and primitive reader, and checks
that they reproduce the files in [vectors](vectors/README.md) byte for byte.
Use `--update-vectors` to refresh those files after changing a fixture.
The Google Draco oracle also checks ordering by persistent
ID, every semantic, SNORM8 and SNORM16 rotation, UNORM8 opacity, and
multiplicity when two complete records are identical.

Twenty-one rejected GLBs exercise the required declaration, stream version
and geometry type, buffer view, semantic-map keys, unresolved and duplicate
attribute IDs, accessor layout and conflicting sources. They also cover
invalid SH storage, zero rotation, NaN SH, incorrect position bounds,
scale less than zero, and opacity outside the accepted range. Each refusal
must report the expected diagnostic. A positive GLB
checks that the immediate Float32 successor of 1 produces exactly 1 in the
Rust accessor bytes. A second verifies preservation of signed zero in scale
and opacity. An additional oracle test checks unsupported required
extensions.

The Float32 rotation vector uses Draco's 8-bit quantizer with range 2,
including values between grid points. The worst component change was
0.003922 and the worst norm deviation was 0.004795. The assertions use the
uniform-grid bounds with a Float32 rounding allowance.

## Rendering

With the web converter built in its app profile, run:

```sh
npm --prefix web run test:browser -- --grep "a splat node rotated|splat instance lighting|draco-compressed splat primitive renders|gpu-instanced splat node|mixed splat asset"
```

The single-instance camera/rotation equivalence check passes. The compressed
path loads `mixed.glb` directly through the app reader and draws a frame
byte-identical to the same scene with interleaved plain splat accessors.
The browser also rejects a variant with an unknown required extension.
A node instantiating the splat
primitive through `EXT_mesh_gpu_instancing` draws one copy at the node
transform; the reader collects the instance transforms, per-instance drawing
is not implemented yet, and the test pins that behavior so implementing the
draft's per-instance rule turns it loud.

With two plain nodes instantiating the same mesh, reversing their enumeration
changes visible SH lighting in the current viewer. Repeating the comparison
with equal rotations produces byte-identical frames. The failing pixel
comparison is marked as an expected failure until the viewer retains a
harmonic frame per instance.

## Fixed-overhead size and loading comparison

Both arms store the splats losslessly, so this comparison isolates container
layout and stream packing on a fixture small enough to sit
in cache. It says nothing about per-attribute quantization -- Draco's main
lever -- or about load times at delivery scale.

```sh
npm install --prefix proposals/gaussian-splatting-draco/target/bench --no-audit --no-fund meshoptimizer@1.3.0 @draco-rust/decoder@0.4.3
node proposals/gaussian-splatting-draco/compare.mjs
```

Both GLBs carry the same mesh stream and produce byte-identical splat data.
The Meshopt arm stores the interleaved source using lossless ATTRIBUTES mode.
The timer covers in-memory GLB parsing, both geometry decodes and materializing
the same interleaved splat output. It uses the same full Draco decoder for
the mesh in both arms. Measurements alternate order across 20 warm-up and
100 measured iterations. Startup, network and rendering are outside the timer.

| Synthetic scene, Node 24 | Draco | Draco mesh + Meshopt splats |
| --- | ---: | ---: |
| GLB bytes | 20,232 | 17,824 |
| GLB bytes after gzip | 6,471 | 7,601 |
| Warm median load (ms), one run | 0.113 | 0.022 |
| Google full decoder, gzip bytes | 100,539 | 82,980 with Meshopt |
| `@draco-rust` point-cloud build, gzip bytes | 68,368 | — |
| `@draco-rust` full build, gzip bytes | 97,981 | — |

The decoder rows compare gzipped resource pairs. The Google rows are the full
and glTF-specific WASM/wrapper pairs shipped by Three.js, the latter with the
standalone Meshopt module. The `@draco-rust` rows are the published npm
package's point-cloud and full builds, measured with Node's default gzip.
The point-cloud build serves splat-only assets. The full build handles both
streams in the mixed sample and is slightly smaller than the measured full
Google distribution. The published point-cloud and full builds agree on the
decoded splat data, which matches the source bytes.

In the maintainer's separate capture harness, the `good` quantization
profile used 9.9 bytes per gaussian for a degree-3 capture. In a separate
Node 24 decode run with two million gaussians, `@draco-rust/decoder` took
1949 ms and Google `draco3d` took 2380 ms.
A matched-quality lossy comparison remains open on a delivery-scale asset.
For Meshopt, use separate buffer views for quaternion, exponential and other
attribute groups, with the stride and storage format each filter requires.
Compare rendered PSNR across the same views, or use relative scale error
when evaluating scale precision in isolation. Absolute scale error can
hide the loss of precision on small Gaussians.
