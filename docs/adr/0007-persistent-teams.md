# ADR 0007: Persistent shared teams and qualified individual mail

Status: accepted scope for #161

Date: 2026-10-26

## Decision

Add a persistent team namespace beneath the selected mailbox home. A team records shared scope, inert guidance references and a coordinator convention. Members record stable qualified identity, optional free-form role and saved starting guidance. Existing agents can discover teammates and resume instructions without scheduling, spawning, authenticated permissions or a project-state engine.

Store additive team tables in the existing `mail-v1` SQLite database, retaining its schema version and existing mail behavior. Membership and a strictly fresh opaque mail identity commit in one immediate transaction. Idempotent registration cannot prove ownership after a crash: it could accept a foreign pre-existing key. A collision fails without adoption; matching committed membership proves an exact join retry. Pre-commit failure rolls back both records; post-commit retry finds both.

Qualify individual mail and binding commands with an explicit team. Resolve identity flags through that roster, without a default team, name inference or legacy adoption. Message IDs remain global within the mailbox home. Receipt, replies, completion, binding generations and at-least-once notification retain their meanings. Roles and coordinator labels confer no project or merge authority.

## Boundaries and validation

Scope/member detail is bounded; references are inert, and roster headers are paged. Discovery creates no runtime state. Membership does not bind a session or replace delivery on restart. Existing mail/lane stores, setup, installed skills and dispatchers remain compatible and untouched by development.

Public acceptance covers four members, individual handoffs and separate request completion, two teams with the same local member name plus legacy mail, fresh-process guidance, explicit rebinding and input/retry limits. Narrow transaction tests inject member-publication failure after identity insertion and prove rollback, committed retry and rejection of an unused/unbound foreign identity.

Informational snapshot fanout, explicit new team-targeted PR lanes, then `merl usage` and a short startup skill follow serially. General edits, leave/removal and skill upgrades need their own contracts. Existing lane recipients remain immutable. No release, global setup or runtime migration follows from this decision.
