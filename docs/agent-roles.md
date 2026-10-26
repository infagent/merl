# Project collaboration roles

The coordinator selects one issue and identifies its developer and independent reviewer. The developer owns implementation, public-boundary tests, governing-document changes and an isolated branch/worktree. The reviewer checks acceptance and the exact final PR head, then identifies confirmed defects or reports approval with evidence. The coordinator adjudicates scope conflicts.

Use one writable checkout per active assignment and preserve other agents' edits. Keep transport instructions independent of these project choices. Messages can point to an issue, PR or test artifact; they do not replace that record. A receipt means the message was received. A completion records one coordination request and grants no permission to merge.

Merge authority comes from the human's instructions for the task. For a review-only assignment, open a reviewed PR and notify the human; leave merging to the human. Do not include private project records, local runtime identifiers or captured conversations in public examples.

## Reviewer startup and activity

A configured reviewer lane persists beyond one PR and one session. At startup, recover pending and outstanding mail, then inspect `lane status` and every page of `lane queue` for the explicitly supplied lane. A fresh session binds the same Merl identity under its current binding generation; it does not create or reset the subscription. See [reviewer lanes](reviewer-lanes.md) for selectors and source limits.

An activity request carries a PR URL, observed head/state and event kinds. Fetch authoritative current state and the relevant prior review checkpoint. Record what was handled, including head and any no-change decision, before explicitly completing that Merl request. Receipt alone is insufficient. Later activity can create another request for the same PR. GitHub authors may be the same human account for every role; do not suppress that account's activity as self.

The daemon performs discovery and notification only. It posts no acknowledgment, review, ready or merge action. Reviewers also avoid GitHub acknowledgment comments for transport receipt; those comments create more source activity. Review findings and publication follow the project's assigned scope and human authority.
