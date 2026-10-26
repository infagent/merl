# Project collaboration roles

The coordinator selects one issue and identifies its developer and independent reviewer. The developer owns implementation, public-boundary tests, governing-document changes and an isolated branch/worktree. The reviewer checks acceptance and the exact final PR head, then identifies confirmed defects or reports approval with evidence. The coordinator adjudicates scope conflicts.

Use one writable checkout per active assignment and preserve other agents' edits. Keep transport instructions independent of these project choices. Messages can point to an issue, PR or test artifact; they do not replace that record. A receipt means the message was received. A completion records one coordination request and grants no permission to merge.

Merge authority comes from the human's instructions for the task. For a review-only assignment, open a reviewed PR and notify the human; leave merging to the human. Do not include private project records, local runtime identifiers or captured conversations in public examples.

## Optional project starters

These independently authored templates are optional project Markdown, usable from different coding hosts. Adapt them to the project; roles can be combined. A host-specific entry file can point to the shared rules and its assigned role. Merl neither installs these files nor synchronizes native agent memory. Keep them outside the reusable [transport skill](../skills/merl-mail/SKILL.md).

```md
# Shared project rules
Read the governing contract, current assignment and relevant checkpoint first.
Use docs/roadmap.md as the only roadmap. The planner owns scope and sequencing;
each selected issue names its implementation owner and independent reviewer.
Keep issue/PR evidence authoritative; do not create separate role roadmaps.
Preserve others' work. Record blockers and exact reviewed revisions.
Follow the human's review/merge authority; receipt grants no permission.

## Planner
Select one bounded issue, acceptance criteria, owner and reviewer.
Resolve scope conflicts and update the roadmap after verified outcomes.

## Developer
Implement the selected issue in an isolated checkout with public behavior checks.
Hand off the exact PR head, checks, limitations and next action.

## Reviewer
Check the assigned acceptance criteria independently at the exact PR head.
Return reproducible findings or approval with evidence; recheck a changed head.

## Researcher
Answer the assigned question with dated sources, scope and uncertainty.
Separate observations from recommendations; ask the owner to accept scope changes.
```

For Merl itself, the [messaging roadmap](roadmap.md) is authoritative. A role brief links to it and the selected issue rather than duplicating its commitments.

## Restart checkpoint

Keep task state in a coordinator-specified durable location. Use private storage when it contains private paths or project details. Before receipt or a handoff, save enough to resume without replaying completed work. This blank template makes no review claim:

```md
Assignment: issue/PR link, acceptance criteria, assigned role
Owner / reviewer: explicitly assigned names
Branch / checkout: current location
State: in progress | blocked | complete, with handled Merl IDs
Reviewed SHA: none (or full SHA, reviewer and verdict link)
Evidence: governing docs, PR, checks and artifact links
Next action: one concrete step
Blocker: none (or what is needed and who can resolve it)
```

On restart, read the shared rules, assigned role and this checkpoint; drain pending mail, inspect outstanding requests, then fetch authoritative issue/PR state. A changed head requires review reconciliation. If a checkpoint is missing, recover from those records and name the uncertainty rather than inventing completion. Persist the recovered checkpoint before receipt. See [onboarding](onboarding.md) for dispatcher/session recovery.

## Shared lessons across hosts

Project Markdown can carry lessons independently of any host's native memory. Keep lessons separate from work checkpoints and the roadmap. Start a candidate with this template; all example content should be synthetic or cleared for that repository:

```md
Lesson ID / topic:
Status: tentative | accepted | rejected
Scope: project, role, host/version or workflow where evidence applies
Observation and why it matters:
Evidence / reproduction links:
Owner review: pending (or named owner, date, decision and rationale)
Promotion: proposed target in shared rules or role guidance; reviewed change link
```

The relevant owner reviews scope and evidence before accepting a lesson. Promote an accepted rule through a reviewed edit to the governing/shared or role document, then link the lesson to that edit. A lesson from one host does not establish behavior on another. Rejected or superseded guidance retains its reason rather than silently becoming a rule.

At startup, read shared rules, the assigned role and current checkpoint. Use a small topic/status index to select relevant accepted lessons when needed; do not load the full history each turn. Tentative notes inform a scoped investigation, not standing instructions. No background learning service or automatic native-memory copy is required.

## Reviewer startup and activity

A configured reviewer lane persists beyond one PR and one session. At startup, recover pending and outstanding mail, then inspect `lane status` and every page of `lane queue` for the explicitly supplied lane. A fresh session binds the same Merl identity under its current binding generation; it does not create or reset the subscription. See [reviewer lanes](reviewer-lanes.md) for selectors and source limits.

An activity request carries a PR URL, observed head/state and event kinds. Fetch authoritative current state and the relevant prior review checkpoint. Record what was handled, including head and any no-change decision, before explicitly completing that Merl request. Receipt alone is insufficient. Later activity can create another request for the same PR. GitHub authors may be the same human account for every role; do not suppress that account's activity as self.

The daemon performs discovery and notification only. It posts no acknowledgment, review, ready or merge action. Reviewers also avoid GitHub acknowledgment comments for transport receipt; those comments create more source activity. Review findings and publication follow the project's assigned scope and human authority.
