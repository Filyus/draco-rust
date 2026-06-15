# Releasing

Normal releases are optimized for a solo maintainer working with Codex:

1. Codex verifies that the working tree is clean and current with `origin/main`.
2. Codex prepares the version bump and release notes locally.
3. The maintainer reviews the `CHANGELOG.md` diff.
4. Codex pushes one release commit to `main`.
5. CI validates that exact commit.
6. Once CI is green, the `Release: publish crates` workflow is started manually
   (`gh workflow run publish.yml --ref main`, or the Actions UI). It runs
   preflight checks, then waits for the `release` environment approval.
7. The maintainer approves the environment deployment. Only then does the
   workflow publish `draco-core`, wait for it to become visible to crates.io,
   publish `draco-io`, create the annotated tag, and create the GitHub Release.
8. The existing `Release` workflow builds the WASM assets for that tag and
   uploads them to the same GitHub Release.

The publish is started by hand rather than automatically after CI: crates.io
Trusted Publishing rejects GitHub's `workflow_run` event, so crates.io publish
uses the `workflow_dispatch` trigger.

Release preparation (version bump + changelog) is done by hand following
[`RELEASING-AGENT.md`](RELEASING-AGENT.md). Nothing in this flow publishes
crates, pushes tags, or creates GitHub Releases outside the gated publish
workflow.

## Normal Release

Use this path after both crates already exist on crates.io and Trusted
Publishing is configured.

### 1. Prepare the Release Locally

Codex must start from `main` and inspect the working tree before generating
anything:

```powershell
git fetch origin
git switch main
git pull --ff-only origin main
git status --short
```

If `git status --short` prints anything, classify the changes before preparing
the release:

- If the current conversation makes it clear what normal feature/fix/docs commit
  is missing, create that commit first, push it to `main`, and wait for CI.
- If the changes are clearly unrelated local work, ask whether to commit, stash,
  discard, or postpone them.
- If the changes are ambiguous, stop and ask. Do not guess and do not fold them
  into the release commit.

The release commit should be easy to review and should not accidentally absorb
in-progress work.

Codex prepares the version bump and changelog by hand, following the step list
in [`RELEASING-AGENT.md`](RELEASING-AGENT.md). In short:

- bump both `crates/draco-core/Cargo.toml` and `crates/draco-io/Cargo.toml` to
  the same version;
- update the `draco-core` dependency requirement in
  `crates/draco-io/Cargo.toml` to the same version;
- write the `CHANGELOG.md` section grouped by the
  [changelog taxonomy](RELEASING-AGENT.md#changelog-taxonomy);
- rewrite terse commit subjects into clear, user-facing notes;
- remove internal noise (C++ bridge only, debug output, lint-only, CI-only,
  benchmark-only, or web-demo-only commits unless the release asset behavior
  changed);
- keep feature/code changes out of the release commit.

Do not run release-only checks manually. The publish workflow's preflight runs
semver compatibility, docs.rs-style docs, duplicate-version, duplicate-tag, and
`cargo publish --dry-run` checks where they can run before publication.

The release commit should normally change only:

- `crates/draco-core/Cargo.toml`;
- `crates/draco-io/Cargo.toml`;
- `CHANGELOG.md`;
- small release-facing docs, only when they must mention the new version.

Codex must show the maintainer the diff before committing:

```powershell
git diff -- crates/draco-core/Cargo.toml crates/draco-io/Cargo.toml CHANGELOG.md README.md crates/draco-core/README.md crates/draco-io/README.md
```

The maintainer reviews the changelog wording and says whether it is ready.

### 2. Commit and Push

After approval, Codex reads the version from `crates/draco-core/Cargo.toml` and
creates one release commit with this exact subject:

```text
release: prepare draco-rust vX.Y.Z
```

Example:

```powershell
$version = python -c "import tomllib; print(tomllib.load(open('crates/draco-core/Cargo.toml', 'rb'))['package']['version'])"

git add crates/draco-core/Cargo.toml crates/draco-io/Cargo.toml CHANGELOG.md README.md crates/draco-core/README.md crates/draco-io/README.md
git commit -m "release: prepare draco-rust v$version"
git push origin main
```

The exact commit subject matters. The publish workflow ignores ordinary pushes
and only continues when the subject matches the shared crate version.

### 3. Start the Publish Workflow and Preflight

The push to `main` starts `Rust CI`. Wait for it to pass.

Once CI is green, start `Release: publish crates` manually against `main`:

```powershell
gh workflow run publish.yml --ref main
```

For a valid release commit, preflight checks:

- a successful `Rust CI` run exists for this commit;
- the commit subject is exactly `release: prepare draco-rust vX.Y.Z`;
- `X.Y.Z` matches both publishable crate versions;
- `draco-io` depends on `draco-core = X.Y.Z`;
- `CHANGELOG.md` has a `## [X.Y.Z]` section;
- `cargo semver-checks` succeeds for each crate that already exists on
  crates.io;
- docs.rs-style nightly documentation builds succeed;
- neither crate version is already published on crates.io;
- tag `vX.Y.Z` does not already exist;
- `cargo publish --dry-run` succeeds for `draco-core`.

`draco-io` cannot complete a full dry run for a new shared version until
`draco-core X.Y.Z` exists in the crates.io index. The publish job therefore
publishes `draco-core`, waits for the index, then dry-runs and publishes
`draco-io` before tagging the release.

### 4. Final Approval

The maintainer approves the waiting `release` environment deployment in GitHub
Actions.

Before approving, check:

- the workflow is `Release: publish crates`;
- the release SHA is the release commit that passed CI;
- both crate versions are the intended version;
- the changelog section is the one reviewed before the release commit.

After approval, the workflow:

1. authenticates to crates.io through Trusted Publishing;
2. publishes `draco-core`;
3. waits until crates.io can resolve `draco-core X.Y.Z`;
4. runs `cargo publish --dry-run` for `draco-io`;
5. publishes `draco-io`;
6. creates annotated tag `vX.Y.Z`;
7. extracts the `CHANGELOG.md` section for `X.Y.Z`;
8. creates the GitHub Release from those notes.

## First Release

Trusted Publishing cannot publish a crate that does not exist yet. For a brand
new crate, do the first publish locally with a short-lived token, then configure
Trusted Publishing for normal updates.

1. Push the release commit to `main` and wait for CI to pass.
2. Create a crates.io token:
   - expiration: short, for example one day;
   - scope: `publish-new`;
   - crate restriction: unrestricted, because the crates do not exist yet.
3. Publish locally in dependency order:

   ```bash
   cargo login <token>
   cargo publish --manifest-path crates/draco-core/Cargo.toml
   # Wait until crates.io resolves draco-core X.Y.Z.
   cargo publish --manifest-path crates/draco-io/Cargo.toml
   cargo logout
   ```

4. Revoke the token.
5. Create annotated tag `vX.Y.Z` from the release commit and push it, or run a
   one-off tag workflow if one exists.
6. Create the GitHub Release from the matching `CHANGELOG.md` section.
7. Configure Trusted Publishing before the next release.

## One-Time Setup

### GitHub Actions Permissions

In the GitHub repository settings, open:

`Settings` -> `Actions` -> `General` -> `Workflow permissions`

Enable:

- `Read and write permissions`.

This is required so `Release: publish crates` can push the annotated tag and
create the GitHub Release with `GITHUB_TOKEN`.

### Release Environment Approval

The publish workflow uses `environment: release` for the real publish job.
Configure that environment with required reviewers so a real publish cannot
proceed without GitHub approval after preflight succeeds.

Expected environment settings:

- Environment name: `release`;
- Required reviewers: `Filyus`;
- Prevent self-review: disabled for a personal repository;
- Wait timer: `0`;
- Deployment branch policy: none. The workflow itself checks that it was started
  from `main`, that `main` `HEAD` is a release commit, and that CI passed for it.

### Trusted Publishing

Trusted Publishing is the expected publishing path for normal updates after both
crates exist on crates.io. Configure one Trusted Publisher entry per crate:

- Publisher: `GitHub`;
- Repository owner: `Filyus`;
- Repository name: `draco-rust`;
- Workflow filename: `publish.yml`;
- Environment name: `release`.

The crates.io form should show that the workflow file exists at
`.github/workflows/publish.yml`.
