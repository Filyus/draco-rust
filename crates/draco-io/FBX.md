# FBX in `draco-io`

FBX in more detail than the crate [README](README.md#supported-formats) carries:
what of a scene survives each direction, which versions and containers are
accepted, the limits an untrusted document is held to, and the corpus each claim
is measured against.

Numbers below refer to the [`ufbx`](https://github.com/ufbx/ufbx) corpus, which
is what the crate's own tests run over: 308 binary files, plus the ASCII twins of
some of them. A claim with a number attached was measured; a claim without one is
a statement about the code.

## What survives

| Data | Read | Write |
| --- | :---: | :---: |
| Mesh geometry | Yes | Yes |
| Normals and UV layers | Yes | Yes |
| Vertex colors | Yes | Yes |
| Tangents and binormals | Yes | Yes |
| `Edges` array | Yes | Yes |
| Edge smoothing (`LayerElementSmoothing`) | Yes | Yes |
| Edge and vertex creases | Yes | Yes |
| Mesh and model names | Yes | Yes |
| Node hierarchy | Yes | Yes |
| Node transforms | Yes | Yes |
| Materials and textures | Yes | Yes |
| Node-TRS animation | Yes | Yes |
| Multiple animation layers | Yes | Yes |
| Animation layer blending | No | No |
| Cameras and lights (`NodeAttribute`) | Yes | Yes |
| Skins, bind poses, and influences | Yes | Yes |
| Blend shapes / morph targets | Yes | Yes |
| `Definitions` property templates | Yes | n/a |
| ASCII container | Yes | Yes |

Not represented at all: FBX pivot settings and inheritance rules, which
`FbxTransform` has no field for; spot cone angles, since no corpus file carries
`InnerAngle` or `OuterAngle`; and the viewport decoration around a camera
(`DisplayTurnTableIcon`, `ShowManipulators`, `BackgroundColor`, `GateFit`).

A `Model` that reaches neither the document root nor a parent `Model` by
object connection is not part of the scene graph and is dropped with an
`unconnected-model-dropped` warning, which is what MotionBuilder intends for
its Producer viewport cameras: they are bound only by `CurrentCamera`
property records, never by object connections, and Blender's importer skips
them the same way. No corpus document reaches geometry through such a Model —
the only two that hold one with any attached at all carry NURBS curves, which
no scene path keeps.

## The version floor

The binary container is read for versions 6000 through 8000 and the ASCII one
from 7000. **Scene content is read only from 7000 and later, in either.**

Earlier versions use a different object model: objects are identified by a
`"name\0\x01Class"` string instead of an `i64` id, connections reference those
strings, geometry lives on the `Model` rather than on a separate `Geometry`, and
array payloads are stored as repeated scalar properties. A pre-7000 document
therefore decodes to a structurally valid but empty scene, and says so through
`FbxWarningCode::NameKeyedObjectModel` rather than looking like a file that
simply has no meshes. 81 of the 308 binary files are version 6100 and fall here.

## The two containers

Both are read and both are written. Binary is accepted in either byte order — a
non-zero endian marker selects big-endian, as `ufbx` does — and ASCII from 7000.
Output is FBX 7500 little-endian, binary unless
`FbxWriter::with_format(FbxFormat::Ascii)` or `FbxScene::to_ascii_bytes` asks for
text.

Only the container layer differs; everything above it is shared, and that is
structural rather than a convention anyone has to maintain. `fbx_container`
decodes the binary container and `fbx_ascii` the text one; both produce a tree of
`FbxNode`, and `fbx_reader` reads only that tree, so it cannot tell which
container it was given. Writing mirrors it: `FbxWriter::build_document` decides
what records the file contains and returns the same `FbxNode` tree, which
`fbx_encoder` spells as records and `fbx_ascii_writer` as text.
`fbx_ascii_syntax` holds what the two containers disagree about — the name/class
separator, the array element-type schema, the `Properties70` type table — in one
place, each convention next to its inverse. `fbx_transform` composes the FBX
transform stack into a local matrix.

So the two containers cannot drift apart semantically. Two of their differences
are normalized on the way in rather than left for consumers:

- object names are written `Class::Name`, where binary uses the reverse order
  with a different separator;
- ASCII values carry no type tag, so a whole number is indistinguishable from an
  integer. The element type comes from the node's schema instead of from how the
  number happens to be written — guessing would type a mesh with integer
  coordinates as an integer array, and 27 of the 369 `Vertices` arrays in the
  corpus are exactly that.

### What writing text costs

Two losses, neither recoverable by trying harder:

- ASCII does not record an integer's width, so an `i64` small enough to fit comes
  back an `i32`. Every reader here accepts either.
- A quotation mark in a name is written `&quot;`, which is not reversible: one
  corpus file holds an object named `"` and another named literally `&quot;`, and
  both are spelled the same way. The same applies to a string that merely happens
  to contain `::`, which the reader splits into a name and a class — `ufbx` reads
  it that way too.

Three shapes are refused outright rather than written wrong: a node with two
array properties, a non-finite float, and raw bytes outside a `Content` node.

### The evidence

110 ASCII documents decode identically to their binary twin, compared across
geometry, layers, skins, morphs, transforms and animation. Six are excluded by
exact name in the corpus test, each with its observed difference stated there:
one relies on an escape ASCII cannot reverse, four are not pairs at all — the two
exports were taken at different points on the timeline — and one prints `f64` too
coarsely to survive narrowing to `f32`.

Everything else round-trips: the corpus test writes all 565 comparable files both
ways and compares the node trees they read back as, record by record.

For one pair at a time, `cargo run --example fbx_twin_diff -- <ascii.fbx>` prints
the same field-by-field comparison, which is how the six were established.

## Property templates

An exporter that gives a whole class the same value writes it once as a
`Definitions/PropertyTemplate` and leaves it off the objects, so those values are
read too — the Revit cameras declare almost nothing directly.

**The object always wins.** 5553 properties in the corpus are declared in both
places, `Lcl Translation` on 928 models among them; letting the template override
would move every one of them to the origin.

Templates are matched to objects by the `ObjectType` name, which is the object
record's node name. `NodeAttribute` is the exception: a document declares one
template for it while the record covers unrelated classes, so the template's own
class must match the object's — `FbxCamera` to `Camera`, `FbxSkeleton` to
`LimbNode`. Nothing derives that pairing from the strings.

## Layer elements

Resolved on the polygon-corner domain, so a UV or hard-normal seam survives
instead of being averaged onto its control point. The Draco mesh then welds
corners that agree on every attribute, which keeps the seams while collapsing
interior duplicates.

Every UV, normal, colour and tangent set is preserved on `FbxMeshInstance`; only
the first of each reaches the Draco mesh, which has no concept of multiple sets.

A layer whose length disagrees with the domain its mapping names is dropped with a
warning rather than kept as misaligned data — seven layers in the corpus are in
that state. A `ByEdge` layer in a geometry with no `Edges` array is a different
case: it addresses the edges an importer would reconstruct, which this crate does
not do, so it is preserved unchecked instead of discarded.

### Tangents and binormals

Tangents are stored as four components with the handedness sign in `w`, the
layout glTF's `TANGENT` uses. FBX itself splits them across two sibling arrays,
`Tangents` and a `TangentsW` that only 7500 and later write, so a set records
whether its handedness was authored or defaulted to `+1`; the writer emits the
sibling array only when it was.

Binormals are read and written for the same reason `Edges` is kept raw — they are
the only carrier of tangent sign in files with no `TangentsW` — but they stop at
`FbxMeshInstance`, since glTF has no binormal to lower them onto.

Draco's `GeometryAttributeType` has no tangent either, so tangents never enter
the Draco mesh or its weld key. They travel on `FbxMeshInstance` and
`FbxRenderMesh`, the route extra UV sets take.

### Edges, smoothing and creases

`Edges` is kept verbatim rather than normalized: FBX does not require it to list
every topological edge, and importers reconstruct the rest from faces, so
discarding the distinction would lose information. It is also the domain `ByEdge`
layers address.

Smoothing flags and crease weights address edges, polygons or control points —
never polygon corners — so they are preserved raw on `FbxMeshInstance` beside
`Edges` rather than resolved onto the render mesh. They have separate types
because smoothing is an integer flag while a crease is a floating-point weight an
integer would flatten. glTF has no equivalent for either, so both survive an
FBX-to-FBX rewrite and travel no further.

## Materials and animation

Materials cover the canonical Phong/Lambert property set (`DiffuseColor`,
`SpecularFactor`, `Shininess`, `EmissiveColor`/`EmissiveFactor`,
`ReflectionFactor`, `TransparencyFactor`/`Opacity`, `BumpFactor`) with diffuse,
normal and emissive textures — embedded `Content` or an external filename — and
per-polygon material indices.

Animation resolves the
`AnimationStack → AnimationLayer → AnimationCurveNode → AnimationCurve` graph
into per-node TRS channels in seconds, one clip per layer, the same choice
Blender's importer makes. Layers are not blended.

## Cameras and lights

`Camera` and `Light` node attributes are read onto `FbxSceneNode::attribute`.
Every field is optional, because FBX omits any property left at its class
default, and the field sets are limited to what the corpus actually contains.

The film back — `FilmWidth`, `FilmHeight`, `FilmAspectRatio`, `ApertureMode` — is
represented, because a focal length alone does not give a field of view: Blender
computes `sensor_width = FilmWidth * 25.4` and substitutes its own 32 mm default
when the property is missing, which silently reframes the shot.

They are written back too — 58 attributes across 20 corpus files survive a
rewrite — and that takes more than mirroring the reader. The reader is blind to
most of what an importer checks: it finds an attribute through its `OO`
connection and reads properties by name, never consulting the `Model`'s class,
`TypeFlags`, the `Definitions` count or a `P` record's declared type. All four are
written, and asserted on the document tree, since no write-and-read cycle can see
them.

Other attribute classes — `LodGroup`, `CameraSwitcher`, `CameraStereo`, IK and FK
effectors — raise `FbxWarningCode::DroppedNodeAttribute`. `LimbNode` and `Null`
do not: a skeleton attribute is consumed by the skin path, and a null carries
nothing but its transform.

## Writing a scene back

Export preserves local affine translation, rotation and scale, skins, bind poses,
morph targets, and authored animation channels.

Decoding a document twice gives the same result: object order, animation channel
order and bind-pose resolution follow FBX object ids rather than hash iteration.

## Reading untrusted input

FBX is a length-prefixed binary container with a decompression path, so
`FbxReadOptions` bounds what one document may allocate and how strictly its
layout is enforced:

```rust
use draco_io::{FbxDecodeLimits, FbxReadOptions, FbxScene};

let options = FbxReadOptions::default()
    .with_limits(FbxDecodeLimits::default().with_max_blob_bytes(16 << 20));
let scene = FbxScene::from_bytes_with_options(&bytes, options)?;
# Ok::<(), std::io::Error>(())
```

Limit violations fail with `ErrorKind::OutOfMemory` and structural violations
with `ErrorKind::InvalidData`, so a caller can tell "too big, retry with
`FbxDecodeLimits::permissive()`" from "corrupt". The defaults are calibrated
against real assets rather than guessed; see `FbxDecodeLimits::default`.

`FbxReadOptions::strict()` additionally rejects anything the container layout does
not permit, including a malformed binary footer. It is off by default because
shipping exporters emit slop every practical reader tolerates: 222 of the 308
real files do not begin their trailing region with the conventional footer id at
all. Deviations accepted in the default mode are reported through
`FbxScene::warnings` as typed `FbxWarningCode` values rather than passing
silently.
