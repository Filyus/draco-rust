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
- `crates/draco-cpp-test-bridge` is internal C++ parity infrastructure and has
  `publish = false`.
- `web/` contains WASM wrapper crates and demo tooling; it is released as GitHub
  release assets, not as crates.io packages.

## Commit conventions

- Use concise, domain-prefixed subjects such as `core:`, `io:`, `gltf:`,
  `edgebreaker:`, `metadata:`, `fuzz:`, `docs:`, `ci:`, or `release:`.
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
cargo test --manifest-path web/Cargo.toml --workspace
cargo package --manifest-path crates/draco-core/Cargo.toml
```

`draco-io` depends on the matching published `draco-core` version, so
`cargo package --manifest-path crates/draco-io/Cargo.toml` can fail before
`draco-core` is published for that version. The release workflow publishes
`draco-core` first, waits for it to appear in the crates.io index, then packages
and publishes `draco-io`.

## Releases

Releases are prepared by hand. The agent prepares the version bump and
`CHANGELOG.md`, shows the diff, and pushes the release commit only after the
maintainer approves the wording. The maintainer alone approves the `release`
GitHub environment that publishes to crates.io.

Full release steps: [`RELEASING.md`](RELEASING.md). Agent checklist and
changelog taxonomy: [`RELEASING-AGENT.md`](RELEASING-AGENT.md).
