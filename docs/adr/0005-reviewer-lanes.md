# ADR 0005: Durable read-only reviewer lanes

Status: accepted

Date: 2026-10-26

## Decision

[#149](https://github.com/infagent/merl/issues/149) adds one opt-in read-only GitHub activity source to the messaging product. A named lane selects one explicit repository and required labels for one durable Merl recipient. It follows many PRs across session replacement. GitHub account names confer no lane identity; shared-account activity stays visible.

Keep configuration, fair source scheduling, per-PR checkpoints, thread fingerprints and delivery intent in a separate `lane-v1` namespace. Advance source progress with staged intent in one transaction. Freeze mail ID/body before publication; an ambiguous send retries the exact immutable batch. Configuration generations and pause state fence content commits after external I/O, while server retry deadlines remain shared resource facts.

Use fixed metadata-only GraphQL operations through explicitly configured authenticated gh against github.com. Validate partial/error/rate responses before committing progress. Selected timeline events plus recurring inline-thread tails provide the activity surface; [reviewer lanes](../reviewer-lanes.md) records initialization, scope exits, resynchronization, polling and edit exclusions. These limits are not archival guarantees.

`lane watch` combines due source work with the existing dispatcher and shares its watch lock. It creates no second host wake path, idle model polling, fallback session, service installation or GitHub write capability. The lane does not bind a session; `mail bind` remains the explicit authority for host delivery.

The failed live dispatcher inherited a deleted development directory, causing Codex configuration loading to fail before connection. Both Codex and gh subprocesses now use canonical runtime storage as their working directory. Removing a checkout must not affect durable operation; deleting runtime storage remains outside recovery guarantees.

## Consequences and validation

The public interface includes `merl mail` and `merl lane`. [ADR 0004](0004-messaging-product.md) still governs removal of the former semantic product and preservation of external runtime data. This adds a narrow source; it does not restore semantic state, general watchers, spawning, model-provider integration or merge authority.

Public synthetic scenarios cover discovery beyond the first PR, independent pages, shared-account comments, scope/pause/configuration races, provider deadlines, frozen mail retry, cursor gaps and stable subprocess working directories. Existing recipient/binding/receipt/completion regressions remain in force. Read-only live acceptance must wake the actual idle reviewer through the candidate lane watcher, with binary provenance and no native nudge. Final-head checks and independent review precede handoff; no merge or release is implicit.
