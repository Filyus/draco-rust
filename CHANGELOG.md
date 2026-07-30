# Changelog

The publishable crates are versioned and released **independently**, each with
its own changelog and its own `<crate>-vX.Y.Z` release tags:

- **draco-core** — [`crates/draco-core/CHANGELOG.md`](crates/draco-core/CHANGELOG.md)
- **draco-io** — [`crates/draco-io/CHANGELOG.md`](crates/draco-io/CHANGELOG.md)
- **draco-gltf** — [`crates/draco-gltf/CHANGELOG.md`](crates/draco-gltf/CHANGELOG.md)

The `web/` WASM wrappers and converter are not published to crates.io. Each
wrapper ships as a zipped release asset with the crate it wraps, stamped with
that crate's version and built by `Release: WASM assets`: obj, ply, stl and fbx
with `draco-io`, gltf with `draco-gltf`, drc with `draco-core`. The converter
itself is deployed to GitHub Pages by `Pages: deploy converter`, from `main`
rather than from a tag — it demonstrates the current code, and pinning it to a
crate release would show neither crate's version honestly.

**draco-io v0.3.0 and draco-gltf v0.2.0 carry no browser assets.** The tag those
releases were made from was pushed with `GITHUB_TOKEN`, for which GitHub raises
no workflow events, so `Release: WASM assets` never ran for either. Both crates
are on crates.io as normal; only the zipped WASM wrappers are absent. Fixed for
subsequent releases — the publish workflow now starts that run itself — and
those two are not being backfilled, since a release's assets should be the ones
its own pipeline produced.

See [`RELEASING.md`](RELEASING.md) for the release process.

## Unreleased web converter

- Added source-neutral SceneDocument capability reporting and FBX/glTF
  hierarchy UI; verified Mixamo, Samba Dancing, and Fox conversion controls.
- Preserved extra UV sets and up to eight skin influences through the
  SceneDocument GLB/typed-FBX paths, with explicit diagnostics for viewer and
  typed-FBX writer limitations. See [`web/README.md`](web/README.md).
- Added STL and standalone Draco (`.drc`) as import **and** export formats,
  through the new `stl-wasm` and `drc-wasm` modules. `stl-wasm` wraps
  `draco-io`'s reader and writer; `drc-wasm` wraps `draco-core` directly, which
  makes it the first web module to ship with a `draco-core` release rather than
  a `draco-io` one.
- Carried a `.drc`'s attributes through a round trip instead of dropping the ones
  the flat mesh has no slot for. A second texture-coordinate or colour set is
  handed over with its type, component count, component type and unique id, and
  put back unchanged; a consumer's ids survive. Nothing in between reads their
  meaning, and the converter reports each one as carried but uninterpreted.
  Where another format has a name for one it gets it — `TEXCOORD_1`, `COLOR_1`
  into glTF — and a generic is reported as left behind rather than invented into
  an `_NAME`.
- Gave the flat formats (OBJ, PLY, STL, `.drc`) a route to glTF and GLB through
  the portable SceneDocument, and JSON glTF a route from every source. Both were
  previously unreachable.
- Baked node placement into flattened exports, so a multi-node scene exported to
  a flat format no longer collapses every object onto the origin. Vertex colours
  and PLY texture coordinates survive those exports too, and the reported Draco
  `Method` is the encoder's own rather than a guess.
- Read FBX `UnitScaleFactor` and the six axis fields instead of assuming
  centimetres and Y-up, made the export space a choice (`meters-y-up` by
  default, `meters-z-up` available), and turned V at every crossing rather than
  one. Both directions and both declarations now come from a single space, and
  Blender is the external oracle for it. See [`web/README.md`](web/README.md).
- Cleared compression statistics and scene fields on import, so a panel never
  describes the previous model.
