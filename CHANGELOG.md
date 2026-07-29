# Changelog

The publishable crates are versioned and released **independently**, each with
its own changelog and its own `<crate>-vX.Y.Z` release tags:

- **draco-core** — [`crates/draco-core/CHANGELOG.md`](crates/draco-core/CHANGELOG.md)
- **draco-io** — [`crates/draco-io/CHANGELOG.md`](crates/draco-io/CHANGELOG.md)
- **draco-gltf** — [`crates/draco-gltf/CHANGELOG.md`](crates/draco-gltf/CHANGELOG.md)

The `web/` WASM wrappers and converter are not published to crates.io. The
wrappers ship as zipped release assets, built by `Release: WASM assets` from
each `draco-io` and `draco-gltf` tag. The converter itself is deployed to GitHub
Pages by `Pages: deploy converter`, from `main` rather than from a tag — it
demonstrates the current code, and pinning it to a crate release would show
neither crate's version honestly.

See [`RELEASING.md`](RELEASING.md) for the release process.

## Unreleased web converter

- Added source-neutral SceneDocument capability reporting and FBX/glTF
  hierarchy UI; verified Mixamo, Samba Dancing, and Fox conversion controls.
- Preserved extra UV sets and up to eight skin influences through the
  SceneDocument GLB/typed-FBX paths, with explicit diagnostics for viewer and
  typed-FBX writer limitations. See [`web/README.md`](web/README.md).
