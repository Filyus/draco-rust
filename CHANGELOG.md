# Changelog

The publishable crates are versioned and released **independently**, each with
its own changelog and its own `<crate>-vX.Y.Z` release tags:

- **draco-core** — [`crates/draco-core/CHANGELOG.md`](crates/draco-core/CHANGELOG.md)
- **draco-io** — [`crates/draco-io/CHANGELOG.md`](crates/draco-io/CHANGELOG.md)
- **draco-gltf** — [`crates/draco-gltf/CHANGELOG.md`](crates/draco-gltf/CHANGELOG.md)

The `web/` WASM wrappers and demo are not published to crates.io; their release
assets are attached to GitHub Releases.

See [`RELEASING.md`](RELEASING.md) for the release process.

## Unreleased web converter

- Added source-neutral SceneDocument capability reporting and FBX/glTF
  hierarchy UI; verified Mixamo, Samba Dancing, and Fox conversion controls.
- Preserved extra UV sets and up to eight skin influences through the
  SceneDocument GLB/typed-FBX paths, with explicit diagnostics for viewer and
  typed-FBX writer limitations. See [`web/README.md`](web/README.md).
