# InstancedQuadsQuantized.gltf

`InstancedQuads.gltf` with the instance rotations stored as normalized SHORT
instead of FLOAT, and nothing else changed. Released under **CC0 1.0
Universal**, like the fixture it is a copy of.

`EXT_mesh_gpu_instancing` permits ROTATION as `5122 SHORT normalized` or
`5120 BYTE normalized` as readily as `5126 FLOAT`; TRANSLATION and SCALE are
float only. No public asset uses either integer form — Khronos ships one
instancing model and it is float, and so is every other instancing file
anyone publishes, including the GPU-instanced Damaged Helmet that three.js
carries. Half of the extension's accessor table was therefore unreachable by
any corpus, and the reader decoded those bytes as float32: not a subtly wrong
matrix but a read off the end of the buffer.

Authored rather than found, because there was nothing to find. It is a copy
of the float fixture on purpose: `web/tests/viewer-instancing.ts` asserts the
two compose to the same matrices, which is an assertion it can only make
because quantization is the single difference between them. The gate needs no
expected values of its own — the float file is the expected value.

The rotations round to the nearest representable step rather than truncating,
since the extension defines the decode as `max(c / 32767, -1)`; the worst
component error over the four quaternions is 1.45e-5.

Everything is embedded as a data URI, so the file stands alone.

Regenerate:

```sh
python tools/make_instancing_fixture.py
```
