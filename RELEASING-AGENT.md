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

- **Agent**: prepares the bump + changelog for one crate and shows the diff.
  **That is the one stop.** After the maintainer OKs the wording it runs the rest
  without asking again: commit, push, wait for `Rust CI` and `Fuzz`, start the
  publish workflow, and then watch `Release: WASM assets` through to the attached
  zips. Asking again before the gate buys nothing — the gate is a button, and it
  cannot be pressed early by anyone but the maintainer.
- **Maintainer**: reviews the changelog wording before the push — the wording is
  what ships, as the GitHub Release body — and presses approve on the `release`
  GitHub environment when the pipeline reaches it. Only that approval publishes.
  The agent cannot and must not approve it.
  - That gate exists because the environment carries a required-reviewer rule in
    the repository settings, not because the workflow names an environment. It
    also allows self-review, so it is a confirmation rather than a second pair of
    eyes.

## Dependency order

`draco-core` <- `draco-io` <- `draco-gltf`. A dependent can only be released
after the dependency version it pins is published on crates.io. If a release
bumps `draco-core`, releasing the dependents that should pick it up is a
**separate** release for each (bump the pin, then its own changelog/commit/tag).

## Steps (for one crate `<crate>` = draco-core | draco-io | draco-gltf)

1. **Preconditions.** On `main`, clean tree, in sync with `origin/main`:
   ```sh
   git fetch origin
   git status --short                                    # must be empty
   git rev-list --left-right --count origin/main...HEAD  # must be "0  0"
   ```
   If the tree is dirty, classify per `RELEASING.md`; never fold stray work into
   the release commit.

   Then establish what is **actually published**, from the registry and the tags
   rather than from the manifest:
   ```sh
   git ls-remote --tags origin | grep "<crate>-v"
   curl -s "https://crates.io/api/v1/crates/<crate>" | grep -o '"max_version":"[^"]*"'
   ```
   A version bumped in `Cargo.toml`, dated in the changelog and never published
   is a real state: preparation that stopped before the commit. Treat the last
   tag as the previous release, not the manifest. When the prepared version is
   absent from crates.io, from the tags and from the GitHub releases, fold the
   new work into its section and re-date it rather than opening the next number
   — nobody can depend on a version that was never there, and skipping it leaves
   a hole plus a compare link pointing at a tag that does not exist.

2. **Decide the version** of `<crate>` from *its* public API surface. Bump the
   field Cargo treats as breaking, which is the leftmost non-zero one — so the
   rule differs by the version the crate is already at:

   | `<crate>` is at | removed/changed public API | new public API only | fix only |
   |---|---|---|---|
   | `1.Y.Z` (draco-core) | major `2.0.0` | minor `1.Y+1.0` | patch `1.Y.Z+1` |
   | `0.Y.Z` (draco-io, draco-gltf) | minor `0.Y+1.0` | patch `0.Y.Z+1` | patch `0.Y.Z+1` |

   For a `0.Y.Z` crate the minor **is** the breaking bump: `^0.3.0` admits
   `0.3.x` and refuses `0.4.0`. Breaking changes there never imply `1.0.0` —
   reaching `1.0.0` is the maintainer declaring the API stable, and the agent
   proposes it only when asked.

   A release usually mixes groups; take the strongest one that applies.

   Bump `version` in `crates/<crate>/Cargo.toml`. If `<crate>` is a dependency of a crate
   you are **also** releasing now, update that dependent's pin in the **same**
   release only when you intend to publish the dependent too (otherwise leave it).

3. **Build the changelog section** in `crates/<crate>/CHANGELOG.md` from
   `git log <crate>-vPREV..HEAD` (or all history for the first release):
   - Heading under `## [Unreleased]`:
     `## [X.Y.Z](https://github.com/Filyus/draco-rust/compare/<crate>-vPREV...<crate>-vX.Y.Z) - YYYY-MM-DD`
   - First release: `## [X.Y.Z] - YYYY-MM-DD`.
   - Include only commits that touched `<crate>` (verify with `git show --stat`). Group
     by the [taxonomy](#changelog-taxonomy), in priority order.
   - Rewrite terse subjects into clear, **user-facing** notes: name the affected
     public types/formats/features and a one-line "why it matters".
   - Drop internal-only groups, and commits touching only test infra, the local
     C++ bridge, debug output, CI wiring, or benchmarks.

4. **Version-facing docs.** The changelog is the one the pipeline checks, so it
   is the one that never gets forgotten. These are the ones that do — nothing
   fails, and it shows on the crates.io page:
   - install snippets in the root `README.md` and `crates/<crate>/README.md`,
     when the minor changed;
   - `keywords` and `categories` in `crates/<crate>/Cargo.toml` — capped at five
     keywords, so a new format may mean choosing;
   - the format/feature tables and support matrices in
     `crates/<crate>/README.md`, and the format list in the root `README.md`;
   - `crates/<crate>/src/lib.rs` module docs, where they enumerate what exists.

   Work from the release's own contents: list every format, feature or public
   type it adds or renames, then grep the docs for each name. v0.3.1 shipped with
   `stl` missing from `keywords` and from the format table, because the changelog
   was written and the enumerations were not re-read.

   Touch no other docs in the release commit.

5. **Run the checks that can fail on the code**, before there is a release
   commit to protect — a preflight failure after the push has to be repaired by
   rewriting `main`.

   The release-specific ones:
   ```sh
   rustup update nightly   # which rustdoc lints fire changes with the toolchain
   RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo +nightly doc \
     --manifest-path crates/<crate>/Cargo.toml --no-deps --all-features
   cargo semver-checks --manifest-path crates/<crate>/Cargo.toml
   cargo metadata --manifest-path crates/Cargo.toml >/dev/null  # the new version still resolves
   cargo publish --dry-run --manifest-path crates/<crate>/Cargo.toml
   ```

   **And the ones `Rust CI` runs, which are not release-specific and are what
   actually fails.** Preparing v0.3.1 pushed a red `main` twice: first a
   `web/` crate that did not compile under `cargo test` and two clippy lints in
   modules added that cycle, then a browser fixture that was a gitignored
   decoding artifact. Neither is anything the three commands above look at.
   ```sh
   cargo fmt --manifest-path crates/Cargo.toml --all -- --check
   cargo fmt --manifest-path web/Cargo.toml --all -- --check
   cargo clippy --manifest-path crates/Cargo.toml --workspace --all-targets --all-features -- -D warnings
   cargo clippy --manifest-path web/Cargo.toml --workspace --all-targets -- -D warnings
   cargo test --manifest-path crates/Cargo.toml -p <crate> --features test
   cargo test --manifest-path web/Cargo.toml --workspace
   npm run --prefix web test:browser        # needs ./build.sh --app over it first
   ```
   Plus the `--no-default-features` feature combinations for `<crate>` from
   `.github/workflows/ci.yml` — a release that adds a feature is exactly when
   feature gating slips. Read that file rather than this list: the workflow is
   the gate, and this list is a copy that can fall behind it.

   A test may only read fixtures the repository carries. `git ls-files
   --error-unmatch <path>` on each; `testdata/` holds ignored decoder output
   (`*.drc.ply`, `*.drc.obj`) that exists only on the machine that produced it.

6. **Show the diff and pause:**
   ```sh
   git diff -- crates/<crate>/Cargo.toml crates/<crate>/CHANGELOG.md crates/<crate>/README.md README.md
   ```
   Wait for the maintainer to OK the changelog wording. Do **not** commit first.
   This pause is owed for every crate separately: a maintainer who delegated the
   commands still reviews each changelog, and silence is not an OK.

   It is also the **only** pause in the release. Everything after it is
   mechanical and reversible until the `release` environment gate, which no
   amount of asking can pass on the maintainer's behalf.

7. **After the maintainer OKs**, commit exactly the release files with the exact
   subject, then push:
   ```sh
   git commit -m "release: prepare <crate> vX.Y.Z"
   git push origin main
   ```
   The subject must match `crates/<crate>/Cargo.toml`'s version exactly (`<crate>` is the
   crate name), or the publish workflow refuses to publish.

8. **Once `Rust CI` and `Fuzz` are both green for that commit, start the publish
   workflow for `<crate>`** — without asking; step 6 already covered it (it does
   not run automatically):
   ```sh
   gh workflow run publish.yml --ref main -f crate=<crate>
   ```
   Both are triggered by the push; `Fuzz` is the slower of the two, so watching
   only `Rust CI` starts the release while fuzzing is still running. Preflight
   refuses that, and waiting for it is the point — not a formality to route
   around. Start the workflow as soon as both are green, in the same turn that
   observes it: it runs against `main` HEAD, which must still be the release
   commit, and any push landing meanwhile invalidates it. This only starts the
   pipeline; it gates at the `release` environment for the maintainer.

9. **If preflight fails**, the fix is a normal commit and a *new* release
   commit, because the subject check reads the head of `main`:
   - fix the cause in its own commit, with its own domain prefix — code never
     belongs in the release commit;
   - put the release commit back on top. Prefer rebuilding the pair over an
     empty marker commit: reset to the commit before the original release
     commit, cherry-pick the fix, cherry-pick the release commit, and
     `push --force-with-lease`. Two commits with the same release subject, one
     of them empty, is what the alternative leaves in the history forever.
   - a force-push to `main` is safe only while nothing points at those commits.
     Once the tag and the crates.io version exist, that door is closed;
   - the rewrite discards the working tree. Commit or copy aside anything
     uncommitted first — `git reset --hard` takes unrelated drafts with it.

10. **Wait at the gate, then see the release finished.** Do not publish, tag,
    create releases, or approve the `release` environment — the pipeline does the
    first three itself once the maintainer presses approve, and the approval is
    theirs alone.

    After it publishes, the release is still not done: `publish.yml` asks
    `Release: WASM assets` for a run, because the tag it pushed raises no events
    of its own. Skipping this is how draco-io v0.3.0 and draco-gltf v0.2.0 came
    to be published with no assets at all.

    So check all four things exist, from outside the repository:
    ```sh
    curl -s https://crates.io/api/v1/crates/<crate> | grep -o '"max_version":"[^"]*"'
    git ls-remote --tags origin | grep "<crate>-v"
    gh release view <crate>-vX.Y.Z --json assets --jq '.assets[].name'
    ```
    Expect one zip per built module — currently seven: obj, ply, stl, fbx, gltf,
    drc, ktx2 — all stamped with this release's version. The whole set goes on
    every release, and the list comes from the build rather than from the
    workflow, so a new module needs nothing added anywhere.

    Watch the run by **id**, and prove the watcher prints before trusting its
    silence: two attempts at this release reported nothing for half an hour
    each — one queried `gh run list --commit` with an abbreviated SHA, which
    returns nothing, and one had a `SyntaxError` in its own parsing.

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
| 09 | Texture codecs | draco-texture | `texture`, `ktx2`, `basis` | drop (crate is unpublished) |
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
