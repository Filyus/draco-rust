# Migrating to draco-gltf 0.2

0.2 exposes `Import::document` as the native lossless `Document`.

- Replace `gltf` iterators with `Document` typed views, for example
  `document.meshes()` and `document.primitive(MeshIndex(0), 0)`.
- `Import::draco_primitives()` yields `PrimitiveRef`, not `(Mesh, Primitive)`.
  Pass that value directly to `Import::decode_primitive`.
- `ImportOptions` now selects `ValidationProfile` and owns an
  `ExtensionRegistry`; the default is the pinned glTF 2.1 draft profile.
- External glTF assets in 2.1 `files` are intentionally not resolved during
  import. Enumerate `external_assets()` and call `load_asset` explicitly.

Unknown fields, `extras`, and unregistered extension JSON remain in `Document`
through parse, edit, and serialization.
