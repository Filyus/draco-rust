# glTF 2.1 draft support

`draco-gltf` 0.2 uses its native lossless `Document` model. The targeted
snapshot and update policy are in `GLTF_2_1_SNAPSHOT.md`.

The draft profile preserves every JSON field and strictly validates the native
surface used by this crate: GLB v3, `files`, shapes, UIDs, and non-sequential
texture-coordinate/color semantics. Draco geometry continues to use the glTF
2.0-compatible component layouts required by KHR_draco_mesh_compression.
