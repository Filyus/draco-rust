# Changelog — draco-gltf

Notable changes to the `draco-gltf` crate. This crate is versioned and released
independently; its release tags are `draco-gltf-vX.Y.Z`. It depends on published
`draco-core` and `draco-io`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- `ImportOptions` carries `draco_decode_limits`
  (`#[cfg(feature = "draco-decode")]`): the caller's
  `draco_core::DecodeLimits` -- ceilings on the points, faces and decoded
  attribute bytes one Draco decode may reconstruct, applied to every primitive
  the import decodes, including `decompress_in_place` and nested assets. The
  defaults match `draco_core::DecodeLimits::default`; a decode past a ceiling
  fails with `ErrorKind::LimitExceeded`, which reaches the caller as
  `Error::Decode` with its kind intact -- the caller's policy refusing a large
  file, distinguishable from the decoder refusing a malformed one. This is
  deliberately a separate knob from `ImportOptions::limits`: those quota
  container resources, these bound reconstructed geometry. The
  `ExtensionHandler::decode_primitive` trait method gained the ceilings as a
  parameter, which is a breaking change for out-of-tree handlers.
- This crate permits `unsafe` in narrow, audited paths, on the same terms as
  `draco-io`, where `SECURITY.md` previously ruled it out for the whole
  workspace at once. Every block must carry a `// SAFETY:` comment naming its
  invariant **and where that invariant was established**;
  `undocumented_unsafe_blocks` is on and CI runs clippy with `-D warnings`, so
  an unjustified block does not build. **No path in the library uses `unsafe`
  today** and nothing that ships changes; what changes is that a measured
  optimisation no longer has to relitigate the policy to land. The split is by
  what the code does rather than by how much its input is trusted — an accessor
  walk reads offsets and strides straight out of a file a hostile caller wrote,
  and it is on this side because each bound is established a line or two from
  the read rather than carried through a decoder's state. `draco-core` keeps the
  rule absolute, with the compiler holding it.

### Added

- `CompressionOptions::quantization` and `QuantizationBits`, which set Draco's
  per-attribute-type quantization for a compressed primitive. `QuantizationBits`
  carries `position`, `normal`, `tex_coord`, `color` and `generic`, each
  `Option<u8>`; `QuantizationBits::GLTF` is Blender's 14/10/12/10/12 and
  `QuantizationBits::NONE` is the default, so existing callers keep the bytes
  they already produce.

  Nothing here quantized before, which cost more than size. An unquantized
  attribute never reaches Draco's integer coder, so no prediction scheme runs on
  it and the entropy stage has nothing to work with: on a 3042-face grid the
  payload was 20,017 bytes with no quantization and 2,334 with these defaults,
  and the encoding speed made no difference to the output at all across 0–9.

### Changed

- `draco-encode` now enables `draco-core/edgebreaker_valence_encode`, matching
  the `edgebreaker_valence_decode` the decode side already had. Without the
  encoder half `select_edgebreaker_traversal` can only answer "standard", so
  every encoding speed below 5 — the whole range where Draco asks for the
  valence traversal — wrote the same stream as speed 5. On the grid above it is
  worth 23% at the default speed, against 1.1 KiB of gzipped WASM.

### Added

- `Import::compress_primitive` accepts `TRIANGLE_STRIP` and `TRIANGLE_FAN`
  source primitives, not only `TRIANGLES`. Draco's connectivity has no
  notion of a strip or a fan, so either is unwound into an ordinary triangle
  list before encoding (`draco_io::decode_geometry`), and the output
  primitive's `mode` is rewritten to `TRIANGLES` to describe the Draco
  stream truthfully -- left untouched when the source was already
  `TRIANGLES`. Previously any mode other than `TRIANGLES` was refused.

### Fixed

- `PackedGeometry::from_draco_mesh` no longer trusts the source primitive's
  declared `mode`; it always tags the result `TRIANGLES`. Decoding a Draco
  mesh is always an explicit triangle list by construction, but
  `KHR_draco_mesh_compression`'s own spec text permits a compressed
  primitive to declare `TRIANGLE_STRIP`, and this crate previously carried
  that declared mode straight into the decoded `PackedGeometry` unchanged --
  mislabeling an ordinary triangle list as a strip for any caller that
  trusts `mode`, though not for reference decoders such as three.js's
  `DRACOLoader`, which already ignore the declared mode for a Draco
  primitive for exactly this reason. No known real-world file was found
  triggering this: no mainstream exporter is known to emit
  `TRIANGLE_STRIP` alongside `KHR_draco_mesh_compression`.

## [0.2.0](https://github.com/Filyus/draco-rust/compare/draco-gltf-v0.1.0...draco-gltf-v0.2.0) - 2026-07-29

### Added

- Lossless `Document` typed views and index types for full glTF scenes; unknown
  fields, `extras`, and unregistered extension JSON survive edits and writes.
- Pinned glTF 2.1-draft validation surface, GLB v3 containers, explicit
  `files` asset loading, extension contracts, and packed geometry views.
- Document-preserving Draco compression with measured output reporting.
- `Import::to_gltf_output` for portable JSON plus companion buffers, alongside
  GLB v2/v3 serialization through `Import::to_bytes`.
- Bidirectional `PackedGeometry` primitive reads and writes, including minimal
  standalone scenes, raw accessors, explicit Draco storage, and GLB v2/v3.
- `Document::to_minified_json_bytes` for forced whitespace-free output.
- `EXT_meshopt_compression` is decoded on import. An asset processed by
  `gltfpack` points its buffer views at a zero-length fallback buffer and keeps
  the bytes in a compressed range, which the loader used to refuse outright;
  the vertex, index and index-sequence streams and the octahedral, quaternion
  and exponential filters are now decoded into that fallback buffer, leaving
  every accessor read downstream unchanged. GLB output rebases the extension's
  compressed range along with the buffer views it merges, so an export no
  longer depends on the compressed source happening to be written first.
- Draco compression accepts a document carrying extensions that name no binary
  data. The guard is whole-document -- an unregistered extension refuses the
  file rather than risk rewriting accessor indices inside JSON nobody
  interpreted -- and only the Draco handler was registered, so a material
  saying how a surface is lit blocked compression of 31 of the 70 corpus
  assets. Those specifications are now declared, each entry an assertion that
  the extension owns no binary references. Unknown names still refuse.
- `EXT_mesh_gpu_instancing` and `EXT_structural_metadata` binary references are
  collected and renumbered by the Draco-only compactor, which drops and
  renumbers whatever nothing points at. Previously both were refused, and each
  would have failed differently: instancing accessors no primitive names would
  have looked unreferenced and vanished, while a metadata property-table column
  is a buffer view no accessor can describe, one fixture pointing at a
  zero-length view a compactor treats as empty.
- `EXT_mesh_features` feature IDs survive Draco compression. The extension
  names no accessor or buffer view -- its `featureIds[].attribute` selects
  `_FEATURE_ID_N` by name -- so what it needs is the encoder leaving those
  attributes alone: a quantized feature ID is not an approximate one, it is a
  different one, and nothing downstream could tell. Verified per vertex record
  rather than per attribute, since Draco reorders vertices.
- `PackedAttribute::source_accessor` and `PackedIndices::source_accessor`, with
  matching `with_source_accessor` builders, so a consumer can tell that two
  primitives were materialized from one document accessor — the usual case for
  a mesh split by material. Set on uncompressed reads only; compressed geometry
  leaves it unset, because its bytes come from the codec stream rather than
  from the accessor the attribute names. Equality of packed geometry ignores
  the field: the same bytes read from a different document are the same
  geometry.
- `AccessorData::accessor_type` and
  `DocumentAccessorSource::read_buffer_view` for generic accessor and embedded
  payload consumers, including WebAssembly bindings.
- Domain feature `accessors` for generic matrix-capable accessor
  materialization; primitive geometry reading remains in `read`.
- `strict-validation` feature for complete glTF graph and POSITION-bounds
  validation. The compact `read` profile keeps basic structural checks and
  bounds-safe accessor materialization without paying for the global pass.

### Changed

- **Breaking.** Replaced the previous external document API. Use `Document`,
  typed views, and index types described in `MIGRATING_0_2.md`.
- **Breaking.** Full-scene operations now live in `draco-gltf`; `draco-io`
  provides only container, resource, accessor, and bitstream contracts.
- **Breaking.** `CompressionMode::DracoOnly` is the default and requires Draco;
  use `CompressionMode::Fallback` to retain ordinary geometry for non-Draco
  readers.
- `Import` is the single geometry read/write entry point; feature `read`
  enables ordinary accessors, while Draco decode and encode remain explicit.
- Packed geometry belongs to `draco-gltf`; `draco-io` remains the low-level
  container, resource, accessor and Draco contract layer.
- Extension transform handlers are registered only when the `write` feature
  asks whether a binary transform may touch a document. A reader build used
  the registry only to decode Draco geometry and paid for twenty handlers it
  never consulted: 115.0 KiB of WASM back down to 113.2.

### Fixed

- Building this crate with `--no-default-features` compiles. It never did:
  `document`, `json`, `extensions` and `import` are unconditional here, and
  `Error`, `ImportOptions` and the `draco_io` re-exports all name types from
  `draco-io`'s `gltf-container`, so a featureless build produced a page of
  unresolved imports rather than a smaller crate. That feature is now enabled
  on the dependency itself. The `document` feature still names it, so a caller
  reading the feature graph sees what it always saw.

- A Draco primitive whose accessors declare fewer points than the stream
  decodes is read rather than rejected. Draco stores connectivity per position
  vertex and re-splits it at attribute seams, so a mesh with a normal or UV
  seam decodes to more points than the accessor written before compression
  says -- glTF-Pipeline, Blender and the Draco encoder all emit such files, and
  20 of the 61 Draco primitives in Three.js's `ferrari.glb` disagree this way.
  The upstream C++ decoder returns the same counts, so the geometry is right
  and only the metadata is stale.
- Normalization is read from the glTF accessor, which
  `KHR_draco_mesh_compression` makes authoritative, rather than from the
  decoded Draco attribute. Third-party encoders leave the Draco flag unset, so
  a `COLOR_0` stored as normalized unsigned short arrived as raw `0..65535`
  values, saturated into the base colour and rendered a fully textured model
  flat white. A round trip through this crate's own encoder cannot show the
  disagreement, because it writes the flag into the payload.
- Primitives sharing a document accessor no longer each get their own copy on
  import. Splitting one mesh by material is how most authored assets are built,
  and the importer reads geometry per primitive already materialized, so the
  bytes alone could not reveal the sharing: `DuplicateMeshes` turned 35
  references to 5 accessors into 35 copies, in the document, in every GLB
  written from it, and in every FBX built from it.
- Draco compression now derives accessor counts, component layouts, and
  `POSITION` bounds from the encoded topology, including when the encoder drops
  unused points. Decode rejects accessor counts that disagree with the stream.
- Strict validation requires finite, ordered three-component bounds on
  `POSITION` accessors.
- Draco compression and decompression are atomic; shared accessors, unknown
  JSON, registered extension references, and retained scene resources are
  preserved safely.
- Strict KHR Draco validation checks extension lists, primitive modes,
  attribute mappings, unique IDs, and Draco-only accessor layouts.
- Binary range arithmetic, resource quotas, output limits, and compaction use
  checked operations; overlapping retained ranges are coalesced.
- Raw primitive writes emit exact `POSITION` bounds, validate topology and
  well-known attribute layouts, and reject incompatible morph targets before
  mutating the document.

## [0.1.0] - 2026-06-24

### Added

- Load and save full glTF scenes with Draco-compressed geometry.
- glTF/GLB import, explicit Draco decoding, and materialization into ordinary
  geometry.
- `compress` to Draco-compress a full scene while preserving materials,
  textures, nodes, animations, skins, and unknown extensions.
- Draco-aware, panic-safe validation on import.
- WebAssembly support, and an optional `image` feature to shrink the build when
  texture pixels are not needed.
