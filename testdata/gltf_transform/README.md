# Draco glTF written by glTF-Transform

Most Draco-compressed glTF in the wild is written by gltf-pipeline or
glTF-Transform, and until these files every Draco fixture here came from
Blender or this repository's own tools. glTF-Transform's encoder is upstream
Draco 1.5.7 compiled to WebAssembly (`draco3dgltf`), so the streams themselves
are ones the C++ parity tests already cover; what these pin is the glTF around
them -- placeholder accessors without a `bufferView`, the extension's own
attribute ids, several primitives' streams in one binary chunk.

| file | source | primitives | Draco |
|---|---|---|---|
| `two_objects_edgebreaker_speed0.glb` | `testdata/two_objects_inverse_materials.gltf` | 2 | EdgeBreaker, standard traversal |
| `two_objects_sequential.glb` | the same | 2 | sequential |
| `sphere_edgebreaker_speed0.glb` | `testdata/SphereWithCircleTexture/sphere_with_circle_texture.gltf` | 1 | EdgeBreaker, valence traversal |
| `sphere_sequential.glb` | the same | 1 | sequential |

Each source had its materials, textures, images and samplers removed first, so
the files carry geometry only. Then, with `@gltf-transform/cli` 4.5.1:

```sh
gltf-transform draco <source>.gltf <name>_edgebreaker_speed0.glb --encode-speed 0 --decode-speed 0
gltf-transform draco <source>.gltf <name>_sequential.glb --method sequential
```

Quantization is the tool's default (position 14, normal 10, texture
coordinate 12 bits), the same for both methods, so each pair decodes to the
same triangles in a different vertex order --
`crates/draco-gltf/tests/gltf_transform_fixtures_test.rs` holds them to that.
