# Release distribution

Release Please manages versions, release PRs and changelogs. The existing GitHub App token creates those PRs and, after a release PR merges and Check passes, a draft release plus an actual Git tag. `draft: true` and `force-tag-creation: true` in the pinned [Release Please schema](https://github.com/googleapis/release-please/blob/v17.6.0/schemas/config.json) make the tag available before any build. The pinned implementation sets `target_commitish` to the same full SHA returned by the action.

The Release Please workflow calls Release assets only when root `release_created == 'true'`, passing its tag and SHA. It does not wait for a draft-release event: GitHub does not dispatch normal release events for drafts. The App token also avoids the normal `GITHUB_TOKEN` suppression of follow-on events when Release Please creates PRs/tags. The final asset job uses a scoped `contents: write` workflow token and needs no follow-on release event. See [GitHub's trigger rules](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow) and [release events](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#release).

## Pre-1.0 versions

Merl remains on `0.x` while its public interface develops. The root `bump-minor-pre-major: true` setting applies to the Rust package through the manifest configuration. Breaking conventional commits advance the minor version during `0.x`; features also advance the minor, and fixes advance the patch. For example, from `0.1.0`, a breaking change or feature produces `0.2.0`, while a fix alone produces `0.1.1`. Keep breaking commits marked accurately even before 1.0.

Moving to `1.0.0` requires a separate reviewed readiness decision and deliberate release-policy/version change. A conventional breaking commit alone does not make that decision during `0.x`. Do not leave a persistent `release-as` override to force one release. After a deliberate 1.0 release, ordinary breaking changes again advance the major version. These rules follow the pinned [17.6.0 version strategy](https://github.com/googleapis/release-please/blob/v17.6.0/src/versioning-strategies/default.ts).

Release Please owns generated versions and release notes. A policy PR does not increment Cargo versions or the manifest to claim a release. After the policy reaches main, the successful Check run lets Release Please regenerate its release PR. Inspect its title, notes, root/member Cargo versions, lockfile and manifest together before merging. With the current `0.1.0` bootstrap manifest and no intervening release, expect `0.2.0`; the bootstrap value alone does not prove a published tag exists. Never correct only the bot PR title or manually move a tag. The generated proposal still passes the normal review and binary-publication gates below.

`scripts/check_release_policy.cjs` uses Release Please's public manifest/proposal API with a synthetic read-only source. It verifies generated versions and Cargo/manifest updates for breaking, feature and fix commits, later `0.x` releases and post-1.0 behavior. It needs Node 20+ and a temporary install of `release-please@17.6.0`; no GitHub client, credentials or release mutation is involved. Run the commands in [CONTRIBUTING.md](../CONTRIBUTING.md). Keep this pin aligned with the action's locked library and the release workflow's dry-run version when upgrading automation.

## Build and publish

The workflow validates a stable tag, its commit, draft `target_commitish`, root Cargo version and Release Please manifest. Builds check out that tag and verify the frozen SHA. Native runners are pinned to Ubuntu 24.04 x86_64/arm64 and macOS 15 Intel/arm64; the macOS deployment target is 15.0. GitHub documents these [runner labels](https://docs.github.com/en/actions/reference/runners/github-hosted-runners). CI also packages and performs a clean-directory installation on all four platforms for PRs, without release credentials or network-backed installer tests.

Each archive contains the binary, matching optional skill, license, version and source revision. Packaging checks binary `--version` and help. Archive headers use fixed timestamps and owner fields; this makes archives of identical inputs repeatable, without claiming independently reproducible compiler output. The full asset set is four archives, a version-bound `install.sh`, and `SHA256SUMS` covering the other five files.

Only after all native jobs pass does the publisher assemble and validate every archive's revision/version/skill/license. It refuses published releases or unexpected existing assets. While the release remains draft, it uploads the exact expected names with `--clobber`, downloads them again, verifies checksums and bytes, and rechecks the draft and remote tag before publishing. GitHub's `make_latest=legacy` selects latest by release date/semantic version rather than forcing a recovered older release to latest. A failure before the final publication call leaves the release draft and the previous latest installable release available. A timeout during publication can be ambiguous; inspect the release before retrying. No rollback of a published release is automatic.

Draft lookup uses authenticated release listing, since the REST by-tag endpoint exposes published releases only. It follows up to 100 pages of 100 entries, then fails with an explicit visibility/tag error if absent. Subsequent publication checks address the same release ID. GitHub requires push-level access to see drafts; the source-validation job requests `contents: write` for this visibility while performing only GET requests. Native build jobs retain read-only repository access. The publisher has `contents: write` for upload/publication; Release Please retains the App permissions it already needs for contents, issues and pull requests. Actions are pinned by full commit. No installer or build starts a service, changes agent configuration or migrates runtime state.

## Recover a failed draft

Prefer **Re-run failed jobs** on the release workflow run. To rebuild all assets for an existing failed draft after checking its tag and SHA:

```sh
gh workflow run release-assets.yml --ref main -f tag=v0.2.0
```

This is a publishing operation for a maintainer, not an installer verification command. Recovery resolves the existing immutable tag and requires the draft's full target SHA to match. It never selects the current main commit as the build source. It rebuilds all four targets and replaces only known draft assets. Existing unknown assets require inspection; the workflow does not delete them. Per-tag concurrency serializes normal publication and recovery. Re-running Release Please alone may find that its draft already exists and emit no new release output; use failed-job rerun or the explicit draft recovery workflow instead.

Published releases fail closed on retry and remain unchanged. This avoids silently rebuilding binaries under an existing public tag. Correct a released defect with a new release PR/tag. Tags without the installation scripts from this change cannot be backfilled through this workflow without a separate reviewed decision.

## Verify locally

Run the full checks in [CONTRIBUTING.md](../CONTRIBUTING.md). Synthetic tests use disposable local Git repositories, fake curl downloads and a fake GitHub CLI; they publish nothing. For the native package smoke on a clean committed checkout, set `tag` to `v` plus the root package version and `target` to your native target from the [installation table](installation.md):

```sh
tag=v0.2.0
target=x86_64-unknown-linux-gnu
revision=$(git rev-parse HEAD)
cargo build --locked --release --bin merl --target "$target"
python3 scripts/release.py package --tag "$tag" --sha "$revision" \
  --target "$target" --binary "target/$target/release/merl" --output /tmp/merl-packages
python3 scripts/smoke_package.py --tag "$tag" \
  --archive "/tmp/merl-packages/merl-$tag-$target.tar.gz"
```

The package command rejects dirty tracked source or a version mismatch. Release jobs additionally require an actual matching tag. The smoke substitutes local downloads and installs into a disposable directory with spaces, then runs the packaged `--version` and help. It does not install globally or use the live mailbox/dispatcher.

## Packaging choice

A single binary, four native targets and an existing Release Please/App path need only an archive builder, one installer and draft publication checks. [cargo-dist](https://github.com/axodotdev/cargo-dist) provides a broader generated distribution workflow and installers; introducing its release orchestration here would add a second configuration surface around the draft/App flow. The small scripts keep that existing path explicit. Revisit the choice if additional package managers, cross compilation, signing or more products justify it. Checksums currently provide integrity, not an independent signature or attestation.
