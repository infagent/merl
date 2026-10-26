# Messaging architecture

Status: implemented contract

Merl has one product boundary: durable local agent messaging through the CLI. [ADR 0004](adr/0004-messaging-product.md) supersedes the former semantic-state architecture. The [UI contract](user-interface.md) defines observable behavior.

## Components

| Component | Responsibility |
| --- | --- |
| Root `merl` binary | stdout/stderr and exit-code routing |
| `merl-cli` | bounded argument parsing, incremental help, JSON envelopes and clock boundary |
| `merl-mail` | message/handling rules, SQLite transactions, explicit host binding and dispatch |

Only `serde_json` and bundled `rusqlite` are workspace dependencies. Clocks, IDs and selected paths enter through explicit boundaries. A host executable is configured by an absolute path; synthetic executables replace it in tests. No runtime service is automatically installed.

## Storage and publication

The caller selects an absolute dedicated mailbox home. Merl opens `mail-v1/mail.sqlite` beneath it and creates private directories and database/lock files on Unix. It never discovers or opens an old project store. The `mail-v1` namespace and existing mail schema remain compatible with the running messaging sessions.

SQLite stores four kinds of record:

- `identities`: explicit name, binding generation, session UUID and socket fingerprint;
- `messages`: immutable authored content, sender/recipient, label, reply and commit order;
- `handling`: read, receipt and completion facts independently of message content;
- `notifications`: durable due work and the last host attempt for a recipient.

A short write transaction validates identities and reply endpoints, then commits message, handling record and notification intent together. Unique IDs and immutable-content checks make interrupted retries safe. Concurrent publishers cannot expose a partial message. SQLite serializes these transactions; network/process I/O runs outside them.

Mail labels identify coordination requests or information. They do not confer project authority. Only the addressed recipient can read or handle a delivery through the API. Local identity flags are trusted claims; the API does not authenticate a hostile same-user process.

## Handling and pagination

Reading sets an idempotent timestamp. Receipt records application handling and stops notification retries for that ID, without creating reply mail. Completion is an explicit operation on an addressed request; a supplied completion reply must match that delivery. Reply linkage alone completes nothing.

Lists return at most 100 headers in commit order with a continuation cursor. Bodies are loaded individually and limited to 8192 UTF-8 bytes. `--pending` includes read-but-unreceipted mail; `--outstanding` includes receipted unfinished requests. A wake instructs the agent to drain all pending pages and persist its own work checkpoints before relying on receipt.

## Binding and dispatch

A name must be registered before it is bound. Binding uses an exact UUID, an explicit existing local Unix socket, and the expected generation. It never selects a session by a human-readable name. Socket identity changes require explicit rebinding, even at the same path.

A scoped dispatch file lock serializes host calls and binding changes. A separate watch lock prevents two foreground watchers for one mailbox. Neither lock stops unrelated processes or mailboxes. Send/receipt remain available during host I/O.

A pass visits at most 100 due recipients, commits a 30-second recovery lease before I/O, and uses `codex queue --remote unix://PATH --thread UUID --message POINTER`. The pointer contains a bounded read command and no message body or mailbox history. There is no `resume`, fork or fresh-session fallback.

One host call has a ten-second deadline. Failed calls use exponential retry delays capped at 60 seconds. Successful enqueue allows a 60-second receipt grace period. New mail joins an existing unreceipted batch's notification window; after full receipt, a new batch becomes due immediately.

Finalization compares the original dispatching state, lease deadline, attempt count and binding generation. If a new batch replaced the claim while host I/O ran, the old host outcome is reported separately and the new pending due time is preserved. The dispatch lock prevents another claimant from recreating the old claim.

The foreground watcher checks every 500 milliseconds. Transient dispatch-lock contention produces `waiting` followed by `resumed`; it does not terminate the watcher. Ctrl-C or SIGTERM stops the owned foreground process and releases its locks. Startup recovers durable due work. No global process cleanup or system-service installation occurs.

## Guarantees and limits

Host success means enqueued, not read or complete. A crash after host acceptance, an ambiguous timeout, or a late receipt may cause duplicate wakes. Delivery is at least once; application work must use durable IDs/checkpoints. Exactly-once host or application execution is not claimed.

Mail remains immutable plaintext through this API, with no erasure, encryption or storage quota. Same-user filesystem access is trusted; an operator requiring isolation must enforce it outside Merl. Host diagnostic output is discarded rather than persisted with possible secrets. Status keeps bounded failure labels and handling counts.

The CLI has no retired-product adapter. Unknown commands reject before storage access. Historical code and ADRs are available through Git; external runtime files are neither migrated nor deleted by this replacement.
