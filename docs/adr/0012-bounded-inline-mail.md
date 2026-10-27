# ADR 0012: Bounded inline mail and explicit compact handling

Status: accepted implementation contract for #172

Date: 2026-10-27

## Decision

The supported existing-session Codex adapter sends `merl.notification/v1` JSON instead of a body-free command pointer. This deliberately supersedes that part of ADR 0004's dispatch contract. Complete short handoffs should be usable without a list/read round trip. The CLI and durable mailbox remain authoritative; no skill, MCP server, model polling or new host is required.

Each notice contains canonical mailbox `home`, stable recipient `agent`, `binding_generation`, and up to eight messages in commit order. Each entry preserves the immutable ID, sender/recipient, kind, reply link and creation time, plus snapshot handling timestamps. Its body is either complete and exact or absent with a `fetch` argv array for `mail show`. No body prefix is presented as complete. `more` is a pending-list argv array after the last selected ID, or null. The ordered IDs identify the selection; there is no separately durable notification-batch ID.

The whole serialized UTF-8 host message is at most 16 KiB, including JSON escaping, paths, instructions and fetch/handling references. This accommodates a full 8 KiB message plus ordinary metadata while bounding host context and argv size. One bounded lookahead detects additional mail. If necessary, omit the largest bodies before reducing the selected suffix; continuation never skips a removed entry. If even a minimal reference cannot fit, retain mail and report existing `pointer_limit`. References are argv data, not shell-interpolated commands.

Headers, bodies and lease creation share one bounded immediate transaction, committed before host I/O. Publication cannot replace the selected batch between snapshot reads and claiming it. Construction/enqueue marks no message read, receipted or complete. Retries rebuild a fresh bounded pending snapshot: message bodies/IDs remain immutable, while selection, handling timestamps and notice bytes can change. Explicit full bodies intentionally enter the bound host's context and retention; the previous body-free exposure guarantee no longer applies. Local same-user sender claims remain trusted labels, not authentication.

## Handling

`mail handle --agent NAME --ids ID,ID --read --receipt` records explicitly selected facts for content the caller has consumed and checkpointed. Each action flag is independent; at least one of `--read`, `--receipt`, `--complete` is required. `--reply` requires `--complete` and exactly one selected ID, and links an already published matching reply. Reply publication still uses `mail send`.

Selections contain 1..100 raw unique keys; empty segments, duplicate IDs and excess occurrences fail. The current Key syntax excludes commas. All IDs, recipient ownership, request kinds and completion links validate in one immediate transaction before any effect commits. One invalid entry rolls back the entire selection. Results contain bounded IDs/timestamps, without bodies or acknowledgment mail. Retries preserve first timestamps and existing completion-reply conflict rules, including absent versus present links. Existing single-message APIs/envelopes remain available and unchanged.

The notice's suggested `handle` argv selects only complete inline bodies; it is null when none fit. Omitted/unfetched bodies must not be acknowledged from headers alone. Agents reconcile immutable IDs with checkpoints, fetch actual overflow, checkpoint before receipt and explicitly complete requests after work. Routine acknowledgments create no reply mail or user-facing narration unless actionable.

## Recovery and obsolete notices

Keep the existing dispatch/binding lock, 30-second crash lease, ten-second host deadline, failed-call backoff and 60-second successful-enqueue grace. Do not substitute exponential success backoff based on the attempts counter: it includes failures, and a longer grace could delay genuinely new content while old IDs remain pending.

Before claiming/enqueuing each selected recipient, recheck its bounded pending snapshot. If empty, return an `obsolete` dispatch result without a host call or claim mutation. New mail published after that observation remains immediately eligible. Conditional finalization still prevents an old host result from postponing a newly published batch during host I/O; binding replacement stays serialized. Partial receipt leaves other pending IDs recoverable. Enqueue never implies receipt; crash/timeout/late receipt can cause duplicates.

The supported `codex queue` interface exposes no cancellation/deduplication key. An event accepted while a session is busy can therefore remain queued after another event consumes and receipts its IDs. Stopping new dispatches does not retract that event. Synthetic tests reproduce this distinction; the observed 14 empty dev drains do not prove the cause of that real episode. Notices carry IDs/checkpoint guidance so obsolete duplicates need no mandatory inbox fetch or acknowledgment narration. Delivery remains at least once, without an exactly-once application guarantee.

## Validation and exclusions

Public synthetic host/CLI scenarios cover full inline consumption, complete serialized byte/count bounds, Unicode/escaped bodies, direct overflow and continuation, explicit atomic handling and reply retry failures, obsolete selections, accepted stale events, and existing receipt/new-batch/generation/crash recovery. Compare an entire synthetic assignment/reply/completion exchange against 0.5, including help/checkpoint/output/notification bytes and command counts. Synthetic event counts are not measured tokens or savings; actual host/model usage remains unmeasured when no model is invoked.

No Claude experiment or unconditional Claude wake claim, cross-machine transport, memory subsystem, setup migration, live runtime replacement, release or merge is included.
