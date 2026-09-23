# tiny-splat.gltf

A `KHR_gaussian_splatting` asset written by an authoring tool rather than by
hand. No file in `KhronosSampleModels` uses the extension, and Khronos
publishes none beside its specification; the files Cesium ships with its
tests are in the pre-ratification draft form and compressed with SPZ.

Written by *3DGS PLY to glTF converter by Huawei*
(<https://github.com/NorbertNopper-Huawei/3dgs_ply2gltf>, commit `4dc8465`),
built from source, run with no flags:

```text
ply2gltf tiny-splat.ply
```

The input is 64 splats of synthetic data from `tinySplatPly(64)` in
`web/tests/smoke-fixtures.ts`: every property 3DGS writes, degree-3
harmonics, values that differ per splat and per property. Nothing in it
comes from a captured scene. Released under **CC0 1.0 Universal**, like the
sample assets beside it.

With no flags the converter keeps the PLY's coordinates as they are and
assumes they are already glTF's Y-up; its node carries no rotation. So the
splats stand the way the PLY stored them, which for 3DGS data is upside
down. That is the tool's choice, not this file's, and it is kept: a reader
has to follow the file, and other converters choose differently.
