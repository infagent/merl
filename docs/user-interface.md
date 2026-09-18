# User interface and behavior

Status: draft behavior contract

This document describes how people and agents use Merl. It defines observable behavior before command parsing, storage, networking, or database design. The examples use a CLI because it is concrete and easy to test. MCP tools, agent skills, and future graphical interfaces should expose the same actions and outcomes.

The command names are the first proposed interface. We may refine spelling during implementation, but the behavioral distinctions in this document are requirements.

## Interface principles

Merl's interface follows these rules:

- The CLI is the reference interface.
- Every mutation reports whether it was accepted, queued, held for review, rejected, or conflicted.
- A local queue never masquerades as accepted project state.
- Project context is explicit when a repository belongs to several projects.
- Short IDs are navigation handles. Human prose carries the meaning.
- Detail is pulled by reference rather than pushed into every view.
- High-impact operations show their intended effect before confirmation.
- Authentication establishes the actor. A command-line flag cannot impersonate another actor.
- Human and machine output expose the same facts.
- GitHub remains sparse and understandable without Merl.

## User-facing surfaces

| Surface | Primary user | Purpose |
| --- | --- | --- |
| `merl` CLI | Humans, agents, CI | Reference interface for every semantic action |
| GitHub | Humans | Discussion, managed status, and significant project events |
| MCP | Agents and tool clients | Small semantic adapter over the same application actions |
| Agent skill | Coding and research agents | Usage guidance, context selection, and compact defaults |
| Daemon status | Operators | Authority health, synchronization, queues, and delivery failures |

Merl has no required graphical interface. A graphical client can arrive later without defining another domain API.

## Command shape

Commands use nouns and semantic actions:

```text
merl <noun> <action> [arguments]
```

Examples:

```bash
merl project view
merl issue view github:acme/atlas#204
merl question resolve Q32 --decision D18
merl experiment record --hypothesis H4 --artifact capture:S1
merl request create --to core --kind infrastructure.change
merl inbox list
```

Common options apply across commands:

```text
--project <project>     select project context
--format human|json    choose presentation
--at <revision>        read accepted state at a project revision
--dry-run              evaluate without committing
--require-accepted     fail unless the authority accepts the mutation now
--yes                  confirm prompts the actor is allowed to confirm
--no-input             disable interactive prompts
```

Machine output has a versioned schema. Human output favors readable names and adds stable IDs afterward.

With `--format json`, every command returns one result envelope. The envelope identifies the schema, action, outcome, project, accepted revision if one exists, affected objects, warnings, and error details. For example:

```json
{
  "schema": "merl.result/v1",
  "action": "question.resolve",
  "outcome": "accepted",
  "project": "atlas",
  "revision": 482,
  "objects": ["Q32", "D18"],
  "warnings": []
}
```

CLI JSON, MCP results, and agent-skill wrappers preserve these fields. An adapter may add transport metadata, but it cannot rename an accepted outcome or conceal a queued one.

## Project context

Merl resolves project context in this order:

1. `--project`
2. `MERL_PROJECT`
3. a node-local context selected for the current workspace
4. the only project-source binding that matches the current repository

If several bindings remain, an interactive terminal asks the user to choose. A non-interactive command fails with `project_context_ambiguous` and lists the candidates. Merl never guesses based on a recently used project.

Users can set a local workspace preference without modifying the repository:

```bash
merl context set atlas
merl context show
merl context clear
```

The preference lives under `MERL_HOME`. It does not create a committed marker.

## Node and project setup

`merl init` initializes the local node:

```bash
merl init
```

It creates private local directories, starts no network service, and does not create a project.

A user creates a local project and attaches the current repository:

```bash
merl project create atlas --local
merl source attach . --project atlas
```

For a provider-backed repository, Merl resolves the Git remote through the provider and stores the immutable provider repository ID. For a local source without a provider identity, Merl creates a node-local source identity.

The same source can attach to another project:

```bash
merl source attach . --project core \
  --select 'label=core' \
  --capability ingest \
  --capability compile
```

Selection and capabilities remain separate. Adding `--select 'label=core'` cannot grant publication or issue-creation authority.

Useful inspection commands include:

```bash
merl project list
merl project status
merl source list
merl source show github:acme/atlas
merl source bindings github:acme/atlas
```

`merl project status` makes authority and freshness visible:

```text
Project: atlas (P17)
Authority: local, reachable
Accepted revision: 482
Local cache: current
Pending commands: 0
Pending deliveries: 1
Sources: 3
```

For a shared project with an unavailable authority, it says `offline` and shows the cache revision. It does not describe the cache as current accepted state.

Local status also states the enforcement boundary:

```text
Local security: trusted same-user processes
Merl policy protects cooperating clients; unrestricted local processes can read node files.
```

`merl security explain` prints the files and credentials at risk, the host controls in effect, and the isolation options available to the operator. Merl never describes same-user local policy as a sandbox.

## Moving setup between computers

A node manifest recreates non-secret local setup:

```bash
# Old computer
merl node export --output merl-node.bundle

# New computer: inspect, then apply
merl node import merl-node.bundle --dry-run
merl node import merl-node.bundle
merl workspace materialize --all
```

The dry run reports what Merl can restore and what needs attention:

```text
Agents: 4 logical identities
Project contexts: 2
Workspace recipes: 3
Pending commands: 1
New node identity: will be created

Missing credential references:
- github-work

Workspace warnings:
- atlas-dev had dirty or unpushed work; files were not exported
```

The default bundle includes agent profiles, personal guidance, project and authority references, node-local delivery preferences, workspace recipes, local context choices, adapter declarations, and pending commands with their IDs.

Project-scoped guidance and subscriptions resynchronize from their authority. The bundle excludes credential values, caches, logs, captured source bodies, and workspace contents. It can contain private metadata and queued payloads, so Merl writes it with private permissions.

Import preserves logical agent IDs but creates a new node ID. It does not require login. Missing credential references disable the affected provider operations until the user configures them through that adapter.

Shared projects synchronize from their authority. Moving a local project authority uses a separate archive:

```bash
# Old computer: create the archive and freeze this authority generation
merl project export atlas --for-transfer --output atlas.merl-project

# New computer
merl project import atlas.merl-project --take-authority
```

Import verifies the archive and advances the authority generation before enabling writes. A node manifest never copies a live local authority or claims to preserve dirty Git files.

## Reading project state

The main read commands are:

```bash
merl project view [--role researcher]
merl project delta --since 481 [--role engineer]
merl issue view github:acme/atlas#204 [--role researcher]
merl pr view github:acme/atlas/pull/229 [--role reviewer]
merl show D18
merl show D18 --history
merl show D18 --source
merl artifact read A81 [--section protocol]
```

Views show current accepted state by default. Superseded objects stay hidden unless the user asks for history. Every compact object can expand to detail and captured evidence.

For provider-backed work, views label the source of each field. GitHub-owned facts such as open or closed state, labels, assignees, timestamps, and merge state appear separately from Merl-owned requirements, blockers, decisions, and findings. A pending provider action appears as a request until GitHub confirms it.

Example compact output:

```text
atlas@482  phase=session2

Keep receive gain and LO fixed for baseline captures. (D18)
Open question: choose storage location for session two. (Q41)
Blocked: session two capture waits on core request X17. (B9)
```

JSON output includes the same meanings, IDs, revision, provenance links, and expansion references. It does not include decorative human prose that changes the semantics.

### Inspecting compiler context

Compiler diagnostics show the bounded input used for one run:

```bash
merl compilation show CR42
merl compilation context CR42
merl compilation context CR42 --rendered
```

```text
Run: CR42
Trigger: SE771
Basis: atlas@482
Objects: D18@4 Q32@2 E37@5
Recent source: SE768 SE769 SE771
Selector: issue-context/v1, budget 6000 chars
Renderer: issue-compiler-context/v1
Input digest: sha256:...
```

The compiler can return `context_required` with a relation, object, or source range it needs. Policy limits the expansion and records a new context. Merl reports unresolved ambiguity when it cannot satisfy the request; it does not load the full thread without recording that choice.

## Changing project state

Mutations use domain language:

```bash
merl decision create --summary 'Keep receive gain fixed' --scope baseline
merl decision supersede D11 --with D18
merl question resolve Q32 --decision D18
merl finding open --pr github:acme/atlas/pull/229 --severity high
merl finding resolve F22 --commit a83c1
merl hypothesis create --statement 'DC impairment dominates baseline errors'
merl experiment record --hypothesis H4 --artifact capture:S1
merl claim create --supports H4 --evidence E37
merl task block T42 --on B9
```

The result names the authority outcome:

```text
ACCEPTED  D18 at atlas@482
```

```text
CANDIDATE C81 awaiting research-owner approval
```

```text
QUEUED CMD91 against atlas@481; authority is offline
```

```text
CONFLICT CMD91 dependency D18 changed from object revision 4 to 5
```

```text
REJECTED actor lacks decision.supersede permission
```

`--dry-run` shows the typed policy inputs, policy outcome, affected objects, publications, inbox deliveries, and cross-project envelopes without committing them.

Direct commands are policy inputs in their own right. `merl show D18 --derivation` may lead to a command and policy evaluation without claiming that a compiler or source event created the decision.

`--require-accepted` is useful in CI and automation. It prevents local queuing and treats candidate, rejected, and conflicted outcomes as failures.

Confirmation never bypasses policy. `--yes` may confirm a local prompt, but it cannot grant permission, approve a merge, or convert a candidate into accepted state.

### Reviewing candidates

Authorized users can inspect interpretations that policy did not accept:

```bash
merl candidate list
merl candidate show C81
merl candidate accept C81
merl candidate reject C81 --reason 'Capture S1 does not measure this effect'
```

Accepting a candidate runs a new policy evaluation against current project state. It does not rewrite the old evaluation or force its proposed events through a stale basis revision.

### Purging retained source content

An authorized administrator can remove source bytes that Merl must no longer retain:

```bash
merl source purge SE771 \
  --reason 'Credential posted in comment' \
  --dry-run

merl source purge SE771 \
  --reason 'Credential posted in comment' \
  --confirm-digest sha256:...
```

The preview lists the payload, compilation runs, assertions, events, projections, and accepted objects whose evidence will become unavailable. It also identifies protected derived payloads that copied the sensitive material. Purge removes the authorized bytes or encryption keys, then records tombstones with their digests, actor, time, and reason. It does not delete record identities, rewrite structural history, or claim that exact replay still works.

After purge, `merl show D18 --source` returns `source_content_unavailable` with the tombstone reference. Derived state remains visible in redacted form unless a separate policy action invalidates it.

### Offline commands and synchronization

Queued commands remain visible and controllable:

```bash
merl command list --pending
merl command show CMD91
merl command cancel CMD91
merl sync
merl command resubmit CMD91 --basis current
```

A command can be cancelled only before the authority records receipt. Resubmission previews the effect against current state and creates a new command ID; it never changes the immutable queued command.

## Requesting and planning work

A request creates durable work without claiming that its owner accepted it:

```bash
merl task request \
  --team software \
  --summary 'Add clipping metadata to the capture reader' \
  --needed-by 2026-10-01 \
  --impact-if-late 'Session two analysis cannot begin'
```

The project manager controls commitment and scheduling:

```bash
merl task accept T44
merl task defer T44 \
  --reason 'Migration work has priority' \
  --review-at 2026-10-01

merl task schedule T44 \
  --target-start 2026-10-15 \
  --target-finish 2026-10-20
```

The compact view keeps the independent facts visible:

```text
Add clipping metadata to the capture reader. (T44)
Commitment: accepted by project manager
Scheduling: deferred; review 2026-10-01
Execution: not started
Requested need: 2026-10-01
Risk: current plan does not meet the requested need
Reason: migration work has priority
```

`needed_by` states the requester's constraint. It is not a delivery promise. `target_start` and `target_finish` are set by the work owner. Merl reports a conflict between them without silently changing the work owner's or agent's priority.

Deferred work remains in project and planning views but leaves the assigned agent's actionable queue. The agent receives the planning delta. Deferring a task that is already in progress fails with `task_in_progress`; the project manager must use an explicit pause operation so the agent can checkpoint and release its execution lease.

```bash
merl task pause T44 \
  --reason 'Migration work has priority' \
  --review-at 2026-10-01
```

## Inbox and agent continuity

Agents and people read durable inbox entries:

```bash
merl inbox list
merl inbox show IN123
merl inbox ack IN123
merl inbox watch
```

`inbox watch` talks to the Merl daemon. The invoking agent does not maintain a fragile shell watcher. If wake-up fails, the entry remains pending and appears in `merl project status`.

Before ending a session, an agent records a checkpoint:

```bash
merl session checkpoint \
  --agent atlas-tech-lead \
  --summary 'Session two planning is blocked on X17' \
  --ref T42 --ref X17 --ref D18
```

A later session resumes from accepted project state plus the checkpoint's references:

```bash
merl session resume --agent atlas-tech-lead
```

The checkpoint does not copy full decisions, tasks, or artifacts. `resume` expands current versions and reports anything that changed after the checkpoint revision.

Agent guidance survives process shutdown, restart, and model-context clearing:

```bash
merl agent practice add \
  --agent atlas-dev \
  --scope project:atlas \
  --statement 'Write a failing behavioral test before implementation'

merl agent lesson propose \
  --agent atlas-dev \
  --scope project:atlas \
  --statement 'Run capture compatibility tests before changing reader framing' \
  --evidence T44

merl agent guidance list --agent atlas-dev
merl agent guidance show AG18
merl agent guidance activate AG18
merl agent guidance retire AG18 --reason 'Superseded by project test policy'
```

Instructions record behavior required by an authorized human or project policy. Practices record adopted working methods. Lessons record observations proposed for reuse and may require approval. Each item has scope, provenance, priority, and lifecycle. Guidance changes how an agent works; it is not project knowledge and cannot override accepted requirements or safety policy.

`merl session resume --agent atlas-dev` assembles a bounded view from identity and role, relevant active guidance, actionable assignments, the latest checkpoint, changes since its cursor, and current referenced project state. Current state wins over stale checkpoint prose.

Context lifetime is configurable per agent:

```bash
merl agent context-policy set atlas-pm --mode continuous
merl agent context-policy set atlas-dev --mode task-scoped
merl agent context-policy set atlas-researcher --mode manual
merl agent context-policy show atlas-dev
```

With `task-scoped`, Merl closes the session after its bound task becomes completed, cancelled, deferred, or otherwise non-actionable. Before asking the host to clear context, it commits a final checkpoint, result references, cursor, proposed lessons, assignment state, and lease release.

```bash
merl session close \
  --agent atlas-dev \
  --task T44 \
  --outcome completed \
  --ref github:acme/atlas/pull/229 \
  --propose-lesson 'Run compatibility tests before changing reader framing'
```

The result separates durable closure from the host side effect:

```text
SESSION CLOSED atlas-dev generation 18
CHECKPOINT CP91 committed at atlas@512
CONTEXT CLEAR pending through host adapter
```

If the adapter cannot reset context, Merl reports `context_reset_unsupported` and gives the user a manual next step. It does not claim that generation 18 was cleared. Starting the next task creates generation 19 with active guidance and current state for that task, without generation 18's transcript.

An agent with `continuous` policy stays in the same generation after completing a task. It still receives compact deltas and bounded views. A manual clear uses the same safe closure path:

```bash
merl session close --agent atlas-pm --checkpoint --clear-context
```

### Advertising runtime and choosing an agent

The host adapter should publish runtime information when a session starts. A manually integrated host can do the same through the CLI:

```bash
merl agent advertise \
  --agent atlas-senior-dev \
  --model anthropic:opus \
  --effort high \
  --cost-class high \
  --capability rust \
  --capability architecture-review \
  --available

merl agent advertise \
  --agent atlas-dev \
  --model anthropic:sonnet \
  --effort medium \
  --cost-class medium \
  --capability rust \
  --available
```

`merl agent list --available` and `merl agent show` display the current session, model, effort, capabilities, cost class, source of each claim, and last observation time. Host-reported values are marked `host-attested`; CLI values are marked `operator-configured` or `self-reported` according to the authenticated actor.

The PM describes the work before asking for candidates:

```bash
merl task requirements set T52 \
  --capability rust \
  --reasoning high \
  --risk high \
  --review senior \
  --budget prefer-capability

merl task candidates T52
```

Example output:

```text
Eligible
1. atlas-senior-dev
   model=anthropic:opus effort=high cost=high
   matches: rust, high reasoning; extra: architecture-review

Ineligible
- atlas-dev
  model=anthropic:sonnet effort=medium cost=medium
  missing: high reasoning
```

For a routine task with medium reasoning demand and `--budget prefer-cost`, both agents may be eligible and the Sonnet-backed developer ranks first. Ranking is advisory and gives reasons. The PM or another authorized actor commits the choice:

```bash
merl task assign T52 --to atlas-senior-dev \
  --reason 'High-risk parser change needs reasoning headroom'
```

A runtime advertisement cannot grant access or satisfy a review requirement. Merl records the advertisement and task requirements used for the assignment so the team can compare the prediction with task outcome and token use.

### Spawning agents within delegated limits

A human defines the approved templates and delegates their use:

```bash
merl agent template create software-dev-medium \
  --role developer \
  --host codex \
  --model openai:codex \
  --effort medium \
  --cost-class medium \
  --context-policy task-scoped \
  --workspace worktree \
  --scratch-limit 10GiB \
  --max-concurrency 2 \
  --project atlas

merl delegation grant \
  --to atlas-pm \
  --template software-dev-medium \
  --max-active 2
```

The template refers to approved permissions and credential references; it never contains secret values. The PM can now staff a task without asking the human to start each process:

```bash
merl agent spawn \
  --template software-dev-medium \
  --task T53
```

```text
SPAWN ACCEPTED SP19
Agent: atlas-dev-4
Workspace reservation: 10 GiB
Provisioning: pending through host adapter
```

The agent appears in `merl agent list --available` only after the host reports it ready and Merl records its current advertisement. A duplicate spawn ID returns the existing result. An ambiguous host response is reconciled before retry, so one request cannot start two agents.

The PM can inspect, drain, or stop agents covered by its delegation:

```bash
merl agent provisioning show SP19
merl agent drain atlas-dev-4
merl agent stop atlas-dev-4
```

Draining prevents new assignments and lets active work checkpoint. Stopping releases workspace and storage reservations only after the host confirms process exit. Attempts to exceed concurrency, select another model, grant a permission, or use an unapproved credential fail with `delegation_exceeded`.

A human may attach a process started elsewhere:

```bash
merl agent attach --template software-dev-medium --task T53
```

The authority assigns or confirms the logical identity. It marks manually supplied runtime fields with their actual provenance.

Clearing context in an agent host does not call a Merl mutation. Guidance stops affecting future sessions only through an explicit retirement or supersession:

```bash
merl agent guidance retire AG18 --reason 'No longer applies'
```

Retirement preserves provenance and audit history. Erasing retained project records for privacy or compliance is a separate administrative concern.

### Direct coordination

Most collaboration changes project state. A request that may be accepted, prioritized, or delayed must be a task. A direct note may provide context, but it does not become schedulable work:

```bash
merl task request --team software \
  --summary 'Review parser memory safety' \
  --needed-by 2026-10-01

merl message send \
  --to atlas-reviewer \
  --summary 'Please look at the unsafe block in the parser' \
  --ref T45 \
  --ref github:acme/atlas/pull/229
```

The note becomes a durable inbox entry with delivery and acknowledgement state. It does not become accepted project knowledge. A PM defers `T45`, not the message. Cross-project work uses versioned requests and exports, not direct messages.

## Workspace safety

Merl treats agent workspaces as local operational state. Assignment can create one automatically:

```bash
merl task assign T52 --to atlas-dev --workspace auto

merl agent register atlas-reviewer --role reviewer
merl workspace check
merl workspace create \
  --agent atlas-dev \
  --task T52 \
  --source github:acme/atlas \
  --strategy worktree
merl workspace list
```

The default layout uses sibling worktrees under `MERL_HOME`:

```text
~/.merl/git/R1.git/
~/.merl/workspaces/atlas/T52-atlas-dev/
~/.merl/workspaces/atlas/T53-atlas-senior-dev/
```

The two worktrees share Git objects. They have different files, indexes, task branches, and current working directories. Merl does not create one agent's worktree inside another checkout.

`workspace check` canonicalizes paths and inspects Git worktree identity. It reports two active agents using the same writable checkout even when one path reaches it through a symbolic link. Commands that assign concurrent write work fail until Merl creates a separate workspace.

Read-only review uses a detached worktree at a pinned commit:

```bash
merl workspace create \
  --agent atlas-reviewer \
  --task T52 \
  --source github:acme/atlas \
  --at a83c1 \
  --read-only
```

Several readers may share that immutable checkout only when the host enforces read-only access. A reviewer never shares a writer's live checkout. A reviewer that may edit receives a separate branch and writable workspace.

Worktree is the default because it avoids duplicate Git objects. A full clone is an explicit or policy-selected fallback:

```bash
merl workspace create \
  --agent atlas-dev \
  --task T52 \
  --source github:acme/atlas \
  --strategy clone
```

Before release, Merl checks for modified or untracked files and commits not reachable from a preserved ref:

```bash
merl workspace status --task T52
merl workspace release --task T52
```

Unsafe release fails with `workspace_not_preserved` and lists the files or commits that need attention. Completing a task or clearing agent context never deletes uncertain work.

Separate workspaces do not eliminate merge conflicts. Tasks may declare likely areas of change so the PM can see overlap:

```bash
merl task affects T52 --path crates/parser --contract capture-format
merl task overlap T52
```

Merl warns or serializes assignments according to project policy. Agents integrate through commits and pull requests rather than copying files between their worktrees.

### Temporary storage and cleanup

Merl launches managed agents with owned or explicitly shared storage instead of an unbounded system `/tmp`:

```text
TMPDIR=~/.merl/scratch/W52/tmp
build=~/.merl/build/W52
tool caches=~/.merl/tool-cache/<tool>
```

`MERL_SCRATCH` can move scratch to another disk. `merl storage status` reports filesystem type, reservations, current use, and cleanup failures:

```bash
merl storage status
merl storage quota set --node 50GiB --workspace 10GiB
merl storage gc --dry-run
merl storage gc
```

```text
Scratch root: /mnt/agent-scratch
Filesystem: ext4, disk-backed
Used: 18.2 GiB of 50 GiB
Reserved: 20 GiB
Reclaimable: 6.4 GiB
Active workspaces: 3
Cleanup failures: 0
```

Tool adapters route known output safely. The Python adapter can set a bytecode-cache location and pytest base temporary directory; the Rust adapter gives each workspace its own target directory. A cache is shared only when its adapter declares concurrent access safe.

If scratch is memory-backed, status reports `memory-backed` and policy may reject new agents. If Merl cannot reserve the template's allowance, spawn fails with `storage_reservation_failed` before the host starts a process.

The janitor waits for the workspace lease and tracked processes to end. It removes only owned scratch and reconstructible output beneath configured roots. It never scans arbitrary `/tmp` paths or removes a dirty workspace, unpreserved commit, or durable artifact.

## Cross-project requests

An origin project creates a request from its own context:

```bash
merl request create \
  --to core \
  --kind infrastructure.change/v1 \
  --summary 'Provision storage for session two captures' \
  --needed-by 2026-10-01 \
  --impact-if-late 'Session two capture is blocked' \
  --snapshot D18@4 \
  --live T42
```

The result distinguishes local acceptance from remote delivery:

```text
ACCEPTED atlas:X17 at atlas@813
DELIVERY PENDING envelope XE52 to core
```

The target sees its own imported aggregate. Core can defer triage without accepting responsibility:

```bash
merl request inbox --project core
merl request show core:XR91
merl request defer core:XR91 \
  --reason 'Higher-priority infrastructure work' \
  --review-at 2026-10-01
merl request need-info core:XR91 --question 'Required retention period?'
```

Alternatively, Core can accept the request and schedule its work:

```bash
merl request accept core:XR91
merl request schedule core:XR91 --target-start 2026-10-15
merl request start core:XR91 --work T91
merl request fulfill core:XR91 --ref github:acme/infra#412
```

The origin observes those updates without gaining authority over Core's work:

```text
atlas:X17
  local: submitted
  requested need: 2026-10-01
  Core commitment: pending
  Core scheduling: deferred; review 2026-10-01
  Core reason: higher-priority infrastructure work
  Core delivery commitment: none
```

If Core accepts the request but schedules it later, the view says `commitment: accepted` and shows Core's target dates. Transport receipt, acknowledgement, and a review date never render as acceptance or a delivery promise.

The origin can amend or withdraw its request:

```bash
merl request amend atlas:X17 --summary 'Need 30-day retention'
merl request withdraw atlas:X17 --reason 'Experiment cancelled'
```

Withdrawal asks the target to stop. It does not mark Core's work cancelled.

Project administrators manage directional links:

```bash
merl link grant \
  --from atlas \
  --to core \
  --contract infrastructure.change/v1 \
  --export 'artifact:A81' \
  --export 'decision:D18'

merl link show atlas --to core
merl link revoke atlas --to core --contract infrastructure.change/v1
```

A target rejects unsupported contract versions and unauthorized exports without trying to infer their meaning.

## GitHub interface

Merl publishes little to GitHub.

Several projects may ingest one Issue, but a project administrator assigns each publication slot to one project:

```bash
merl publication slot assign \
  github:acme/atlas#204/merl \
  --project atlas

merl publication slot show github:acme/atlas#204/merl
```

Other bindings remain ingest-only and fail with `publication_slot_not_owned` if they try to write. Published output carries a stable publication and project identity. Receiving authorities verify that identity against the provider actor and content hash; they do not trust a hidden marker by itself.

Routine machine changes produce no comment. A significant accepted batch may produce one readable comment:

```text
Keep receive gain and LO fixed during baseline captures. This resolves the
gain question and unblocks session two. (D18)
```

Removing `(D18)` leaves the meaning intact.

Each tracked issue or pull request may have one managed status surface:

```text
Merl status
Current through project revision 482

Phase: session 2
Goal: complete baseline capture
Blocked: waiting on Core infrastructure request X17

Open
- Choose capture storage location. (Q41)
- Preserve clipping metadata in the reader. (T44)

Recent decision
- Keep receive gain and LO fixed. (D18)
```

Character and item budgets bound the status. Merl replaces excess detail with a count and an expansion link. Editing the managed surface creates a drift warning and never changes accepted state.

Publication diagnostics are available through:

```bash
merl publication list --failed
merl publication show PP42
merl publication retry PP42
merl projection show github:acme/atlas#204/merl-status
merl projection restore github:acme/atlas#204/merl-status
```

## Errors and non-interactive use

Human errors state what happened, what remained unchanged, and the next safe action. Machine errors have stable codes and structured details.

Non-interactive commands never prompt. Ambiguous context, missing authority, stale basis revisions, and required confirmation produce explicit results. Merl does not silently select a project, drop a command, or reinterpret an unsupported contract.

Exit success means Merl durably performed the action it reported. For a queued offline command, success means the command is safely queued. Callers that require accepted state use `--require-accepted`.

## Behavior-driven test contract

Gherkin is the initial behavior language. It is readable by users and gives the implementation room to change. We should not create another test language until repeated scenarios expose a concrete limitation.

Scenarios interact through public actions and observable results. They do not query SQLite tables or call private Rust modules. The same scenario can run through several drivers:

- CLI subprocess against a local daemon
- application service in process
- client against a shared authority
- MCP adapter where the action is exposed

Provider scenarios use controlled GitHub and artifact-store fakes at the external boundary. The runner supplies deterministic time and aliases for generated IDs while assertions use public output.

Useful tags include `@first_release`, `@local`, `@shared`, `@offline`, `@github`, `@cross_project`, and `@recovery`. The [first release plan](first-release.md) owns which scenarios carry `@first_release`; this larger contract also describes deferred behavior.

### Project context is never guessed

```gherkin
Feature: Select project context

  Scenario: One repository is attached to two projects
    Given repository "acme/atlas" is attached to projects "atlas" and "core"
    And no local workspace context is selected
    When I run "merl project view" without an interactive terminal
    Then the command fails with code "project_context_ambiguous"
    And the result lists projects "atlas" and "core"
    And neither project is read or changed

  Scenario: An explicit project resolves the ambiguity
    Given repository "acme/atlas" is attached to projects "atlas" and "core"
    When I run "merl project view --project atlas"
    Then I see the accepted Atlas project revision
```

### Local mode states its trust boundary

```gherkin
@first_release
Feature: Explain the local trust boundary

  Scenario: Local mode does not claim to sandbox same-user processes
    Given project "atlas" uses a local authority
    When I inspect its security model
    Then Merl reports that unrestricted same-user processes are trusted
    And it identifies local policy as governance and audit
    And it does not claim that local files are protected from those processes
```

### Offline work reports queued state

```gherkin
Feature: Submit a command while offline

  Scenario: A disconnected client queues a mutation
    Given shared project "atlas" is accepted through revision 481
    And its authority is unavailable
    When I resolve question "Q32" with decision "D18"
    Then the result is "queued"
    And it includes a durable command ID
    And project revision 481 remains the latest accepted revision

  Scenario: Automation requires immediate acceptance
    Given shared project "atlas" is accepted through revision 481
    And its authority is unavailable
    When I resolve question "Q32" with decision "D18" requiring acceptance
    Then the command fails with code "authority_unavailable"
    And no command is queued
    And no accepted project state changes
```

### Machine output preserves outcomes

```gherkin
Feature: Consume Merl from an automated client

  Scenario: JSON reports a queued command without implying acceptance
    Given project "atlas" is accepted through revision 481
    And its authority is unavailable
    When I resolve question "Q32" with decision "D18" as JSON
    Then the result schema is "merl.result/v1"
    And the outcome is "queued"
    And the result contains a command ID
    And the result does not contain an accepted revision later than 481
```

### Compact views remain expandable

```gherkin
@first_release
Feature: Read compact project state

  Scenario: A researcher expands a decision to its evidence
    Given project "atlas" has accepted decision "D18"
    When I view issue "github:acme/atlas#204" as a researcher
    Then the view contains the meaning of decision "D18"
    And the view does not contain the full source thread
    When I expand decision "D18" to its source
    Then I receive the captured source and provenance
```

### A new comment produces a pollable delta

```gherkin
@first_release
Feature: Read an incremental project change

  Scenario: An agent receives references instead of the full thread
    Given agent "atlas-dev" has read project revision 481
    And a new Issue comment produces accepted revision 482
    When the agent polls its inbox
    Then one entry identifies revision 482 and the changed object references
    And the entry does not contain the full Issue history
    When the agent acknowledges the entry
    Then its durable cursor advances through revision 482
```

### Compiler context is bounded and reproducible

```gherkin
@first_release
Feature: Compile an incremental Issue comment

  Scenario: A comment uses earlier context
    Given issue "204" has current decision "D18" and open question "Q32"
    And comment "SE771" says "Do that after session two"
    When Merl builds a compilation context for "SE771"
    Then the context records its basis revision
    And it records the selected object revisions and recent source events
    And it records the selector, renderer, budget, and rendered-input digest
    And it does not contain the full Issue history

  Scenario: Bounded context cannot resolve a reference
    Given a new comment says "The frame issue above is fixed"
    And the selected context contains two possible frame issues
    When Merl compiles the comment
    Then the result requests more context or records unresolved ambiguity
    And it does not invent which issue the author meant

  Scenario: Recorded input is reproducible
    Given compilation run "CR42" has available source content
    When I rebuild its rendered compiler input
    Then the rebuilt bytes match the recorded input digest
```

### Policy provenance matches the input

```gherkin
@first_release
Feature: Explain accepted state

  Scenario: An extracted decision expands to its source
    Given decision "D18" came from observed assertion "OA91"
    When I inspect the derivation of "D18"
    Then I see its policy evaluation, assertion, compilation run, context, and source event

  Scenario: A direct command has no fabricated compiler provenance
    Given an authorized human created decision "D19" through a command
    When I inspect the derivation of "D19"
    Then I see its command and policy evaluation
    And I do not see a fabricated assertion, compilation run, or source event
```

### Provider facts remain provider-owned

```gherkin
@first_release
Feature: Separate external facts from Merl semantics

  Scenario: A close request waits for GitHub
    Given GitHub reports issue "204" as open
    When an authorized user requests that issue "204" close
    Then Merl shows a pending provider action
    And the provider mirror remains open
    When GitHub reports issue "204" as closed
    Then the provider mirror becomes closed

  Scenario: Merl derives a blocker without changing GitHub facts
    Given GitHub reports issue "204" as open
    When policy accepts blocker "B9" for issue "204"
    Then the Merl semantic overlay contains blocker "B9"
    And the provider mirror remains open
```

### Source content can be purged without rewriting history

```gherkin
@first_release
Feature: Remove retained sensitive content

  Scenario: An administrator purges captured bytes
    Given source event "SE771" retains sensitive content
    And decision "D18" derives from that event
    When an authorized administrator purges "SE771" with a reason and matching digest
    Then the retained source bytes are unavailable
    And the source event keeps an audit tombstone, digest, actor, time, and reason
    And decision "D18" reports that its evidence is unavailable
    And replay reports that it cannot reproduce the affected compiler input
```

### Unrelated changes do not invalidate policy

```gherkin
@first_release
Feature: Validate policy dependencies

  Scenario: An unrelated object changes before commit
    Given a policy evaluation read "D18@4" and "Q32@2"
    And another transaction changes unrelated task "T91"
    When the authority commits the evaluation
    Then the dependency check succeeds
    And the authority assigns the next project revision

  Scenario: A predicate dependency changes before commit
    Given a policy evaluation recorded that no active decision uses key "capture.gain"
    And another transaction activates a decision with that key
    When the authority commits the evaluation
    Then the dependency check fails
    And policy reevaluates or reports a conflict
```

### GitHub publication stays sparse

```gherkin
Feature: Publish accepted state for humans

  Scenario: Routine machine changes stay inside Merl
    Given an accepted batch contains only cursor and acknowledgement changes
    When publication policy evaluates the batch for issue "204"
    Then the publication outcome is "none"
    And no GitHub write is attempted

  Scenario: One significant batch produces one readable comment
    Given one accepted batch activates a decision, resolves a question, and unblocks a task
    When publication policy evaluates the batch for issue "204"
    Then at most one comment publication is created for issue "204"
    And removing Merl IDs leaves the comment understandable

  Scenario: Merl does not compile its own status projection
    Given Merl published the managed status for issue "204" at revision 482
    When GitHub returns that comment through ingestion
    Then Merl captures the provider event
    And no observed assertion is created from it

  Scenario: One project owns a shared publication slot
    Given projects "atlas" and "core" both ingest issue "204"
    And project "atlas" owns its Merl publication slot
    When project "core" attempts to publish to that slot
    Then the command fails with code "publication_slot_not_owned"
    And no GitHub write is attempted

  Scenario: Another authority recognizes Merl output
    Given project "atlas" published a verified Merl comment to issue "204"
    And project "core" also ingests issue "204"
    When Core captures the comment
    Then Core records the stable publication and publishing-project identities
    And Core does not compile the generated prose as ordinary source
    And a hidden marker by itself would not be sufficient verification
```

### Cross-project work preserves authority

```gherkin
Feature: Request work from another project

  Scenario: Origin state and envelope commit together
    Given project "atlas" may send "infrastructure.change/v1" to project "core"
    When Atlas submits an infrastructure request
    Then Atlas accepts an OutboundRequest
    And a pending envelope exists for the same origin transition
    And both appear in one Atlas project revision

  Scenario: The target owns its work
    Given Core durably received Atlas request "X17"
    When Core accepts the request and creates task "T91"
    Then task "T91" belongs to Core
    And Atlas observes a reference to "T91"
    And Atlas cannot change the task owner or priority

  Scenario: The target defers without making a commitment
    Given Atlas needs request "X17" by 2026-10-01
    And Core has not accepted responsibility for its inbound request
    When Core defers the request until review on 2026-10-15
    Then Atlas sees Core commitment as "pending"
    And Atlas sees Core scheduling as "deferred"
    And Atlas sees the reason and review date
    And Atlas does not see a delivery commitment
    And Atlas sees that the current plan cannot meet its requested date

  Scenario: A repeated envelope does not duplicate work
    Given Core processed envelope "XE52" into inbound request "XR91"
    When Core receives envelope "XE52" again
    Then Core returns the stored receipt
    And no second inbound request or task is created

  Scenario: A legitimate causal loop is allowed
    Given correlation "X17" traveled from Atlas to Core to Platform
    When Platform asks Atlas a new question in correlation "X17"
    Then Atlas accepts the question envelope
    And Merl does not reject it because Atlas appears earlier in the lineage
```

### Delayed work remains unambiguous

```gherkin
Feature: Plan work requested by another agent

  Scenario: A project manager accepts a task but defers its schedule
    Given a researcher requested software task "T44" needed by 2026-10-01
    And the project manager accepted responsibility for task "T44"
    When the project manager defers task "T44" until review on 2026-10-15
    Then task "T44" has commitment "accepted"
    And task "T44" has scheduling "deferred"
    And task "T44" has execution "not_started"
    And the task reports that its current plan misses the requested need
    And task "T44" is absent from the software agent's actionable queue
    And the researcher and software agent receive the planning change

  Scenario: A delayed review is not an acceptance
    Given a researcher requested software task "T44"
    And nobody has accepted responsibility for task "T44"
    When the project manager defers triage until 2026-10-01
    Then task "T44" has commitment "pending"
    And Merl does not report an owner delivery date

  Scenario: Scheduling does not silently stop active work
    Given software agent "atlas-dev" is executing task "T44"
    When the project manager defers task "T44" without pausing it
    Then the command fails with code "task_in_progress"
    And task "T44" remains "in_progress"
```

### Session continuation uses current state

```gherkin
Feature: Resume an agent session

  Scenario: Referenced state changed after checkpoint
    Given agent "atlas-tech-lead" checkpointed at revision 481 with reference "D18"
    And decision "D18" changed at revision 484
    When the agent resumes
    Then the resume view uses decision "D18" at revision 484
    And it reports that "D18" changed after the checkpoint
    And the old checkpoint prose does not override accepted state
```

### Concurrent agents need separate writable workspaces

```gherkin
Feature: Protect agent workspaces

  Scenario: Two active writers share a directory
    Given agents "atlas-dev" and "atlas-reviewer" use the same writable directory
    When Merl checks workspace safety
    Then it reports a writable workspace collision
    And Merl refuses a concurrent write assignment
    And it offers to create a separate workspace

  Scenario: Two writers receive separate worktrees
    Given agents "atlas-dev" and "atlas-senior-dev" have writable assignments in repository "acme/atlas"
    When Merl creates their workspaces
    Then each agent receives a different working directory and index
    And each agent receives a unique task branch
    And both worktrees may share the same Git object store

  Scenario: A reviewer does not share a writer's live checkout
    Given agent "atlas-dev" holds a writable workspace
    When agent "atlas-reviewer" requests a read-only review at its current commit
    Then Merl creates or reuses an enforced read-only checkout pinned to that commit
    And the reviewer does not use the writer's working tree

  Scenario: Dirty work prevents release
    Given task "T52" has untracked files or commits without a preserved reference
    When Merl releases its workspace
    Then the command fails with code "workspace_not_preserved"
    And no workspace files are removed
```

### Agent setup survives a computer move

```gherkin
Feature: Move Merl setup to another computer

  Scenario: Import portable setup without copying machine identity
    Given node "old" has agent "atlas-dev" with active TDD practice "AG18"
    And node "old" has a pending command "CMD91"
    When I export node "old" and import it on a new computer
    Then agent "atlas-dev" keeps its logical identity
    And guidance "AG18" remains active
    And pending command "CMD91" keeps its idempotency identity
    And the imported node has a different node identity

  Scenario: Default export contains no secrets or workspace contents
    Given node "old" has provider credentials and a dirty agent workspace
    When I create a default node export
    Then the bundle contains no credential values
    And the bundle contains no workspace files
    And the result warns that the dirty work was not preserved

  Scenario: Clearing model context preserves agent guidance
    Given agent "atlas-dev" has an active practice to use TDD
    And the agent has a checkpoint that references task "T44"
    When its model context is cleared and the agent resumes
    Then the resume view contains the active TDD practice
    And the resume view contains current state for task "T44"
    And stale checkpoint prose does not override current project state
```

### Task-scoped agents start unrelated work cleanly

```gherkin
Feature: Control agent context lifetime

  Scenario: A developer gets a clean context after completing a task
    Given agent "atlas-dev" uses "task-scoped" context policy
    And generation 18 is bound to task "T44"
    And the agent has an active TDD practice
    When task "T44" is completed with its result references
    Then Merl durably closes generation 18 before requesting a context reset
    And Merl records the checkpoint, cursor, assignment, and lease transitions
    When the agent starts unrelated task "T52"
    Then generation 19 contains the active TDD practice
    And generation 19 contains current state for task "T52"
    And generation 19 does not contain the transcript from generation 18

  Scenario: A project manager keeps continuous context
    Given agent "atlas-pm" uses "continuous" context policy
    When one assigned task is completed
    Then Merl does not request a context reset
    And the agent continues to receive bounded project deltas

  Scenario: The host cannot reset context automatically
    Given a task-scoped session is durably closed
    And its host adapter does not support context reset
    Then Merl reports "context_reset_unsupported"
    And Merl does not report the old context as cleared
    And the checkpoint remains available for manual restart
```

### Assignment considers capability and cost

```gherkin
Feature: Choose an agent for a task

  Scenario: Difficult work favors the stronger eligible runtime
    Given "atlas-senior-dev" advertises model "anthropic:opus" at high effort and high cost
    And "atlas-dev" advertises model "anthropic:sonnet" at medium effort and medium cost
    And task "T52" requires Rust and high reasoning for a high-risk change
    When the project manager lists candidates for task "T52"
    Then "atlas-senior-dev" is eligible
    And "atlas-dev" is ineligible because it misses the reasoning requirement
    And neither advertisement grants new source access

  Scenario: Routine work favors the cheaper eligible runtime
    Given both developer sessions meet task "T53" requirements
    And task "T53" prefers lower cost
    When the project manager lists candidates for task "T53"
    Then "atlas-dev" ranks before "atlas-senior-dev"
    And the result explains the cost tradeoff
    And no assignment exists until an authorized actor chooses one

  Scenario: A stale advertisement is not eligible
    Given "atlas-senior-dev" last advertised availability before its current session
    When the project manager lists candidates for a new task
    Then "atlas-senior-dev" is excluded as stale
```

### A PM can provision only approved agents

```gherkin
Feature: Delegate agent provisioning

  Scenario: A PM spawns an approved developer
    Given a human delegated template "software-dev-medium" to "atlas-pm"
    And the template allows two active agents with 10 GiB each
    When "atlas-pm" spawns an agent for task "T53"
    Then Merl durably accepts one spawn request
    And Merl reserves the workspace and scratch allowance before calling the host
    And the agent is unavailable until the host reports a ready session

  Scenario: A PM cannot widen its delegation
    Given "atlas-pm" may spawn only template "software-dev-medium"
    When it requests another model or a third active agent
    Then the request fails with code "delegation_exceeded"
    And no host provisioning call occurs

  Scenario: An ambiguous host response does not duplicate an agent
    Given spawn request "SP19" may already have reached the host
    When provisioning retries after a lost response
    Then Merl reconciles using provisioning identity "SP19"
    And at most one host agent belongs to the request
```

### Agent scratch stays bounded

```gherkin
Feature: Bound agent temporary storage

  Scenario: Managed test processes avoid system tmp
    Given workspace "W52" has an owned disk-backed scratch directory
    When Merl launches its agent and pytest adapter
    Then the process TMPDIR belongs to workspace "W52"
    And pytest temporary files stay under the owned scratch root

  Scenario: Storage reservation fails before spawn
    Given the node cannot reserve the template's scratch allowance
    When a PM requests another agent
    Then the request fails with code "storage_reservation_failed"
    And no host process starts

  Scenario: Cleanup preserves unknown and active data
    Given an expired workspace has reclaimable scratch
    And another workspace still has a live process
    When the janitor runs
    Then it removes only expired Merl-owned scratch
    And it leaves the live workspace, system tmp, and unknown paths unchanged

  Scenario: Memory-backed scratch is visible
    Given MERL_SCRATCH resolves to a memory-backed filesystem
    When I inspect storage status
    Then the result identifies it as "memory-backed"
    And project policy may prevent another spawn
```

## Test implementation guidance

Step definitions should call a small behavior-driver interface rather than shelling out from every step. A CLI driver renders actions as commands and parses public results. Other drivers call the corresponding public service or MCP action.

The driver vocabulary should stay close to user actions:

```text
create_project
attach_source
view_issue
inspect_compilation_context
submit_command
inspect_derivation
purge_source_content
request_task
plan_task
read_inbox
advertise_agent
list_task_candidates
spawn_agent
create_workspace
release_workspace
inspect_storage
collect_scratch
inspect_security_boundary
export_node
import_node
create_request
publish_projection
resume_session
close_session
```

This helper API is a test harness, not another product protocol. If Gherkin becomes cumbersome, we can add a typed Rust scenario builder underneath it while keeping the feature files as the behavior contract.

Tests should assert outcomes, public state, emitted references, and visible side effects. They should avoid internal table names, private scheduler order, private function calls, and implementation-specific timing. Asynchronous assertions use bounded eventual checks against public status rather than sleeps.
