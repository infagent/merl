# Releasing Merl

Release Please maintains one release pull request from conventional commits on `main`. Merging that pull request updates the version and changelog. After the `Check` workflow passes on the merge commit, the release workflow creates the matching `vX.Y.Z` tag and GitHub release. This repository does not publish a package or installation artifact yet.

## GitHub App credentials

The release workflow authenticates as a GitHub App. The app must be installed on `infagent/merl` with these repository permissions:

- Contents: read and write
- Issues: read and write
- Pull requests: read and write

The app needs no package, administration, workflow-write, or organization permission. If its permissions change, approve the updated installation before expecting the workflow to use them.

Store the credentials as repository secrets named `INFAGENT_RELEASE_APP_CLIENT_ID` and `INFAGENT_RELEASE_APP_PRIVATE_KEY`. These commands prompt for the client ID and read the private key from its file without putting either value in shell history:

```sh
gh secret set INFAGENT_RELEASE_APP_CLIENT_ID --repo infagent/merl
gh secret set INFAGENT_RELEASE_APP_PRIVATE_KEY --repo infagent/merl < /path/to/private-key.pem
```

The workflow exchanges those credentials for a short-lived installation token restricted to this repository and the three listed permissions. The token action masks the token and revokes it when the job ends.

## Release gate

The release workflow listens for completed runs of `Check`. It proceeds only when that run succeeded, came from a `push`, and ran on the repository's default branch. A pull request, fork, failed check, or non-default branch cannot mint the release token through this path.

Rust checks receive neither GitHub App credential. They run with read-only repository permissions through the `pull_request` event. The separate size-label workflow uses `pull_request_target` only to apply a label; it checks out trusted default-branch configuration and never runs pull-request code.

## Dry run

Manual dispatch validates the configuration without opening a pull request or creating a release:

```sh
gh workflow run release-please.yml --repo infagent/merl
gh run watch --repo infagent/merl
```

The dry-run job requests a read-only installation token. It uses Release Please 17.6.0, the same version bundled by the pinned Release Please action, and reports the changes it would propose from the default branch.

## Required checks

Protect `main` and require the `Rust checks` status from the `Check` workflow. CI runs these same commands locally and in GitHub Actions:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps
```

Cargo's download cache excludes `target/` and separates pull-request entries from trusted pushes. A fork cannot write a cache used by the default branch.
