# Merl architecture

Status: draft for implementation

This document defines Merl's implementation architecture. The [product description](product-description.md) explains the product and its intended use. The [user interface and behavior contract](user-interface.md) defines what callers can observe. This document fixes the boundaries, consistency model, and failure behavior that the code must preserve.

## Decision summary

| Area | Decision |
| --- | --- |
| Implementation | Rust workspace with a transport-independent domain core |
| Project boundary | A project may span repositories and other source systems |
| Source relationship | Projects and sources are many-to-many through explicit bindings |
| Cross-project work | Separate state aggregates coordinate through durable envelopes |
| Deployment | Local authority or shared authority behind one domain API |
| Write ownership | Exactly one authority accepts mutations for each project |
| History | Append-only record envelopes with audited erasure of protected payload bytes |
| Current state | Materialized from accepted domain events and rebuildable |
| Compiler context | Bounded, reproducible input selected from source and accepted state |
| Policy input | Assertions, commands, receipts, and administrative actions share one typed boundary |
| Concurrency | Global revision ordering with dependency and write-set validation |
| Delivery | Inbox entries commit with state; wake-up happens after commit |
| Work planning | Commitment, scheduling, and execution remain independent |
| Agent continuity | Structured guidance and checkpoints outlive model context |
| Agent selection | Session advertisements match task requirements with visible reasons |
| Agent provisioning | PMs instantiate only human-approved templates within delegated limits |
| Workspace isolation | One writable worktree and branch per active assignment |
| Temporary storage | Owned disk-backed scratch with reservations, quotas, and safe cleanup |
| Node portability | Secret-free manifests recreate local setup on a new node |
| Human publication | Post-commit, sparse, target-scoped, and retryable |
| Provider authority | External facts remain distinct from Merl semantic overlays |
| Local trust | Same-user processes are trusted unless the host adds OS isolation |
| Source integration | Polling and webhooks feed one incremental GitHub capture path |
| Agent interfaces | CLI first, then a thin MCP adapter over the same core |
| Extraction | Versioned compiler protocol with no authority to mutate state |
| Local storage root | `MERL_HOME`, defaulting to `~/.merl` |

## System shape

Merl has an event-sourced kernel, two state planes, and several adapters.

```mermaid
flowchart TB
    subgraph activity[External project activity]
        GH[GitHub repositories]
        AR[Artifact stores]
        RR[Research systems]
    end

    subgraph authority[Merl Project Authority]
        SA[Source adapters]
        SE[(SourceEvent log)]
        CX[CompilationContext]
        CR[CompilationRun]
        OA[ObservedAssertion]
        PI[PolicyInput]
        PE[PolicyEvaluation]
        DE[(DomainEvent log)]
        KP[Knowledge plane]
        CP[Control plane]
        MP[Materialized projections]
        IN[Durable inbox]
        CMD[Authenticated commands]
        PUB[Publication worker]

        SA --> SE --> CX --> CR --> OA --> PI --> PE --> DE
        CMD --> PI
        DE --> KP --> MP
        DE --> CP --> MP
        MP -.-> CX
        MP --> IN
        DE -.-> PUB
    end

    subgraph clients[Merl clients]
        A[Developer A cache]
        B[Developer B cache]
        CLI[CLI]
        MCP[MCP]
    end

    GH --> SA
    AR --> SA
    RR --> SA
    MP --> A
    MP --> B
    MP --> CLI
    MP --> MCP
    CLI --> CMD
    MCP --> CMD
    IN --> WA[Wake-up adapters]
    PUB --> GH
```

The event kernel supplies six shared primitives: events, objects, relations, revisions, source references, and actors. Neither state plane owns those primitives.

The knowledge plane holds accepted project knowledge and durable work. The control plane records how agents carry out that work. Control objects refer to knowledge objects by stable ID. They do not copy project state into agent records.

## Project boundary and sources

A `Project` owns one accepted event history, revision sequence, authority policy, membership list, and subscription space. It is the unit Merl serializes. A code repository is one source attached to that project.

The architecture keeps three categories of state separate:

| Category | Examples | Owner |
| --- | --- | --- |
| External evidence | GitHub comments, commits, papers, experiment output | Source system |
| Shared Merl state | Decisions, claims, tasks, findings, cross-source relations | Project authority |
| Local node state | Credentials, caches, logs, workspaces, pending commands | `MERL_HOME` |

A `ProjectSource` identifies an external namespace. Source kinds include repositories, artifact stores, experiment systems, local artifact directories, and future communication connectors.

A `ProjectSourceBinding` connects one source to one project. This many-to-many relationship lets a project use several repositories while one repository feeds several projects. The binding stores three independent concerns:

```yaml
selection:
  labels: [atlas]

capabilities:
  ingest: true
  compile: true
  publish_comment: true
  create_issue: false

credential_ref: github-work
```

Selection decides relevance. Capabilities decide permission. The credential reference supplies provider access. A selector never grants authority. Each binding keeps its own ingestion cursor and publication configuration while accepted effects enter the project's revision sequence.

A `Repository` stores a Merl ID, provider, immutable provider repository ID, current human-readable name, and URL. Repository names and ownership can change. Provider identity preserves continuity across a rename or transfer. Issues and pull requests refer to their repository ID plus an immutable provider object ID where available.

Provider-backed work has two projections. The provider mirror records facts owned by GitHub or another source: title, body version, open or closed state, labels, assignees, timestamps, commit SHAs, review state, and merge state.

The Merl semantic overlay records project interpretations such as requirements, phase, blockers, decisions, findings, and research claims. A Merl command may request a provider action, but it cannot report a provider-owned fact as changed until the provider reports that change.

Merl discovers a provider-backed repository from its Git remote and provider API. Local or unrecognized sources receive node-local identities when a user runs `merl source attach`. Repository discovery does not require a committed Merl marker.

Project-level objects do not need a repository owner. A hypothesis may draw evidence from commits in two repositories and a dataset in an artifact store. Relations connect those sources and objects within the same project.

GitHub, Git commits, experiment output, and papers remain external evidence. Merl owns their immutable captures and the accepted interpretations derived from them. It does not claim ownership of the upstream objects.

When several projects consume one provider event, each project runs its own compilation and policy evaluation through its binding. An implementation may deduplicate captured bytes at one authority, but the project interpretations remain independent. A separate `ProviderEvent` layer is a schema optimization rather than an architectural requirement.

## Event pipeline

Merl separates external evidence, machine interpretation, authority, and accepted state. Each layer answers a different debugging question.

### Source events

A `SourceEvent` records an observation outside Merl's accepted state and normally retains the observed bytes. Sources include GitHub comments, issue edits, pull request reviews, commits, local files, experiment output, and evidence uploaded by an agent. A semantic CLI command is a `Command`, not a source event.

A source event contains:

- a stable Merl ID
- the source kind and external entity ID
- a captured payload or content-addressed artifact reference
- a content hash and source version when one exists
- the actor and observation time
- a link to any source event it supersedes

Merl never edits a source event. If a user edits GitHub comment 771, Merl captures a new event with the same external entity ID, a new content hash, and a `supersedes` link to the previous capture. The pair `(external identity, source version or content hash)` provides the ingestion idempotency key.

Merl normally retains captured content because upstream content can change or disappear. An external URL alone is not enough for replay or audit. The metadata record and the retained bytes have different lifetimes: source metadata is append-only, while policy may require deletion or redaction of the content.

An authorized administrative purge removes the retained payload or destroys its encryption key. Merl keeps a tombstone with the source identity, digest, actor, time, scope, and reason. It also marks dependent derivations as having unavailable evidence and reports that affected history can no longer be replayed from the retained store. Purge is an audited exception to byte retention, not a rewrite that pretends the source never existed.

Append-only envelopes must not become a hiding place for copied secrets. Source prose and other protected text live behind erasable payload references wherever practical. A purge preview follows derivation links and identifies assertions, events, projections, or rendered inputs that contain protected material. Policy may erase those payloads too, leaving structural tombstones and redacted projections. Merl reports the resulting loss of replay fidelity.

### Compilation contexts

A new comment rarely makes sense by itself. Phrases such as "same as the first run" or "the frame issue above" require accepted state and a small amount of conversational history. Merl records that input in an immutable `CompilationContext` rather than recompiling an entire thread or asking the compiler to guess.

The context manifest contains:

- triggering source-event IDs;
- the basis project revision;
- selected object and relation revisions;
- collection or predicate snapshots used during selection;
- recent source-event IDs and their order;
- the context-selection policy, budget, and version;
- the renderer version and exact rendered-input digest;
- a content-addressed copy of the rendered input when retention policy permits it.

The context builder starts with the triggering event, current materialized state for the external object, relevant unresolved objects, and a bounded recent window. A versioned selector may expand that set through relations. If the input remains ambiguous, the compiler returns an unresolved assertion or a structured request for more context. It does not silently load the full history.

Replay uses the recorded manifest and available source bytes. A purge can make exact replay impossible; the run remains auditable through its manifest and digests.

### Compilation runs and observed assertions

A `CompilationRun` records one attempt to interpret a `CompilationContext`. Its provenance includes the compiler name and version, context ID, configuration, prompt or ruleset hash, model identity when applicable, mode, and start and completion times.

Compilation modes are:

- `live` for normal ingestion
- `replay` for rebuilding or comparing prior results
- `eval` for experiments against the frozen corpus

Mode describes why the run happened. It does not grant permission to change accepted state.

Each run emits zero or more `ObservedAssertion` records. An assertion describes what that compiler inferred, such as a question, a proposed decision, or a relationship between an experiment and a claim. Assertions retain confidence and precise source provenance when the compiler can supply them.

Compilation runs and assertions are append-only. Running compiler version 0.7 against a source previously handled by version 0.4 creates another run and another set of assertions. It does not replace the earlier interpretation.

Compilers cannot write accepted objects or domain events. Built-in Rust compilers call a narrow core API. Experimental compilers use a versioned process or service protocol and return assertions through the same boundary.

### Policy evaluations

A `PolicyInput` is an immutable semantic proposal presented to policy. Its kinds include `ObservedAssertion`, `Command`, `CrossProjectReceipt`, and `AdministrativeAction`. This keeps direct user actions honest: a command-created decision does not pretend to have a source event or compilation run.

A `PolicyEvaluation` decides how Merl should treat one or more policy inputs. It evaluates them together with accepted state at a recorded project revision.

The evaluation records:

- typed input IDs and their individual dispositions
- the policy version and configuration
- the basis project revision
- object, relation, collection, and predicate dependencies read during evaluation
- the proposed write set
- the actor or process that requested evaluation
- the result and reasons
- proposed state mutations
- links to domain events appended by a successful commit

Possible input dispositions include accepted, candidate, rejected, duplicate, and conflict. One evaluation may accept one input while rejecting another, even when it considered both together.

The evaluation record remains immutable. A successful commit adds the evaluation-to-event links as separate append-only relations in the same transaction as the domain events.

Policy applies authority as well as confidence. Merl may accept a deterministic observation from a trusted machine artifact. It may accept an explicit decision from a human who has authority over that project. An agent's interpretation of research direction can remain a candidate. Closing an issue, approving a merge, or superseding a major decision can require explicit permission.

Replay and evaluation results do not enter accepted state on their own. Promotion creates a new policy evaluation against current state. The original inputs remain unchanged.

### Actors and commands

A client asks the project authority to act by submitting a `Command`. Commands include semantic operations such as resolving a question, assigning a task, or recording an experiment result. They are requests for evaluation, not accepted events.

A command contains:

- a globally unique command ID
- the project ID and basis revision
- the requested operation and arguments
- the authenticated actor
- the submission time and client metadata

The authority stores command receipt and outcome so a client can retry the same command ID without applying it twice. A command ID cannot be reused with another payload. The command remains immutable; append-only outcome records mark it accepted, rejected, conflicted, or pending review. The command enters policy as a typed input and may produce a domain-event batch.

A disconnected client queues commands, not domain events. On reconnect, the authority checks the basis revision, authenticates the actor, applies current policy, and either commits a new domain-event batch or returns a conflict. Only the project authority creates accepted domain events.

An `Actor` has a stable Merl identity and may link to provider identities such as an immutable GitHub user ID. `ProjectMembership` assigns roles and permissions within a project. The authority derives the actor from authenticated credentials; it does not trust a client-supplied display name.

### Domain events and materialized state

A `DomainEvent` records an accepted change. Examples include `decision.activated`, `question.resolved`, `claim.invalidated`, and `assignment.created`.

Every accepted transaction groups its domain events under one stable `DomainEventBatch` ID and project revision. Subscription matching and human publication evaluate the batch as a whole rather than treating each event as a separate human occurrence.

Domain events are append-only. A correction emits another event that supersedes or reverses the earlier effect. Merl does not rewrite the historical record.

Materialized objects and relations are the current projection of accepted domain events. They exist for fast reads and compact rendering. Merl must be able to rebuild them from the domain log.

## State planes

### Knowledge plane

The knowledge model covers concepts that Merl's intended users already use:

```text
Work             Issue, PullRequest, Task
Coordination     Decision, Requirement, Question, Blocker
Knowledge        Fact, Artifact, Finding, Claim, Evidence
Research         Hypothesis, Experiment
Software         Contract
Cross-project    OutboundRequest, InboundRequest
```

The model should preserve these concrete names. A premature abstract ontology would make agents and users translate ordinary project language into Merl's language.

Some types need a discriminator rather than another top-level type. `Finding.kind` can distinguish a review finding from an experimental finding. `Claim.domain` can distinguish research, software, and other claims. `Artifact` remains broad enough for files, commits, logs, datasets, papers, reports, and remote blobs.

Relations connect objects without embedding copies. Examples include:

```text
Question resolves_to Decision
Decision supersedes Decision
Finding blocks PullRequest
Experiment tests Hypothesis
Evidence supports Claim
Contract applies_to PullRequest
Task depends_on Task
```

### Work requests and planning

A `Task` may begin as a request from a person, an agent, ingestion, or an inbound cross-project request. Its existence establishes that the project knows about the work. It does not establish that the project has committed to doing it.

Tasks and inbound requests expose three independent facets:

| Facet | States | Meaning |
| --- | --- | --- |
| Commitment | `pending`, `accepted`, `declined` | Whether the owner has taken responsibility |
| Scheduling | `unscheduled`, `deferred`, `scheduled` | Whether and when the owner plans to revisit or perform it |
| Execution | `not_started`, `in_progress`, `blocked`, `completed`, `cancelled` | What has happened in carrying it out |

Not every combination is legal. Declined work cannot start. Completed work cannot return to `in_progress` without a new corrective transition. Policy owns those rules; the model does not encode them in one overloaded lifecycle enum.

Deferral requires a reason and at least one reconsideration trigger: a review time or a relation to an accepted project condition. A deferred item can remain `pending` when the owner has made no commitment, or `accepted` when the owner has committed but has not scheduled the work. Renderers always show commitment with scheduling.

Requester constraints and owner commitments use different fields. `needed_by` and `impact_if_late` state the requester's need. `target_start` and `target_finish` state the owner's schedule. Merl detects and displays a mismatch but does not resolve it by changing priority.

Planning transitions are accepted project state. They advance the project revision and notify requesters, assignees, and other subscribers. An agent's actionable queue contains accepted, assigned work whose scheduling permits execution. Deferred work remains visible in planning views but is not actionable.

Deferring an `in_progress` task requires an explicit pause transition. The authority does not treat a date change as an instruction to kill a running process. The pause records or requests a checkpoint, releases any execution lease according to policy, and preserves the assignment history.

### Control plane

The control model contains:

```text
Agent, AgentTemplate, SpawnRequest, ProvisioningAttempt,
AgentSession, AgentAdvertisement, ContextPolicy, Assignment, Subscription,
InboxEntry, Cursor, Lease, Checkpoint, Handoff, WaitCondition, Acknowledgement,
AgentProfile, AgentGuidance, CrossProjectEnvelope, EnvelopeReceipt
```

An `Assignment` connects an agent to a knowledge-plane `Task`. If the assignment expires, the task remains. A `WaitCondition` describes a runtime pause; a knowledge-plane `Blocker` describes an accepted project constraint. A task's durable review condition belongs to its scheduling state, while a `WaitCondition` parks one agent execution.

Current leases and process sessions may expire. Assignment history, checkpoints, handoffs, acknowledgements, and failed delivery records remain available for recovery and diagnosis.

A checkpoint records the project revision an agent understood and references the relevant knowledge objects. It does not copy those objects into a session summary. A handoff may include short prose, but its durable context comes from object and artifact references.

### Agent guidance and context assembly

An `AgentProfile` gives one logical agent a stable identity independent of a process, session, model provider, or computer. Restarting the agent or clearing its model context does not create another logical agent.

`AgentGuidance` stores durable operating guidance with one of three kinds:

- `instruction` for behavior required by a human or project policy
- `practice` for an adopted working method
- `lesson` for an observation proposed for reuse

Each record has scope, statement, provenance, author, authority, priority, lifecycle, and creation revision or local profile version. Lifecycle states are `proposed`, `active`, `superseded`, and `retired`. Policy decides who may activate guidance at each scope. Project-scoped guidance is durable control state in the project authority. Personal or cross-project guidance belongs to the portable agent profile on the user's node.

Guidance is not a message and does not become project knowledge. It may shape how an agent works, but accepted project requirements, decisions, and safety policy take precedence. Structured records allow Merl to select relevant guidance without injecting an unbounded memory transcript.

Session resume is a deterministic context projection, subject to a rendering budget. It considers, in order:

1. logical agent identity and current role
2. active guidance relevant to the project and task
3. actionable assignments
4. the latest checkpoint and its references
5. accepted changes since the checkpoint cursor
6. current versions of referenced project objects

A checkpoint preserves continuity, not truth. Current accepted state wins when checkpoint prose has gone stale. Removing model-host context leaves profiles, guidance, checkpoints, and project state untouched. Retiring or superseding guidance requires an explicit durable operation. Retirement removes guidance from future context but preserves its audit history.

### Delegated agent provisioning

An `AgentTemplate` defines the limits within which a project manager may create agents. A human or another actor with delegation authority sets:

- role and allowed projects
- host adapter and model or effort bounds
- project permissions and credential references
- context, workspace, review, and storage policies
- concurrency and relative cost limits

The template contains credential references, never secret values. Changing a template or its delegation requires the same authority as the original grant.

An authorized PM submits a `SpawnRequest` against one template and task. Policy checks the delegation, task state, concurrency, cost allowance, source access, and storage reservation before accepting it. The accepted transition creates a logical agent instance or selects a reusable identity, reserves its resources, and writes a durable provisioning intent.

A host worker acts after commit. Each `ProvisioningAttempt` records the host, template version, requested runtime, time, result, and external session reference. Host failure leaves the request failed or retryable; it does not mark the agent available. Only a ready session may advertise availability, receive a workspace lease, and start an assignment.

PMs may drain or stop agents created under their delegation. Draining blocks new assignments and lets current work checkpoint. Stopping follows the session-closure rules and releases reservations only after the host confirms exit. A PM cannot widen permissions, reveal credentials, change template limits, or provision an unapproved runtime.

Humans may attach an externally started process to an approved logical identity. The authority still checks project membership and template constraints. Merl marks runtime fields with their actual provenance rather than treating an attached process as host-attested.

### Context lifetime

An `AgentSession` is one disposable model-context generation for a logical agent. `ContextPolicy` controls when Merl asks the host adapter to end that generation:

| Mode | Behavior |
| --- | --- |
| `continuous` | Keep context across related assignments until explicitly closed |
| `task-scoped` | Close context after the bound task becomes non-actionable |
| `manual` | Record checkpoints and closure readiness but never clear automatically |

The mode is configurable by agent and may have a project-specific override. Roles can provide defaults, but Merl does not assume every developer is task-scoped or every manager is continuous.

A task-scoped session closes through a durable transition before any host side effect. The accepted transition records the final checkpoint, task outcome and references, cursor, proposed guidance, assignment state, and lease release. After commit, a host adapter clears or replaces the model context. Failure leaves a retryable context action and never rolls back the checkpoint or task state.

Automatic closure requires the bound task to be completed, cancelled, deferred, or otherwise non-actionable according to policy. Merl does not reset an agent with active work merely because another task arrived. A user can request an early close, but the same checkpoint and lease-release rules apply.

The next session receives a new generation ID and the bounded resume projection. Active guidance such as a TDD practice survives; transcript tokens and incidental reasoning from the previous task do not. A continuous session still consumes role deltas and bounded views so its context need not grow without limit.

### Runtime advertisements and assignment

An `AgentAdvertisement` describes the runtime available for one `AgentSession`. It records:

- provider and model identifiers as versioned strings
- host reasoning-effort value and a normalized effort class when available
- declared capabilities and tool or source access
- availability and current assignment load
- context-policy and host reset support
- relative cost class and measured usage when available
- provenance and observation time for every field

The advertisement belongs to the session, not the logical `AgentProfile`. A new session publishes a new advertisement. Historical advertisements remain attached to their assignments and outcomes, while stale sessions are not eligible for new work.

Host-attested values are preferable. Operator-configured and self-reported values remain useful but carry their actual provenance. Merl does not present a self-reported model, capability, or effort level as independently verified.

Effort names are not assumed to mean the same thing across hosts or providers. Merl preserves the raw host value and applies a versioned, provider-specific normalization only when assignment policy needs a comparable class.

A `Task` may carry assignment requirements: necessary capabilities and access, minimum reasoning demand, risk, required review path, budget preference, and concurrency constraints. These fields describe the work rather than naming a preferred agent.

Candidate selection first removes ineligible sessions, then orders the remainder through a versioned assignment policy. The result explains each inclusion, exclusion, and tradeoff. A cost-preferring policy selects the least expensive eligible runtime; it never chooses a cheaper session that misses a hard requirement. A capability-preferring policy can favor additional headroom for risky or difficult work.

Selection is advisory until an authorized actor commits an `Assignment`. That record captures the chosen session advertisement, task requirements, policy version, and human or automated rationale. Runtime properties grant no project membership, source capability, or authority.

### Concurrent repository work

A `Workspace` is node-local operational state that connects an agent session and assignment to one source checkout. Its record contains the source ID, task and assignment IDs, canonical path, filesystem identity where available, access mode, strategy, Git object store, branch or detached commit, base revision, creation time, and lease state.

The default strategy for a writable assignment on one node is a managed Git worktree:

```text
~/.merl/
  git/<repository-id>.git/                 shared object store
  workspaces/<project>/<task>-<agent>/     independent worktree
```

Each writer receives a unique branch, such as `merl/<task-id>/<agent-id>`. Worktrees may share the object database, but they do not share a working directory, index, checked-out branch, or process current directory. Merl does not place managed worktrees inside another repository's working tree.

An attached user checkout can serve as a workspace only when it has an exclusive writable lease. A second writer receives another worktree. Read-only assignments use a detached worktree at a pinned commit by default. Multiple readers may share that immutable checkout only when the host or filesystem adapter enforces read-only access; they do not share a writer's live working tree.

The workspace registry canonicalizes paths, resolves symbolic links where possible, and inspects Git worktree identity before granting a lease. Two active writable leases cannot resolve to the same working tree. The shared Git object store is allowed and managed separately from workspace ownership.

Merl uses a full clone when worktrees are unsupported, an isolation policy requires separate object stores, or repository-specific behavior makes sharing unsafe. The selected strategy is visible in the workspace record. Containers or remote workspaces can implement the same interface later.

Reviewers do not check out a writer's active branch in the same worktree. A read-only review workspace uses the reviewed commit in detached mode; a reviewer that may commit receives a separate branch and writable workspace.

Build outputs remain workspace-local by default. Tool caches may be shared only when their adapter declares concurrent access safe; sharing the Git object store does not authorize sharing compiler output directories.

Workspace release is conservative. Merl checks the index, untracked files, commits not reachable from a preserved ref, and publication state before removing or reusing a workspace. Dirty or unpreserved work blocks automatic cleanup. Completing or closing an agent session does not override those checks.

Each workspace has separate storage classes:

```text
scratch/<workspace>/tmp/        disposable process temporary files
build/<workspace>/              reconstructible build and test output
tool-cache/<tool>/              bounded cache, shared only when safe
artifacts/                      durable referenced outputs
```

`MERL_SCRATCH` selects the scratch root and defaults to `${MERL_HOME}/scratch`. The host adapter sets `TMPDIR` for the agent process. Tool adapters may also set `XDG_CACHE_HOME`, Python bytecode paths, pytest's base temporary directory, Rust's target directory, or comparable tool settings. Build output stays workspace-local unless an adapter guarantees safe concurrent sharing.

Merl inspects the scratch filesystem and reports memory-backed or swap-sensitive mounts. A project can forbid them. Node high and low watermarks, per-workspace allowances, and spawn-time reservations keep one agent from consuming the machine. Portable filesystems may provide accounting without hard enforcement; adapters for filesystem quotas or containers can add hard limits.

The janitor owns only paths created beneath a configured Merl storage root and carrying the expected ownership marker. It waits for the workspace lease and tracked processes to end, then removes scratch and trims reconstructible caches. It never sweeps the system `/tmp`, a user checkout, durable artifacts, or unknown paths.

An agent started outside the Merl host adapter may bypass environment routing and resource enforcement. Node status reports that gap. Merl can audit known workspace and scratch paths, but it does not claim control over unowned process output.

Workspace separation prevents local filesystem corruption, not semantic merge conflicts. Tasks may advertise affected paths, components, or contracts. Assignment policy can warn about overlapping active work or serialize it, while Git and pull-request integration decide how changes merge.

## Cross-project coordination

Projects act as bounded contexts. Each project owns its accepted history and local interpretation of an exchange. Cross-project coordination synchronizes related state without creating shared mutable objects.

```mermaid
flowchart LR
    subgraph A[Origin project]
        OR[OutboundRequest X17]
        OA[Origin accepted transition]
        OE[CrossProjectEnvelope]
        OA --> OR
        OA --> OE
    end

    OE --> T[Asynchronous authenticated transport]

    subgraph B[Target project]
        ER[Durable envelope receipt]
        IP[Target policy]
        IR[InboundRequest XR91]
        TW[Target-owned work T91]
        ER --> IP --> IR --> TW
    end

    T --> ER
    TW --> UE[Lifecycle update envelope]
    UE --> OR
```

### Separate request aggregates

The origin owns an `OutboundRequest` that states what it needs, why it needs it, and which constraints or exports support the request. The target imports the exchange as a separate `InboundRequest`. The objects share correlation but never share mutable fields.

The origin controls its need, `needed_by` constraint, impact statement, exported context, amendments, and withdrawal requests. The target controls commitment, scheduling, local priority, implementation, ownership, and fulfillment. A withdrawal request does not cancel target work until the target accepts it. A target fulfillment claim does not force the origin to accept that its need has been met.

The end-to-end view is a projection over both aggregates. Request exchange may be submitted, withdrawn, or disputed. The target reports its commitment, scheduling, and execution facets separately. A target can defer a pending request without accepting it, or accept responsibility and schedule it for later. Each transition belongs to the project that made it; the other project records an observation of remote state.

### Directional links and contracts

A `ProjectLink` is directional and defaults to no access. It grants specific request kinds, contract versions, and export selectors from one project to another. Reverse traffic requires another grant.

Cross-project contracts type the coordination act and its durable consequence without encoding the sender's full reasoning. A request contains a versioned kind, readable summary, small typed payload, constraints, and references:

```yaml
kind: infrastructure.change
version: 1
summary: provision storage for capture data
constraints:
  required_before: E42
  needed_by: 2026-10-01
  impact_if_late: session two capture is blocked
refs:
  - snapshot: atlas:D18@4
  - live: atlas:T42
```

The target advertises supported kinds and versions. It rejects an unknown version with a durable compatibility outcome rather than guessing at its meaning.

Cross-project references have three forms:

- `LiveRef` resolves current state exposed by the origin project.
- `PinnedRef` resolves one exported object revision.
- `SnapshotExport` carries an immutable projection and digest.

Requests usually use pinned references or snapshots for the information that justified submission. Live references provide later updates. Object IDs do not grant ambient read access; exports and link policy define what the target may resolve.

### Atomic outbound intent

An accepted outbound transition and its `CrossProjectEnvelope` commit in the same origin-project transaction:

```text
BEGIN
  append origin DomainEventBatch
  update origin materialized state
  advance origin project revision
  insert origin InboxEntries
  insert pending CrossProjectEnvelopes
COMMIT
```

A transport worker delivers pending envelopes after commit. A process crash cannot leave accepted state claiming submission without a durable delivery intent.

The envelope contains a stable envelope ID, origin and target projects, origin revision and transition ID, correlation ID, causation ID, contract kind and version, payload hash, references, and hop count. The human-readable summary must remain understandable without Merl IDs.

### Durable receipt and target policy

The target authenticates the origin project and checks its directional link before acknowledging transport. It persists the immutable envelope receipt idempotently, then acknowledges possession. A transport acknowledgement means the target has the envelope; it does not mean the target accepted the request.

Target policy processes the receipt and may create an `InboundRequest` in a later target-project transaction. That transaction can also create target-owned tasks, questions, inbox entries, and an outbound lifecycle update. No transaction spans two projects or an external provider.

The same protocol applies when one process hosts both projects. The implementation cannot bypass the boundary through direct database mutation.

### Idempotency and lineage

Merl handles duplicates at distinct layers:

- Repeated envelope ID means transport retry and returns the stored receipt.
- Repeated origin transition ID means the transition was already imported.
- Similar request content is a policy question and may represent legitimate new work.

Correlation and causation IDs retain the exchange lineage through requests, tasks, GitHub issues, and updates. Repeated project identity in a lineage is valid. A path such as Atlas to Core to Platform and back to Atlas may carry a needed question and answer.

Merl detects duplicate work from the same correlation, request kind, semantic transition, and processed origin transition. It may flag suspected semantic recursion for policy. A configurable maximum hop count provides a safety valve without treating every graph cycle as an error.

Derived external artifacts retain their request lineage. If the origin later ingests a target-created GitHub issue, it updates the existing exchange rather than inferring another request from that issue.

## Revisions and concurrency

Each project has a monotonically increasing `project_revision`. Each materialized object also has an `object_revision`. The project revision orders accepted changes and drives cursors; it does not make every prior evaluation stale.

A policy evaluation records its basis revision, read dependencies, and proposed write set. Read dependencies may name an object or relation revision, a collection version, or a predicate guard such as "no active decision exists for this key." Merl commits an accepted batch as follows:

```mermaid
sequenceDiagram
    participant C as Client or ingestion worker
    participant A as Project authority
    participant DB as Transactional store
    participant W as Wake adapter

    C->>A: Submit change based on revision N
    A->>A: Compile and evaluate policy
    A->>DB: Begin serialized transaction
    A->>DB: Validate read dependencies and write set
    alt Dependencies remain valid
        A->>DB: Append DomainEvents
        A->>DB: Update objects and relations
        A->>DB: Advance current revision by one
        A->>DB: Insert InboxEntries and outbound envelopes
        A->>DB: Commit
        A-->>C: New revision committed
        A->>W: Attempt wake-up
    else Dependency or write conflict
        A->>DB: Roll back
        A-->>C: Reevaluate or report conflict
    end
```

An unrelated project change does not invalidate an evaluation when every recorded dependency still holds and the write set does not conflict. A changed dependency forces policy reevaluation or a visible command conflict. Compilation output remains immutable and need not run again merely because policy reevaluates it.

Object IDs alone cannot describe every dependency. A policy that depends on the absence of an object records a predicate or collection guard so a concurrent insertion invalidates the evaluation. The authority serializes the final transaction after these checks.

An accepted batch commits in full or not at all. Domain events never become visible without their materialized state, revision, and inbox entries.

## Inbox and wake-up behavior

Inbox delivery follows the transactional outbox pattern. Subscription matching runs before commit and creates one durable `InboxEntry` for each affected agent. An entry contains the agent ID, committed project revision, relevant object references, delivery state, and acknowledgement state. Large source text does not belong in the entry.

After commit, the daemon asks the configured host adapter to wake the agent. Host adapters may use Codex App Server, a Claude integration, a process signal, or another supported mechanism. Wake-up failure leaves the inbox entry pending. A background dispatcher retries with bounded backoff.

Delivery is at least once. Agents process inbox entry IDs idempotently and acknowledge them after they have persisted their cursor. Duplicate wake-ups are harmless. A missed wake-up delays work but cannot hide a committed change.

Subscriptions select semantic changes, not message senders. A reviewer may subscribe to findings and contract changes for a pull request. A researcher may subscribe to hypotheses, experiment results, and research claims. Merl computes the delta from the agent's cursor when the agent reads its inbox.

## Human publication

GitHub and similar providers are sparse human collaboration surfaces. Merl keeps its domain-event log inside the project authority and publishes selected human-relevant changes.

Ingestion and publication use separate capabilities. Several projects may ingest one external object, but one configured project owns each publication slot. A slot covers an append-only comment stream or a managed status surface for a specific external object. For the first implementation that supports shared sources, a multiply-bound Issue has one publishing project; every other binding is ingest-only.

Publication starts only after an accepted batch commits:

```mermaid
flowchart TB
    DB[Committed DomainEventBatch] --> PD[PublicationDecision per target]
    PD --> N[none]
    PD --> S[status]
    PD --> C[comment]
    S --> MP[ManagedProjection]
    C --> CP[CommentPublication]
    MP --> PA[PublicationAttempt]
    CP --> PA
    PA --> EX[External human surface]
```

A publication worker reads committed batches from a durable cursor. For each affected external surface, it records an append-only `PublicationDecision` with the batch ID, project revision, target, policy version, outcome, and reason. One batch may yield decisions for several issues or pull requests. Evaluating the batch as a whole prevents one human event from becoming several comments.

Most decisions choose `none`. Publication policy reserves `comment` for human-significant events such as accepted decisions, changed requirements, serious blockers, answers to human questions, significant research conclusions, and merge-blocking findings. Routine facts, acknowledgements, cursor changes, handoffs, and minor derived transitions remain in Merl.

Publication classification records whether it used deterministic rules or a model. Model-backed decisions retain the classifier version, model identity, prompt hash, and input references. Classification never runs inside the accepted-state transaction.

### Comment publications

A `CommentPublication` is an append-only human-facing event. It may summarize several domain events from one accepted batch. Its stable publication ID, target, source object references, renderer version, project revision, content hash, and desired body support audit and idempotent delivery.

Generated comments lead with ordinary language. A Merl ID appears as a handle after the meaning, for example:

```text
Keep receive gain fixed during baseline captures. (D18)
```

Removing Merl IDs from a generated comment must leave its human meaning intact.

### Managed status projections

A `ManagedProjection` is a bounded materialized view for one external slot, such as `github:issue:204/merl-status`. It tracks desired revision and hash, published revision and hash, external object ID, and drift state.

Status renderers show only current phase, goal, blockers, questions that need human input, next work, recent decisions, and important artifacts. Configurable character and item limits provide deterministic bounds. Merl records estimated token count for observability while enforcing the character and item limits.

Status is replaceable and coalescible. If revisions 483 through 485 arrive before publication, Merl may publish only the desired state at revision 485. Publication decisions preserve the audit trail even when an intermediate rendering never reaches GitHub.

### Attempts, reconciliation, and drift

A `PublicationAttempt` records one external write or reconciliation attempt. It includes the desired publication identity, provider, target, authenticated publishing actor, attempted content hash, time, and result. Attempts are append-only and happen after accepted state commits. Provider failure cannot roll back a project revision.

Every external write has a stable idempotency identity. Where the provider permits it, Merl embeds a non-disruptive marker. Merl also verifies its own publication record, authenticated provider identity, target, object type, and content hash. If a request may have succeeded before the connection failed, Merl reconciles against the provider before retrying.

Merl publications carry a stable, cross-project publication ID and the publishing project ID. The provider actor, target, object type, and content hash support verification; a hidden marker alone is never trusted. Authorities that share a source can recognize authenticated Merl output even when another project created it.

When a verified publication returns through source ingestion, Merl captures the provider event and marks it as `origin=merl_projection`. It does not send that event through ordinary semantic compilation. Cross-project meaning travels through an explicit export or request rather than by compiling another project's generated prose.

If someone edits a managed projection, Merl records projection drift and warns or restores the desired view according to project policy. The edit never becomes accepted-state input.

### Renderer constraints

A renderer receives accepted objects at a recorded project revision. It may express those objects in readable prose, but it cannot introduce new project facts. Deterministic renderers are preferred. A model-backed renderer records its version, model identity, prompt hash, input references, and output hash.

## Views and progressive disclosure

A role view projects accepted objects into the smallest useful representation for that role. Views hide superseded objects by default but retain their references for expansion.

Merl exposes four levels of detail:

```mermaid
flowchart LR
    N[Notification] --> D[Role delta]
    D --> O[Object detail]
    O --> S[Captured source]
```

Renderers must be deterministic for a fixed state and configuration. Merl provides compact text and JSON. YAML may help humans inspect state. A symbolic codec must earn its complexity through measured task performance.

Token metrics include the initial view, expansions, source reads, clarification turns, compiler work, and corrections. Merl does not claim a saving when a small but ambiguous view moves cost into later calls.

## Runtime and process model

Exactly one authority serializes accepted mutations for a project. The authority runs ingestion, policy evaluation, projection updates, inbox dispatch, and API handling. Clients submit commands through it rather than writing the accepted store.

In local mode, one daemon and SQLite database under `MERL_HOME` form the authority. The CLI may enter an embedded mode when the daemon is absent, but it must first acquire the same exclusive ownership lock. Read-only diagnostic tools may open SQLite in read-only mode when they can tolerate a point-in-time view.

In shared mode, a reachable Merl service forms the authority. Each client may keep a local cache and a durable queue of pending commands. The cache may be incomplete or stale. Its cursor states the latest project revision it has observed; it never competes with the authority as an accepted state store.

Moving a project between authorities requires an exclusive handoff. Merl freezes writes, exports and verifies the event history, initializes the new authority with the same project ID and revision sequence, records a new authority generation, redirects clients, then resumes writes. The generation prevents the old authority from accepting mutations after the handoff.

Moving a user's setup is separate from moving project authority. A node export contains portable configuration, personal agent guidance, and local operational records but no accepted history for shared projects. Project-scoped guidance and subscriptions remain at their authority. Import creates a new `Node` identity, restores logical agent identities and pending command IDs, resolves authority references, and reports missing local prerequisites.

An authority recovers by inspecting durable state rather than trusting memory. On startup it resumes incomplete source ingestion, retries pending inbox deliveries, expires overdue leases, and verifies projection metadata. Store transactions handle process crashes during accepted mutation commits.

Context actions use the same durable-side-effect pattern as wake-ups. Merl commits session closure first, then asks the host adapter to clear or replace context. Adapters advertise whether they support automatic reset. An unsupported or failed reset remains visible to the operator and can fall back to a manual instruction.

Provisioning follows the same rule. Merl commits the spawn request and resource reservation before calling the host adapter. Ready, failed, drain, and stop results return as durable control transitions. Recovery retries ambiguous attempts by their stable provisioning identity instead of starting another agent blindly.

## Rust workspace

The workspace separates business rules from adapters:

```text
crates/
  merl-core       IDs, events, objects, relations, policies, transitions
  merl-store      persistence interfaces, SQLite, migrations, replay
  merl-compiler   compiler protocol and built-in deterministic extraction
  merl-daemon     project authority, ingestion, subscriptions, dispatch
  merl-client     authority client, local cache, pending commands
  merl-cli        command-line interface
  merl-github     GitHub capture and incremental polling
  merl-publish    publication policy, rendering, and delivery records
  merl-federation cross-project contracts, envelopes, and delivery
  merl-host       spawn, wake, context, process, and resource adapters
  merl-mcp        thin MCP adapter
```

`merl-core` has no dependency on SQLite, GitHub, MCP, CLI parsing, or an async runtime. It accepts explicit clocks and ID providers where behavior depends on them. Domain transitions remain testable without the network or filesystem.

Adapters translate external types at their boundaries. Public core APIs do not expose types from GitHub, SQLite, or MCP libraries. The workspace follows Microsoft's [Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/guidelines/index.html) in spirit, including strong types, small crates, structured telemetry, mockable I/O, and documented error behavior.

The project will not add a custom allocator or raise the release CPU baseline without measurements. Curl-installed binaries must remain portable across their declared target.

## Storage and configuration

`MERL_HOME` defaults to `~/.merl`. It is the home of one Merl node, not the universal home of a project's accepted state. Its logical layout is:

```text
~/.merl/
  config/
  credentials/
  git/            managed Git object stores
  projects/       local authoritative stores
  cache/          shared-project caches
  pending/        commands awaiting submission
  scratch/        disposable agent temporary files
  build/          reconstructible workspace output
  tool-cache/     bounded caches for concurrency-safe tools
  artifacts/
  logs/
  run/
  workspaces/
```

The exact filenames are a schema decision. A local project's authoritative SQLite database may live under `projects/`. A shared project's accepted store lives at its authority; the local node keeps only a cache, cursor, and pending commands.

Merl creates private directories and files by default. Logs use structured fields and redact captured content, credentials, and model prompts unless a user enables a diagnostic mode.

### Local trust boundary

Local mode does not defend Merl from a process with unrestricted access as the same OS user. Such a process can read credential files, open SQLite databases, or modify local state outside the domain API. In this deployment, membership and capability checks provide governance, validation, and audit for cooperating clients. They are not an OS security boundary.

The host should avoid placing provider secrets in agent environments even when agents are trusted at the filesystem boundary. A deployment that must constrain local agents needs enforcement outside the initial local design: a separate daemon identity, restricted IPC, host sandbox rules, containers, filesystem permissions, or a credential broker. Shared authorities enforce their API boundary against remote clients, but they cannot repair an unrestricted client host.

A default node manifest contains no credential values, provider tokens, caches, logs, captured source bodies, or workspace contents. It may still contain private names, agent guidance, project addresses, and queued command payloads, so Merl writes it with private permissions and labels it sensitive. Credential references remain unresolved after import until configured through the relevant adapter.

Workspace export records recipes: source identity, desired revision or branch, agent ownership, and clone or worktree strategy. Before export, Merl reports dirty and unpushed work that the recipe cannot reproduce. Import never claims those bytes were preserved.

A local-project export is a different artifact. It contains the canonical event history, accepted projections or rebuild metadata, required source captures, control history, pending outboxes, and an integrity manifest. Import verifies the archive and requires an exclusive authority takeover before accepting writes. Node export cannot silently duplicate a local authority.

Stable project and agent IDs do not depend on local paths. Workspace records map assignments to current directories and filesystem identities. Merl can therefore reject shared writable checkouts, create managed worktrees, and retain uncertain work for inspection.

Merl discovers provider-backed repositories through Git remotes and immutable provider IDs, then looks up bindings at the project authority. Local configuration attaches sources that lack a provider identity. Merl requires no committed marker and never commits generated accepted state into a code repository.

Merl sends no telemetry by default. Export and diagnostic commands require explicit user action.

## External interfaces

### CLI

The `merl` CLI is the reference interface for humans, skills, CI, and evaluation. Commands expose semantic operations such as viewing an issue, reading a delta, resolving a question, or recording an experiment result. Mutating commands go through the project authority. Machine output has an explicit format and version.

### MCP

The MCP server remains a thin adapter. It exposes a small surface around state views, mutation batches, expansion, artifact reads, and inbox operations. MCP handlers call the same application services as the CLI. Protocol concerns do not enter the domain core.

### Compiler protocol

External compilers receive an immutable rendered `CompilationContext` and return assertions in a versioned envelope. They cannot access the database or retrieve extra history directly. Merl records the context manifest and configuration needed to compare runs, while secrets remain outside persisted provenance.

### GitHub

The GitHub adapter records immutable provider repository and actor IDs along with current names, upstream object IDs, update timestamps, content hashes, and ingestion cursors. Polling and webhooks feed the same ingestion path; they do not create separate event models. Provider mirrors retain GitHub-owned facts separately from Merl semantic overlays. Outbound comments and managed status updates use explicit publication-slot ownership and the retry rules above.

### Cross-project protocol

The cross-project protocol transports immutable, versioned envelopes between authorities. An authority advertises supported request contracts, authenticates linked projects, persists receipts before acknowledgement, and exposes delivery status without implying request acceptance. A loopback transport uses the same protocol when both projects share one process.

## Testing and evaluation

The architecture depends on replay and transaction boundaries, so tests must exercise both.

- Unit tests cover domain transitions, authority rules, rendering, and subscription matching.
- Property tests check monotonic revisions, idempotent ingestion and commands, and legal supersession.
- Store tests inject failures around every step of the accepted-batch transaction.
- Context tests reconstruct exact compiler input, enforce selection budgets, request expansion for unresolved references, and refuse silent full-history fallback.
- Provenance tests cover assertions, direct commands, receipts, administrative actions, and source content made unavailable by purge.
- Concurrency tests change unrelated objects between evaluation and commit, then change object and predicate dependencies to verify the different outcomes.
- Publication tests inject ambiguous provider failures, duplicate callbacks, and managed-comment edits.
- Work-planning tests distinguish pending from accepted responsibility, deferred from scheduled work, and requester constraints from owner commitments.
- Agent-continuity tests clear process context and verify that active guidance, checkpoints, and current project state still shape resume output.
- Context-policy tests verify task-scoped reset ordering, continuous-session retention, host capability fallback, and clean context generations between unrelated tasks.
- Assignment tests compare agents with different model, effort, capability, availability, and cost advertisements and verify every ranking explanation.
- Provisioning tests cover delegation limits, duplicate spawn requests, resource reservation, ambiguous host failure, readiness, drain, and stop.
- Workspace tests cover concurrent writers, enforced read-only sharing, path aliases, unique branches, dirty cleanup refusal, and full-clone fallback.
- Storage tests route agent temporary files, enforce configured watermarks, inject cleanup crashes, and prove that the janitor cannot leave owned roots or touch protected data.
- Portability tests round-trip node manifests, reject secret material, preserve logical agent and command IDs, create a new node ID, and report unreproducible workspace state.
- Cross-project tests cover duplicate envelopes, crash points around the origin outbox and target receipt, contract-version mismatch, directional authorization, deferral without acceptance, and legitimate correlation loops.
- Replay tests rebuild projections from domain events and compare them with the live database.
- Golden tests compare role views and deltas for fixed project revisions.
- Corpus evaluations keep development, held-out, and adversarial threads separate and compare compiler output with independently reviewed state.
- Context-policy benchmarks compare correctness and total tokens per completed task across continuous and task-scoped agents handling unrelated work.

Evaluation reports total induced token cost rather than wire size alone. It also reports task correctness, missed blockers, stale-state errors, provenance accuracy, expansion frequency, clarification turns, human-label disagreement, and latency. Break-even curves show when compilation pays for itself across repeated reads by several roles.

## Architectural invariants

The implementation must preserve these rules:

- A `Project` is the unit of accepted history, revision ordering, policy, and membership.
- Projects and sources relate many-to-many through `ProjectSourceBinding` records.
- Source selection, capabilities, and credentials remain separate concerns.
- Exactly one authority serializes accepted mutations for a project.
- Only the project authority creates accepted `DomainEvent` records.
- Offline clients queue idempotent `Command` records rather than domain events.
- Command payloads are immutable, and command outcomes are append-only.
- The authority authenticates command actors and enforces project membership.
- A `Repository` is a project source, not the project boundary.
- Provider IDs establish external identity; mutable names are display attributes.
- Provider-owned Issue and pull request facts remain separate from Merl semantic overlays.
- Merl does not report a provider-owned fact as changed until the provider reports it.
- `MERL_HOME` contains local node state and is authoritative only for local-mode projects.
- Accepted project state does not live in a code repository.
- Provider-backed source discovery does not require a committed Merl marker.
- Work commitment, scheduling, and execution are independent accepted facts.
- Deferral records why and when or under what condition the work will be reconsidered; it never implies acceptance or a delivery date.
- Requester need dates and owner target dates remain distinct.
- Logical agent identity does not depend on process, model context, workspace path, or node identity.
- Clearing process context never deletes durable guidance, checkpoints, or accepted project state.
- Agent guidance is scoped, attributable, lifecycle-managed, and subordinate to project policy.
- Logical agents and disposable model-context generations are separate identities.
- Context clears occur only after durable checkpoint, cursor, task-reference, assignment, and lease transitions commit.
- A failed or unsupported host reset cannot masquerade as a cleared context.
- Task-scoped reset removes prior transcript context but preserves active guidance and current project references.
- Model, effort, availability, and cost describe an agent session rather than its logical identity.
- Runtime advertisements retain field provenance and observation time; stale sessions are ineligible for new assignments.
- Runtime advertisements never grant project membership, provider capability, or source access.
- Candidate ranking excludes agents that miss hard task requirements and explains ordering among eligible agents.
- An authorized assignment records the task requirements and session advertisement used to make the choice.
- PM agent creation stays within a human-approved template, delegation, concurrency limit, cost limit, and credential scope.
- An agent is unavailable until its provisioning attempt produces a durable ready session.
- Duplicate or ambiguous provisioning attempts cannot start two agents for one accepted spawn request.
- Concurrent writable assignments never share a working tree, index, or checked-out branch.
- Sharing a Git object store does not imply sharing workspace ownership.
- Read-only workspace sharing requires an immutable checkout and enforcement by the host or filesystem adapter.
- Workspace cleanup never discards dirty files, untracked files, or commits without a preserved reference.
- Agent scratch defaults to an owned path outside the system `/tmp` and is configurable through `MERL_SCRATCH`.
- Merl reserves storage before provisioning and refuses the spawn when it cannot honor the allowance.
- Cleanup acts only on owned scratch, build output, and reconstructible caches after leases and processes end.
- Memory-backed scratch is visible and may be forbidden by policy.
- Default node exports contain no credential values, caches, logs, captured source bodies, or workspace contents.
- Node import creates a new node identity while preserving logical agent and pending command IDs.
- Node export never duplicates a local project's authority; canonical local state moves through verified exclusive takeover.
- Cross-project requests use separate origin and target aggregates.
- A project never mutates another project's accepted state directly.
- Project links are directional, capability-scoped, and default-deny.
- Accepted outbound transitions and their envelopes commit atomically.
- Targets persist authenticated envelope receipts before acknowledging delivery.
- Transport acknowledgement never implies policy acceptance.
- Cross-project delivery is asynchronous, idempotent, and versioned.
- Cross-project references are live, revision-pinned, or immutable snapshots.
- No transaction spans projects or external providers.
- Correlation lineage prevents duplicate work without rejecting legitimate graph cycles.
- `SourceEvent`, `CompilationContext`, `CompilationRun`, `ObservedAssertion`, `PolicyEvaluation`, and `DomainEvent` envelopes are append-only.
- Protected source and derived payload bytes remain erasable through audited administrative purge.
- Purge removes retained bytes or keys, preserves a tombstone and digest, and exposes the resulting replay limitation.
- Every compilation run references a bounded context manifest with exact source, state, selector, renderer, and input-digest provenance.
- Compilers request expansion or emit ambiguity instead of silently loading full history or inventing missing context.
- Recompilation creates a new run and never rewrites old assertions.
- Replay and evaluation runs cannot change accepted state without promotion.
- Corrections create new domain events.
- Materialized accepted state is reconstructible from accepted domain events.
- Each accepted domain-event batch advances one project revision exactly once.
- Objects carry independent revisions for update checks and future concurrency refinement.
- Policy evaluations accept typed derivation inputs; direct commands do not require fabricated assertions or source events.
- Every accepted object expands to its policy evaluation and derivation inputs.
- Policy evaluations record read dependencies, predicate or collection guards where needed, and proposed write sets.
- A project revision orders accepted changes but does not invalidate an evaluation whose recorded dependencies remain valid.
- Every derived record retains compiler, policy, actor, and source provenance where applicable.
- External ingestion is idempotent.
- Domain events, projections, the project revision, and inbox entries commit atomically.
- Wake-up occurs only after commit.
- Inbox delivery is at least once, and consumers act idempotently.
- Publication evaluation and external writes occur only after accepted state commits.
- Each external publication slot has one owning project even when several projects ingest the same source.
- Cross-project publication recognition uses verified provider provenance and stable publication identity, not a hidden marker alone.
- Publication decisions evaluate domain-event batches and are scoped to an external target.
- Most accepted batches produce no external publication.
- Comment publications are append-only human-significant events.
- Managed status projections are bounded, replaceable, and coalescible.
- Publication attempts are append-only and idempotent.
- External publication failure cannot roll back accepted state.
- Generated human prose remains meaningful without Merl IDs.
- Publication renderers cannot introduce facts absent from their accepted-state inputs.
- Known Merl publications and edits to managed projections never become semantic inputs.
- Projection drift is observable.
- Control-plane records reference knowledge-plane objects instead of copying them.
- Compilers and transport adapters cannot bypass policy to mutate accepted state.
- Same-user local processes with unrestricted filesystem access are trusted; Merl's local policy API is not a sandbox boundary.

## Alternatives rejected

Merl will not use GitHub prose as the agent-facing state store. That forces every agent to infer current state from the whole history and makes role-specific deltas impractical.

Merl will not maintain a mutable summary as its source of record. Summaries lose provenance and accumulate drift after repeated edits.

Merl will not allow compilers to write current state directly. Extraction quality and authority are separate concerns and need separate audit trails.

Merl will not compile every new comment in isolation or reload the full thread by default. Recorded bounded context gives the compiler enough local meaning without recreating the original token cost.

Merl will not commit generated project state or organizational bindings into code repositories. Git branches cannot provide the transaction, authority, privacy, or cross-repository semantics the accepted state requires. Provider identity and local configuration make a committed marker unnecessary.

Merl will not merge independently accepted histories from multiple peers. Many decisions, ownership changes, and supersession operations do not commute. One project authority provides a clear serialization point.

Merl will not claim that same-user local permissions contain a malicious agent. That guarantee requires enforcement from the operating system or agent host.

Merl will not implement cross-project work as direct mutation or shared request objects. Each side keeps an authoritative local aggregate and exchanges durable lifecycle transitions.

Merl will not use a distributed transaction across projects, GitHub, or another provider. Transactional outboxes, idempotent receipts, and reconciliation provide recovery without coupling authorities.

Merl will not publish every accepted transition to GitHub. That would recreate the transcript and token problems that the state model removes. Publication policy keeps routine machine state inside Merl.

Merl will not start with MCP as its domain interface. A CLI is easier to replay, test, inspect, and benchmark. MCP will expose proven operations from the same core.

Merl will not infer competence or authority from a model name. Runtime advertisements support planning; measured outcomes, project policy, and authorized judgment govern assignment.

Merl will not introduce Kafka or a distributed event framework to implement local mode. SQLite supplies its transaction boundary. A shared authority may use another transactional store without changing the event model.

Merl will not invent a compact symbolic language before ordinary compact views have a measured baseline. A shorter encoding that causes mistakes or extra retrieval costs is a regression.

## Decisions left for the schema pass

This architecture does not yet choose:

- the concrete ID representation
- SQL table and index definitions
- the SQLite access library
- the shared-authority storage backend
- serialized payload formats and versioning details
- the local IPC transport
- the remote authority protocol
- the authority-policy configuration language
- the compilation-context selection algorithm and default budgets
- protected-payload encryption and key management
- the authentication and membership implementation
- the cross-project transport and contract encoding
- the publication-policy configuration language
- the exact compact text grammar

Those choices must preserve the invariants above. The schema and vertical slice will expose whether any need their own architecture decision record.
