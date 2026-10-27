# ADR 0010: Qualified recipients for new PR subscriptions

Status: accepted scope for #167

Date: 2026-10-27

## Decision

Extend the full creation form of `lane start` with an explicit `scope/member`
recipient and optional `--team` shorthand for a bare member name. Any existing
member is eligible, including a roleless member. Names, roles, coordinator labels,
GitHub authors and lane names confer no routing policy or authority. Keep the
existing `lane` vocabulary and required-label AND selector.

An explicit scope/member wins over a different or nonexistent syntactically valid
default team. Validate names and resolve only the selected actor. A bare actor
without a team remains the exact legacy mail identity. Resolve existing mailbox
state read-only before lane creation; a missing actor fails without fallback,
registration, adoption, binding or new runtime state. Full creation output adds
the same bounded `resolved["--agent"]` metadata as qualified individual mail.

Store the resolved stable mail key through the existing `Configuration.agent`
boundary. Lane-v1 schema, public Rust configuration and frozen outbox recipients
remain unchanged. Equivalent qualified/shorthand creation retries use that key;
a different valid recipient under the old lane name conflicts. A new recipient
needs a new named subscription. Never rewrite existing lanes or batches when a
same-named team member joins.

Resume uses only the saved lane name and expected generation, with no actor
resolution. Stop/update/status/queue/scan/watch gain no team default or retargeting
operation. Session rebinding remains an explicit mail operation and changes no
subscription recipient or source checkpoint.

## Display and handling

JSON status retains existing raw keys and fields and performs no optional team
lookup. Human status may add `qualified_agent` while retaining raw `agent`, using
one read-only mailbox open and at most 16 saved-key lookups. It reads no member
guidance. Absent mappings leave the key visible; any unavailable/unsupported
metadata abandons further optional lookups without failing status or creating
mail state. Display is not a dependency of resume or dispatch.

Existing source queries, checkpoints, fairness, generations, batch IDs/bodies,
sender, pacing and dispatch stay unchanged. Notifications retain the `review`
request kind even for recipients with another role or no role. Receipt, replies
and explicit request completion remain separate. This does not generalize the
source, notification kind, scheduler, host adapters or group work handling.

## Validation and boundaries

Public synthetic scenarios cover distinct same-named members and legacy mail,
explicit/default precedence, no fallback or state creation, equivalent retries,
rejected retargeting, optional human display and unchanged JSON, restart/rebinding,
and a previously frozen legacy batch surviving later joins. Existing lane/mail
integrity and launch-directory scenarios remain required. No live GitHub/model
call, watcher transition, release, global setup or runtime migration is authorized
by this implementation.
