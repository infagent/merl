# Compilation policy evidence for the first-release audit

This record covers [#74](https://github.com/infagent/merl/issues/74) under
[#52](https://github.com/infagent/merl/issues/52). The first-release obligations
come from [compilation contexts](../architecture.md#compilation-contexts),
[policy commands](../user-interface.md#node-and-project-setup), and the
[release acceptance criteria](first-release.md#acceptance-criteria).
None of these obligations moves to deferred work.

The implementation uses the authorized binding-policy path from #73. Domain
selection lives in `merl-core::compilation_policy`; the store retains accepted
snapshots and resolves live captures in the source transaction. GitHub capture
supplies the author's provider type. CLI scenarios run provider and compiler
substitutes with controlled input and count actual compiler invocations.

| Obligation | Public acceptance evidence in `tests/issue_capture_cli.rs` |
| --- | --- |
| One binding compiles selected human comments and keeps bot, routine-agent, and unknown traffic optional and cold | `compilation_selectors_use_trusted_authors_and_preserve_capture_policy` |
| Selectors use provider author metadata and authorized account mappings; prose, logins, and refresh flags cannot choose policy | `compilation_selectors_use_trusted_authors_and_preserve_capture_policy` |
| Kind rules leave descriptions alone; exact, kind, class, and override precedence is deterministic | `compilation_selectors_resolve_precedence_and_keep_mode_independent` |
| Eager/optional compiles and required/cold leaves a coverage gap | `compilation_selectors_resolve_precedence_and_keep_mode_independent` |
| Unknown authors fall back without invented classification | `compilation_selectors_use_trusted_authors_and_preserve_capture_policy` |
| Changes require administrative authority, remain binding-scoped, and preserve retry outcomes | `compilation_selector_administration_preserves_authority_and_binding_isolation` |
| Inspection names the winning rule and effective pair in human and versioned JSON output; ambiguous or unsupported requests fail | `compilation_policy_inspection_explains_selection_and_rejects_ambiguous_requests` |
| Restart and rebuild retain rules; edits use new policy while earlier captures and compiler work keep theirs | `compilation_selectors_use_trusted_authors_and_preserve_capture_policy` |
| Policy administration guards grants and competing binding changes and reaches revision/delta/inbox | Existing `prepared_binding_changes_guard_their_binding_and_authority` and `binding_policy_changes_require_authority_and_preserve_retry_results` |

Run this evidence with `cargo test --locked --test issue_capture_cli`. CI also
checks acceptance-scenario conventions, the workspace tests, formatting, Clippy,
and rustdoc. The release audit must link the reviewed implementation commit and
its passing CI run before freeze; this record does not claim held-out evaluation
evidence or close the rest of #52.
