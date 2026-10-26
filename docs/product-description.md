# Product description

Merl helps coding agents hand off work without a human copying messages between conversations. It retains the authored message and notifies the explicitly selected existing session. A new or restarted dispatcher can recover pending delivery from the mailbox.

## Who uses it

A human or coordinator names the participating agents and binds each to a known host session. A developer sends a review request; the reviewer records receipt, works on the change, returns a linked reply and explicitly completes the coordination request. The human still controls project policy and merge permission.

Roles are a project concern. The reusable transport does not impose a particular planner/developer/reviewer hierarchy, infer an identity from a working directory, or select a new agent for a stale registration.

## What the product records

Messages have caller-supplied unique IDs, explicit sender and recipient, a bounded UTF-8 body, a label and optional reply linkage. Publication is atomic and immutable. An exact send retry reuses its original message; changed content under the same ID conflicts.

Four independent facts describe handling: unread/read, application receipt, explicit coordination-request completion, and host notification state. A linked reply preserves the conversation. It completes no request automatically. Receipt creates no acknowledgment message and therefore no notification loop.

The recipient can page through unreceipted messages, unread messages, or unfinished requests. Wake pointers select unreceipted messages so an interrupted read does not strand its receipt. Already receipted requests remain outstanding until explicitly completed.

## Notification

The operator supplies an existing Codex session UUID and local Unix control socket. A generation guard rejects stale binding changes, and a socket fingerprint prevents a replaced host endpoint from silently inheriting a registration. Only the configured exact session is targeted.

An opt-in foreground dispatcher coalesces notification by recipient, persists a recovery lease, and invokes the supported `codex queue` command with a bounded pointer. Waiting itself makes no model calls. Successful host enqueue grants time for application receipt; failed calls retry with bounded backoff. New messages join a pending batch, while a fully receipted batch does not delay genuinely new work. An old host result cannot overwrite a new batch created during its I/O.

Host enqueue is not proof of reading or completion. Crashes and ambiguous results can duplicate notifications. Agents must inspect durable IDs and handling state before repeating work.

## Scope and trust

This release is one local mailbox, a Rust CLI and a verified Codex adapter. It provides no spawning, model-provider integration, generalized scheduler, remote authority, authenticated multi-user service or project-state engine. Another host requires a verified adapter before Merl claims it can wake that host.

The mailbox retains bounded plaintext. Private file permissions protect ordinary local access, but unrestricted same-user processes can read or impersonate cooperating clients. There is no encryption, purge, retention scheduler or quota. Operators choose suitable content and manage backups under their own policy.

The [architecture](architecture.md) defines persistence and dispatch invariants; the [UI contract](user-interface.md) defines the shipped interface. [ADR 0004](adr/0004-messaging-product.md) records removal of the former product and the requirement to leave existing runtime data untouched.
