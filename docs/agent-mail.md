# Durable agent mail

`merl mail` delivers bounded coordination messages to explicitly named local agents. It can notify existing Codex sessions with `codex queue`. This is Merl's primary interface, tracked in [#145](https://github.com/infagent/merl/issues/145). The [product replacement decision](adr/0004-messaging-product.md) defines its scope.

## Start and bind

Build with `cargo build --locked --bin merl`. Choose one dedicated absolute mailbox home and share its path with the cooperating agents. These examples use synthetic identities and paths:

```sh
merl mail register --home /tmp/team-mail --agent alice --json
merl mail register --home /tmp/team-mail --agent bob --json
merl mail status --home /tmp/team-mail --agent bob --json
merl mail bind --home /tmp/team-mail --agent bob \
  --session 00000000-0000-4000-8000-000000000001 \
  --socket /tmp/codex.sock --expected-generation 0 --json
```

Replace the sample UUID with the recipient's explicitly supplied **existing** session UUID and the socket with the running host's control socket. Both names must be registered before sending. The socket must exist and be a Unix socket. The adapter was verified with Codex CLI 0.164.0; check `codex queue --help` for support. Merl does not start the Codex host. A socket replacement or host restart invalidates its fingerprint: inspect status, then bind explicitly using the current generation. Binding never selects a session by a human-readable name. `mail unbind --agent bob --expected-generation 1` disables notification without losing mail; include the same `--home`.

Start one foreground dispatcher using the absolute installed Codex executable:

```sh
merl mail watch --home /tmp/team-mail --codex /usr/local/bin/codex --json
```

Stop it with Ctrl-C or SIGTERM directed at that process. Restart the same command to recover durable work. Never use `killall` or process-name matching. `mail dispatch` with the same flags performs one bounded due-work pass and exits. No system service is installed. Ordinary host permissions must permit the chosen executable, control socket and Codex runtime storage. Failure leaves mail pending; there is no unrestricted fallback. The pointer uses the running Merl executable's absolute path, so keep that binary available to recipients.

## Send, handle and finish

Write a bounded UTF-8 handoff file, then send it with a unique ID:

```sh
merl mail send --home /tmp/team-mail --from alice --to bob \
  --id handoff-1 --kind assign --body-file /tmp/handoff.txt --json
merl mail list --home /tmp/team-mail --agent bob --pending --limit 100 --json
merl mail read --home /tmp/team-mail --agent bob --id handoff-1 --json
merl mail receipt --home /tmp/team-mail --agent bob --id handoff-1 --json
```

`list` returns headers without body text or a read side effect. `--pending` selects unreceipted messages even after an interrupted read; `--unread` selects only unread messages; `--outstanding` selects unfinished coordination requests even after receipt. Use only one folder filter. Follow `next_after` with `--after` until null, even if another wake arrives. `show` inspects one body without marking it read; `read` records that explicit action. Read/receipt timestamps are idempotent. Receipt stops retries for that message and creates no acknowledgment mail. Persist work/checkpoint state before treating receipt as a processing checkpoint. A duplicate notification is a reason to inspect the same IDs, not repeat completed application work.

After doing the work, an agent may send a linked reply and explicitly complete its coordination request:

```sh
merl mail send --home /tmp/team-mail --from bob --to alice \
  --id response-1 --kind reply --in-reply-to handoff-1 \
  --body-file /tmp/response.txt --json
merl mail complete --home /tmp/team-mail --agent bob \
  --id handoff-1 --reply response-1 --json
```

The reply must return to the original sender from the original recipient. It does not close anything automatically. Completion can target only an addressed request (`assign`, `ready`, `review`, `blocked`, `question`); `note` and `reply` are informational. A receipt never means agreement or task completion. Use stable IDs for interrupted send retries: identical bytes and metadata return `unchanged`; changed content under the same ID returns `MAIL_ID_CONFLICT`.

## Status and recovery

Every result has `schema: merl.mail/v1` and an action such as `mail.status`. Errors use `merl.error/v1`. `merl help mail` lists operations; `merl help mail send --json` loads one contract. `--format human` formats the same fields for inspection.

Status distinguishes `unread`, `unreceipted`, `outstanding`, the explicit binding generation and the last host notification. Delivery states include `pending`, `dispatching`, `offline`, `failed`, `stale_binding`, `pointer_limit` and `enqueued`. These are transport facts, never task outcomes. `MAIL_BUSY` means another dispatcher or binding action holds the scoped lock. The foreground watcher reports `waiting` on transient dispatch-lock contention and `resumed` when it can dispatch again, rather than exiting. Unknown agents must be registered; stale generations must be inspected rather than retried blindly.

The dispatcher visits at most 100 due recipients per pass and coalesces all unreceipted mail into one pointer per recipient. It reserves a 30-second crash-recovery lease and uses exponential failed-host retry delays capped at 60 seconds. A successful enqueue gets a 60-second application-receipt grace period before another wake; the foreground watcher checks every 500 milliseconds. One host enqueue is limited to ten seconds, then its own child process is stopped and reaped. Timeout is an ambiguous enqueue failure and can cause a duplicate wake on retry. Messages arriving during an existing unreceipted batch join its notification window instead of queuing another immediate wake. Once that batch is fully receipted, genuinely new mail becomes due immediately. Host finalization is conditional on its original dispatch claim: if a fully receipted batch is replaced during host I/O, the old attempt reports its host outcome but preserves the new batch as immediately pending. Application receipt stops retries; unreceipted messages remain durable across process restarts. Source bodies never appear in wake pointers.

This is local same-user cooperation, not a sandbox or authenticated multi-user service. Read-only Codex policy does not hide mail from unrestricted same-user processes. Mail is retained as immutable plaintext in `HOME/mail-v1/mail.sqlite`. No mail purge or storage quota exists yet. Old runtime files, external artifacts, existing mailboxes and installed skills are not migrated or removed. See [ADR 0004](adr/0004-messaging-product.md) for the replacement and data boundary.

The optional [Merl mail skill](../skills/merl-mail/SKILL.md) gives agents a short startup/recovery routine. Project roles and merge policy belong in [project collaboration guidance](agent-roles.md).

[Validation evidence](agent-mail-validation.md) separates public automated scenarios, native host bootstrap and the live three-role Merl roundtrip.
