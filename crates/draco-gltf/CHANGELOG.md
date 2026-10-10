# Changelog — draco-gltf

Notable changes to the `draco-gltf` crate. This crate is versioned and released
independently; its release tags are `draco-gltf-vX.Y.Z`. It depends on published
`draco-core`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.0](https://github.com/Filyus/draco-rust/compare/draco-gltf-v0.6.0...draco-gltf-v0.7.0) - 2026-10-10

### Added

- **`Import::read_primitives` reads several primitives at once, side by side
  on threads.** It returns what calling `read_primitive` for each would, in
  the order asked, and on a failure the error of the first primitive that
  fails.
  - Every Draco primitive is a stream of its own, so they decode without
    waiting on one another. `ImportOptions::draco_decode_threads` sets how
    many decode at once, and each takes one thread. One by default.
  - The document is validated once for the whole call instead of once a
    primitive, which makes it faster than a loop of `read_primitive` even on
    one thread.
  - Measured, best of five on a laptop with 16 hardware threads:

    | scene | primitives | loop of `read_primitive` | 1 thread | 8 threads |
    |---|---:|---:|---:|---:|
    | city scene, Draco by this crate | 61 | 194 ms | 191 ms | 56 ms |
    | car model, Draco by this crate | 81 | 159 ms | 161 ms | 56 ms |
    | VirtualCity, Draco by glTF-Transform | 167 | 54.5 ms | 3.8 ms | 1.9 ms |
    | BrainStem, Draco by glTF-Transform | 59 | 17.6 ms | 13.1 ms | 3.7 ms |

  - Up to that many decodes are in flight at once, each held to the Draco
    ceilings on its own. WebAssembly reads on the calling thread.
- **`Document::as_json` reads the document without building a tree.** It
  returns a `JsonRef`, a copyable position in the parsed document.
  - It reads the way `JsonValue` does, with `get`, `as_str`, `as_u64`,
    `as_f64`, `as_array`, `as_object` and `is_object`, and `at` in place of
    indexing: a key or a position, reading as null where there is nothing,
    as `[]` on the tree does.
  - Beyond `JsonValue` it has `as_bool`, `as_number` (the lexeme as written),
    `is_null`, `is_array`, and `pointer`, which resolves a JSON Pointer
    (RFC 6901), the way `KHR_animation_pointer` names the property an
    animation drives: `root.pointer("/nodes/0/translation")`.
  - `to_value` copies a subtree out as a tree and `to_vec` serializes one.
    Arrays (`JsonArray`) reach an item by position in one step, and objects
    (`JsonObject`) iterate their members in document order. Both iterate
    from either end and print with `{:?}`.

### Changed

- **Breaking: the typed views return `JsonRef` where they returned
  `&JsonValue`.** This covers `value()` on every view, `extras()`,
  `Shape::definition` and `PrimitiveRef::extension`. `extensions()` and
  `PrimitiveRef::attributes` return `Option<JsonObject>`, `morph_targets`
  iterates `JsonObject`s, and `meshopt_extension` takes and returns `JsonRef`.
  Indexing is `at`, which carries on as null where there is nothing, as `[]`
  on the tree does: `value["meshes"][0]["name"]` becomes
  `value.at("meshes").at(0).at("name")`. `get` is the form that returns an
  `Option`. `Document::as_value` still
  returns the whole tree, which it now builds on first use and keeps.
  - Why: the document is parsed into one flat table of values with strings
    and numbers left in the source text, rather than into a tree with a heap
    block for every key, string, number and container. A tree is built only
    when the document is edited (`as_value_mut`) or asked for (`as_value`).
  - Opening VirtualCity, a scene with 490 kB of JSON, takes 1.1 ms in
    WebAssembly instead of 1.86, and the JSON parse takes 0.44 ms natively
    instead of 2.3. Strict validation is about 1.4x slower natively than on
    the tree, which leaves parse plus validation 3.5x faster.
  - Output is unchanged: an untouched document still writes its source bytes,
    and an edited one writes the same minified JSON as before, twice as fast:
    VirtualCity's edited JSON writes in 0.17 ms instead of 0.34, because runs
    of a string with nothing to escape are copied rather than written a
    character at a time.
- **Breaking: Draco point clouds need the new `draco-point-cloud-decode`
  feature.** `KHR_draco_mesh_compression` allows only `TRIANGLES` and
  `TRIANGLE_STRIP` primitives, decoded as meshes, and upstream's glTF decoder
  is built without point clouds. `draco-decode` no longer reads a point-cloud
  stream; with `draco-point-cloud-decode`, which `full` and so the default
  features include, it does as before, on several threads if asked. Without
  it such a primitive fails with the decoder's error saying point-cloud
  support is disabled. Leaving it out takes 11% off a WebAssembly reader.
- **`DracoExtension` opts into binary transforms only with the `write`
  feature.** Without it, the builds that cannot run a binary transform, its
  `allows_binary_transform` is false and it keeps and remaps no references,
  like any handler that does not opt in. A reader therefore carries no code
  for editing a document as a tree.
- **Breaking: `Import::document` is no longer a public field.** Read the
  document with `Import::document()`, change it with `Import::document_mut()`,
  and take it out with `Import::into_document()`.
  - Why: the import now remembers that its document passed validation, and
    `read_primitive`, `read_primitives` and the Draco decodes skip validating it
    again until it changes. A public field let it change without the import
    knowing; `document_mut` is the one way to change it, and the next such read
    validates the document as it is then.
  - An import is marked validated when it is parsed, so reading primitives one
    call at a time no longer validates the whole document on every call --
    which, with `strict-validation`, was most of the cost of reading a scene of
    many small primitives that way.
  - `Import::validate` itself still validates on every call, against whatever
    registry it is given.
- **Draft-profile validation no longer formats a location for every object.**
  The `uid` checks of `strict-validation` collect UIDs first and stop when a
  document has none; refusals name the same objects as before.
- **`Import::decompress_in_place` decodes primitives side by side** on the
  threads `ImportOptions::draco_decode_threads` allows, a batch of that many
  at a time, so no more decoded geometry is held at once than there are
  threads. The file it writes is the same on any count.

### Fixed

- **A `\u` escape takes four hex digits and nothing else.** The parser read
  `"\u+041"` as `"A"`, accepting the sign `u16::from_str_radix` allows; JSON
  does not, and the escape is now refused.

## [0.6.0](https://github.com/Filyus/draco-rust/compare/draco-gltf-v0.5.0...draco-gltf-v0.6.0) - 2026-10-04

### Added

- **Draco point-cloud primitives can decode on several threads. Off by
  default.** `ImportOptions::draco_decode_threads` takes the thread count for
  one Draco decode:
  - `1` (default): the calling thread only, as before.
  - `0`: as many threads as the machine has, up to 16.
  - Any other count: that many threads, up to 16.

  A large cloud of many attributes decodes three to four times faster on 16
  threads. Mesh primitives decode on the calling thread whatever the count,
  and the decoded geometry is the same on any count.
  `DracoPrimitiveContract::with_threads` does the same for a primitive decoded
  on its own.
- **`DracoDecodeOptions` holds the ceilings and the threads of a Draco decode
  in one value**, built from `Default` with `with_limits` and `with_threads`.
- **Accessor sources can be held to the import's resource limits.**
  `Import::accessor_source` returns an accessor source held to the
  `ResourceLimits` the import was read with, and
  `DocumentAccessorSource::with_limits` does the same for one built by hand.
  `read_primitive` and the geometry readers use the former.

### Changed

- **Breaking: decoding a Draco primitive takes `&DracoDecodeOptions` instead of
  `&DecodeLimits`.** This affects `ExtensionHandler::decode_primitive`,
  `ExtensionRegistry::decode_primitive` and `parse_with_options`. For the old
  behaviour pass `&DracoDecodeOptions::default().with_limits(limits)`. It is
  one value so that what a decode is told can grow again without these
  changing shape.
- **Breaking: `ImportOptions` has a new field**, so a struct literal of it
  needs `..ImportOptions::default()`.
- **Requires draco-core 2.3.0**, which adds the threaded decode.

### Fixed

- **An accessor reads its own buffer view and nothing past it.** Only the end
  of the buffer was checked, so an accessor whose `count` ran past its view's
  `byteLength` returned the next view's bytes as its data. It is refused now,
  and so is a sparse accessor's index or value that runs past its view.
- **A sparse accessor with no buffer view no longer aborts the process.** Its
  zeros are backed by nothing in the file, and a `count` asking for more memory
  than there is aborted the process. It now reports a failed allocation. An
  accessor with a view has its last element checked against the view before
  anything is reserved for it.
  - On Linux that alone is not enough. The system grants a reservation up to
    its RAM and swap, and kills the process when the zeros are written if a
    container allows less: 14 GiB reserved under a 13.4 GiB cgroup, then
    SIGKILL. So those zeros are also held to
    `ResourceLimits::max_resource_bytes` when the import sets it, and refused
    past it before anything is reserved.
- **A Draco extension's buffer view with a `byteOffset` past 4 GiB is refused
  on a 32-bit target**, WebAssembly included, instead of being truncated to an
  offset that happened to fit.

## [0.5.0](https://github.com/Filyus/draco-rust/compare/draco-gltf-v0.4.2...draco-gltf-v0.5.0) - 2026-09-29

Follows the glTF 2.1 draft as it stands on Khronos's `draft-2.1` branch. GLB
version 3 files written by earlier versions no longer read, and the buffer
resolution API changed. Both are under Changed.

### Added

- A GLB version 3 buffer can name the chunk that holds it with `chunk`, so a
  file can carry several BIN chunks. The index counts every chunk in the file
  from zero. Buffer 0 with neither `chunk` nor `uri` still uses the chunk at
  index 1, as in glTF 2.0. Naming a chunk that is not a BIN chunk, naming both
  a chunk and a URI, and naming a chunk in a file that is not a GLB are
  errors. Writing still puts every buffer in one chunk.
- `GlbBinChunk` and `GltfContainer::bin_chunks` list the BIN chunks of a
  container with their indices. `GltfBufferReference::chunk` carries a
  buffer's `chunk`.
- A `files` entry can list `aliases`. A nested glTF loaded from it reads a URI
  that matches an alias exactly from the file the alias names, a buffer view or
  a URI, before asking the caller's resolver. `File::aliases` lists them and
  validation checks their shape. Aliases are not inherited by files nested
  deeper.
- With `strict-validation`, shape parameters are checked against the draft's
  schema and degenerate shapes are refused. A core shape must not carry another
  core shape's parameters. A bounding volume's `rotation`, `scale` and
  `translation` must be arrays of the right length, with `rotation` a
  quaternion in range.
- `KHR_materials_diffuse_transmission`, `KHR_materials_retroreflection`,
  `KHR_node_visibility`, `KHR_node_hoverability` and `KHR_node_selectability`
  no longer block Draco compression of a document that uses them. They hold no
  accessor or buffer view, so compression has nothing to remap. Before, one of
  them anywhere in a file refused the whole file.

### Changed

- Breaking: GLB version 3 files use the chunk layout the draft defines. A chunk
  header is the type, the encoding and then the 64-bit length, and every chunk
  starts on an 8-byte boundary. This crate had the length first and padded
  chunks to 4, so a file written by an earlier version has the old layout and
  no longer reads, and a file from another writer of the draft now does.
  Version 2 is unchanged.
- Breaking: `resolve_gltf_buffers` takes the BIN chunks as a `&[GlbBinChunk]`
  instead of an `Option<&[u8]>`. `GltfContainer` is no longer `Copy`, and
  `GltfBufferReference` has the new `chunk` field.
- A version 3 file may have chunks of unknown type anywhere, more than one JSON
  chunk (the first is the glTF JSON, wherever it sits) and BIN chunks in any
  position. Its BIN chunk may be up to 7 bytes longer than the buffer it holds,
  the padding to 8. Version 2 keeps its limit of 3.
- A nested glTF embedded in a buffer view no longer finds sibling files by
  their `name`. The draft redirects URIs with `aliases` on the file that
  contains them and gives `name` no such meaning. Files that relied on names
  need an `aliases` entry for each URI.
- A core shape's own parameter object is optional, as in the draft, and its
  parameters take their defaults. A `{"type": "box"}` with no `box` object was
  refused before.

### Fixed

- With `strict-validation`, a `uid` equal to a name is a conflict with every
  other object of that name. When two objects shared a name only the last was
  compared, so a `uid` equal to the earlier one passed.

## [0.4.2](https://github.com/Filyus/draco-rust/compare/draco-gltf-v0.4.1...draco-gltf-v0.4.2) - 2026-09-29

Requires `draco-core` 2.2.1. Earlier versions can report wrong point and face
counts, and `DracoOnly` writes its accessor counts from that report.

### Added

- `DracoPrimitiveExtension` and `DracoPrimitiveContract`: decode a
  `KHR_draco_mesh_compression` primitive for a host that parsed the file with
  another glTF reader and loaded the buffers through its own asset system,
  without re-parsing the document here. `Import::read_primitive` and
  `Import::decode_draco_primitive` go through the same path, so all of them
  give identical output.
- `DracoPrimitiveEncoding` and `DracoAccessor`: the encoding counterpart. It
  takes a `PackedGeometry` and returns the bitstream, the extension object for
  the buffer view the host stores it in, and what each accessor has to declare
  (count, type, component type, `normalized`, POSITION bounds) plus the index
  count. The declarations come from the encoder's report, since the encoder
  may merge duplicate points. `decoded()` gives the data for the host's own
  fallback and `keeps_vertex_order` tells a host with morph targets whether
  its vertices survive. `DracoPrimitiveExtension::to_json` writes the
  extension object. `HOST_INTEGRATION.md` walks through both directions,
  including what a host must write from the stream and not from its input.

### Fixed

- `Fallback` compression writes the uncompressed accessors from the stream
  decoded back, as the extension requires, instead of leaving the source
  accessors in place. Where the encoder merged vertices, the source accessors
  no longer matched the stream, and a reader with Draco and one without saw
  different geometry. The cost is the decode and the copy: `Fallback` is about
  1.2 to 1.4 times slower, `DracoOnly` is unchanged.
- A primitive with morph targets is encoded sequentially and refused if its
  vertices do not decode at their own indices, or if EdgeBreaker was forced.
  EdgeBreaker renumbers vertices, which left every Draco reader animating the
  wrong ones.
- `Import::read_primitive` also returns the attributes a Draco primitive
  keeps outside the extension, which the extension says a loader must read as
  usual. Before, `decompress_in_place` silently dropped them. A count that
  disagrees with the stream is an error.
- Compressing a primitive whose POSITION holds an infinity is refused. The
  document it wrote carried non-finite bounds, which glTF forbids and this
  crate's own import rejected.

## [0.4.1](https://github.com/Filyus/draco-rust/compare/draco-gltf-v0.4.0...draco-gltf-v0.4.1) - 2026-09-25

### Added

- `KHR_gaussian_splatting` is in `BINARY_FREE_EXTENSIONS`, so a file that
  mixes Gaussian splats with ordinary meshes can now be Draco-compressed.
  Before, one splat primitive refused compression for the whole file. The
  splat primitives themselves are left uncompressed (they are `POINTS`).
  Compression extensions layered on top, such as SPZ, still refuse.

## [0.4.0](https://github.com/Filyus/draco-rust/compare/draco-gltf-v0.3.0...draco-gltf-v0.4.0) - 2026-09-15

A breaking release that takes ownership of glTF whole. The container parser,
resource resolution, accessor materialization and the `EXT_meshopt_compression`
decoders lived in `draco-io` and this crate wrapped them; they are here now, and
the dependency on `draco-io` is gone.

glTF is the one format in the workspace that embeds a Draco bitstream, which is
what made this the right side of the line: everything from the GLB header up to
the scene document is one format's concern, and `draco-io` covers the formats
that never meet the codec. The two crates no longer constrain each other's
versions, so an FBX change cannot force a release here.

### Added

- `container`, `geometry` and `meshopt` modules, and the `GltfError` they
  share, moved from `draco-io` unchanged. Every name is re-exported from the
  crate root under the spelling it had there, so a caller changes the crate in
  the path and nothing else: `draco_io::parse_gltf_container` becomes
  `draco_gltf::parse_gltf_container`, `draco_io::decode_geometry` becomes
  `draco_gltf::decode_geometry`, and so on. `GlbRangeReader`, `ResourceLimits`,
  `ResourceResolver`, `ExternalFilePolicy`, `FileResourceResolver`,
  `GltfContainerFormat` and `GltfError` were already re-exported here and keep
  their paths exactly.
- `legacy-bitstream-decode`, which was reached through `draco-io` before.

### Changed

- **Breaking.** `Error::DracoIo` is `Error::Container`, and its message reads
  `container error:` rather than `draco-io error:`. The crate it named is no
  longer a dependency.
- Mesh construction ends through `draco_core::mesh::Mesh::finalize`, which is
  what raises the `draco-core` requirement to 2.1.0. Same steps in the same
  order; decoded geometry is unchanged.
- The manifest declares `rust-version = "1.88.0"`, which is the toolchain this
  crate already required. Cargo now says so before the build does, and a CI job
  holds the floor so it cannot drift upward unnoticed.
- `document` no longer enables anything. It named `draco-io/gltf-container`,
  which was never optional in practice -- the document, JSON, extension and
  import modules are unconditional and all name types from it. The feature
  stays so the graph reads correctly for a caller enabling features one at a
  time, but turning it off compiles no less than leaving it on.

## [0.3.0](https://github.com/Filyus/draco-rust/compare/draco-gltf-v0.2.0...draco-gltf-v0.3.0) - 2026-09-05

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

- `Import::compress_primitive` accepts `TRIANGLE_STRIP` and `TRIANGLE_FAN`
  source primitives, not only `TRIANGLES`. Draco's connectivity has no
  notion of a strip or a fan, so either is unwound into an ordinary triangle
  list before encoding (`draco_io::decode_geometry`), and the output
  primitive's `mode` is rewritten to `TRIANGLES` to describe the Draco
  stream truthfully -- left untouched when the source was already
  `TRIANGLES`. Previously any mode other than `TRIANGLES` was refused.

### Changed

- The JSON parser no longer recurses, and no longer caps how deeply a document
  may nest. Parsing, serializing, cloning, comparing and dropping a value each
  carry the nesting on an explicit heap stack, so depth is bounded by what the
  document pays for -- a level cannot be opened without spending an input byte
  on its bracket -- rather than by the call stack a ceiling had to protect. The
  Draco-only safety walk over the document is iterative for the same reason.
  Documents an authoring tool nests past the old 128-level limit are accepted;
  everything the parser accepted before parses identically, with the same error
  text where it does not.

  `JsonValue` now implements `Drop`, so a payload can no longer be moved out of
  it by pattern matching (E0509) -- a breaking change for code that destructured
  a value by value. Use the new `into_array`, `into_object` and `into_string`
  instead.

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

- `draco-encode` now enables `draco-core/edgebreaker_valence_encode`, matching
  the `edgebreaker_valence_decode` the decode side already had. Without the
  encoder half `select_edgebreaker_traversal` can only answer "standard", so
  every encoding speed below 5 — the whole range where Draco asks for the
  valence traversal — wrote the same stream as speed 5. On the grid above it is
  worth 23% at the default speed, against 1.1 KiB of gzipped WASM.

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
