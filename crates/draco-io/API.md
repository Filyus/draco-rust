# draco-io 0.3 API

`draco-io` contains low-level format I/O (OBJ, PLY, FBX) and the reusable
glTF container/resource/accessor contracts enabled by feature `gltf`.

For complete glTF documents, typed views, GLB serialization, compact views and
Draco compression/decompression, use `draco-gltf` 0.2.

The 0.2 `GltfReader`, `GltfWriter`, `gltf-compact` and byte-compression APIs
were intentionally removed in this breaking release.
