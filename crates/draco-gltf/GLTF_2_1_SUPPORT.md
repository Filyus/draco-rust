# glTF 2.1 draft support status

Status as of 2026-07-18 for `draco-gltf` 0.2 and `draco-io` 0.3.

glTF 2.1 is still a draft. This implementation targets the Khronos glTF
repository at commit
[`77b44be7bef26e01fb0b140e3d5bb1716421c5e9`](https://github.com/KhronosGroup/glTF/commit/77b44be7bef26e01fb0b140e3d5bb1716421c5e9).
It does not claim conformance with a future final glTF 2.1 specification.
Updating the snapshot is an explicit compatibility change, not an automatic
dependency update.

## Status summary

| Area | Status | Notes |
| --- | --- | --- |
| glTF 2.0 core | Supported | Lossless JSON document, typed root views, resources, accessors, scenes, animations, skins and materials. Complete cross-reference validation is available with `strict-validation`. |
| Unknown JSON and extensions | Supported | Unknown properties, `extras`, number lexemes and unregistered extension payloads survive parse/edit/write. Untouched JSON is returned byte-for-byte. |
| GLB v2 | Supported | Read, write and validation. |
| GLB v3 draft | Supported | Read/write, `u64` lengths, zero reserved chunk encoding, checked range descriptors and seekable input. Slice APIs remain available for small files. |
| Unified `files` references | Supported | URI and buffer-view payloads, packaged nested assets, explicit loading, quotas, provenance, chain-depth limits and cycle rejection. |
| External assets | Supported | References are exposed by typed views and loaded explicitly. Automatic recursive scene composition is intentionally left to the caller. |
| Shapes | Structural support | Typed shape views, root references and shape type/subobject structure are validated. Exact numeric parameters await a pinned Khronos schema. |
| Bounding volumes / BVH links | Structural support | `node.boundingVolume.shape` is typed and range-checked, and the surrounding node hierarchy retains its normal reference checks. Transform and shape-parameter semantics are not claimed yet. |
| UIDs | Partial semantic support | UIDs are exposed, must be strings, are file-wide unique and may not collide with names. Final character-set rules are deferred until Khronos pins them. |
| Asset thumbnail | Supported | `asset.thumbnail` is exposed and validated as an image index. Image pixel decoding is outside this crate. |
| Single preferred scene | Supported | New assets use one preferred scene; readers retain glTF 2.0 multiple-scene compatibility. |
| Non-sequential attributes | Supported | `TEXCOORD_n` and `COLOR_n` no longer need to start at zero or be consecutive under the draft profile. |
| Expanded component definitions | Supported in the document/raw path | `i32`, `f16`, `f64`, `i64` and `u64` definitions are parsed and written without normalization or conversion. Individual consumers still enforce their allowed types. |
| Promoted 2.0 extension functionality | Pass-through | Unknown syntax is preserved, but there is no dedicated typed API or required-consumer conformance claim yet for WebP, emissive strength, mesh quantization or node visibility in their future core form. |
| Full final 2.1 schema validation | Not available | Khronos had not published the final 2.1 schema at the pinned snapshot. Validation intentionally avoids speculative rules. |

## Draco behavior

`KHR_draco_mesh_compression` remains governed by its glTF 2.0 extension
contract. Ordinary and Draco primitives both read into `PackedGeometry`.

- Draco decode validates the extension mapping and bitstream before exposing
  geometry.
- Raw writes preserve draft component storage, including 64-bit and floating
  component definitions.
- Draco encode currently accepts triangles and the component layouts supported
  by the KHR contract. `f16`, `f64`, `i64` and `u64` are rejected with a typed
  error; they are never cast or normalized silently.
- Compression and decompression are atomic and preserve unrelated scene data.
  A transform fails when an unknown extension may own binary references that
  cannot be remapped safely.

## API profiles

The default `full` feature includes all document, read, write and Draco
capabilities. Smaller builds can select the same model without a second parser:

```toml
# Ordinary accessor reader
draco-gltf = { version = "0.2", default-features = false, features = ["read"] }

# Ordinary + Draco reader
draco-gltf = { version = "0.2", default-features = false, features = ["read", "draco-decode"] }
```

The browser release uses `gltf-wasm` with `read` and `draco-decode` enabled by
default. Raw writing and Draco encoding remain opt-in features.

## References

- [Khronos: Introducing glTF 2.1 with Complex Scenes](https://www.khronos.org/blog/introducing-gltf-2.1-with-complex-scenes)
- [Khronos glTF repository](https://github.com/KhronosGroup/glTF)
- [Pinned upstream commit](https://github.com/KhronosGroup/glTF/commit/77b44be7bef26e01fb0b140e3d5bb1716421c5e9)
- [glTF 2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html)
- [`KHR_draco_mesh_compression` specification](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_draco_mesh_compression)
- [Snapshot and update policy](GLTF_2_1_SNAPSHOT.md)
- [Short draft overview](GLTF_2_1.md)
- [Crate API and feature guide](README.md)
