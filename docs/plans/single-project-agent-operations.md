# Single-project agent operations

Status: draft

This document owns the scope and acceptance criteria for Phase 4. It follows the [first release](first-release.md) and precedes [delegated agent management](delegated-agent-management.md).

## Goal

Phase 4 makes Merl useful to a team of agents working in one project. A human attaches the repository once and chooses when to start each process. Merl keeps the selected project surfaces current, then prepares each agent's stable identity, accepted assignment, isolated workspace, and bounded project view before launch.

The phase tests a larger version of the original token claim: several agents should coordinate through current state and compact deltas instead of sending each other full histories or rebuilding context after every restart.

One local project authority is enough. The project may refer to more than one source, though the acceptance scenario uses one Git repository so synchronization and workspace behavior stay easy to inspect.

## Boundary

Phase 4 includes the operations needed after a human starts an agent:

- logical agent identity, project role, and durable guidance;
- session generations, checkpoints, and bounded resume;
- accepted tasks, assignments, execution leases, and handoffs;
- repository attachment, selective Issue and pull request synchronization, and durable source policy;
- managed worktrees, branches, scratch space, and safe cleanup;
- assignment-aware views and inbox delivery;
- structured work requests and optional direct prose;
- GitHub pull request capture, review state, and sparse publication;
- local daemon delivery and attached-host wake or reset;
- task-scoped context closure when the host supports it;
- context and token accounting for the team workflow.

Phase 4 does not let a manager create, drain, stop, or replace another agent. It does not rank runtimes or choose models. Those operations require templates and delegated authority, which belong to Phase 5.

## Operating model

```mermaid
flowchart TB
    H[Human selects a logical agent] --> T[Assign an accepted Task]
    T --> W[Create or lease an isolated workspace]
    W --> L[Human starts the process in that workspace]
    L --> A[Attach the session]
    A --> R[Resume bounded task context]
    R --> X[Agent performs work]
    X --> D[Commit structured state and compact deltas]
    D --> I[Route relevant inbox entries]
    I --> X
    X --> C[Checkpoint and close session]
    C --> N[New session resumes current state]
```

The human chooses the model host. Merl prepares the workspace and bootstrap before the human starts it. The attached process receives one bootstrap line:

```text
Run `merl session resume --agent <id> --format json` before work; use `merl help <topic>` when needed.
```

Merl does not inject a command catalog or require a large agent skill. Durable guidance lives in the agent profile and appears in the resume projection when relevant.

## Agent identity and guidance

An `AgentProfile` names one logical agent across process exits, model changes, computers, and context resets. An `AgentSession` names one disposable context generation.

Project membership and role belong to durable policy. A runtime claim cannot grant either one. One active session owns the logical agent's actionable inbox lease. Other sessions may inspect state but cannot advance its cursor or execute its assignment.

Guidance has three forms:

- an instruction records behavior required by an authorized human or project policy;
- a practice records a working method adopted by the agent;
- a lesson records an observation proposed for later reuse.

Guidance keeps its scope, author, authority, provenance, priority, and lifecycle. Merl selects a bounded set during resume. It does not append an agent's memory transcript to every new context.

## Tasks, assignments, and handoffs

A Task remains knowledge-plane state. An Assignment connects one logical Agent to that Task for a period of responsibility. The Assignment can exist before a process starts and survives session replacement.

Phase 4 supports direct assignment by an authorized human, assignment and reassignment by a planner within human-established project policy, and self-claim where that policy allows it. The planner coordinates existing logical agents; a human still starts their processes. Assignment authority does not grant runtime selection or process control. Merl records assignment history rather than rewriting ownership in place. An `AgentSession` may hold the Assignment's execution lease. The lease prevents two sessions from acting as the same worker, but its expiry does not end the Assignment.

The task keeps its independent planning facets:

| Facet | Question |
| --- | --- |
| Commitment | Has the owner accepted responsibility? |
| Scheduling | May the work proceed now? |
| Execution | What has happened while carrying it out? |

Deferring or pausing work removes it from the actionable queue without erasing the assignment history. Pausing in-progress work requires a checkpoint before Merl releases the execution lease.

A handoff references the Task, current project revision, relevant objects, artifacts, and an optional short note payload. The next agent receives current objects. It does not inherit a copied snapshot that can drift from accepted state.

### Planner decisions and outstanding obligations

The daily-use workflow has one planner per project coordinating implementers, researchers, and reviewers. A worker can raise a Question with alternatives and evidence. The planner records a Decision within its authority, or leaves the question pending for an authorized human. A conditional decision links to the Tasks and Blockers that express its prerequisites and follow-up work, with an owner and an explicit completion or verification criterion for each obligation. These use existing knowledge types and relations.

Revising one part of a decision must preserve its unaffected obligations. A reviewer can challenge a factual premise or requirement and return it to the planner for disposition. The finding remains visible until an authorized resolution is recorded; delivery or acknowledgement alone does not resolve it.

Provider merge state, permission to proceed, and verification of an outcome remain separate facts. Several independent prerequisites may constrain a Task. A local Task may record an external prerequisite and the evidence needed to clear it, without automating another project or probing external resources. A merge cannot by itself clear those prerequisites or verify the promised outcome.

## Repository synchronization

The first release captures one named Issue at a time. Phase 4 lets a human attach a repository once and records which parts of it Merl should maintain:

```bash
merl repository attach \
  --project project-a \
  --repository acme/project-a

merl repository sync --project project-a
```

The binding stores a versioned selection policy. It may include open Issues in an active milestone, selected labels, explicit Issue or pull request numbers, and provider objects referenced by accepted Tasks. A project can retain a closed Issue when current work still depends on it.

Selection begins with structural provider metadata. Merl does not read cold body text to decide whether that text deserves compilation. The sync worker may keep a cheap catalog of Issue and pull request identity, state, labels, milestone, assignees, and update time while capturing full source versions only for selected surfaces.

Capture and compilation remain separate. A practical policy might compile human comments on active assigned work eagerly, retain bot comments as `capture_only`, and leave an unrelated backlog uncaptured or catalog-only. Changing the policy requires an authorized operation and does not rewrite the policy recorded on earlier captures.

The local daemon runs bounded incremental sync, and the CLI can request it directly. Provider pagination, rate limits, and incomplete responses stay visible. An incomplete listing cannot imply that an Issue, comment, or pull request was deleted. Repeating a sync is idempotent and does not redispatch completed compiler work.

This is selective project maintenance, not an instruction to compile an entire repository. Sync reports what it cataloged, captured, compiled, skipped, or could not verify.

## Assignment context

Resume derives relevance from the active assignment. Role affects ordering after that selection.

The context contains, within a recorded budget:

1. identity, role, and active guidance;
2. the assigned Task and its planning state;
3. requirements, decisions, contracts, blockers, and findings connected to the Task;
4. the latest checkpoint and current versions of its references;
5. relevant accepted changes since the session cursor;
6. expandable references for omitted detail.

Relation kind and direction affect priority. A blocker should appear before an incidental mention. The response reports selector version, coverage scope, and truncation so an agent cannot mistake a bounded view for proof that no other state exists.

A planner's coordination assignment needs a bounded view across the work it oversees: questions awaiting rulings, challenged assumptions, blocked or deferred Tasks, outstanding obligations, and reported outcomes awaiting acceptance. Selection follows that explicit coordination scope and project policy. It reports the same budgets, omissions, and expansion references as a worker view; a planner role does not justify loading the whole project. A fresh planner session must recover this queue from accepted state without reconstructing it from conversations.

Phase 4 does not need staffing requirements such as preferred model, reasoning effort, or cost class. Those fields help Phase 5 choose among runtimes. Product requirements and review obligations that affect implementation are ordinary accepted project state and belong in the Phase 4 context.

## Workspaces

Every writable assignment gets an isolated workspace. The default local strategy uses one managed Git object store with a separate worktree and branch for each writer:

```text
~/.merl/
  git/<repository-id>.git/
  workspaces/<project>/<task>-<agent>/
```

The workspace record links the source, Task, Assignment, logical agent, path, Git identity, branch, base revision, access mode, creation time, and lease state. It belongs to the durable work, not to one disposable session. The current session may hold separate execution and workspace leases.

Merl canonicalizes paths and inspects Git worktree identity before granting a lease. Two writers cannot hold leases for the same working tree. Review work uses a detached read-only worktree at a pinned commit unless the reviewer needs a separate writable branch.

Build output and temporary files remain workspace-local. Merl routes process scratch beneath a directory it owns and records the configured allowance. A full clone remains available when Git worktrees cannot provide the required isolation.

Workspace cleanup is conservative. Dirty files, unpreserved commits, active processes, or unpublished work block removal. Merl cleans only marked paths beneath its storage root. It never sweeps a user checkout or the system temporary directory.

A normal manual flow looks like this:

```bash
merl agent register project-a-dev --role engineer
merl task assign T52 --to project-a-dev --workspace auto
merl agent attach --agent project-a-dev --task T52
merl session resume --agent project-a-dev --format json
```

Assignment returns the workspace path, branch, and bootstrap line. The human starts the process in that path. Attach binds the external process to a new session, verifies that it is using the assigned workspace, and acquires the Assignment's execution lease. A replacement session reuses the Assignment and workspace after the previous lease closes or expires.

## Inbox and direct coordination

Assignment and subscription policy route reference-only inbox entries to agents. One accepted transition may fan out to several agents without copying source prose or recompiling for each recipient.

An active attached session may use the local daemon through `merl inbox watch`. A failed wake leaves the entry pending. Polling remains a complete fallback.

Most project work uses structured commands. If an engineer already knows that a Task is blocked, it records that state directly instead of writing prose for another model to extract.

Direct prose remains available for nuance:

```bash
merl task request --team software \
  --summary 'Review parser memory safety'

merl message send \
  --to project-a-reviewer \
  --ref T45 \
  --summary 'Please look at the unsafe block in the parser'
```

The Task is the schedulable state change. The note is immutable source evidence with separate delivery records. Its payload stays cold in ordinary views. Delivery and acknowledgement do not imply agreement, task acceptance, or completion.

## Pull requests and review

An agent team needs a path from workspace changes to review. Phase 4 extends the Issue source integration to pull requests, reviews, commits, and merge state.

GitHub remains authoritative for provider facts such as head commit, open or closed state, review submission, and merge result. Merl owns the interpreted intent, affected contracts, findings, blockers, and review obligations. Provider observations and semantic state share the project revision and inbox path without overwriting each other.

An engineer may publish its assignment branch through a structured command. The resulting pull request retains the Task, Assignment, workspace, branch, base revision, and accepted requirements that justified the change. A reviewer receives a detached read-only workspace at the reviewed commit and a focused view of intent, contract changes, findings, and test evidence.

Merl publishes sparingly. Routine cursor movement, checkpoints, and derived state remain internal. Publication policy may update one bounded status surface or append ordinary prose for a decision, changed requirement, serious blocker, or merge-blocking finding. GitHub writes happen after Merl commits accepted state and remain retryable after an ambiguous provider response.

## Session closure and resume

Before a session ends, Merl records a checkpoint with the understood project revision and references to current work. The checkpoint may carry a short summary payload, but current accepted state wins when that prose becomes stale.

```bash
merl session checkpoint \
  --agent project-a-dev \
  --ref T52 --ref B9 --ref D18

merl session close \
  --agent project-a-dev \
  --assignment A52 \
  --reason context-replacement
```

`continuous`, `task-scoped`, and `manual` context policies control closure behavior. Closing a session commits its checkpoint, cursor, closure record, and execution and workspace lease releases before asking the host to clear context. It does not complete the Assignment or Task. If work is finished, the agent first records the Task outcome through a separate accepted transition; policy may then close the Assignment. If work remains, the Assignment and workspace remain ready for another session. If the host cannot reset context, Merl reports that fact and prints a manual next step.

A later session receives a new generation ID and acquires new leases on the existing Assignment and workspace. It never inherits the prior session's leases. Resume uses active guidance and current task state without replaying the prior transcript.

## Team evidence

Phase 4 measures whether a team can work reliably with less repeated context. Each Assignment records, where the host makes the data available:

- resume and delta tokens;
- object and source expansions;
- direct-note payload reads;
- compiler work caused by agent prose;
- checkpoint and restart cost;
- model usage and task outcome;
- missed blockers, requirements, and stale-state mistakes;
- handoff or restart failures;
- review findings caught, missed, or rediscovered;
- unnecessary source and history rereads.

The ledger covers the planner and all workers, including decision reconciliation, review and revision loops, failed attempts, and context replacements. Record every human copy/paste or repeated instruction needed to finish the workflow, with its reason. Report manual interventions separately from measured token usage; unavailable host measurements remain unavailable. Both comparison arms use these accounting boundaries.

The report compares Merl with a transcript-oriented team running the same scenario. Merl succeeds only if the team completes the workflow at least as reliably while consuming substantially less repeated project context. The report includes failures and disagreements rather than treating low token use as success by itself.

## Deliverables

Phase 4 includes:

- agent profiles, project membership, roles, and durable guidance;
- repository attachment and versioned selection, capture, and compilation policies;
- bounded incremental synchronization for selected Issues and pull requests;
- session generations and one actionable inbox lease per logical agent;
- Task assignments, execution leases, pause, handoff, and assignment history;
- assignment-derived and relation-weighted project views;
- managed writable and read-only Git workspaces;
- workspace-local scratch routing and safe cleanup;
- session checkpoint, close, resume, and supported context reset;
- local daemon-backed inbox watch with polling fallback;
- structured work requests and optional cold direct messages;
- pull request capture, review-focused context, and provider-owned merge state;
- branch publication and sparse GitHub status or comment output;
- packaged local CLI and daemon artifacts with a documented install path;
- correctness, continuity, token, and context evidence across agents and restarts;
- a CLI-only multi-agent behavior scenario.

## Acceptance criteria

Phase 4 is complete when:

- a human can attach an existing GitHub repository once and inspect its effective synchronization policy;
- repository sync discovers eligible Issues and pull requests from provider metadata without reading cold body text to make the selection;
- open, milestone, label, explicit-reference, and accepted-Task selectors have deterministic, versioned behavior;
- selected existing Issues capture their available descriptions, comments, edits, and provider facts;
- repeated sync creates no duplicate sources, accepted effects, or completed compiler work;
- incomplete provider responses do not invent deletions or claim a complete refresh;
- eager, on-demand, capture-only, required, and optional behavior follows the durable binding policy;
- unrelated backlog prose does not enter compilation merely because its repository is attached;
- logical agent identity survives process restart and model-context replacement;
- project role and membership come from durable policy rather than runtime claims;
- one active session holds a logical agent's actionable inbox and Assignment execution leases;
- ending or replacing that session does not end the Assignment or discard its workspace;
- an authorized human or policy-authorized planner can assign a Task to an existing logical agent, while an allowed agent can claim eligible work;
- assignment outside the planner's authority is rejected without changing responsibility;
- assignment history survives reassignment, pause, deferral, completion, and cancellation;
- two agents assigned different Tasks receive different default context without supplying `--focus`;
- focus selection uses relation kind and direction, records its selector version, and reports truncation;
- session resume returns bounded guidance, assignment state, checkpoint references, relevant deltas, and current accepted objects;
- current accepted state overrides stale checkpoint prose;
- planner resume recovers pending rulings, challenges, obligations, blockers, and outcomes awaiting acceptance within an explicit coordination scope and budget;
- a partial decision revision preserves unaffected obligations and routes the relevant changes to their owners;
- merge observation does not clear independent prerequisites or imply outcome verification;
- a new writable assignment receives a unique branch and worktree;
- two active writers cannot obtain leases for the same working tree;
- read-only review work cannot modify the writer's checkout;
- dirty files and unpreserved commits block automatic workspace removal;
- Merl never deletes an unmarked path or a path outside its managed storage root;
- structured agent actions reach policy without compiler extraction;
- one direct note creates one source event and several deliveries without recipient-specific compilation;
- raw note payloads stay out of ordinary inbox and assignment views until requested;
- acknowledgement records receipt or cursor movement without changing Task or semantic state;
- a missed wake leaves durable work discoverable through polling;
- publishing an assignment branch links the pull request to its Task, Assignment, workspace, and base revision;
- pull request capture keeps GitHub-owned facts separate from interpreted intent, findings, and blockers;
- a reviewer receives the reviewed commit in a separate read-only workspace;
- routine machine state stays out of GitHub while significant configured events produce bounded, retryable publication intents;
- an ambiguous GitHub write cannot duplicate a pull request, status update, or comment after reconciliation;
- task-scoped closure commits checkpoint, session state, and lease releases before any host reset;
- session closure does not complete or cancel the Assignment or Task;
- unsupported host reset produces an honest pending or manual result;
- a restarted session can acquire new leases on the existing Assignment and workspace and continue unfinished work without the previous transcript;
- two manually started agents using different supported host adapters can attach to the same project and coordinate through the same accepted state and inbox path;
- the acceptance scenario runs through public CLI commands without private Rust or SQLite access;
- the report accounts for correctness, missed state, review outcomes, rendered context, expansions, restarts, and measured host usage where available;
- the Merl team completes the reference workflow at least as reliably as the transcript-oriented comparison while reducing repeated project-context consumption.

## Acceptance scenario

The release scenario uses an existing GitHub repository plus a PM, an engineer, and a researcher that a human starts and attaches. At least two workers use different supported host adapters; they may also use different model families.

1. A human attaches the repository and selects its active milestone and agent-work labels.
2. Merl catalogs the repository, captures selected existing Issues, and leaves unrelated backlog prose out of compiler input.
3. A later sync records one new comment and one provider-state change without duplicating earlier work.
4. Within human-established policy, the planner assigns separate accepted Tasks to the engineer and researcher.
5. Merl creates an isolated writable worktree for each Task that needs repository access.
6. The engineer and researcher attach through different host adapters and resume into different assignment-focused contexts.
7. The researcher raises a Question with alternatives and evidence. The planner records a conditional Decision and links owned prerequisite and follow-up Tasks.
8. The engineer receives a compact delta and expands the decision and relevant evidence. A supplemental note does not duplicate the structured work.
9. The engineer publishes its assignment branch as a pull request.
10. A reviewer opens the pinned commit in a separate read-only workspace and challenges one premise of the decision. The planner revises that part while preserving an independent obligation.
11. While work remains unfinished, the planner and engineer checkpoint and close. Their Assignments, outstanding obligations, and workspaces remain durable.
12. Fresh sessions acquire new leases and recover the planner's pending queue and the engineer's current requirements without prior transcripts. Stale checkpoint prose cannot restore the superseded premise.
13. The team resolves the review finding against the applicable commit and GitHub reports the final merge state through the project revision and inbox.
14. An independent prerequisite remains open after merge. The team records the required evidence before accepting the dependent outcome.
15. Merl reports task correctness, lost obligations, missed state, review outcomes, continuity failures, manual copying, rereads, and whole-team context cost for both comparison arms.

Use synthetic project content for the published scenario and evidence. Freeze its expected obligations, revision boundaries, restart points, and verification rules before the comparative run.

### Task ownership

The existing Phase 4 tasks own these requirements:

| Work | Tasks |
| --- | --- |
| Planner assignment authority and obligation lifecycle | #87 |
| Bounded planner context and worker/planner restart | #97, #98 |
| Planner routing and conditional decision coordination | #100, #102 |
| Review challenges, disposition, and merge versus verification | #106, #107 |
| Frozen scenario, whole-team ledger, and comparison | #109, #111, #112 |
| Exploratory planner workflow and manual-copy findings | #138 |

The exploratory checkpoint does not replace the frozen comparison or reorder the release milestones.

## Deferred work

Phase 4 excludes:

- manager-selected agent templates and delegation grants;
- runtime capability and cost ranking;
- manager-initiated spawn, drain, stop, replacement, or scaling;
- host provisioning and ambiguous-spawn reconciliation;
- cross-project requests and transport;
- shared or remote project authorities;
- node export and project authority transfer.

The first four items belong to Phase 5. Later release plans will own the remaining product work.
