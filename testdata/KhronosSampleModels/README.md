# Khronos glTF sample assets

Models copied from [KhronosGroup/glTF-Sample-Assets](https://github.com/KhronosGroup/glTF-Sample-Assets),
which is where they should be refreshed from. Each model keeps its own
`LICENSE.md` alongside the geometry; the ones added for material-extension and
compression coverage are all **CC0 1.0 Universal**, so the corpus carries no
attribution obligation. Several `Compare*` models also carry the glTF logo as a
texture, under Khronos's non-copyrightable logo mark rather than a copyright
licence — the same terms the ones already here were taken under. The
`LICENSE.md` files themselves are CC BY 4.0 as Khronos metadocumentation, and
the link above is the attribution:

| Model | Covers |
| --- | --- |
| `CompareClearcoat` | `KHR_materials_clearcoat` over varying base materials |
| `CompareIor` | `KHR_materials_ior` |
| `CompareSpecular` | `KHR_materials_specular`, colour and strength textures |
| `CompareEmissiveStrength` | `KHR_materials_emissive_strength` |
| `CompareTransmission` | `KHR_materials_transmission` on its own, with no volume |
| `CompareVolume` | transmission plus `KHR_materials_volume` |
| `CompareDispersion` | `KHR_materials_dispersion`, over transmission, volume and IOR |
| `CompareAnisotropy` | `KHR_materials_anisotropy`, with `KHR_texture_transform` |
| `CompareIridescence` | `KHR_materials_iridescence` |
| `AnisotropyStrengthTest` | the anisotropy strength parameter alone |
| `AnisotropyDiscTest` | the anisotropy texture, direction and all |
| `TransmissionThinwallTestGrid` | thin-walled versus volumetric, over a checkered backdrop |
| `TransmissionOrderTest` | transmission against the blend modes it has to be drawn among |
| `SimpleInstancing` | `EXT_mesh_gpu_instancing` as Khronos writes it |
| `AnimatedColorsCube` | `KHR_animation_pointer`, which nothing here interprets |
| `PointLightIntensityTest` | `KHR_lights_punctual`, which SceneDocument drops |
| `MeshoptCubeTest` | meshopt under its pre-ratification name, and the `COLOR` vertex filter |

Each is the `glTF-Binary` variant only. The separate-file variants carry the
same scene at several times the size, and nothing here reads a `.gltf`
differently from a `.glb` except the corpus gate, which already has plenty of
both.

`web/tests/gltf-corpus-parity.mjs` walks all of `testdata` and now also asserts
that every extension the viewer claims to interpret is used by some file in it.
Four were not, before these arrived — dispersion and transmission among them,
after a month of work on exactly that path. Hand-built fixtures state what a
reader does with a field; only a file an authoring tool actually wrote can say
whether the whole thing still goes through.

Two of these earned their place immediately. `AnimatedColorsCube` was rejected
outright by strict validation, which knew only the four core animation channel
paths and threw away the entire document over `KHR_animation_pointer`'s
`pointer` — geometry included, for an animation a reader is free to skip.
`MeshoptCubeTest` was for a long time the one file in the corpus that did not
load: gltfpack writes it under `KHR_meshopt_compression`, the name the extension
carried before it was ratified as `EXT_`, and it uses the `COLOR` vertex filter.
Both are read now, and both stay as the coverage for what they broke.

The older models here predate this file; their licences are as published by
Khronos and several are CC BY 4.0.
