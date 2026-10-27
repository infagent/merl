# ADR 0011: Reuse setup and retire owned legacy aliases

Status: accepted scope for #169

Date: 2026-10-27

## Decision

Reuse saved hosts, custom roots, mailbox and dispatcher-instructions preference
when rerunning setup. Determine whether work is needed from a validated plan,
unfinished receipt and mailbox-directory observation. A binary version change or
`configured` status alone is insufficient: selected bytes may be current across
releases, while an intact outdated skill still needs an update.

A current setup returns `unchanged` without questions, locks or file/receipt
writes. Borrowed or unmanaged retained aliases differing from the current skill
are deliberate keeps, not an upgrade trigger. Recorded local edits remain an
actionable preflight conflict, preserving every file without any apply question.
Actual changes or recovery show concrete actions and one apply confirmation,
reusing saved choices. Dispatcher questions belong only to fresh setup unless an
explicit option changes the saved preference. No runtime health is inferred.

The default terminal installer delegates to `init --offer`: optional first setup
is offered only when no saved/recorded choices exist. Ordinary explicit `init`
still enters first setup directly. A pure `init --offer --help` capability probe
lets an installer targeting an older binary retain its old terminal offer. An
assessment failure never falls back to another questionnaire. `--offer` is not
unattended consent and cannot combine with show, dry-run or uninstall. Explicit
`--setup` forwarding, `--no-setup` and unattended binary-only defaults remain.

## Owned alias retirement

This supersedes ADR 0008's blanket retention of unchanged owned legacy aliases.
After the next concrete preview and consent, retire at most one recorded
`ROOT/merl-mail/SKILL.md` per host once canonical `ROOT/merl/SKILL.md` and settings
are published and verified. Preserve borrowed/unmanaged aliases and their honest
associations. Edited recorded files, unsafe paths and foreign content still
conflict before all writes; there is no detach, adoption or force overwrite.

Reuse receipt v2's owned pending-remove primitive. Intent precedes unlink and
parent sync; keep its ownership proof through load and every subsequent save.
Keep `complete=false` through empty-directory cleanup. Acknowledge file removal,
legacy association and any removed directory record together. A crash after
unlink or directory removal but before acknowledgment resumes without recreating
the alias or relabelling it unmanaged. Only an empty inode-matching recorded
legacy directory may be removed; host roots, nonempty/replaced directories and
unrelated files remain. Canonical files and saved settings precede every alias
deletion, so interruption cannot leave the old default pointing to a deleted
primary skill. Incomplete v1 migration still fails closed.

Configured summaries point to `merl usage` rather than assuming binding is
missing. Dispatcher health remains `not_checked`; setup opens no mailbox store,
process or socket and starts/stops no host. Archive compatibility bytes remain;
the consented retirement applies only to verified user-owned installed aliases.

## Validation and release boundary

Synthetic installer/terminal scenarios protect current no-op identity/bytes,
version-only upgrades, saved-choice migration/cleanup with one confirmation,
cancellation, retained ownership and optional first-setup decline. Public plan
scenarios interrupt unlink and directory removal before acknowledgment and cover
borrowed/unmanaged/edit/symlink/nonempty/replaced paths. Existing recovery,
uninstall, setup summary and unattended consent checks remain required. No real
user settings or runtime are opened by validation.

Removing the formerly retained owned alias changes that contract; mark the
conventional commit as breaking under the pre-1.0 policy rather than forcing a
patch version. The planner coordinates the separately authorized merge/release
after exact-head review and green checks. Global installation and runtime
transition remain outside this work.
