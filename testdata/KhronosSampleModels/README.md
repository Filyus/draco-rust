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
| `MeshoptCubeTest` | meshopt under its pre-ratification name, and the `COLOR` vertex filter |

`MeshoptCubeTest` was for a long time the one file in the corpus that did not
load. It is written by gltfpack under `KHR_meshopt_compression`, the name the
extension carried before it was ratified as `EXT_`, and it uses the `COLOR`
vertex filter; both are read now, and `web/tests/gltf-corpus-parity.mjs` carries
all 69 files. It stays as the coverage for both.

The older models here predate this file; their licences are as published by
Khronos and several are CC BY 4.0.
