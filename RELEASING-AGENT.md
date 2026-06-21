# Release preparation (agent)

How an AI agent prepares a release **by hand**. Companion to the human-facing
[`RELEASING.md`](RELEASING.md); referenced from [`AGENTS.md`](AGENTS.md).

The three crates — `draco-core`, `draco-io`, `draco-gltf` — are versioned and
released **independently**, one crate per release. Each has its own
`crates/<crate>/CHANGELOG.md` and its own `<crate>-vX.Y.Z` tags. Our commits use
**domain prefixes** (not conventional `feat:`/`fix:`), and the changelog is
grouped using the [taxonomy](#changelog-taxonomy) below. The publish pipeline
(`.github/workflows/publish.yml`, run per crate) does its own preflight, tagging,
and GitHub release.

## Roles (keep them separate)

- **Agent**: prepares the bump + changelog for one crate, shows the diff, and —
  only after the maintainer OKs the wording — creates and pushes the release
  commit. Stops there unless explicitly asked to start the workflow.
- **Maintainer**: reviews the changelog wording before the push, and approves the
  `release` GitHub environment after CI + preflight. Only that approval publishes.
  The agent cannot and must not approve it.

## Dependency order

`draco-core` <- `draco-io` <- `draco-gltf`. A dependent can only be released
after the dependency version it pins is published on crates.io. If a release
bumps `draco-core`, releasing the dependents that should pick it up is a
**separate** release for each (bump the pin, then its own changelog/commit/tag).

## Steps (for one crate `C` = draco-core | draco-io | draco-gltf)

1. **Preconditions.** On `main`, clean tree, in sync with `origin/main`:
   ```sh
   git fetch origin
   git status --short                                    # must be empty
   git rev-list --left-right --count origin/main...HEAD  # must be "0  0"
   ```
   If the tree is dirty, classify per `RELEASING.md`; never fold stray work into
   the release commit.

2. **Decide the version** of `C` from *its* public API surface (pre-1.0):
   - new public API only -> minor (`0.Y+1.0`);
   - bug/behavior fix only -> patch;
   - removed/changed public API -> major.

   Bump `version` in `crates/C/Cargo.toml`. If `C` is a dependency of a crate
   you are **also** releasing now, update that dependent's pin in the **same**
   release only when you intend to publish the dependent too (otherwise leave it).

3. **Build the changelog section** in `crates/C/CHANGELOG.md` from
   `git log C-vPREV..HEAD` (or all history for the first release):
   - Heading under `## [Unreleased]`:
     `## [X.Y.Z](https://github.com/Filyus/draco-rust/compare/C-vPREV...C-vX.Y.Z) - YYYY-MM-DD`
   - First release: `## [X.Y.Z] - YYYY-MM-DD`.
   - Include only commits that touched `C` (verify with `git show --stat`). Group
     by the [taxonomy](#changelog-taxonomy), in priority order.
   - Rewrite terse subjects into clear, **user-facing** notes: name the affected
     public types/formats/features and a one-line "why it matters".
   - Drop internal-only groups, and commits touching only test infra, the local
     C++ bridge, debug output, CI wiring, or benchmarks.

4. **Version-facing docs.** If the minor changed, update install snippets in the
   root `README.md` and `crates/C/README.md`. Touch no other docs in the release
   commit.

5. **Show the diff and pause:**
   ```sh
   git diff -- crates/C/Cargo.toml crates/C/CHANGELOG.md crates/C/README.md README.md
   ```
   Wait for the maintainer to OK the changelog wording. Do **not** commit first.

6. **After the maintainer OKs**, commit exactly the release files with the exact
   subject, then push:
   ```sh
   git commit -m "release: prepare C vX.Y.Z"
   git push origin main
   ```
   The subject must match `crates/C/Cargo.toml`'s version exactly (`C` is the
   crate name), or the publish workflow refuses to publish.

7. **After CI passes, start the publish workflow for `C`** (it does not run
   automatically):
   ```sh
   gh workflow run publish.yml --ref main -f crate=C
   ```
   It runs against `main` HEAD, which must still be the release commit. This only
   starts the pipeline; it gates at the `release` environment for the maintainer.

8. **Stop.** Do not publish, tag, create releases, or approve the `release`
   environment unless the maintainer explicitly asks for that specific action.

## Changelog taxonomy

Commit **domain prefixes** map to a crate and a changelog group, rendered in
priority order (low number first). The crate column says which
`crates/<crate>/CHANGELOG.md` the change belongs to; cross-cutting prefixes go to
whichever crate the commit actually touched (`git show --stat`).

| Prio | Group | Crate | Example prefixes | Changelog |
|---:|---|---|---|---|
| 00 | API | the touched crate | `api`, `config`, `errors`, `defaults` | keep |
| 01 | Safety and Hardening | the touched crate | `safety`, `security`, `hardening`, `fuzz`, `unsafe` | keep |
| 02 | Core codec | draco-core | `core`, `decoder`, `encoder`, `bitstream`, `edgebreaker`, `sequential`, `kd-tree`, `rans`, `ans`, `symbol` | keep |
| 03 | Geometry model | draco-core | `mesh`, `point-cloud`, `attribute`, `quantization`, `prediction`, `normal`, `metadata` | keep |
| 04 | Format I/O | draco-io | `io`, `obj`, `ply`, `fbx`, `gltf`, `glb`, `scene`, `compress` | keep |
| 05 | glTF scene bridge | draco-gltf | `draco-gltf`, `bridge` | keep |
| 06 | Compatibility | the touched crate | `compat`, `interop`, `legacy`, `cpp`, `parity` | keep if user-facing |
| 07 | Performance | the touched crate | `perf`, `speed`, `memory` | keep if measured and user-facing |
| 08 | WASM and release assets | web / the touched crate | `wasm`, `web`, `demo` | depends |
| 20 | Documentation | the touched crate | `docs`, `readme`, `rustdoc`, `examples` | case-by-case |
| 90 | Tests | — | `test`, `tests`, `fixture` | drop unless a compatibility guarantee changed |
| 91 | Refactoring | — | `refactor`, `cleanup`, `internal`, `module` | drop |
| 92 | Lint | — | `lint`, `fmt`, `clippy`, `style` | drop |
| 99 | Build, CI, Packaging | — | `build`, `ci`, `deps`, `workflow`, `github`, `publish`, `bench` | drop unless release behavior changed |
| — | (skipped) | — | `release`, `repo`, `changelog` | drop (never in notes) |

Rules of thumb:
- `gltf:` is `draco-io`'s glTF format support (KHR_draco in glTF/GLB). The
  full-scene bridge crate uses the `draco-gltf:` prefix.
- "keep" groups are crate-user-facing; write a clear bullet per change.
- Mention WASM only when published release assets or a crate's WASM API change;
  omit browser-demo-only polish.
- Mention C++ bridge work only when it changes a documented compatibility claim.
- Never include `release:` commits in release notes.
