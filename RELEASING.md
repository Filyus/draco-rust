# Releasing

The three crates — `draco-core`, `draco-io`, `draco-gltf` — are versioned and
released **independently**, one crate per release. Each has its own version in
`crates/<crate>/Cargo.toml`, its own `crates/<crate>/CHANGELOG.md`, and its own
`<crate>-vX.Y.Z` release tags.

In the steps below, **`C` is the crate being released** — substitute
`draco-core`, `draco-io`, or `draco-gltf`. For example, `crate=C` means
`crate=draco-gltf` when releasing `draco-gltf`.

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

`draco-core` <- `draco-io` <- `draco-gltf`. A dependent can only be released once
the dependency version it pins is published. Release in dependency order, and
treat each crate as its own release (its own commit, changelog, and tag).

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
[`RELEASING-AGENT.md`](RELEASING-AGENT.md). In short, for crate `C`:

- bump `version` in `crates/C/Cargo.toml`;
- if releasing a dependent that should pick up a just-published dependency,
  update that pin too (separate release per crate);
- write the `crates/C/CHANGELOG.md` section, grouped by the
  [changelog taxonomy](RELEASING-AGENT.md#changelog-taxonomy), including only
  commits that touched `C`;
- rewrite terse subjects into clear, user-facing notes;
- remove internal noise (C++ bridge, debug output, lint/CI/bench-only, or
  demo-only commits);
- keep feature/code changes out of the release commit.

Do not run release-only checks manually — the publish preflight runs semver,
docs.rs-style docs, duplicate-version, duplicate-tag, and `--dry-run` checks.

Show the maintainer the diff before committing:

```powershell
git diff -- crates/C/Cargo.toml crates/C/CHANGELOG.md crates/C/README.md README.md
```

### 2. Commit and push

After approval, create one release commit with this exact subject (`C` is the
crate name, e.g. `draco-gltf`):

```text
release: prepare C vX.Y.Z
```

```powershell
git add crates/C/Cargo.toml crates/C/CHANGELOG.md crates/C/README.md README.md
git commit -m "release: prepare C vX.Y.Z"
git push origin main
```

The exact subject matters: the publish workflow ignores ordinary pushes and only
continues when the subject matches `crates/C/Cargo.toml`'s version.

### 3. Start the publish workflow and preflight

The push to `main` starts `Rust CI`. Wait for it to pass, then start the publish
workflow for the crate:

```powershell
gh workflow run publish.yml --ref main -f crate=C
```

Preflight checks, for crate `C`:

- a successful `Rust CI` run exists for this commit;
- the commit subject is exactly `release: prepare C vX.Y.Z`;
- `X.Y.Z` matches `crates/C/Cargo.toml`;
- every internal dependency `C` pins is already published at the pinned version;
- `crates/C/CHANGELOG.md` has a `## [X.Y.Z]` section;
- `cargo semver-checks` succeeds if `C` already exists on crates.io;
- docs.rs-style nightly docs build for `C`;
- `C X.Y.Z` is not already published;
- tag `C-vX.Y.Z` does not already exist;
- `cargo publish --dry-run` succeeds for `C`.

### 4. Final approval

The maintainer approves the waiting `release` environment deployment. Before
approving, check the workflow is `Release: publish crate`, the crate and version
are intended, and the changelog section is the one reviewed.

After approval, the workflow: authenticates to crates.io through Trusted
Publishing; publishes `C`; creates annotated tag `C-vX.Y.Z`; extracts the
`crates/C/CHANGELOG.md` section for `X.Y.Z`; and creates the GitHub Release.

## First Release

Trusted Publishing cannot publish a crate that does not exist yet. For a brand
new crate, do the first publish locally with a short-lived token, then create the
tag with the one-off workflow.

1. Push the `release: prepare C vX.Y.Z` commit to `main` and wait for CI.
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
5. Create the `C-vX.Y.Z` tag with the one-off workflow (it verifies the version
   is published, then tags): run `First release only: create tag`
   (`tag-first-release.yml`) with the crate, the version, and the confirmation
   string — e.g. for `draco-gltf`: `crate=draco-gltf`, `version=0.1.0`,
   `confirm=tag draco-gltf`. The tag triggers the GitHub Release.
6. Configure Trusted Publishing for `C` before its next release.

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
