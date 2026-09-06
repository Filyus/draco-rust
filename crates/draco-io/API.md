# draco-io 0.4 API

`draco-io` reads and writes OBJ, PLY, STL and FBX against the `draco-core`
geometry model. None of these formats embeds a Draco bitstream, so no feature
here enables the codec.

glTF is the format that does embed one, and `draco-gltf` 0.3 owns it whole:
GLB containers, resource resolution and accessors as well as documents, typed
views and document-preserving Draco compression.
