# Releasing

The three crates — `draco-core`, `draco-io`, `draco-gltf` — are versioned and
released **independently**, one crate per release. Each has its own version in
`crates/<crate>/Cargo.toml`, its own `crates/<crate>/CHANGELOG.md`, and its own
`<crate>-vX.Y.Z` release tags.

In the steps below, **`<crate>` is the crate being released** — substitute
`draco-core`, `draco-io`, or `draco-gltf`. For example, `crate=<crate>` means
`crate=draco-gltf` when releasing `draco-gltf`.

### The web assets ride along

The `web/` WASM wrappers and converter are not published to crates.io, and a
crate release no longer carries them: the zipped modules it used to attach were
removed on 2026-10-10, and the modules ship as `@draco-rust/*` npm packages
instead ([npm packages](#npm-packages)). The converter is deployed to GitHub Pages by `Pages: deploy
converter`, from `main` rather than from a tag — it demonstrates the current code, and pinning it
to a crate release would show neither crate's version honestly.

Converter changes are recorded in [`web/CHANGELOG.md`](web/CHANGELOG.md).

Normal releases are optimized for a solo maintainer working with an agent:

1. The agent verifies the working tree is clean and current with `origin/main`.
2. The agent prepares one crate's version bump and release notes locally.
3. The maintainer reviews the crate's `CHANGELOG.md` diff.
4. The agent pushes one release commit to `main`.
5. CI validates that exact commit.
6. Once CI is green, the `Release: publish crate` workflow is started manually
   for that crate (`gh workflow run publish.yml --ref main -f crate=<crate>`, or
   the Actions UI). It runs preflight, then waits for the `release` environment
   approval.
7. The maintainer approves the environment deployment. Only then does the
   workflow publish the crate, create the annotated `<crate>-vX.Y.Z` tag, and
   create the GitHub Release.

The publish is started by hand rather than automatically after CI: crates.io
Trusted Publishing rejects GitHub's `workflow_run` event, so the publish uses the
`workflow_dispatch` trigger.

Release preparation (version bump + changelog) is done by hand following
[`RELEASING-AGENT.md`](RELEASING-AGENT.md). Nothing in this flow publishes crates,
pushes tags, or creates GitHub Releases outside the gated publish workflow.

## Dependency order

`draco-core` <- `draco-io` and `draco-core` <- `draco-gltf`. The two format
crates are siblings: neither depends on the other, so once `draco-core` is out
they can be released in either order, or only one of them. A dependent can only
be released once the dependency version it pins is published. Treat each crate
as its own release (its own commit, changelog, and tag).

## Normal Release

Use this path after the crate already exists on crates.io and Trusted Publishing
is configured for it.

### 1. Prepare the release locally

Start from `main` and inspect the working tree:

```powershell
git fetch origin
git switch main
git pull --ff-only origin main
git status --short
```

If `git status --short` prints anything, classify the changes before preparing
the release (commit the missing change first and wait for CI, or ask). Do not
fold in-progress work into the release commit.

Prepare the bump and changelog by hand, following
[`RELEASING-AGENT.md`](RELEASING-AGENT.md). In short, for crate `<crate>`:

- bump `version` in `crates/<crate>/Cargo.toml`;
- if releasing a dependent that should pick up a just-published dependency,
  update that pin too (separate release per crate);
- write the `crates/<crate>/CHANGELOG.md` section, grouped by the
  [changelog taxonomy](RELEASING-AGENT.md#changelog-taxonomy), including only
  commits that touched `<crate>`;
- rewrite terse subjects into clear, user-facing notes;
- remove internal noise (C++ bridge, debug output, lint/CI/bench-only, or
  demo-only commits);
- keep feature/code changes out of the release commit.

Run the three checks that can fail on code rather than on the release itself
**before** committing — the preflight runs them too, but by then the release
commit is pushed and has to be HEAD, so a failure costs a history rewrite:

```powershell
rustup update nightly
$env:RUSTDOCFLAGS = "--cfg docsrs -D warnings"
cargo +nightly doc --manifest-path crates/<crate>/Cargo.toml --no-deps --all-features
cargo semver-checks --manifest-path crates/<crate>/Cargo.toml
$env:PIN_CHECK_ALLOW_DIRTY = "1"
bash .github/scripts/check-pin-floors.sh <crate>
```

The last one matters for `draco-io` and `draco-gltf`: it builds the package
with `draco-core` held at the version the pin names, which is the only build
that notices a pin left below the API the code calls.

Update nightly first: the docs build fails on rustdoc lints, and which lints
fire changes with the toolchain, so a stale local nightly passes what the
runner rejects. The remaining preflight checks — duplicate version, duplicate
tag, `--dry-run` — depend on the push and are not worth reproducing by hand.

Show the maintainer the diff before committing:

```powershell
git diff -- crates/<crate>/Cargo.toml crates/<crate>/CHANGELOG.md crates/<crate>/README.md README.md
```

### 2. Commit and push

After approval, create one release commit with this exact subject (`<crate>` is the
crate name, e.g. `draco-gltf`):

```text
release: prepare <crate> vX.Y.Z
```

```powershell
git add crates/<crate>/Cargo.toml crates/<crate>/CHANGELOG.md crates/<crate>/README.md README.md
git commit -m "release: prepare <crate> vX.Y.Z"
git push origin main
```

The exact subject matters: the publish workflow ignores ordinary pushes and only
continues when the subject matches `crates/<crate>/Cargo.toml`'s version.

### 3. Start the publish workflow and preflight

The push to `main` starts `Rust CI` and `Fuzz`. Wait for **both** — `Fuzz` takes
longer, so a release started when `Rust CI` alone goes green races it — then
start the publish workflow for the crate:

```powershell
gh workflow run publish.yml --ref main -f crate=<crate>
```

Start it as soon as both are green: nothing else guards the release commit's
place at the head of `main`, and any push that lands meanwhile invalidates it.

Preflight checks, for crate `<crate>`:

- a successful `Rust CI` run exists for this commit;
- a successful `Fuzz` run exists for this commit;
- the commit subject is exactly `release: prepare <crate> vX.Y.Z`;
- `X.Y.Z` matches `crates/<crate>/Cargo.toml`;
- every internal dependency `<crate>` pins is already published at the pinned version;
- the package builds with every internal dependency held at its pinned version;
- `crates/<crate>/CHANGELOG.md` has a `## [X.Y.Z]` section;
- `web/CHANGELOG.md`'s `Unreleased` section is copied into the GitHub
  release's notes and renamed here to the shipping date;

- `cargo semver-checks` succeeds if `<crate>` already exists on crates.io;
- docs.rs-style nightly docs build for `<crate>`;
- `<crate> X.Y.Z` is not already published;
- tag `<crate>-vX.Y.Z` does not already exist;
- `cargo publish --dry-run` succeeds for `<crate>`.

### 4. Final approval

The maintainer approves the waiting `release` environment deployment. Before
approving, check the workflow is `Release: publish crate`, the crate and version
are intended, and the changelog section is the one reviewed.

After approval, the workflow: authenticates to crates.io through Trusted
Publishing; publishes `<crate>`; creates annotated tag `<crate>-vX.Y.Z`; extracts the
`crates/<crate>/CHANGELOG.md` section for `X.Y.Z`; and creates the GitHub
Release. Nothing else listens for the tag: GitHub raises no workflow events for
a ref pushed with `GITHUB_TOKEN`, so anything that must follow a release has to
be a step of this workflow.

Each crate ships the modules that wrap it, stamped with its own version —
`draco-io` carries obj, ply, stl and fbx; `draco-gltf` carries gltf;
`draco-core` carries drc. A module that is built but assigned to no crate fails
the packaging step rather than being silently left out.

## First Release

Trusted Publishing cannot publish a crate that does not exist yet. For a brand
new crate, do the first publish locally with a short-lived token, then create the
tag with the one-off workflow.

1. Push the `release: prepare <crate> vX.Y.Z` commit to `main` and wait for CI.
2. Create a crates.io token: short expiration; scope `publish-new`; unrestricted
   (the crate does not exist yet).
3. Publish locally, in dependency order if releasing several for the first time:

   ```bash
   cargo login <token>
   cargo publish --manifest-path crates/draco-core/Cargo.toml
   # wait until crates.io resolves draco-core X.Y.Z, then:
   cargo publish --manifest-path crates/draco-io/Cargo.toml
   # wait, then:
   cargo publish --manifest-path crates/draco-gltf/Cargo.toml
   cargo logout
   ```

4. Revoke the token.
5. Create the `<crate>-vX.Y.Z` tag with the one-off workflow (it verifies the version
   is published, then tags): run `First release only: create tag`
   (`tag-first-release.yml`) with the crate, the version, and the confirmation
   string — e.g. for `draco-gltf`: `crate=draco-gltf`, `version=0.1.0`,
   `confirm=tag draco-gltf`. That workflow also creates the GitHub Release
   itself, because a tag pushed with `GITHUB_TOKEN` raises no workflow events
   and so triggers nothing by itself.
6. Configure Trusted Publishing for `<crate>` before its next release.

## One-Time Setup

### GitHub Actions permissions

`Settings` -> `Actions` -> `General` -> `Workflow permissions` -> enable
`Read and write permissions`, so the publish/tag/release workflows can push tags
and create Releases with `GITHUB_TOKEN`.

### Release environment approval

The publish workflow uses `environment: release` for the real publish job.
Configure it with required reviewers so a publish cannot proceed without
approval after preflight:

- Environment name: `release`;
- Required reviewers: `Filyus`;
- Wait timer: `0`;
- Deployment branch policy: none (the workflow checks it was started from `main`,
  that `HEAD` is the matching release commit, and that CI passed).

### Trusted Publishing

Configure one Trusted Publisher entry **per crate** (`draco-core`, `draco-io`,
`draco-gltf`), all pointing at the same workflow:

- Publisher: `GitHub`;
- Repository owner: `Filyus`;
- Repository name: `draco-rust`;
- Workflow filename: `publish.yml`;
- Environment name: `release`.

## npm packages

The seven `@draco-rust/*` packages share one version, in `web/npm/VERSION`,
and are released together by `Release: publish npm packages`
(`.github/workflows/npm.yml`):

1. The agent bumps `web/npm/VERSION`, turns `Unreleased` in
   `web/CHANGELOG.md` into `## [X.Y.Z] - date`, and shows the diff.
2. After the maintainer approves the wording, the agent pushes it as one commit
   with the subject `release: prepare npm vX.Y.Z` and waits for CI on it.
3. The workflow is started from `main`. Its build job refuses anything but that
   commit with a changelog section and green CI, builds every package with
   `build-tool --npm`, and runs `test:npm-packages` on the tarballs.
4. The maintainer approves the `release` environment; the publish job uploads
   each package not yet at that version, with provenance.
5. The agent checks each package's registry shasum against the workflow's
   artifact and that jsDelivr serves the wasm.

Each package has a Trusted Publisher on npmjs.com (package -> Settings ->
Trusted publishing): `GitHub Actions`, owner `Filyus`, repository
`draco-rust`, workflow `npm.yml`, environment `release`. A new package is
published once by hand (`npm publish` from `web/npm/dist/<name>`, with 2FA)
before it can be given one.
