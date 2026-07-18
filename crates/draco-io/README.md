# draco-io

Low-level Draco format I/O for OBJ, PLY and FBX plus glTF binary contracts.

`draco-io` 0.3 deliberately does not provide a glTF document parser, writer,
scene model, or document-preserving compressor. Use `draco-gltf` for lossless
glTF/GLB 2.0 and 2.1-draft documents, compact primitive access and Draco
encoding or decoding.

Enable the small `gltf` feature only for GLB/container parsing, resource
resolution and the `AccessorSource` geometry contract.

For large GLB v3 inputs, `GlbRangeReader<R: Read + Seek>` opens only the
header and chunk descriptors. It materializes individual chunks on demand with
a caller-provided byte limit, without requiring the complete container in
memory.
