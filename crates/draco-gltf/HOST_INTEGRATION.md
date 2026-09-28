# Hosts with their own document model

An engine that parsed a file with another glTF reader and loaded its buffers
through its own asset system needs only the codec from this crate. This page
covers that use: decoding and encoding one `KHR_draco_mesh_compression`
primitive without `Import` and without this crate's document.

Everything here works on the host's own values: the extension object as JSON,
the bytes of a buffer view, and what the primitive's accessors declare. The
crate never sees the rest of the file.

## Decoding

Needs feature `draco-decode`.

1. Parse the primitive's `KHR_draco_mesh_compression` object with
   `DracoPrimitiveExtension::from_json`. It checks the object's shape only.
2. Describe the accessors the primitive declares in a
   `DracoPrimitiveContract`: `with_attribute(semantic, count, normalized)`,
   `with_indices(count)`, and optionally `with_mode`, `with_limits` and
   `with_profile`.
3. Call `DracoPrimitiveExtension::decode(payload, &contract)`, where `payload`
   is the byte range of `extension.buffer_view()`.

List every accessor the primitive names, compressed or not: the contract is
checked against the stream, and a count the stream cannot supply is refused.

The result holds the compressed attributes only, as a `u32`-indexed triangle
list with each accessor's `normalized` flag applied. The specification has a
primitive keep further attributes outside the extension, and a loader must read
those as ordinary accessors. The host does that itself; `Import::read_primitive`
does it for a document held here and appends them after the compressed ones.

## Encoding

Needs feature `draco-encode`.

`DracoPrimitiveEncoding::encode(&geometry, &options)` takes a `PackedGeometry`
of `TRIANGLES`, `TRIANGLE_STRIP` or `TRIANGLE_FAN`, and returns what the host
has to store. Strips and fans become triangle lists, so the host writes `mode`
as `TRIANGLES`. `options.mode` is ignored, and `max_output_bytes` caps the
bitstream.

- **Bitstream and extension.** `bytes()` (or `into_bytes()`) goes into a buffer
  view. `extension(buffer_view)` is the object to write under the primitive's
  `KHR_draco_mesh_compression` key, and `to_json()` serializes it.
- **Accessor declarations.** Declare the attribute accessors from `accessors()`
  and the index accessor from `index_count()`, whose type is `UNSIGNED_INT`.
  Each `DracoAccessor` carries `semantic`, `count`, `components`,
  `accessor_type`, `component_type`, `normalized` and, for POSITION, `bounds`.
  Do not take them from the input. The encoder may merge duplicate points, so
  the stream can hold fewer vertices than the input, and POSITION bounds are
  the quantized ones. A count that disagrees with the stream describes a file
  that does not hold that data.
- **Fallback.** To keep an uncompressed copy, write it from `decoded()`, the
  stream read back, not from the input. The specification asks for fallback
  data "decompressed from the Draco buffer", and Draco merges, splits and
  reorders vertices, so a copy made from the input can disagree with the
  stream. Readers without Draco then see the geometry readers with it see.
  Without a fallback, list the extension in `extensionsRequired`.
- **Morph targets.** Targets index the vertices, and EdgeBreaker renumbers
  them. A primitive with targets stays valid only if every vertex decodes at
  its own index, which `keeps_vertex_order(&input)` reports. Set
  `encoding_method` to `1` (sequential) for such a primitive rather than let
  EdgeBreaker choose, and refuse the primitive if `keeps_vertex_order` is still
  `false`. `Import::compress_primitive` does exactly this.

Quantization, speeds and the connectivity coder are the same
`CompressionOptions` `Import::compress_primitive` takes.

## Example

Encoding a triangle, then decoding it the way a host reads a file: the
extension object and the declared counts come from what the host stored.

```rust
use draco_gltf::{
    ComponentType, CompressionOptions, DracoPrimitiveContract, DracoPrimitiveEncoding,
    DracoPrimitiveExtension, PackedAttribute, PackedGeometry, PackedIndices, PrimitiveMode,
};

let corners: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
let bytes = corners.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
let position = PackedAttribute::new("POSITION", 3, 3, ComponentType::F32, false, bytes)?;
let indices = PackedIndices::new(3, ComponentType::U8, vec![0, 1, 2])?;
let geometry = PackedGeometry::new(PrimitiveMode::Triangles, vec![position], Some(indices))?;

let encoded = DracoPrimitiveEncoding::encode(&geometry, &CompressionOptions::default())?;

// The host stores `encoded.bytes()` in buffer view 0 and writes this under
// the primitive's `KHR_draco_mesh_compression` key.
let _extension = encoded.extension(0).to_json();

// Accessors declare what the stream holds, not what went in.
let declared = encoded.accessors()[0].count();
assert_eq!(declared, encoded.decoded()?.vertex_count());

// Decoding. A host reads the extension object out of its own document and
// the bytes out of its own buffer view, and states what the accessors declare.
let extension = DracoPrimitiveExtension::from_json(&encoded.extension(0).to_json())?;
let contract = DracoPrimitiveContract::new()
    .with_attribute("POSITION", declared as u64, false)
    .with_indices(encoded.index_count() as u64);
let geometry = extension.decode(encoded.bytes(), &contract)?;
assert_eq!(geometry.vertex_count(), declared);
# Ok::<(), Box<dyn std::error::Error>>(())
```
