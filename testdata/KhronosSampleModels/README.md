# Khronos glTF sample assets

Models copied from [KhronosGroup/glTF-Sample-Assets](https://github.com/KhronosGroup/glTF-Sample-Assets),
which is where they should be refreshed from. Each model keeps its own
`LICENSE.md` alongside the geometry; the ones added for material-extension and
compression coverage are all **CC0 1.0 Universal**, so the corpus carries no
attribution obligation:

| Model | Covers |
| --- | --- |
| `CompareClearcoat` | `KHR_materials_clearcoat` over varying base materials |
| `CompareIor` | `KHR_materials_ior` |
| `CompareSpecular` | `KHR_materials_specular`, colour and strength textures |
| `CompareEmissiveStrength` | `KHR_materials_emissive_strength` |
| `PointLightIntensityTest` | `KHR_lights_punctual`, which SceneDocument drops |
| `MeshoptCubeTest` | `KHR_meshopt_compression`, which the reader does not decode |

`MeshoptCubeTest` is deliberately a file that does not load: it is the corpus
marker for the newer meshopt codec, and `web/tests/gltf-corpus-parity.mjs`
records it by name with that reason. Removing it would remove the record.

The older models here predate this file; their licences are as published by
Khronos and several are CC BY 4.0.
