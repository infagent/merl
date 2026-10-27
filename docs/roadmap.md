# Messaging roadmap

The current product is durable local agent messaging. [ADR 0004](adr/0004-messaging-product.md) replaces the former semantic-state/compiler/evaluation product and its active plans. Git history retains that work; its benchmark and corpus commitments are not gates for this product.

This is the single active project roadmap. The coordinator owns scope and sequencing; each selected issue names its implementation owner and independent reviewer. Issues and PRs hold execution evidence. Role briefs and checkpoints link here rather than maintaining separate roadmaps; see [project roles](agent-roles.md).

## Current release

The release provides explicitly named identities, immutable atomic messages, validated reply links, independent read/receipt/completion, bounded header paging, visible delivery status, and recoverable coalesced host notification. Its first verified adapter targets existing Codex sessions over a local Unix socket. A foreground dispatcher is explicitly started and stopped by the operator.

Readiness requires:

- public regressions for messaging, concurrent publication, incorrect recipient/reply handling and changed retries;
- recovery tests for offline delivery, interrupted reads, duplicate wakes, stale bindings, temporary contention and batch changes during host I/O;
- a messaging-only command/help surface and rejection of retired commands without changing old runtime data;
- coherent current contracts, dependency/CI cleanup, explicit historical ADR supersession and preservation of external files;
- a relevant three-agent smoke using the replacement application binary, with the dispatch binary and session boundary recorded accurately;
- formatting, Clippy, the complete reduced workspace tests, rustdoc and Python checks passing;
- independent review of the final head and a reviewed PR handed to the human before any merge.

Release distribution provides version-bound Linux/macOS binaries and an optional matching skill through the [installation contract](installation.md). Draft publication, verification and recovery follow [release distribution](release-distribution.md). The verified installer offers reusable user setup through `merl init`: matching local skills and an opt-in mailbox default, with pure preview, safe ownership/recovery and scoped uninstall. [ADR 0006](adr/0006-user-setup.md) extends explicit mailbox selection without project discovery. Human setup and read-only status share a concise summary; full versioned status JSON is available with `--verbose`/`-v`. Dispatcher instructions remain foreground-only; existing bindings, running processes and mailbox contents are untouched.

[Reviewer lanes](reviewer-lanes.md) add durable read-only repository/required-label subscriptions through [ADR 0005](adr/0005-reviewer-lanes.md). Readiness requires synthetic multi-PR/paging/generation/outbox/deadline regressions and an actual watcher-originated wake to an idle reviewer, with exact application/dispatch provenance. This is distinct from recovered manual-mail delivery. Scope/backfill/edit/resynchronization limitations remain explicit; no GitHub writes, actor spawning or merge authority are added. Durable service setup remains later work.

## Persistent team coordination

[#161](https://github.com/infagent/merl/issues/161) adds shared scope, durable member guidance, paged scope/roster discovery, optional project references and independently qualified individual mail. A project-wide member can coordinate several epic scopes without duplicate membership; roles and coordinator metadata stay optional. Atomic membership/fresh-identity creation preserves legacy mail compatibility and rejects foreign identity adoption. The CLI onboarding is usable without a skill. General edit/removal lifecycle is deferred.

Merged [#163](https://github.com/infagent/merl/issues/163) adds pure `merl usage` and the short `/merl` / `$merl` startup with its bounded canonical/retained-legacy setup transition. This uses the merged scope/mail foundation. [#165](https://github.com/infagent/merl/issues/165) adds atomic informational snapshot notices and read-only accepted-snapshot inspection. [#167](https://github.com/infagent/merl/issues/167) adds qualified recipients for new named PR subscriptions, with immutable stable keys and optional human name display; existing lane recipients remain immutable. These follow-ups remain product commitments, not automatic authorization for a release, global setup or runtime migration.

## Later decisions

Evidence from daily coordination should guide retention/erasure policy, storage quotas, authenticated multi-user operation, remote delivery and additional host adapters. Each requires a separate design and verified boundary. Agent spawning, scheduling, workspace management and model-provider integration are not part of the current release.

Project roles, worktree ownership, review and merge policy stay in project documentation rather than the reusable transport skill. No old runtime data is migrated merely because the product direction changed.
