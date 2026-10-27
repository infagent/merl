# Merl

Merl is durable local messaging for coding agents. Named agents exchange immutable messages, keep receipt separate from completed work, and receive notifications in their existing host sessions. A shell dispatcher waits for mail without asking a model to poll.

Run `merl usage` for a short setup/join/resume/handoff primer, or start with the [first-message walkthrough](docs/onboarding.md) for a disposable setup and optional project role/checkpoint guidance. Automatic wake is currently verified for Codex; other hosts can use the CLI without a verified automatic wake adapter.

[Teams](docs/teams.md) save shared scope, a discoverable roster and durable member guidance, with individual mail addressed through an explicit team. This requires a release containing #161; v0.3.0 does not include team commands. CLI onboarding works without a skill. The usage primer and canonical `/merl` / `$merl` startup require a release containing #163; v0.3.0 still provides `merl-mail`.

Informational [team announcements](docs/teams.md#informational-announcements), available in a release containing #165, publish ordinary notes atomically to one bounded roster snapshot. Exact retries retain the original recipients; receipt and work completion stay individual.

A coordinator can hand work to a developer, a developer can request a review, and a reviewer can return findings. The project decides roles and merge authority. Merl records the messages and handling facts; a receipt alone never completes an assignment.

## Install and try it

Install a binary release without Rust:

```sh
curl -fsSL https://github.com/infagent/merl/releases/latest/download/install.sh | sh
merl --version
merl help
```

The installer uses `~/.local/bin`, verifies the archive checksum and version before replacing a regular file, and prints PATH guidance. It then offers reusable `merl init` setup through the controlling terminal: matching Codex/Claude skills, a durable mailbox default, and optional dispatcher start instructions. Use `--no-setup` for binary-only installation. Setup never starts a service or guesses a session. Guided setup requires a release containing #155; v0.2.0 and earlier installers are binary-only. See [installation](docs/installation.md) for exact versions, supported Linux/macOS baselines, manual downloads, optional skill setup and uninstall.

To build from source, install the toolchain in `rust-toolchain.toml`:

```sh
cargo build --locked --bin merl
./target/debug/merl help
./target/debug/merl help mail send --json
```

Run `target/debug/merl` directly or put it on your PATH as `merl`. Choose a dedicated absolute mailbox home and register both identities before sending:

```sh
merl mail register --home /tmp/team-mail --agent alice --json
merl mail register --home /tmp/team-mail --agent bob --json
printf 'Please review the proposed change.\n' > /tmp/handoff.txt
merl mail send --home /tmp/team-mail --from alice --to bob \
  --id handoff-1 --kind assign --body-file /tmp/handoff.txt --json
merl mail list --home /tmp/team-mail --agent bob --pending --json
merl mail read --home /tmp/team-mail --agent bob --id handoff-1 --json
merl mail receipt --home /tmp/team-mail --agent bob --id handoff-1 --json
```

Offline recipients keep their mail. To wake an existing Codex session, explicitly bind its UUID and local control socket, then run `merl mail watch` in the foreground. Follow the [setup and recovery guide](docs/agent-mail.md) for binding, replies, completion, start and stop. The current adapter uses `codex queue`; it creates no substitute session and installs no system service.

## Product and limits

The public interface includes pure `merl usage` guidance, `merl init` for user setup, `merl status` for a read-only setup summary, `merl team` for shared scope and roster, `merl mail` for registration, binding and message handling, plus `merl lane` for optional read-only GitHub PR activity. The summary/status update requires a release containing #158; v0.3.0 users can inspect setup with `merl init --show --json`. A named reviewer lane selects a repository and required labels, follows many PRs across sessions, and sends bounded pointers through the same mail transport. It makes no GitHub writes or idle model calls. See [reviewer lanes](docs/reviewer-lanes.md) for setup, pause/resume, independent cursors and polling limits.

Results and incremental help are versioned JSON when requested. The [product contract](docs/product-description.md), [UI contract](docs/user-interface.md), [architecture](docs/architecture.md) and [roadmap](docs/roadmap.md) describe this messaging product.

Mail is retained as plaintext in a private local SQLite store. Same-user identity flags are trusted claims, not authentication or an OS sandbox. Notification is at least once, so agents must handle duplicate wakes. There is no mail purge, encryption or storage quota yet. The [optional skill](skills/merl/SKILL.md) is short transport guidance; [project role guidance](docs/agent-roles.md) stays separate.

The previous semantic-state/compiler/evaluation product has been removed from the working product. Git history retains it. Old runtime stores and external artifacts are not migrated, overwritten or deleted. [ADR 0004](docs/adr/0004-messaging-product.md) records the replacement and supersedes the old plans.

## Contributing and license

Read [CONTRIBUTING.md](CONTRIBUTING.md) for the complete checks and review process. Merl is available under the [MIT License](LICENSE).
