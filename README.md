# Merl

Merl is durable local messaging for coding agents. Named agents exchange immutable messages, keep receipt separate from completed work, and receive notifications in their existing host sessions. A shell dispatcher waits for mail without asking a model to poll.

A coordinator can hand work to a developer, a developer can request a review, and a reviewer can return findings. The project decides roles and merge authority. Merl records the messages and handling facts; a receipt alone never completes an assignment.

## Build and try it

Install the toolchain in `rust-toolchain.toml`, then build:

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

The public interface is `merl mail`: registration, binding, send/read/receipt/completion, status and notification dispatch. Results and incremental help are versioned JSON when requested. The [product contract](docs/product-description.md), [UI contract](docs/user-interface.md), [architecture](docs/architecture.md) and [roadmap](docs/roadmap.md) describe this messaging product.

Mail is retained as plaintext in a private local SQLite store. Same-user identity flags are trusted claims, not authentication or an OS sandbox. Notification is at least once, so agents must handle duplicate wakes. There is no mail purge, encryption or storage quota yet. The [optional skill](skills/merl-mail/SKILL.md) is short transport guidance; [project role guidance](docs/agent-roles.md) stays separate.

The previous semantic-state/compiler/evaluation product has been removed from the working product. Git history retains it. Old runtime stores and external artifacts are not migrated, overwritten or deleted. [ADR 0004](docs/adr/0004-messaging-product.md) records the replacement and supersedes the old plans.

## Contributing and license

Read [CONTRIBUTING.md](CONTRIBUTING.md) for the complete checks and review process. Merl is available under the [MIT License](LICENSE).
