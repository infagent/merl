# ADR 0002: Serialize accepted state through a project authority

Status: accepted

Date: 2026-09-19

## Context

Several agents can work on one project while GitHub polling, model calls, and host operations are in flight. Their results arrive in no useful commit order. Merl still needs one accepted project revision sequence and a place to check whether a result relied on state that has since changed.

SQLite can commit a batch atomically. It cannot decide who may accept the batch or whether an old policy result remains valid. An async runtime can wait for external work without blocking a thread, but it cannot make that result authoritative.

## Decision

Each project has one logical Project Authority. It serializes accepted mutations, checks recorded read dependencies and write conflicts at commit, and owns the authoritative store. Concurrent clients and workers submit bounded requests to it. Queue saturation has a visible outcome; workers cannot grow an unbounded in-memory backlog.

For a local project, a daemon owns the authority when running. One synchronous store worker owns its `rusqlite::Connection`. The CLI may act as an embedded authority only after acquiring the same exclusive project-ownership lock. SQLite's writer lock protects database consistency; the ownership lock prevents two processes from acting as Merl authorities.

External work runs outside the store worker's accepted-state transaction and does not borrow its connection while waiting. The authority commits any work that must survive restart before dispatch. A compiler run, for example, starts with an immutable intent and captured context. A worker executes the compiler, then submits a result. The authority records that result separately and evaluates its assertions against current accepted state. A later accepted batch commits its domain events, projections, revision, and inbox entries together. Wake-up follows the commit.

Pending work has a stable identity. On restart, the authority finds unfinished intents and retries or reconciles them. Duplicate results cannot create duplicate accepted transitions. Runtime tasks carry out work; durable records say what work exists.

The domain and store crates remain synchronous and runtime-independent. The daemon and external adapters may use Tokio for orchestration. This ADR does not require a daemon in the first compiler slice or fix the shape of its request enum.

## Consequences

Slow model or provider calls do not hold a project write transaction or stall the store worker. A result may become stale while external work runs; final dependency checks must still reject or reevaluate it. The mailbox orders submissions, while policy and SQLite determine correctness.

The daemon will need bounded queues, recovery dispatch, and an ownership lock. Those are runtime responsibilities. Read-only connections may be added later for throughput without creating another accepted-state writer.

## Alternatives

Multiple CLI writers using SQLite's lock would serialize database writes but leave Merl's authority rules spread across processes. Sharing `Arc<Mutex<Store>>` across arbitrary async tasks would make it too easy to hold the store while waiting on external I/O. An async SQLite API would not change the acceptance rule. A service database may suit a shared deployment later; it does not remove the per-project authority boundary.
