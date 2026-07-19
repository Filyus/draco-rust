# draco-gltf

Lossless glTF 2.0 and pinned 2.1-draft scene handling with optional
`KHR_draco_mesh_compression` geometry.

```rust,no_run
use draco_gltf::{MeshIndex, PrimitiveIndex};

let scene = draco_gltf::import("model.glb")?;
let geometry = scene.read_primitive(PrimitiveIndex::new(MeshIndex(0), 0))?;
println!("{} vertices", geometry.vertex_count());
# Ok::<(), draco_gltf::Error>(())
```

`Import` owns a lossless `Document`, resolved resources and container
provenance. Typed views cover complete scenes, while unknown properties,
`extras`, extension JSON and draft fields survive edits and serialization.
`Document::to_json_bytes` retains untouched source bytes;
`to_minified_json_bytes` explicitly emits whitespace-free JSON while retaining
object order and number lexemes.

`PackedGeometry` is the shared primitive boundary for read and write APIs.
`Import::read_primitive` reads ordinary accessors or decodes Draco. With
feature `write`, `write_primitive`, `push_primitive`, and `from_geometry` write
the same value back as ordinary accessors. Feature `draco-encode` additionally
allows `GeometryEncoding::Draco`; Draco-only is explicit and fallback storage
must also be selected explicitly.

```rust,no_run
use draco_gltf::{
    GeometryWriteOptions, Import, OutputFormat, ValidationProfile,
};
# let geometry = todo!();
let scene = Import::from_geometry(
    &geometry,
    ValidationProfile::Gltf20,
    GeometryWriteOptions::default(),
)?;
let glb = scene.to_bytes(OutputFormat::GlbV2)?;
# Ok::<(), draco_gltf::Error>(())
```

Raw writing preserves profile-valid glTF 2.1 scalar storage such as `f16`,
`f64`, `i64`, and `u64`. Draco encoding never normalizes or casts unsupported
types: it returns a typed error instead. Primitive writes are atomic, do not
mutate shared accessors in place, and preserve material, extras, morph targets
with compatible counts, and unrelated extensions. Generated `POSITION`
accessors include exact `min`/`max` bounds for every supported scalar type.

For portable `.gltf` output, `Import::to_gltf_output` returns JSON plus named
companion resources. `Import::to_bytes` emits JSON without new companions or a
self-contained GLB v2/v3. Explicit glTF 2.1 `files` loading remains controlled
by the caller and resource limits.

## Features

- `document`: lossless DOM, typed views, containers and serialization.
- `geometry`: accessor materialization and `PackedGeometry`.
- `draco-decode`: `KHR_draco_mesh_compression` decoding.
- `resources`: explicit URI and `files` resolution.
- `scene-validation`: strict scene-reference validation.
- `read`: ordinary primitive reading with resources and validation.
- `accessors`: generic accessor materialization, including matrix accessors.
- `write`: raw geometry construction and document mutation.
- `draco-encode`: Draco writing; depends on `write` and `draco-decode`.
- `full`: the default complete profile.

Use `default-features = false, features = ["read", "draco-decode"]` for a
small Draco-capable reader. Add `write` for raw output or `draco-encode` for
compressed output, and `accessors` when animation, skin, morph, or other
non-primitive payloads must be materialized. Every profile uses the same
document and packed-geometry types; no second parser or scene model exists.

See [`GLTF_2_1_SUPPORT.md`](GLTF_2_1_SUPPORT.md) for the support matrix and
upstream links. `GLTF_2_1_SNAPSHOT.md` records the pinned draft and update
policy.

## License

Apache-2.0.
