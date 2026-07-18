# draco-io

Low-level Draco format I/O for OBJ, PLY and FBX plus glTF binary contracts.

`draco-io` 0.3 deliberately does not provide a glTF document parser, writer,
scene model, or document-preserving compressor. Use `draco-gltf` for lossless
glTF/GLB 2.0 and 2.1-draft documents, compact views and Draco transforms.

Enable the small `gltf` feature only for GLB/container parsing, resource
resolution and the `AccessorSource` geometry contract.
