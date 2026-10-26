# Install Merl

Binary releases include Merl and need no Rust toolchain. Run:

```sh
curl -fsSL https://github.com/infagent/merl/releases/latest/download/install.sh | sh
merl --version
merl help
```

The installer writes `~/.local/bin/merl` without sudo. If that directory is outside your PATH, it prints guidance; add the directory to your shell configuration yourself. It does not edit profiles, configure agents, install a skill or service, start a dispatcher, or touch a mailbox. Install commands become available after the first binary release finishes; older source-only releases have no installer asset.

To select an exact version and destination, replace the example tag with a published binary release:

```sh
curl -fsSL https://github.com/infagent/merl/releases/download/v0.2.0/install.sh \
  | sh -s -- --version v0.2.0 --bin-dir "$HOME/.local/bin"
```

Each published installer defaults to its own tag. The latest URL selects that installer once; every subsequent asset download uses the same immutable tag. The source copy of `install.sh` resolves latest once if no version is supplied. Only stable `vMAJOR.MINOR.PATCH` tags are supported.

## Supported systems

| System | Archive target | Build and support baseline |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | Ubuntu 24.04; glibc 2.39 or newer |
| Linux arm64 | `aarch64-unknown-linux-gnu` | Ubuntu 24.04; glibc 2.39 or newer |
| macOS Intel | `x86_64-apple-darwin` | macOS 15 or newer |
| macOS Apple Silicon | `aarch64-apple-darwin` | macOS 15 or newer |

Native CI builds and installs each archive on these baselines. Earlier glibc/macOS versions and musl/Alpine are not supported by these assets. SQLite is bundled; a separate SQLite installation is unnecessary. The installer needs a POSIX shell, curl with HTTPS support, tar/gzip, standard Unix file tools, and `sha256sum` or macOS `shasum`. The current host adapter uses Unix sockets; there is no Windows release. macOS binaries are unsigned and not notarized. The installer runs `--version` before replacing anything, so an OS policy or runtime incompatibility leaves an existing installation intact.

The installer checks the archive against the same release's `SHA256SUMS`, extracts only the binary into a temporary file in the destination directory, verifies its version, and renames it over the previous regular file. Downloads, bad checksums, permissions and incompatible binaries fail before replacement. Directory and symlink destinations are rejected, including dangling symlinks. Its cleanup removes only its own temporary files. This boundary trusts the user's filesystem and PATH; it does not defend against a hostile same-user process racing the install.

Checksums detect corruption and mismatched assets. They are not independent proof of publisher identity: both archive and checksums come from the GitHub release over HTTPS. To inspect the script before running it, download the versioned `install.sh`, read it, then run `sh install.sh --version TAG`.

## Manual download and optional skill

Download `SHA256SUMS` and `merl-TAG-TARGET.tar.gz` from the [release page](https://github.com/infagent/merl/releases). For example, on Linux x86_64:

```sh
tag=v0.2.0
asset="merl-$tag-x86_64-unknown-linux-gnu.tar.gz"
curl -fsSLO "https://github.com/infagent/merl/releases/download/$tag/$asset"
curl -fsSLO "https://github.com/infagent/merl/releases/download/$tag/SHA256SUMS"
grep "  $asset\$" SHA256SUMS | sha256sum --check
tar -xzf "$asset"
./merl/merl --version
./merl/merl help
```

On macOS, select the matching target and use `shasum -a 256 --check` for verification. Stop if verification fails. Move the verified binary into a user-writable directory on PATH. Each archive also contains `LICENSE`, `VERSION`, `REVISION`, and `skills/merl-mail/SKILL.md` from that exact release commit.

Skill setup is optional. After verifying and unpacking the archive, copy `merl/skills/merl-mail/SKILL.md` into a skill location supported by your coding host. For Codex, an explicit fresh-directory setup is:

```sh
mkdir -p "$HOME/.agents/skills"
mkdir "$HOME/.agents/skills/merl-mail"
cp merl/skills/merl-mail/SKILL.md "$HOME/.agents/skills/merl-mail/SKILL.md"
```

If the skill directory exists, inspect it before updating it; do not overwrite local guidance by accident. The archive's `VERSION` records which binary release the skill accompanies. Registration, binding and dispatcher startup remain explicit steps in the [mail setup guide](agent-mail.md).

## Upgrade and uninstall

Rerun the latest installer to upgrade, or use the pinned command for a chosen version. A running dispatcher keeps using its loaded executable; an install does not restart it. Stop and restart only your own foreground dispatcher when you choose to use the new binary.

Remove the installed binary to uninstall, for example `rm "$HOME/.local/bin/merl"`. Remove an optional skill only after checking its location and any local edits. Mailboxes and their contents remain in place; uninstalling does not erase mail or stop a running process.

## Build a tag with Cargo

With the toolchain specified by that tag and a C toolchain for bundled SQLite:

```sh
cargo install --git https://github.com/infagent/merl --tag v0.2.0 --locked --bin merl merl
merl --version
```

This uses Cargo's normal installation directory, usually `~/.cargo/bin`. For development, build the checked-out source with `cargo build --locked --bin merl`. See [contributing](../CONTRIBUTING.md) for checks and [release distribution](release-distribution.md) for publishing/recovery details.
