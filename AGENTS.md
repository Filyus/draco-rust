# Agent guide

Operational notes for an AI agent (Codex/Claude) working in this repo.

## Rust formatting

Rust code follows the Chromium Rust Style Guide, which currently relies on the
public Rust Style Guide for mechanical formatting and the Rust API Guidelines
for API design.

Use stable `rustfmt` for all Rust formatting. Before finalizing Rust changes,
run:

```sh
cargo fmt --manifest-path crates/Cargo.toml --all -- --check
cargo fmt --manifest-path web/Cargo.toml --all -- --check
```

To format the workspaces, run the same commands without `-- --check`.

## Workspace layout

- `crates/draco-core` is the publishable core Draco bitstream crate.
- `crates/draco-io` is the publishable file-format crate and depends on
  `draco-core`.
- `crates/draco-gltf` is the publishable full-scene glTF crate; it owns the
  lossless document model and depends on both `draco-core` and `draco-io`.
- `crates/draco-texture` reads KTX2 and transcodes Basis Universal so the web
  converter can show `KHR_texture_basisu` textures. Nothing in it is about
  Draco, so it stays out of the published crates and has `publish = false`.
  `crates/draco-texture/STATUS.md` records what is finished, what was left out
  and why, and what upstream has been doing; read it before adding a codec or a
  target, so the decisions already taken are not retaken.
- `crates/draco-cpp-test-bridge` is internal C++ parity infrastructure and has
  `publish = false`.
- `web/` contains WASM wrapper crates and demo tooling; it is released as GitHub
  release assets, not as crates.io packages.

The three publishable crates are versioned and **released independently** (see
[Releases](#releases)).

## Memory safety

The `unsafe` rule is **per crate**, and which crate is under which rule is
decided by the build:

- `draco-core` and `draco-texture` forbid it (`[lints.rust] unsafe_code =
  "forbid"`). Do not propose lifting it for speed; both run table-driven
  decoding on bitstream-controlled indices and are the wrong place to be unable
  to reason about a crash. `draco-texture` is in despite `publish = false` —
  it ships in `ktx2-wasm` and transcodes whatever the converter is handed.
- `draco-io` and `draco-gltf` permit it in narrow, audited paths, with a
  `// SAFETY:` comment on every block naming the invariant *and where it was
  established*. CI runs clippy with `-D warnings`, so
  `undocumented_unsafe_blocks` is binding.

The depth differs: `forbid` checks the property, while
`undocumented_unsafe_blocks` only checks that a comment exists. On the
permissive side everything past the comment's presence is review.

The split is by what the code does, not by how much its input is trusted: a
glTF accessor walk reads offsets and strides out of a file a hostile caller
wrote, and it is on the permissive side because each bound is established a line
or two from the read rather than carried through a decoder's state.

Before writing one, read [`SECURITY.md`](SECURITY.md#memory-safety-unsafe): it
lists what a block has to carry, and it wants the path added to its table and
covered by a fuzz target. Also **price the safe version first and say what it
measured** — the permission exists so a measured win does not have to argue the
policy, not so `unsafe` is the first thing reached for.

## Commit conventions

- Use concise, **domain-prefixed** subjects — not conventional `feat:`/`fix:`.
  The prefix names the area and the
  [changelog taxonomy](RELEASING-AGENT.md#changelog-taxonomy) decides which crate
  changelog (if any) the change reaches. Examples: `core:`, `io:`, `gltf:`,
  `decoder:`, `encoder:`, `edgebreaker:`, `metadata:`, `fuzz:`, `docs:`, `ci:`,
  `release:`.
- Prefer the prefix that maps to the affected crate's domain, so the changelog
  groups land in the right per-crate `CHANGELOG.md`.
- Put benchmark tables and long compatibility notes in the commit body or docs,
  not in the subject.
- Keep generated/debug output out of commits unless it is intentional test data.
  The repo has many ignored temporary files under `crates/`; do not clean or
  revert them unless the maintainer explicitly asks.

## Tests and packaging

The default local checks before publication-facing changes are:

```sh
cargo fmt --manifest-path crates/Cargo.toml --all -- --check
cargo fmt --manifest-path web/Cargo.toml --all -- --check
cargo test --manifest-path crates/Cargo.toml -p draco-core --lib
cargo test --manifest-path crates/Cargo.toml -p draco-core --test drc_edge_cases_test
cargo test --manifest-path crates/Cargo.toml -p draco-io
cargo test --manifest-path crates/Cargo.toml -p draco-gltf
cargo test --manifest-path web/Cargo.toml --workspace
cargo package --manifest-path crates/draco-core/Cargo.toml
```

[`TESTING.md`](TESTING.md) maps the parity/compatibility test targets, and
[`PERFORMANCE.md`](PERFORMANCE.md) the benchmark and profiling targets.

A profiling or optimization round is written up in
[`PERFORMANCE-LOG.md`](PERFORMANCE-LOG.md), not in `PERFORMANCE.md` -- including
one that measured to nothing, which is the case most worth recording. Read that
log's index before starting a round: an idea already sitting there as `null` or
`rejected` has been paid for once. `PERFORMANCE.md` carries only what is
currently true, and a round updates it only when it moves a headline figure.

`draco-io` depends on a published `draco-core`, and `draco-gltf` on published
`draco-core` + `draco-io`, so `cargo package`/`cargo publish` for a dependent
crate can fail before its dependencies are published at the pinned versions.
Release the crates **dependency order first**: `draco-core` -> `draco-io` ->
`draco-gltf`, waiting for each to appear in the crates.io index before the next.

## Releases

The three crates are versioned and **released independently**. Each has its own
`crates/<crate>/CHANGELOG.md` and its own `<crate>-vX.Y.Z` release tags
(`draco-core-vX.Y.Z`, `draco-io-vX.Y.Z`, `draco-gltf-vX.Y.Z`).

Releases are prepared by hand, one crate at a time. The agent prepares the
version bump + that crate's changelog, shows the diff, and pushes the release
commit only after the maintainer approves the wording. The maintainer alone
approves the `release` GitHub environment that publishes to crates.io.

Full release steps: [`RELEASING.md`](RELEASING.md). Agent checklist and
changelog taxonomy: [`RELEASING-AGENT.md`](RELEASING-AGENT.md).
