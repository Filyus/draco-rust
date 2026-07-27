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
- `crates/draco-cpp-test-bridge` is internal C++ parity infrastructure and has
  `publish = false`.
- `web/` contains WASM wrapper crates and demo tooling; it is released as GitHub
  release assets, not as crates.io packages.

The three publishable crates are versioned and **released independently** (see
[Releases](#releases)).

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
