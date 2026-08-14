# draco-rust

[![Rust CI](https://github.com/Filyus/draco-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/Filyus/draco-rust/actions/workflows/ci.yml)
[![Fuzz](https://github.com/Filyus/draco-rust/actions/workflows/fuzz.yml/badge.svg)](https://github.com/Filyus/draco-rust/actions/workflows/fuzz.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Draco geometry compression in pure Rust, with the file formats and glTF scene
handling around it. Nothing here links the original library; bitstream
compatibility is a property of the implementation, checked against reference
fixtures and the upstream binaries themselves.

This project is independent and is not an official Google Draco release.

## Try it in a browser

**[Draco 3D Studio](https://filyus.github.io/draco-rust/)** — a converter and
viewer for the crates below, running at `filyus.github.io/draco-rust`. No
upload, no server: the WASM modules do the work in the page, and the files never
leave the machine.

Open an OBJ, PLY, STL, DRC, FBX or glTF/GLB asset and it reports what the file
actually contains — hierarchy, materials, skins, morph targets, animation clips —
renders it, and exports any of them back out, with Draco compression optional
along the way.

Converting between formats loses things, and the
[per-format conversion rules](web/README.md#source-neutral-scene-conversion)
say which: what survives an FBX to glTF trip, what has no equivalent to survive
into, and where the viewer's limits are narrower than the exporter's.

## Crates

### [`draco-core`](crates/draco-core)

[![crates.io](https://img.shields.io/crates/v/draco-core.svg)](https://crates.io/crates/draco-core)
[![docs.rs](https://docs.rs/draco-core/badge.svg)](https://docs.rs/draco-core)

The codec: Draco bitstream encode and decode for triangle meshes and point
clouds, and the geometry model everything else speaks.

### [`draco-io`](crates/draco-io)

[![crates.io](https://img.shields.io/crates/v/draco-io.svg)](https://crates.io/crates/draco-io)
[![docs.rs](https://docs.rs/draco-io/badge.svg)](https://docs.rs/draco-io)

Formats: OBJ, PLY, STL, binary and ASCII FBX, plus strict glTF/GLB container,
resource and accessor contracts.

### [`draco-gltf`](crates/draco-gltf)

[![crates.io](https://img.shields.io/crates/v/draco-gltf.svg)](https://crates.io/crates/draco-gltf)
[![docs.rs](https://docs.rs/draco-gltf/badge.svg)](https://docs.rs/draco-gltf)

Whole glTF 2.0 and pinned 2.1-draft documents, losslessly: typed scene views,
packed-geometry read/write, document-preserving Draco compression, GLB v2/v3.

Full glTF applications should depend on `draco-gltf`. `draco-io` deliberately
exposes no glTF scene API — it is the layer below, for callers that want a
container parser, a resource policy, or accessor-level geometry without a scene
model on top.

`draco-texture` (KTX2 and Basis Universal transcoding) and the `web/` WASM
wrappers are part of the repository but not published to crates.io; the wrappers
ship as assets on each `draco-io` and `draco-gltf` release.

## Getting started

```toml
[dependencies]
draco-gltf = "0.2"
```

Read a primitive, whether or not it arrived Draco-compressed:

```rust,no_run
use draco_gltf::{MeshIndex, PrimitiveIndex};

let scene = draco_gltf::import("model.glb")?;
let geometry = scene.read_primitive(PrimitiveIndex::new(MeshIndex(0), 0))?;
println!("{} vertices", geometry.vertex_count());
# Ok::<(), draco_gltf::Error>(())
```

Or stay at the format layer and convert geometry between files:

```toml
[dependencies]
draco-io = { version = "0.3", default-features = false, features = ["ply-reader", "obj-writer"] }
```

Every crate is feature-gated down to what an application actually calls, which
matters most on `wasm32`: the format readers, the writers, the codec halves and
the strict-validation passes are all separable. Each crate's README lists its
features and what they cost.

## Compatibility and status

`draco-core` is at 1.0 and its public API is covered by SemVer. Point clouds,
sequential meshes and both EdgeBreaker traversals encode and decode; the
per-algorithm matrix lives in
[`crates/draco-core/SUPPORT_MATRIX.md`](crates/draco-core/SUPPORT_MATRIX.md).

`draco-io` and `draco-gltf` are pre-1.0 and still move: a breaking change bumps
the minor, which is the breaking field for a `0.x` crate. Each crate keeps its
own changelog and its own release tags — see [`CHANGELOG.md`](CHANGELOG.md).

Beyond `KHR_draco_mesh_compression`, glTF reading covers `EXT_meshopt_compression`,
and Draco compression preserves the extensions a document carries rather than
refusing the file — including feature IDs, GPU instancing and structural
metadata.

## Safety

Decoders here are assumed to be pointed at untrusted bytes. Malformed input must
fail as a typed error rather than panic, hang, or allocate without bound, and
decode limits are the caller's to set.

- [`SECURITY.md`](SECURITY.md) — threat model, what is guaranteed, and the
  resource limits a caller should enforce.
- [`FUZZING.md`](FUZZING.md) — how the decode paths are fuzzed. Targets cover
  `.drc` decode, glTF import and compression, FBX read and round-trip, and KTX2
  transcode; they run per push and through ClusterFuzzLite.

## Building the converter locally

```powershell
./build.ps1 -Serve            # from web/, builds the WASM modules and serves the app
```

```sh
cd web && npm ci && bash ./build.sh --app
```

`build.sh` defaults to the lightweight release profile that the size budget is
measured against; `--app` is the converter profile, which the front-end needs.
The [web workspace guide](web/README.md) covers the module features, the
TypeScript build, and the Node and Playwright suites.

## Performance

The crates build like any Rust library (`-Copt-level=3`) and carry no runtime
knobs. Decode and encode are CPU-bound and branchy.

- [`PERFORMANCE.md`](PERFORMANCE.md) — how to benchmark and profile
  decode/encode, the Rust-vs-C++ speed snapshot, and per-stage breakdowns.
- [`PGO.md`](PGO.md) — profile-guided optimisation: how it relates to a source
  library, the two-pass build, training-corpus guidance, expected benefit, and
  the WASM-target tradeoff.
- [`TRICKS.md`](TRICKS.md) — dated, code-level optimization techniques found in
  this repo, each with a real before/after snippet: dispatch, error layout,
  corner-table and entropy-coding tricks, a five-commit KD-tree case study, and
  what was measured and rejected.

## Repository layout

| Path | Contents |
|---|---|
| `crates/` | The published crates, plus `draco-texture` and the C++ test bridge. |
| `web/` | WASM wrappers, the converter front-end, and its Node and Playwright suites. |
| `fuzz/`, `.clusterfuzzlite/` | Fuzz targets and the continuous fuzzing setup. |
| `testdata/`, `dev/`, `tools/` | Fixtures and development tooling. |

## License

Apache-2.0. See [`LICENSE`](LICENSE).
