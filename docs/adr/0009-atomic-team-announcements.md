# ADR 0009: Atomic informational team announcements

Status: accepted scope for #165

Date: 2026-10-27

## Decision

Publish informational notices to one explicit scope as ordinary individually
addressed notes. Require an all-members selector or an exact free-form role OR
selector, capped at 16 supplied occurrences before deduplication. Exclude the
author by default; explicit self inclusion retains only an already selected
member. An author may belong to a different scope. Project references never
expand recipient selection or confer authority.

Filter role and self predicates in SQLite before fetching at most 101 roster
headers. Reject empty selections or more than 100 recipients without accepting
an announcement. The cap bounds note bodies at 819,200 bytes plus bounded
snapshot metadata and one original body; it is not a latency measurement.

All records already share mail-v1, so one immediate transaction freezes the
author identity, exact body, canonical selector, recipient/mail keys and child
IDs, and publishes all notes, handling records and existing coalesced notification
intents. Add independently versioned announcement tables without changing the
mail schema version. Body-file reads and bounded candidate ID acquisition happen
before the transaction. Candidates reserve nothing. Every consumed child ID must
be fresh, even when a foreign note has identical content. Reuse the private mail
publisher without changing ordinary send retry or notification rules.

An accepted ID is global among announcements in one mailbox, independently of
the existing message ID namespace. Exact retries return the committed snapshot
before expanding the roster or acquiring candidates; changed immutable inputs
conflict. A raced caller rechecks inside the transaction. Pre-commit failure
rolls back all records and notification changes. Lost responses and uncertain
commit failures require lookup/retry of the same ID; errors never promise zero
publication when commit acceptance is unknown.

## Boundaries and validation

Read-only accepted-snapshot lookup returns bounded metadata and recipient/message
IDs without body, source-file access, handling aggregates, mutation or host probes.
It creates no state. Each recipient uses existing mail read/receipt semantics.
Publication does not imply host enqueue, reading, receipt or completed work, and
receipting a note completes no existing request.

Public synthetic scenarios cover selectors and independent authors, frozen and
conflicting retries, a later-child collision with whole-transaction rollback,
lost acknowledgment, concurrent identical calls, maximum-size publication,
empty/overflow rejection and read-only inspection. Existing mail, notification,
binding and lane regressions protect the shared publication helper.

No partial publication journal, prepare/resume API, retry pump, group assignment
or completion, team lifecycle edits, lane retargeting, service, skill expansion,
release or runtime transition is part of this decision.
