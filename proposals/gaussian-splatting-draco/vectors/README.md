# Draco splat conformance vectors

These synthetic GLBs exercise the experimental
`TEST_gaussian_splatting_compression_draco` profile. Each combines a
Draco-compressed mesh with a Gaussian splat primitive. See
[sample.md](../sample.md) for reproduction instructions and a browser test.

## Accept

| File | Expected behavior |
| --- | --- |
| `sequential.glb` | Decode lossless Float32 attributes. |
| `kd-tree.glb` | Decode reordered points with matching persistent IDs and SNORM8 rotation. |
| `duplicates.glb` | Preserve coincident records and decode SNORM16 rotation. |
| `lossy-float32-rotation.glb` | Decode 8-bit quantized Float32 rotation. |
| `opacity-one-ulp.glb` | Expose opacity as exactly 1. |
| `signed-zero-scale-opacity.glb` | Accept Float32 −0 for scale and opacity. |

## Reject

The other 21 GLBs are negative cases. Each filename describes its
expected rejection. The corresponding diagnostic checks are in
[conformance.cjs](../conformance.cjs).
