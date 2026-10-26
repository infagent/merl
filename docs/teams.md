# Teams and individual handoffs

Teams save a shared scope and roster inside one explicitly selected mailbox. Members keep their starting guidance across host-session replacement. Roles are optional labels such as `planning`, `implementation` or `research and docs`; Merl assigns no permissions or merge authority from them.

This interface requires a release containing #161. Released v0.3.0 does not include team commands. Use the candidate source binary before release. No skill is required.

## Create scope and join

Prepare small UTF-8 files containing the shared scope and each member's starting guidance. Choose a dedicated absolute mailbox home, or use an already approved `merl init` default. Explicit `--home` bypasses settings as usual.

```sh
merl team create --home /tmp/example-mail --name alpha --coordinator planner \
  --scope-file /tmp/example-scope.txt --guidance-ref /work/demo/coordination.md
merl team join --home /tmp/example-mail --team alpha --member planner \
  --role planning --description-file /tmp/example-planner.txt
merl team join --home /tmp/example-mail --team alpha --member dev-a \
  --role implementation --description-file /tmp/example-dev.txt
merl team join --home /tmp/example-mail --team alpha --member reviewer \
  --role review --description-file /tmp/example-reviewer.txt
merl team show --home /tmp/example-mail --team alpha
merl team members --home /tmp/example-mail --team alpha --limit 100
merl team member --home /tmp/example-mail --team alpha --member dev-a
```

The coordinator is an explicit local convention, not an authenticated administrator. Additional implementers, documenters and researchers join the same way. A member name does not imply a role. Joining saves the exact description and references and creates one fresh mail identity. It never binds or starts a host session.

Team and member names use 1..64 ASCII letters, digits, dot, underscore or hyphen. Scope and description files contain 1..8192 UTF-8 bytes. Optional role labels contain 1..128 bytes without control characters. Repeat `--guidance-ref` up to 16 times, each with 1..2048 bytes without control characters. Pointers are stored as supplied; Merl never opens, fetches or evaluates them. Keep detailed project lessons in those external documents.

Human discovery shows qualified names such as `alpha/dev-a`. JSON detail also exposes the opaque `mail_key` used by the transport. `beta/dev-a` and unscoped `dev-a` are separate identities; neither inherits the other's messages or binding. `team members` pages headers in member-name order; follow `next_after` with `--after`. Full guidance loads only with `team show` or `team member`. Discovery of absent state creates nothing.

## Hand work to one member

```sh
merl mail send --home /tmp/example-mail --team alpha --from planner --to dev-a \
  --id alpha-build-1 --kind assign --body-file /tmp/example-assignment.txt
merl mail list --home /tmp/example-mail --team alpha --agent dev-a --pending
merl mail read --home /tmp/example-mail --team alpha --agent dev-a --id alpha-build-1
# Persist a work checkpoint before relying on receipt.
merl mail receipt --home /tmp/example-mail --team alpha --agent dev-a --id alpha-build-1
merl mail send --home /tmp/example-mail --team alpha --from dev-a --to reviewer \
  --id alpha-review-1 --kind review --body-file /tmp/example-review-request.txt
merl mail send --home /tmp/example-mail --team alpha --from reviewer --to planner \
  --id alpha-outcome-1 --kind note --body-file /tmp/example-outcome.txt
# Each recipient completes its own request after finishing the work.
merl mail complete --home /tmp/example-mail --team alpha --agent reviewer --id alpha-review-1
merl mail complete --home /tmp/example-mail --team alpha --agent dev-a --id alpha-build-1
```

`--team` resolves every identity flag through that roster. Unknown teams or members fail without falling back to unscoped names. Message IDs remain globally unique within the mailbox home. The `alpha-` prefix is a caller convention, not a separate ID namespace. Exact mail retries retain existing immutable-content checks.

Receipt stops notification retries and does not complete work. A linked reply also completes nothing automatically. Review and build requests have different owners and require separate completion. Other members with the same role receive no implicit assignment. Follow every pending page before recording receipts. See [mail handling](agent-mail.md) for replies, filters and recovery.

## Restart and binding

Read your saved starting guidance and work state without resupplying it:

```sh
merl team member --home /tmp/example-mail --team alpha --member dev-a
merl mail status --home /tmp/example-mail --team alpha --agent dev-a
merl mail list --home /tmp/example-mail --team alpha --agent dev-a --pending
merl mail list --home /tmp/example-mail --team alpha --agent dev-a --outstanding
```

Membership survives process restart. An explicit `mail bind --team alpha --agent dev-a` still requires the existing session UUID, socket path and current **binding** generation. Team/member metadata generations do not authorize a rebind. A repeated join preserves the saved binding and work; a stale binding generation fails. The [mail guide](agent-mail.md) explains verification and foreground dispatch. Codex wake remains the only verified adapter; Claude can use the CLI without a claimed wake adapter.

Identical create/join inputs return `unchanged`. Changed guidance, roles or references return `TEAM_CONFLICT` and preserve saved bytes. General edits, leave and removal are deferred. Joining commits membership and a strictly fresh identity in one transaction. A crash before commit publishes neither; a retry after commit recognizes the complete pair. A raw-key collision returns `TEAM_IDENTITY_CONFLICT`, even if the existing identity is unused and unbound.

## JSON and boundaries

Append `--json` for noninteractive `merl.team/v1` records. Leaf contracts load through `merl help team <operation> --json`. Qualified mail keeps `merl.mail/v1` and adds `resolved` metadata keyed by `--agent`, `--from` or `--to`, containing `team`, `member`, `qualified` and `mail_key`. Unqualified results keep their existing shape. Errors use `merl.error/v1`, exit 2, and stable codes including `TEAM_INPUT`, `TEAM_NOT_FOUND`, `TEAM_MEMBER_MISSING`, `TEAM_CONFLICT`, `TEAM_IDENTITY_CONFLICT` and `TEAM_SCHEMA`.

Additive team tables share `mail-v1` so membership and identity can commit atomically. Mailbox schema version 1 and existing mail/lane behavior remain compatible. Team discovery opens existing state read-only. Top-level `merl status` remains a setup summary and does not inspect teams or runtime health. Same-user identity claims remain trusted, and mail remains plaintext.

Later serial work covers informational team notices, explicit new team-targeted PR lanes, then `merl usage` and the short `/merl` / `$merl` startup skill. Existing lane recipients are immutable and are not repointed. This first interface changes no installed skills, global setup, running dispatchers or existing bindings.
