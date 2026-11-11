# glTF 2.1 draft support

`draco-gltf` uses its lossless `Document` model. The targeted
snapshot and update policy are in `GLTF_2_1_SNAPSHOT.md`.
The feature-by-feature implementation status and upstream references are in
[`GLTF_2_1_SUPPORT.md`](GLTF_2_1_SUPPORT.md).

The draft profile preserves every JSON field and strictly validates the
surface used by this crate, including GLB v3, `files`, shapes, versions,
accessor formats, and indexed attribute semantics. The support table records
partial UID and geometric shape validation. Primitive reads and raw writes
support the formats enabled by a required `KHR_mesh_quantization` declaration.
Draco encoding enforces its own supported component layouts.
