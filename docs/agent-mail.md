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

Stop it with Ctrl-C or SIGTERM directed at that process. Restart the same command to recover durable work. Never use `killall` or process-name matching. `mail dispatch` with the same flags performs one bounded due-work pass and exits. No system service is installed. Ordinary host permissions must permit the chosen executable, control socket and Codex runtime storage. Failure leaves mail pending; there is no unrestricted fallback. Notification references use the running Merl executable's absolute path, so keep that binary available to recipients.

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

With a release containing #172, supported Codex notifications carry `merl.notification/v1` JSON with at most eight entries and 16 KiB of complete serialized UTF-8 content. Use complete inline bodies directly, without mandatory list/read fetches. Omitted bodies have direct `show` argv references; `more` continues pending headers after the last selected ID. Consume/fetch the actual content and save a checkpoint, then explicitly record selected handling facts:

```sh
merl mail handle --home /tmp/team-mail --agent bob \
  --ids handoff-1,handoff-2 --read --receipt --json
```

The suggested notice command includes only complete inline bodies. Never receipt an omitted body from its header. `handle` requires 1..100 unique comma-separated IDs and at least one independent action flag. Validation and all effects are atomic: foreign/missing IDs, duplicate/empty/excess selections and invalid completion links change nothing. Results give first timestamps, not bodies. Exact retries preserve them. `--complete` is always explicit; optional `--reply` links one completed ID to an existing matching response. Existing single-message commands remain usable. V0.5.0 notifications still contain only command pointers.

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

The dispatcher visits at most 100 due recipients per pass and coalesces pending mail into one bounded notice per recipient. Snapshot reads and the 30-second crash-recovery lease share a bounded immediate transaction, committed before host I/O. An empty selection returns `obsolete` without contacting the host or mutating its claim. Failed-host retry delays remain capped at 60 seconds; successful enqueue retains the 60-second receipt grace. The foreground watcher checks every 500 milliseconds. One host enqueue is limited to ten seconds, then its own child is stopped and reaped. Timeout is ambiguous and can cause a duplicate on retry. New messages join an existing unreceipted notification window; after full receipt, a genuinely new batch is due immediately. Conditional finalization prevents an old host outcome from postponing a new batch published during I/O.

An already accepted event may still be queued after receipt; supported `codex queue` has no cancellation/deduplication key. Historical `enqueued` alone does not establish pending work. Reconcile notice IDs against checkpoints rather than repeat work, mandatory inbox fetches or acknowledgment narration. A reply is for substantive results, not transport acknowledgment. Inline bodies intentionally reach the explicitly bound host's context/retention; [ADR 0012](adr/0012-bounded-inline-mail.md) supersedes the former body-free wake contract. At-least-once recovery remains; no exactly-once execution is promised.

This is local same-user cooperation, not a sandbox or authenticated multi-user service. Read-only Codex policy does not hide mail from unrestricted same-user processes. Mail is retained as immutable plaintext in `HOME/mail-v1/mail.sqlite`. No mail purge or storage quota exists yet. Old runtime files, external artifacts, existing mailboxes and installed skills are not migrated or removed. See [ADR 0004](adr/0004-messaging-product.md) for the replacement and data boundary.

The optional [Merl skill](../skills/merl/SKILL.md) gives agents a short startup/recovery routine. Project roles and merge policy belong in [project collaboration guidance](agent-roles.md).

[Validation evidence](agent-mail-validation.md) separates public automated scenarios, native host bootstrap and the live three-role Merl roundtrip.
