# Install Merl

For a disposable first install, message and optional host binding, follow [onboarding](onboarding.md). It also separates local-artifact verification from the first public-release install check.

Binary releases include Merl and need no Rust toolchain. Run:

```sh
curl -fsSL https://github.com/infagent/merl/releases/latest/download/install.sh | sh
merl --version
merl help
```

The installer writes `~/.local/bin/merl` without sudo and prints PATH guidance if needed. After verifying and installing the binary, it offers `merl init` on the controlling terminal. That optional setup installs matching user skills, creates a chosen durable mailbox directory, saves a default and offers dispatcher start instructions. It edits no shell profiles or native host configuration and starts no process or service.

Guided setup is implemented in source by #155. Use a binary release that includes it; v0.2.0 and earlier installers are binary-only and do not recognize setup flags. `merl help init` checks whether a binary supports the initializer.

To select an exact version and destination, replace the example tag with a published binary release:

```sh
curl -fsSL https://github.com/infagent/merl/releases/download/v0.2.0/install.sh \
  | sh -s -- --version v0.2.0 --bin-dir "$HOME/.local/bin"
```

Each published installer defaults to its own tag. The latest URL selects that installer once; every subsequent asset download uses the same immutable tag. The source copy of `install.sh` resolves latest once if no version is supplied. Only stable `vMAJOR.MINOR.PATCH` tags are supported.

## Guided and unattended setup

After a package-manager, Cargo or manual installation, the same setup is available directly:

```sh
merl init
merl status
merl status -v  # full JSON
```

The guide detects Codex/Claude from executable or skill-directory presence without running a host. Select one, both or `none`; choose a durable absolute mailbox home, then optionally **show dispatcher start instructions**. Review concrete paths before accepting. `--global` means user-global scope, with no project config discovery.

Prompts read `/dev/tty`, never the install script's stdin. Without a controlling terminal the installer defaults to binary-only and prints how to run setup later. `--unattended` also defaults to binary-only. To skip the offer explicitly, pipe to `sh -s -- --no-setup`. To request unattended setup, forward exact initializer arguments after `--`:

```sh
# Use an installer from a release containing guided setup.
sh install.sh --unattended --setup -- \
  --global --host codex --host claude --home /absolute/durable-mail \
  --dispatcher foreground --yes --json
```

No argument string is evaluated. The unattended guard follows option/value boundaries: a path value spelled `--yes` is not consent. Unsupported argument grammar fails before invoking init. Explicit unattended setup requires `--yes` (or read-only `--dry-run`/`--show`). First setup also requires `--host` and `--home`; a known setup can reuse saved choices. JSON never prompts. `--setup` failure exits nonzero and says that the verified binary remains installed while setup is incomplete. Retry `merl init` with the same choices; binary download/checksum/version guarantees are unchanged.

Preview and inspect without creating even directories or locks:

```sh
merl init --global --host codex --home /absolute/durable-mail --dry-run --json
merl init --show --json
```

The shared summary/status update is implemented by #158 and requires a release containing it; v0.3.0 users can inspect setup with `merl init --show --json`. Default init preview/results/show use the same concise summary as `merl status`. Preview marks proposed changes before confirmation; it does not dump internal JSON or placeholder session commands. `merl status -v`/`--verbose` returns full JSON; explicit `merl init --json` keeps the existing automation record. Status is read-only and never infers that saved foreground instructions mean a dispatcher is running. Interrupted receipt choices are shown as planned rather than active defaults. See the [status field/source contract](user-interface.md#setup-status).

## Setup paths and defaults

| Artifact | Default location |
| --- | --- |
| Codex user skill | `~/.agents/skills/merl/SKILL.md` |
| Claude user skill | `~/.claude/skills/merl/SKILL.md` |
| Linux settings | `$XDG_CONFIG_HOME/merl/config.json`, or `~/.config/merl/config.json` |
| Linux suggested mailbox | `$XDG_DATA_HOME/merl/mail`, or `~/.local/share/merl/mail` |
| macOS settings | `~/Library/Application Support/merl/config.json` |
| macOS suggested mailbox | `~/Library/Application Support/merl/mail` |

Host locations follow [Codex skills](https://learn.chatgpt.com/docs/build-skills) and [Claude local skills](https://code.claude.com/docs/en/skills). `CLAUDE_CONFIG_DIR` selects the Claude profile root; `--claude-skills-dir` or `--codex-skills-dir` explicitly chooses a personal skill root in another profile. New setup receives `merl/SKILL.md`. These canonical paths require a release containing #163; v0.3.0 installs `merl-mail/SKILL.md`. An upgrade retains that legacy file and ownership instead of rewriting it. Setup uses the exact short skill embedded in that binary, offline, for either host; it never downloads a skill from main. AGENTS.md, CLAUDE.md, host settings, hooks and permissions are untouched.

`--config /absolute/config.json` selects the settings file and anchors its receipt/lock. Mail and lane commands use `--home` first and skip every settings read when it is supplied. Otherwise they read only the selected config or known user location. Missing settings ask for explicit home or setup; malformed settings return `INIT_CONFIG` before opening mail. Help, version and unknown commands do not load settings. There is no default production `/tmp` home or project-directory search.

The config schema is `merl.config/v1`, with `mail_home`, `skills` (host-to-absolute-SKILL.md paths), and `dispatcher` (`foreground` or `none`). Setup does not open an existing mailbox store, migrate data, register agents, infer a session, replace a binding or alter a running dispatcher.

With foreground instructions selected, `merl init --show --json` returns `dispatcher.state=not_started` and a command using the current Merl and detected real Codex absolute paths. Run the listed register/status/bind steps with your supplied identity, existing UUID/socket and current generation, then run `start_command` from a stable directory. Stop only that foreground process with Ctrl-C. If Codex is absent, state is `missing_codex` and there is no start command. Claude skill setup still works; automatic Claude wake is not supported. See [mail setup](agent-mail.md) for binding and recovery.

## Ownership and failed setup

Unchanged reruns report `unchanged`. Upgrades replace only unchanged Merl-owned files; identical pre-existing skills remain borrowed, even through recovery. Changed/foreign content or symlink destinations produce `INIT_CONFLICT`, including under `--yes`. The whole conflict plan is checked before writes. The known sibling transition from `ROOT/merl-mail/SKILL.md` to `ROOT/merl/SKILL.md` preserves the selected custom root and legacy ownership. Other destination moves still require uninstalling that host first. Borrowed or edited settings that would need different bytes also conflict before canonical publication. Files and setup metadata must be outside the mailbox subtree.

Beside `config.json`, `.config.json.merl-init.json` records generated expected bytes, ownership and pending publication identity. New canonical setup uses `merl.init-receipt/v2`, adding at most one retained legacy path per host while keeping configuration v1. V2 intent precedes canonical publication and settings come last; it contains no backups of host settings or unrelated content. A stable zero-byte `.config.json.merl-init.lock` serializes setup. Files and new directories are private on Unix. Different config filenames use the corresponding basename in these sibling names.

An I/O failure or cancellation can leave recorded partial progress. `merl init --show --json` reports `incomplete`; retry the same options to finish it, or use scoped uninstall. Publication uses file identity to recognize interrupted writes without adopting foreign identical files. Unknown `.NAME.merl-stage-N` files are preserved; at most 32 staging slots are attempted per destination, and reads/receipts are bounded. If all slots are occupied, inspect that specific directory and preserve any unrecognized files before retrying. Filesystems must support hard links, atomic rename, Unix identity and sync; failures have no weaker fallback.

## Canonical startup and legacy recovery

After supporting setup, invoke `$merl scope/member starting guidance` in Codex or `/merl scope/member starting guidance` in Claude. The short skill reads `merl usage` and only needed leaf help. It asks for missing mailbox access, scope/identity and guidance only for a fresh member; resume reuses saved values. Role labels and example names imply no fixed organization. Startup neither replaces a binding nor starts a dispatcher.

Preview/status distinguishes the canonical skill from retained legacy files, with ownership separate from matching the canonical bundle. Deliberately retained old bytes need not match that bundle. Scoped uninstall handles the selected host's canonical and retained legacy entries, removing only unchanged owned files; borrowed/unmanaged files and all mail survive. Modified recorded files remain conflicts, including during uninstall: preserve edits and reconcile the reported selected file before retrying. There is no force overwrite or detach mode.

An incomplete v1 receipt cannot be migrated, and rejection writes nothing. With the selected config, the new binary can perform existing safe legacy uninstall and then retry setup:

```sh
merl init --config /absolute/config.json --uninstall --yes --json
merl init --config /absolute/config.json --host codex --home /absolute/durable-mail --yes --json
```

Choose the required hosts/home explicitly after full removal. The diagnostic includes the actual selected config. Resuming with the original compatible initializer is another option. Legacy uninstall keeps v1 rather than promoting unfinished intent; interrupted v2 canonical setup instead resumes with the same current options. Older initializers reject v2 before writing, while their unqualified mail commands still read the saved v1 configuration. No running dispatcher is restarted by any upgrade.

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

On macOS, select the matching target and use `shasum -a 256 --check` for verification. Stop if verification fails. Move the verified binary into a user-writable directory on PATH. Each supporting archive also contains `LICENSE`, `VERSION`, `REVISION`, canonical `skills/merl/SKILL.md`, and unchanged compatibility `skills/merl-mail/SKILL.md` from that exact release commit. Releases through v0.3.0 contain only the legacy skill.

Skill setup is optional. With a supporting binary, prefer `merl init` for ownership-aware setup and upgrades. For older releases or manual ownership, after verifying and unpacking the archive, copy the matching packaged skill (canonical `merl/skills/merl/SKILL.md` in a release containing #163) into a skill location supported by your coding host. For Codex with a canonical-skill archive, an explicit fresh-directory setup is:

```sh
mkdir -p "$HOME/.agents/skills"
mkdir "$HOME/.agents/skills/merl"
cp merl/skills/merl/SKILL.md "$HOME/.agents/skills/merl/SKILL.md"
```

If the skill directory exists, inspect it before updating it; do not overwrite local guidance by accident. The archive's `VERSION` records which binary release the skill accompanies. Registration, binding and dispatcher startup remain explicit steps in the [mail setup guide](agent-mail.md).

## Upgrade and uninstall

Rerun the latest installer to upgrade, or use the pinned command for a chosen version. A running dispatcher keeps using its loaded executable; an install does not restart it. Stop and restart only your own foreground dispatcher when you choose to use the new binary.

For setup owned by a supporting initializer:

```sh
merl init --uninstall --host claude --yes --json  # only Claude; retain default and Codex
merl init --uninstall --yes --json                # all unchanged owned skills/settings
```

Local edits conflict and survive. Borrowed skills and every mailbox directory/descendant survive, including empty mailboxes. Only recorded empty setup directories with matching identity may be removed. The zero-byte coordination lock and its containing directory remain to avoid racing another initializer's open lock. Uninstall never removes a binary or stops a process.

Remove the installed binary separately, for example `rm "$HOME/.local/bin/merl"`. Manually copied skills remain manually owned: inspect them before removing anything. A running dispatcher keeps its loaded executable until you explicitly stop it.

## Build a tag with Cargo

With the toolchain specified by that tag and a C toolchain for bundled SQLite:

```sh
cargo install --git https://github.com/infagent/merl --tag v0.2.0 --locked --bin merl merl
merl --version
```

This uses Cargo's normal installation directory, usually `~/.cargo/bin`. For development, build the checked-out source with `cargo build --locked --bin merl`. See [contributing](../CONTRIBUTING.md) for checks and [release distribution](release-distribution.md) for publishing/recovery details.
