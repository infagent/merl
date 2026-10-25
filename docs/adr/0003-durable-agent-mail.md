# ADR 0003: Ship durable local agent mail before wider team operations

Status: accepted for implementation

Date: 2026-10-25

## Context

The maintainer authorized a product-direction change in [#145](https://github.com/infagent/merl/issues/145): refine the agent-mail workflow into Merl, then prove real coordination among existing agent sessions. The first-release plan deferred direct messaging and host wake until after the Issue benchmark. This slice now proceeds independently of those evaluation gates. Existing semantic state, historical implementation, and evaluation artifacts remain intact; their evidence is not a messaging acceptance gate.

## Decision

Add `merl mail` with a standalone local `mail-v1/mail.sqlite` beneath an explicitly selected mailbox home. This database is coordination transport, not the Project Authority or an accepted knowledge store. It never compiles prose or changes semantic project objects. Labels such as `assign` describe local coordination requests, not accepted semantic tasks. One transaction publishes immutable message content, handling metadata and notification intent. SQLite serializes these short transactions; a separate file lock serializes binding changes and host dispatch. No host process runs inside a SQLite transaction. This scoped transport does not implement the broader authority daemon in ADR 0002.

A name is registered explicitly. An operator binds it to an exact session UUID and an explicit local Unix socket under an expected generation. Socket identity changes require explicit rebinding, even if the path is unchanged. Binding changes cannot race dispatch. No name lookup, guessed replacement session, `resume`, fork, fresh exec fallback or provider adapter is used.

The installed Codex CLI provides `codex queue --remote unix://PATH --thread UUID --message TEXT`. Shell-originated probes woke two existing idle sessions without a native-message nudge. This verifies the installed host seam; a real three-role Merl test remains the integration gate. The queue adapter passes a bounded command pointer, not message content or mailbox history. Host command success records **enqueued**, not read, receipt or completed work. See the [official CLI reference](https://learn.chatgpt.com/docs/cli/reference).

Read state, application receipt, request completion and host notification are independent facts. A reply must reverse the original delivery endpoints. Reply linkage alone completes nothing. `mail complete` explicitly closes only the addressed coordination request and may link a matching reply. It does not resolve a semantic task or confer authority. `mail receipt` creates no response message or notification, preventing acknowledgment loops. Agents preserve their own work checkpoints and check handling state on duplicate wakes; Merl does not promise exactly-once application execution.

An opt-in foreground `mail watch` polls durable due notifications without model calls. It coalesces by recipient, uses a persisted retry lease before host I/O, and retries until application receipt. Crashing after host enqueue can duplicate a wake. A single scoped watch lock prevents competing watchers; stopping its process releases that lock. No service, global watcher, or host process is installed or stopped automatically.

## Retention and trust

Message bodies are bounded plaintext in this **separate** private local mailbox. They are immutable through the API; no retention purge, migration, encryption or semantic payload erasure is implemented in this slice. Project-store purge does not erase mail. Operators must choose suitable mail content and manage mailbox/backups under their own retention policy. Do not claim the semantic store's erasure guarantees for these records. Private directories and database/lock files are created with restrictive Unix modes. Existing unrelated directories are not chmodded.

Same-user identity flags are trusted audit claims, not cryptographic authentication. An unrestricted same-user process can read files or impersonate a sender. No new OS isolation claim is made. Host diagnostic output is discarded; status retains bounded stable failure labels rather than message bodies, secrets, or host transcripts. One dispatch visits at most 100 recipients; messages are at most 8192 bytes and list pages at most 100 headers. Local queues are durable but have no storage quota in this slice.

Legacy agent-mail scripts and records are design references only. Their standalone source license was not established, so no implementation code is copied. There is no destructive migration, automatic mailbox enrollment, installed-skill modification, or importer in this slice. Legacy `ack`/reply closure semantics do not become completion semantics here.

## Consequences

ADR 0001 remains in force: the CLI is the public interface and the new skill is optional guidance. Repository role, review, worktree and merge policy remains in project documentation, not reusable transport instructions. Generalized scheduling, agent spawning, role profiles, context reset, cross-project transport and semantic integration remain deferred. Other hosts need a separately verified adapter.
