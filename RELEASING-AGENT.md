# Release preparation (agent)

How an AI agent prepares a release by hand. Companion to the human-facing
[`RELEASING.md`](RELEASING.md); referenced from [`AGENTS.md`](AGENTS.md).

The version bump and changelog are written by hand. The publish pipeline
(`.github/workflows/publish.yml`) runs its own preflight and tagging.

## Roles

- **Agent**: prepares the bump + changelog, shows the diff, and, only after the
  maintainer approves the wording, creates and pushes the release commit. Stops
  there unless explicitly asked to start the workflow.
- **Maintainer**: reviews the changelog wording before the push, and approves
  the `release` GitHub environment after CI + preflight. Only that approval
  publishes. The agent cannot and must not approve it.

## Steps

1. **Preconditions.** On `main`, clean tree, in sync with `origin/main`:
   ```sh
   git fetch origin
   git status --short
   git rev-list --left-right --count origin/main...HEAD
   ```
   The status must be empty and the rev-list result must be `0  0`. If the tree
   is dirty, classify per `RELEASING.md`; never fold stray work into the release
   commit.

2. **Decide the version** from the public API surface (pre-1.0):
   - new public API only -> minor (`0.Y+1.0`);
   - bug/behavior fix only -> patch;
   - removed/changed public API -> major.

   Bump both publishable crates to the same version:
   - `crates/draco-core/Cargo.toml`;
   - `crates/draco-io/Cargo.toml`;
   - the `draco-core` dependency version inside `crates/draco-io/Cargo.toml`.

3. **Build the changelog section** from `git log vPREV..HEAD`:
   - Heading inserted directly under `## [Unreleased]`:
     `## [X.Y.Z](https://github.com/Filyus/draco-rust/compare/vPREV...vX.Y.Z) - YYYY-MM-DD`
   - For the first release, use `## [X.Y.Z] - YYYY-MM-DD`.
   - Group commits by the [taxonomy](#changelog-taxonomy) below, in priority
     order.
   - Rewrite terse subjects into clear, user-facing notes. Name the affected
     public types, formats, features, or compatibility paths and say why they
     matter.
   - Drop internal-only groups and commits touching only test infrastructure,
     local C++ bridge code, debug output, CI wiring, or benchmark harnesses
     unless the change affects crate users or release assets.

4. **Version-facing docs.** If the minor changed, update install snippets:
   - root `README.md`;
   - `crates/draco-core/README.md`;
   - `crates/draco-io/README.md`.

5. **Show the diff and pause:**
   ```sh
   git diff -- crates/draco-core/Cargo.toml crates/draco-io/Cargo.toml CHANGELOG.md README.md crates/draco-core/README.md crates/draco-io/README.md
   ```
   Wait for the maintainer to approve the changelog wording. Do not commit
   first.

6. **After approval**, commit exactly the release files with the exact subject,
   then push:
   ```sh
   git add crates/draco-core/Cargo.toml crates/draco-io/Cargo.toml CHANGELOG.md README.md crates/draco-core/README.md crates/draco-io/README.md
   git commit -m "release: prepare draco-rust vX.Y.Z"
   git push origin main
   ```

7. **After CI passes**, the publish workflow may be started manually:
   ```sh
   gh workflow run publish.yml --ref main
   ```
   It gates at the `release` environment for the maintainer's approval.

8. **Stop.** Do not publish, tag, create releases, or approve the `release`
   environment unless the maintainer explicitly asks for that specific action.

## Changelog taxonomy

Commit domain prefixes map to changelog groups, rendered in priority order.
"Changelog" says whether the group normally reaches user-facing release notes.

| Prio | Group | Example prefixes | Changelog |
|---:|---|---|---|
| 00 | API | `api`, `core`, `io`, `config`, `errors`, `metadata` | keep |
| 01 | Safety and Hardening | `safety`, `security`, `hardening`, `fuzz` | keep |
| 02 | Draco Core | `core`, `decoder`, `encoder`, `bitstream`, `rans`, `ans`, `kd-tree`, `edgebreaker` | keep |
| 03 | Meshes and Point Clouds | `mesh`, `point-cloud`, `attribute`, `normal`, `quantization`, `prediction` | keep |
| 04 | Format I/O | `io`, `obj`, `ply`, `fbx`, `gltf`, `glb`, `scene` | keep |
| 05 | WASM and Release Assets | `wasm`, `web`, `demo` | depends |
| 06 | Compatibility | `compat`, `interop`, `legacy`, `cpp`, `parity` | keep if user-facing |
| 07 | Performance | `perf`, `speed`, `memory`, `bench` | keep if measured and user-facing |
| 20 | Documentation | `docs`, `readme`, `rustdoc`, `examples` | case-by-case |
| 90 | Tests | `test`, `tests`, `fixture` | drop unless compatibility guarantee changed |
| 91 | Refactoring | `refactor`, `cleanup`, `internal`, `module` | drop |
| 92 | Lint | `lint`, `fmt`, `clippy` | drop |
| 99 | Build, CI, and Packaging | `build`, `ci`, `deps`, `workflow`, `github`, `publish`, `release` | drop unless release behavior changed |

Rules of thumb:
- Keep bullets crate-user-facing and concrete.
- Mention WASM changes only when published release assets or WASM APIs change;
  omit browser-demo-only polish.
- Mention C++ bridge work only when it changes the documented compatibility
  claim, not when it only adjusts local test machinery.
- Never include `release:` commits in release notes.
