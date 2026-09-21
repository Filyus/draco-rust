# Web converter changelog

The converter and its WASM wrappers are not published to crates.io: every
wrapper ships as a zipped release asset on whatever crate release builds them
(stamped with that crate's version, since the set travels together), and the
converter itself deploys to GitHub Pages from `main`. This file records what
has changed between those shippings. When a release is prepared, the
`Unreleased` section is copied into that release's notes — the release page is
what the public reads — and renamed here to the shipping date, which is the
only anchor a shipping without a version of its own has.

## Unreleased

- `create_drc` takes a `point_cloud` option, which writes the input through
  Draco's point-cloud coder instead of the mesh one. Off by default, since a
  caller passing no indices today gets a mesh with no faces and switching that
  silently would change the geometry type its decoder reports. It is what a
  file whose payload is per-point attributes actually is -- a scan, or a
  Gaussian splat read with the PLY reader's generic attributes turned on.

- `create_drc` takes the two point-cloud encoder options as well:
  `prediction_search` lets the encoder choose each attribute's prediction
  scheme by the estimated cost of the candidates instead of by upstream's
  fixed rule, and `spatial_point_order` emits the points in Morton order so
  the difference predictor has a spatial neighbour to predict from. Both act
  on the point-cloud coder only, so they need `point_cloud` on, and both are
  off by default because the module's output is otherwise byte-identical to
  C++ Draco's for the same input. Together they take a Gaussian splat scene
  from 53.02 to 45.43 bytes per point and a photogrammetry capture of eight
  million coloured points from 6.26 to 4.23. The order one reorders the
  decoded points, which matters to anything outside the file that indexes into
  it by point number, and it can make a file bigger when an attribute varies
  along the order it came in rather than through space.

- `parse_ply_bytes` reports the vertex properties, face properties and whole
  elements the reader could not carry, through the `warnings` array it already
  returned. A Gaussian-splat PLY keeps everything but the position in
  properties this reader has no attribute for, so it parsed into a bare point
  cloud and said only that it had succeeded. Until now the array was filled
  only by the fallback path for a malformed file, which meant a broken file
  was explained and an intact one that lost most of its payload was not.
- The FBX preview draws every node's own geometry again. A geometry FBX
  carries without polygons — a curve, a lattice — was dropped from the mesh
  list but still counted by the walk that numbers them, so every node past the
  first such geometry drew its neighbour's mesh, with its material and morph
  targets shifted to match. One file of 501 geometries showed as a fragment of
  the model with the rest scattered.
- A file the selection did not hold is reported with the gesture that supplies
  it, not only by name: the folder for a single-file selection, and for FBX the
  reason a file that *was* supplied may still not have matched — its texture
  paths belong to the machine that authored the file, so only the name is
  compared. The glTF parse error no longer repeats that advice.
- `ktx2-wasm` names the single- and two-channel transcode targets the crate
  gained — `bc4`, `bc5`, `eac_r11`, `eac_rg11` — so a viewer can take a
  normal map or a channel mask in the format drawn for it.
- The viewer's format choice ranks a texture every material samples through
  `normalTexture` as a normal map and takes BC5 on the desktop family and
  EAC RG11 on the mobile one, with the color ranking as the fallback.

## 2026-09-05

- Added source-neutral SceneDocument capability reporting and FBX/glTF
  hierarchy UI; verified Mixamo, Samba Dancing, and Fox conversion controls.
- Preserved extra UV sets and up to eight skin influences through the
  SceneDocument GLB/typed-FBX paths, with explicit diagnostics for viewer and
  typed-FBX writer limitations. See [`web/README.md`](web/README.md).
- Added STL and standalone Draco (`.drc`) as import **and** export formats,
  through the new `stl-wasm` and `drc-wasm` modules. `stl-wasm` wraps
  `draco-io`'s reader and writer; `drc-wasm` wraps `draco-core` directly, the
  first web module to depend on neither `draco-io` nor `draco-gltf`.
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
