# ADR 0004: Replace the former product with durable agent messaging

Status: accepted

Date: 2026-10-25

## Context

The maintainer approved a full product replacement on [#145](https://github.com/infagent/merl/issues/145), continuing [PR #146](https://github.com/infagent/merl/pull/146). The initial implementation added messaging beside the semantic-state product. That left two maintained directions. The new approval supersedes the instruction to preserve the old implementation in the working tree.

## Decision

Merl is now the local agent-messaging product. Keep the Rust CLI and mail store, meaningful messaging tests, build/release infrastructure and the acceptance convention utility. Remove the semantic-state, compiler, policy, ingestion, corpus and evaluation crates, their CLI modules/tests/fixtures/dependencies and obsolete active plans. Git history preserves the previous work; this branch does not maintain a legacy subsystem.

The active contracts are the [product description](../product-description.md), [architecture](../architecture.md), [UI contract](../user-interface.md) and [messaging roadmap](../roadmap.md). ADRs 0001 through 0003 are superseded historical records. The CLI remains the public agent interface; the optional skill is guidance, not a prerequisite. Project role, worktree, review and merge policy belongs in project documents.

Keep the existing `merl mail` command vocabulary, versioned envelopes and `mail-v1` store compatible with the running coordination workflow. The caller still chooses an absolute mailbox home. No retired command or database flag selects or migrates old runtime state.

## Runtime and data boundary

Source removal is limited to the reviewed tracked files on the feature branch. External evaluation workspaces, runtime mailboxes, installed skills, unrelated worktrees and user files are not removed. Old runtime data is never silently opened, migrated, overwritten or deleted. A future importer or erasure operation requires a separate explicit contract.

The old working binary and live foreground dispatcher remain available while a replacement is built in a separate target directory. Smoke evidence identifies which binary handles application commands and which owns notification dispatch. Any dispatcher transition is deliberate and scoped; no service is installed automatically.

Mail remains immutable bounded plaintext with independent read, receipt, completion and notification facts. Same-user identity claims are trusted, and host delivery is at least once. No model polling, semantic compilation, provider integration or fresh-session substitution is required.

## Validation and consequences

Public regressions must show a messaging-only root/help tree and rejection of retired commands without changing opaque old runtime bytes. Retain the messaging concurrency, retry, recipient/reply, binding and recovery scenarios. The complete reduced CI suite and an independently reviewed final head replace old-product checks for this PR. The old benchmark does not become messaging evidence, and historical approvals do not approve this replacement head.

The relevant three-agent smoke must use the replacement application binary and record dispatch provenance. No merge is authorized by the implementation or its receipt/completion messages; the human receives a reviewed PR.
