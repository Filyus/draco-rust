# draco-gltf-bevy

`KHR_draco_mesh_compression` for Bevy's glTF loader, decoded by
[`draco-gltf`](../../crates/draco-gltf).

**Status: experimental.** Built against Bevy 0.19. `publish = false` until the
questions at the end are settled.

```rust
use bevy::prelude::*;
use draco_gltf_bevy::{draco_loader_settings, DracoGltfPlugin};

App::new()
    .add_plugins((DefaultPlugins, DracoGltfPlugin::default()))
    .add_systems(Startup, |mut commands: Commands, assets: Res<AssetServer>| {
        commands.spawn(WorldAssetRoot(assets.load_with_settings(
            GltfAssetLabel::Scene(0).from_asset("truck.glb"),
            draco_loader_settings,
        )));
    })
    .run();
```

`draco_loader_settings` turns off gltf-rs validation. The reason is that gltf-rs
rejects any file listing an extension it does not implement in
`extensionsRequired`, and it does this before Bevy asks any extension handler.
Files that list Draco only in `extensionsUsed` load without it.

## How it works

The plugin registers a `GltfExtensionHandler`, which is Bevy's own hook for
decompressing primitives. For each primitive that carries the extension:

1. **Decode.** `draco-gltf`'s document-independent entry point
   (`DracoPrimitiveExtension` + `DracoPrimitiveContract`) takes three things:
   the extension object, the bytes of its buffer view, and what the
   primitive's accessors declare. It is the same path
   `Import::read_primitive` takes, so it has the same count checks, limits and
   `normalized` handling, and a test pins that equivalence.
2. **Convert.** The decoded, tightly packed attributes are described to Bevy
   as a one-buffer glTF document, and Bevy's own `convert_attribute` turns
   them into mesh attributes. Semantic mapping, normalized-integer widening,
   custom attributes and coordinate conversion all stay Bevy's, so the
   adapter does not carry a second copy that drifts with each Bevy release.

Details that are easy to get wrong, and are handled here:

- Draco attributes are matched to semantics by **unique id**, not by their
  position in the stream.
- The accessor's `normalized` flag wins over the Draco attribute's own flag,
  which encoders usually leave unset.
- Attributes the primitive lists but the extension does not compress are
  read from their ordinary accessors, as the extension spec requires.
- Morph targets stay uncompressed and are applied only if every target
  accessor has as many values as the stream decoded. Otherwise the stream has
  not kept vertex order, and the targets are dropped with a warning.
- Indices narrow to `u16` when the **vertex** count allows it.
- If a primitive fails to decode, it becomes an empty mesh and an error is
  logged. If the hook returned nothing instead, Bevy would read the
  primitive's placeholder accessors and panic computing flat normals
  (`tests/load.rs` covers this case).
- Registration works in either plugin order. The plugin-wide
  `GltfPlugin::convert_coordinates` default is honoured, not assumed `false`.
- The decode is synchronous on every target. On wasm there is no JavaScript
  decoder, no `spawn_local` and no copy of the payload.

## Why a separate crate, not a `bevy` feature of draco-gltf

- Bevy breaks its API every major release, roughly every three to four months.
  As an optional dependency of `draco-gltf`, each Bevy bump would be a breaking
  release of the codec crate, on Bevy's schedule.
- CI's feature-powerset checks and docs.rs `all-features` would build Bevy's
  ~300 crates for `draco-gltf`.
- `draco-gltf` depends on `draco-core` alone (see `AGENTS.md`).
- Bevy integrations conventionally ship as their own crates.

This crate is its own workspace for the same reason. Nothing under `crates/`
builds or locks Bevy.

## Checks

```sh
cargo fmt --manifest-path integrations/bevy/Cargo.toml -- --check
cargo clippy --manifest-path integrations/bevy/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path integrations/bevy/Cargo.toml
```

## Before merging or publishing

- **Name.** `draco-gltf-bevy` follows this repository. The Bevy ecosystem
  mostly uses `bevy_*`, and `bevy_gltf_draco` is already taken.
- **Bevy version policy.** One crate version per Bevy minor, plus a
  compatibility table.
- **CI job.** Nothing runs these checks in CI yet.
- **Release order.** The crate would be released after `draco-gltf`, and its
  `draco-gltf` requirement must name a published version that carries
  `DracoPrimitiveExtension`.
- **Fixtures.** No fixture has Draco together with morph targets or
  uncompressed extra attributes, so those two paths have no test yet.
