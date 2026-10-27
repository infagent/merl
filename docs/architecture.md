# Messaging architecture

Status: implemented contract

Merl has one product boundary: durable local agent messaging through the CLI. [ADR 0004](adr/0004-messaging-product.md) supersedes the former semantic-state architecture. The [UI contract](user-interface.md) defines observable behavior.

## Components

| Component | Responsibility |
| --- | --- |
| Root `merl` binary | stdout/stderr and exit-code routing |
| `merl-cli` | bounded argument parsing, incremental help, JSON envelopes and clock boundary |
| `merl-mail` | message/handling rules, SQLite transactions, explicit host binding and dispatch |
| `merl-lane` | read-only PR metadata, lane generations, fair checkpoints and frozen mail intent |
| `merl-setup` | known user settings, preflight and ownership of generated setup files |

Only `serde_json` and bundled `rusqlite` are workspace dependencies. Clocks, IDs and selected paths enter through explicit boundaries. A host executable is configured by an absolute path; synthetic executables replace it in tests. No runtime service is automatically installed.

## Reviewer-lane source

[ADR 0005](adr/0005-reviewer-lanes.md) adds one narrow GitHub source. `lane-v1/lanes.sqlite` stores named repository/label subscriptions, durable scheduling tickets, independent PR timeline/thread progress, thread-tail fingerprints, staged activity and immutable mail batches. It does not change the existing mail schema. Subscription identity is a Merl name, separate from its current host session.

Source progress and staged intent commit together. A batch then freezes its ID, recipient and exact body before mail I/O. Mail publication uses the existing idempotent send API; a crash after publication retries the same bytes. Final acknowledgment affects only that batch. Configuration changes/pause can commit during I/O; generation checks fence stale content responses. Authenticated server retry deadlines survive that fence because they apply to the shared credential, not the old selector.

One scan lock serializes source/publication passes. Durable tickets rotate discovery and per-PR pages under a bounded request budget. Incomplete/error responses retain progress and never infer absence. Initialization, scope exits and cursor resynchronization have visible state/gap records. [Reviewer lanes](reviewer-lanes.md) defines exact source coverage, backfill and limits.

The configured authenticated gh runs fixed metadata-only GraphQL queries against github.com with response/time limits. No discussion bodies or authors are requested, and no GitHub mutation is available. Both gh and Codex subprocesses use canonical runtime storage as their working directory; a removed development checkout cannot break configuration loading. Credentials stay with the configured tools.

`lane watch` combines scans with existing mail dispatch under the same watch lock as `mail watch`. It adds no second wake path, service framework or idle model call. Published mail remains governed by ordinary receipt/completion even after a lane pauses.

## Storage and publication

The caller selects an absolute dedicated mailbox home, either through `--home` or an opt-in `merl init` user default. An explicit home bypasses settings entirely. Without it, only the selected `--config` or known platform user settings file is read; malformed settings fail before mailbox access. There is no project-config discovery. Help, version and unknown commands open neither settings nor runtime. [ADR 0006](adr/0006-user-setup.md) defines this extension to ADR 0004.

Merl opens `mail-v1/mail.sqlite` beneath that home and creates private directories and database/lock files on Unix. It never discovers or opens an old project store. The `mail-v1` namespace and existing mail schema remain compatible with the running messaging sessions.

Setup itself only creates the selected mailbox directory; it never opens a mail/lane store, registers identities or changes bindings. Generated skill/settings publication uses a separate settings-scoped lock and bounded ownership receipt, preserving borrowed files and edits. Preview/show and top-level `merl status` create nothing. Status performs bounded settings/receipt/selected-skill reads and directory-type observations, without opening runtime stores, taking locks or probing processes/sockets. It separates saved dispatcher guidance from runtime health, which remains unobserved. Human init/status share a summary renderer; explicit init JSON remains compatible. Uninstall removes unchanged owned setup artifacts while preserving mailbox directories and contents. It leaves the stable zero-byte setup lock in place, starts/stops no process and removes no binary. [Installation](installation.md) records paths and recovery commands.

SQLite stores four kinds of record:

- `identities`: explicit name, binding generation, session UUID and socket fingerprint;
- `messages`: immutable authored content, sender/recipient, label, reply and commit order;
- `handling`: read, receipt and completion facts independently of message content;
- `notifications`: durable due work and the last host attempt for a recipient.

A short write transaction validates identities and reply endpoints, then commits message, handling record and notification intent together. Unique IDs and immutable-content checks make interrupted retries safe. Concurrent publishers cannot expose a partial message. SQLite serializes these transactions; network/process I/O runs outside them.

Mail labels identify coordination requests or information. They do not confer project authority. Only the addressed recipient can read or handle a delivery through the API. Local identity flags are trusted claims; the API does not authenticate a hostile same-user process.

## Usage and canonical startup

[ADR 0008](adr/0008-usage-and-startup.md) adds pure usage documentation and a short canonical skill. Both route to existing CLI contracts; no startup action infers roles, scope or sessions. Canonical publication reuses the settings lock and file journal, with receipt v2 adding a bounded retained-legacy association. Intent commits before canonical files and config last. Unmanaged legacy content is not journaled; unchanged owned/borrowed legacy entries keep their ownership and bytes. Config v1 remains readable by old mail tools, while old initializers reject v2. Incomplete v1 migration fails before writes; safe v1 uninstall keeps its version. Shared status rendering distinguishes both skill paths without runtime probes.

## Shared teams

[ADR 0007](adr/0007-persistent-teams.md) adds shared scope and roster tables to mail-v1 without changing its schema version or existing mail records. A strict fresh identity insert and membership commit in one immediate transaction. An existing raw key conflicts; idempotent mail registration is never ownership proof. Matching committed membership recognizes a retry. Team schema metadata is independently versioned; unsupported versions fail closed.

Qualified mail resolves each explicit scope/member endpoint independently. An optional --team supplies the scope for unqualified actors; other unqualified actors keep legacy semantics. The opaque mail key remains stable across session replacement; local names from other teams or legacy mail are never adopted. IDs remain global per mailbox home. Scope/member guidance is bounded and references are inert. Optional coordinator/role labels are descriptive. An optional immutable reference to an existing project scope enables bounded header discovery and exact filtering without recursive traversal or authority inheritance. Existing discovery opens read-only and absent discovery creates nothing. Role/coordinator labels confer no permissions; host bindings and completion stay separate. See [teams](teams.md) for the first workflow and deferred lifecycle.

[ADR 0010](adr/0010-qualified-subscription-recipients.md) resolves qualified
recipients for new named PR subscriptions before lane creation. The CLI passes
the existing stable mail key to unchanged lane configuration/outbox boundaries;
no schema or recipient migration is involved. Full creation adds bounded
resolved-actor metadata. JSON status keeps raw keys and no optional mail access;
human status can label at most 16 recipients through best-effort read-only
header lookup, stopping on metadata errors without failing status. Resume and
dispatch use saved keys independently of display or team metadata. Existing
review-labelled requests still carry no role or merge authority.

## Handling and pagination

[ADR 0009](adr/0009-atomic-team-announcements.md) adds bounded informational
announcements in mail-v1. A filtered snapshot of 1..100 recipients, strictly fresh
ordinary notes, handling rows and coalesced notification intents commit in one
immediate transaction. Body-file reads and candidate acquisition occur outside
it. Any pre-commit failure rolls back the entire publication. Accepted-ID retries
compare immutable inputs before roster expansion and retain original child IDs;
concurrent exact callers observe one accepted snapshot. Read-only lookup exposes
accepted metadata and mappings without body or group handling state. Existing
message retry, receipt, completion and dispatch semantics remain unchanged.

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
