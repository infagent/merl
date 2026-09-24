# User interface and behavior

Status: draft behavior contract

This document describes how people and agents use Merl. It defines observable behavior before storage, networking, or database design. The CLI is the public interface for people, agents, scripts, and CI.

The command names are the first proposed interface. We may refine spelling during implementation, but the behavioral distinctions in this document are requirements.

## Interface principles

Merl's interface follows these rules:

- The CLI is the public interface.
- Help is hierarchical and loaded on demand; agents need not carry the full command catalog.
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
| `merl` CLI | Humans, agents, CI | Public interface for every semantic action |
| GitHub | Humans | Discussion, managed status, and significant project events |
| Daemon status | Operators | Authority health, synchronization, queues, and delivery failures |

Merl has no required graphical interface. A graphical client can arrive later by invoking the same application services, but it does not define the agent interface.

## Command shape

Commands use nouns and semantic actions:

```text
merl <noun> <action> [arguments]
```

Examples:

```bash
merl project view
merl issue view github:acme/project-a#204
merl question resolve Q32 --decision D18
merl experiment record --hypothesis H4 --artifact capture:S1
merl request create --to project-b --kind infrastructure.change
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

Command discovery is incremental:

```bash
merl help
merl help find checkpoint
merl help session
merl help session checkpoint --format json
```

Top-level help lists command groups, not every flag and example. Subcommand help describes arguments, outcomes, error codes, examples, and related commands. JSON help uses a versioned schema so an agent can inspect one operation without loading the whole command surface.

Capture a live Issue after initializing a local project:

```bash
merl project init --id project-a --database project.sqlite
merl issue capture --project project-a --database project.sqlite \
  --repository acme/project-a --issue 204 \
  --program /path/to/compiler --compiler-version v1 --model model-id \
  --prompt-digest sha256:<64-hex-digits> --format json
```

Install GitHub CLI (`gh`) and authenticate with `gh auth login` or its supported
environment credentials. Merl invokes `gh api graphql` and keeps credentials out
of project records. `--github-program` selects a compatible executable for a
controlled provider. Merl records observation time from the node clock; callers
cannot supply it as an argument. Acceptance drivers inject a deterministic clock
through the CLI's Rust entry point.

The first capture establishes a binding by immutable repository ID and records
`github_capture_v1` policy. The defaults are `--mode eager --coverage required`.
Use `--mode capture_only --coverage optional` to retain prose without compiler
work. `on_demand` also leaves sources cold. Use `--mode eager --coverage optional`
to compile new evidence without making its interpretation a completeness
requirement. A refresh reuses the binding's stored policy, even when the
caller supplies different defaults; capture flags cannot change an existing
binding's policy.

Merl holds an exclusive capture lock across provider fetch and compiler dispatch.
An overlapping capture reports `AUTHORITY_BUSY` without dispatching work.

Repeat `issue capture` to refresh the Issue. Merl retains each observed body,
links edits to the previous capture, and records missing comments after a complete
provider response. A missing comment gets a bodyless deletion observation at
capture time. Its earlier body remains available; Merl does not infer the upstream
deletion time. Partial pages cannot establish that a comment disappeared. Merl rejects older
provider timestamps before appending source versions.

Each new eager body gets a causal compiler intent before execution. A successful
or failed completed run keeps its identity on refresh and does not run again.
An interrupted pending intent can resume with the same compiler configuration.
Compilation records assertions; deterministic provider policy accepts open/closed
state, labels, assignees, and timestamps through the project revision and inbox.
Use `source assertions` to inspect a run and `source apply` to evaluate its assertions through policy.

Both output modes report `captured`, `unchanged`, `failed`, or `incomplete`.
JSON uses `merl.issue-capture/v1` and reports the binding policy, new version and
run IDs, capture and compiler counts, observation head, and accepted revision.
`failed` reports provider-request or compiler failures; `incomplete` means the
provider response could not establish a complete snapshot. A failed compiler
leaves its captured source and recorded intent available for inspection. Invalid
arguments and store conflicts use the existing error envelope and a nonzero exit.

Merl bounds each provider response at 16 MiB. This command performs one refresh;
it does not start polling or publish to GitHub.

Import a development fixture without GitHub credentials:

```bash
merl issue import-fixture --project project-a --database project.sqlite --fixture issue.json --format json
```

It reports the number of new source versions, the observation head, and the accepted revision. A repeat import does not create another observation or accepted provider snapshot. The fixture's terminal provider facts enter at capture time; Merl does not place them at earlier historical cutoffs.

A retry of the same source version and body must preserve its entity author. If the stored capture and retry provide different stable provider author IDs, Merl reports a source conflict. A matching ID, or a missing ID on either side, leaves the first capture unchanged. Older captures with no recorded author remain unknown, even if a retry supplies one. An edit by another person remains valid: the entity author and version editor are separate identities.

With `--format json`, every command returns one result envelope. The envelope identifies the schema, action, outcome, project, accepted revision if one exists, affected objects, warnings, and error details. For example:

```json
{
  "schema": "merl.result/v1",
  "action": "question.resolve",
  "outcome": "accepted",
  "project": "project-a",
  "revision": 482,
  "objects": ["Q32", "D18"],
  "warnings": []
}
```

Human and JSON output preserve these fields. Compact output cannot rename an accepted outcome or conceal a queued one.

## Project context

Merl resolves project context in this order:

1. `--project`
2. `MERL_PROJECT`
3. a node-local context selected for the current workspace
4. the only project-source binding that matches the current repository

If several bindings remain, an interactive terminal asks the user to choose. A non-interactive command fails with `project_context_ambiguous` and lists the candidates. Merl never guesses based on a recently used project.

Users can set a local workspace preference without modifying the repository:

```bash
merl context set project-a
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
merl project create project-a --local
merl source attach . --project project-a
```

For a provider-backed repository, Merl resolves the Git remote through the provider and stores the immutable provider repository ID. For a local source without a provider identity, Merl creates a node-local source identity.

The same source can attach to another project:

```bash
merl source attach . --project project-b \
  --select 'label=project-b' \
  --capability ingest \
  --capability compile
```

Selection and capabilities remain separate. Adding `--select 'label=project-b'` cannot grant publication or issue-creation authority.

Useful inspection commands include:

```bash
merl project list
merl project status
merl project coverage [--role researcher]
merl source list
merl source show github:acme/project-a
merl source bindings github:acme/project-a
merl source compilation-policy github:acme/project-a
merl source compilation-policy set github:acme/project-a \
  --kind issue_comment \
  --mode eager \
  --coverage required
```

Compilation policy is part of the project-source binding. Mode is `capture_only`, `on_demand`, or `eager`; coverage is `required` or `optional`. Mode controls when extraction runs. Coverage controls whether an unprocessed observation blocks semantic completeness. A sender cannot override either field on an individual message. Structured commands and trusted provider observations bypass prose compilation.

`source compile` records its authorization before starting the compiler. The authority rejects a missing request or a mismatch in source version, run, compiler and version, executable configuration, model, prompt digest, or work limits without creating compiler work. Repeating an existing `source require` with the same content returns the accepted requirement unchanged, even when another actor asks for it.

`merl project status` makes authority and freshness visible:

```text
Project: project-a (P17)
Authority: local, reachable
Accepted revision: 482
Source observation head: 771
Semantic coverage: required sources complete through observation 771; no required gaps
Optional cold sources: 2; none attached to the current view
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

## Project authority grants

The local CLI establishes the first administrator when it initializes a project:

```bash
merl project init --id P1 --database project.sqlite --administrator owner
merl project authority list --project P1 --database project.sqlite --json
merl project authority grant --project P1 --database project.sqlite \
  --id grant-alice-decisions --actor owner --subject alice \
  --permission decision_author --reason 'Alice owns project decisions' --json
merl project authority revoke --project P1 --database project.sqlite \
  --id revoke-alice-decisions --actor owner --subject alice \
  --permission decision_author --reason 'Alice changed roles' --json
```

`decision_author` allows policy to accept that source author's explicit decisions when their provenance supports the claim. `command_actor` allows structured semantic commands. The grants are independent; neither grants administrative authority. Administrators remain part of the project's bootstrap configuration in this release.

Grant and revoke require an administrator. Results name the disposition, acting administrator, policy version, configuration digest used for evaluation, and accepted revision. A rejection has no accepted revision. The list command reports current grants and their digest. Both permissions survive restart and projection rebuild. `merl help project authority` lists the commands; leaf help supplies arguments and examples.

Each change needs a request `--id` and a reason. The reason stays in an erasable payload linked from the accepted event. Repeating the same request returns its recorded result, even if the permission has since been revoked. Reusing an ID with different content fails with `POLICY_INPUT_CONFLICT`; a new request needs a new ID. Work prepared before a grant change may return `POLICY_CONFLICT` and require reevaluation.

In local mode, `--actor` is a cooperating client's audit claim. Merl trusts processes with unrestricted access as the same OS user; these commands do not authenticate that user or sandbox the database.

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
- project-a-dev had dirty or unpushed work; files were not exported
```

The default bundle includes agent profiles, personal guidance, project and authority references, node-local delivery preferences, workspace recipes, local context choices, adapter declarations, and pending commands with their IDs.

Project-scoped guidance and subscriptions resynchronize from their authority. The bundle excludes credential values, caches, logs, captured source bodies, and workspace contents. It can contain private metadata and queued payloads, so Merl writes it with private permissions.

Import preserves logical agent IDs but creates a new node ID. It does not require login. Missing credential references disable the affected provider operations until the user configures them through that adapter.

Shared projects synchronize from their authority. Moving a local project authority uses a separate archive:

```bash
# Old computer: create the archive and freeze this authority generation
merl project export project-a --for-transfer --output project-a.merl-project

# New computer
merl project import project-a.merl-project --take-authority
```

Import verifies the archive and advances the authority generation before enabling writes. A node manifest never copies a live local authority or claims to preserve dirty Git files.

## Reading project state

The main read commands are:

```bash
merl project view [--role researcher|engineer|pm] [--focus T42]
merl project coverage [--role researcher]
merl project delta --since 481
merl project batch --batch DB482 [--offset 20]
merl issue view github:acme/project-a#204 [--role researcher|engineer|pm]
merl issue coverage github:acme/project-a#204 [--role researcher]
merl pr view github:acme/project-a/pull/229 [--role reviewer]
merl show D18
merl show D18 --history
merl show D18 --source
merl artifact read A81 [--section protocol]
```

Views show current accepted state by default. Superseded objects stay hidden unless the user asks for history. Every compact object can expand to detail and captured evidence. Accepted revision describes committed state; it does not imply that Merl has interpreted every captured source.

An explicit focus object and its direct relations come before role ordering. For example, engineers focused on T17 and T44 receive different first items even though both use the engineer role. Without a focus, the PM view puts tasks, blockers, decisions, and open questions ahead of research detail. Every view uses the same accepted state and reports truncation at its item limit. The first release takes focus from the command; a later assignment system can supply it for the agent.

An inbox poll or project delta may show only the first page of a large accepted batch. The result names the batch and the next offset. Use `merl inbox show --revision 482 --offset 20` or `merl project batch --batch DB482 --offset 20` to read the rest. Acknowledging an entry does not remove its batch pages. New subscribers start at the current project revision and receive changes from that point forward.

`merl show D18 --source --history` follows the accepted evidence history even if a later command changed D18. It shows the original assertion's run, index, and exact source span alongside the object's current support status. A historical span is evidence of what Merl accepted then; its presence alone does not claim that the source still supports the decision now.

Coverage is scoped to the view or question. It reports the source-observation head, the contiguous processed cutoff, and required gaps grouped as cold, pending, failed, purged, or excluded. A watermark never hides an earlier required hole. Negative answers remain qualified until all required observations in scope are processed. Optional cold sources do not weaken completeness; structurally linked ones appear separately as attachments.

For provider-backed work, views label the source of each field. GitHub-owned facts such as open or closed state, labels, assignees, timestamps, and merge state appear separately from Merl-owned requirements, blockers, decisions, and findings. A pending provider action appears as a request until GitHub confirms it.

Example compact output:

```text
project-a@482  source_head=771  required_coverage=complete:771 gaps=0

Keep receive gain and LO fixed for baseline captures. (D18)
Open question: choose storage location for session two. (Q41)
Blocked: session two capture waits on project-b request X17. (B9)
```

When coverage is incomplete, the same field is explicit:

```text
Blocked: none accepted; 2 required sources remain uncompiled. (SE804 SE811)
Optional cold attachments: 1 note linked to T42. (SE900)
```

JSON output includes the same meanings, IDs, revision, provenance links, and expansion references. It does not include decorative human prose that changes the semantics.

### Inspecting compiler context

Compiler diagnostics show the bounded input used for one run:

```bash
merl compilation list --project P1 --database project.sqlite
merl compilation show --project P1 --database project.sqlite --run CR42
merl compilation context --project P1 --database project.sqlite --run CR42
```

```text
Run: CR42
Trigger: SE771
Interpretation basis: project-a@482
Source cutoff: observation 771
Mode: live
Objects: D18@4 Q32@2 E37@5
Recent source: SE768 SE769 SE771
Selector: issue_context_v2, budget 6000 bytes
Renderer: issue-compiler-context/v1
Input digest: sha256:...
Output budget: assertions=12 bytes=16384 tokens=1200 context_requests=2 expansions=2 payload_bytes=2048
Output: accepted by protocol, 4 assertions
```

The first-release compiler can return `context_required` with named object or
source-version references. Relation traversal and source ranges require a later
protocol extension. Merl records the request as durable work and keeps required
coverage open until a successor completes.

`compilation list` pages through 100 requests at a time with `--offset` and JSON
`next_offset`. It includes pending, running, blocked, exhausted, failed, and
completed work. `compilation show --run` reports the reserved child run, round,
references, failure code, and contributing contexts. `compilation context --run`
reads the exact retained input, including its rendered bytes.

Execute one round with the same executable, arguments, compiler version, model,
and prompt digest used by the parent:

```bash
merl compilation expand --project P1 --database project.sqlite --run CR42 \
  --program ./compiler --compiler-version v1 --model model-a \
  --prompt-digest sha256:<hex> --json
```

Repeat that command after interruption to resume its reserved child. If the
child requests more context, use its run ID for the next round. The command
inherits the original limits and accepts no budget overrides. It does not
apply assertions; use `source assertions` and `source apply` on the final run.

An unavailable or disallowed reference reports `COMPILER_EXPANSION_REFERENCE`;
repeated context reports `COMPILER_EXPANSION_LOOP`. Exhausted rounds report
`COMPILER_EXPANSION_ROUND_BUDGET`, and input overflow reports
`COMPILER_INPUT_BUDGET`. Erased evidence reports `MISSING_EVIDENCE`. A changed
compiler configuration reports `COMPILER_UNAUTHORIZED` and leaves the pending
work unchanged. Blocked and exhausted requests retain their original response
and cannot satisfy required coverage. A new compilation attempt needs a new
run identity and the applicable spend authorization.

The response schema accepts typed assertions, spans, relations, confidence, attribution, `context_required`, and `unresolved`. It does not accept rationale or summary essays, copied source passages, or chain-of-thought. Exceeding an output or expansion limit fails the run with a stable error; Merl does not keep a truncated subset as though compilation succeeded.

Historical imports display `replay` only when the recorded context excludes all later observations. A late on-demand run uses the triggering source's historical interpretation basis and cutoff even when policy evaluates its assertions at a much newer project revision. If provider history is incomplete, or a caller deliberately supplies later knowledge, Merl displays `hindsight`; that run cannot enter accepted state unless an authorized user promotes its outputs through policy.

## Changing project state

Mutations use domain language:

```bash
merl decision create --summary 'Keep receive gain fixed' --scope baseline
merl decision supersede D11 --with D18
merl question resolve Q32 --decision D18
merl finding open --pr github:acme/project-a/pull/229 --severity high
merl finding resolve F22 --commit a83c1
merl hypothesis create --statement 'DC impairment dominates baseline errors'
merl experiment record --hypothesis H4 --artifact capture:S1
merl claim create --supports H4 --evidence E37
merl task block T42 --on B9
```

The result names the authority outcome:

```text
ACCEPTED  D18 at project-a@482
```

```text
CANDIDATE C81 awaiting research-owner approval
```

```text
QUEUED CMD91 against project-a@481; authority is offline
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

### First-release semantic commands

These operations accept authored content through the public CLI:

| Command | Required semantic arguments |
| --- | --- |
| `decision create`, `question create`, `finding create` | `--subject ID --summary TEXT` |
| `hypothesis create`, `claim create` | `--subject ID --summary TEXT` (`--statement` is an alias) |
| `question resolve`, `finding resolve` | `ID --summary ANSWER` |
| `task request` | `--subject ID --summary TEXT` |
| `task accept`, `task start`, `task complete` | `ID` |
| `task defer` | `ID --reason TEXT --review-at YYYY-MM-DD` |

Each command also takes `--project`, `--database`, `--actor`, and a stable retry
`--id`. A current `command_actor` grant authorizes the action. In local mode,
`--actor` remains the cooperating client's audit claim. New objects may use
`--issue SCOPE`; later commands inherit that scope and cannot change the kind.
Provider-owned objects and fields stay outside this interface.

```bash
merl decision create --project P1 --database merl.db --actor alice \
  --id choose-gain --subject D1 --summary 'Keep receive gain fixed' --json
merl task request --project P1 --database merl.db --actor alice \
  --id request-reader --subject T1 --summary 'Add clipping metadata'
merl task accept T1 --project P1 --database merl.db --actor alice --id accept-reader
merl task defer T1 --project P1 --database merl.db --actor alice --id defer-reader \
  --reason 'Migration has priority' --review-at 2026-10-01
```

T1 now shows commitment `accepted`, scheduling `deferred`, and execution
`not_started`. The reason and review date survive restart and projection rebuild.
Deferring work in progress returns a rejected disposition with `task_in_progress`.
Starting pending or deferred work, or restarting completed work, is rejected.
This slice uses review dates as reconsideration triggers; condition relations,
owner schedules, and execution leases remain outside these commands.

Add `--note TEXT` or `--note-file PATH` to retain supplemental prose. Statements,
reasons, and notes each allow at most 8192 UTF-8 bytes. Merl retains the command
receipt and note together. The note stays `capture_only` and `optional`; ordinary
views expose its reference. `show ID --source --history` expands notes across later
object updates, and `source show --version VERSION` shows the originating command,
with an affected-object reference only after acceptance. Erasure leaves the reference
and an unavailable result. Retrying the command cannot restore erased bytes.

An authorized later compile sees the note's originating intent. Policy records a
positive request matching the accepted command's subject, kind, and nonempty
represented value reference as `duplicate`. A same-subject request with a different
value reference or `none` remains a `candidate` with reason `supplemental_correction`.
A request of the same kind with a different subject becomes a `candidate` with reason
`possible_supplemental_duplicate`. It may repeat the original act under another ID
or express a separate request. Inspect it through `candidate show`, then accept,
reject, or correct it. Added findings or other predicates still receive their own
policy dispositions; corrections to the original object require review.

Results use `merl.semantic-command/v1`; previews use
`merl.semantic-command-preview/v1`. Human and JSON results distinguish `accepted`,
`rejected`, and `conflict`. Policy dispositions return exit success; malformed input
returns `INVALID_INPUT`, and changed content under an existing request ID returns
`POLICY_INPUT_CONFLICT`. Exact retries return the recorded disposition and revision.
`--dry-run` retains no content, source, or receipt. `merl help task defer` describes
that operation without loading the whole command catalog.

`show` resolves the object's statement and command provenance. Resolved questions
and findings show `status=resolved`; their accepted lifecycle stays `active` because
the resolution remains project knowledge. Task views expose all three planning
facets. A compiler or review replacement without planning data makes those facets
unknown rather than carrying older values into a new interpretation.

### Applying compiler assertions

After a live compiler run finishes, inspect its output and request policy evaluation:

```bash
merl source assertions --project P1 --database project.sqlite --run CR42 --json
merl source apply --project P1 --database project.sqlite --run CR42 \
  --actor worker --id apply-CR42-1 --json
```

Inspection uses `merl.assertions/v1`. It reports each assertion's index, exact source span, semantic axes, attribution, and whether Merl has accepted it. It also lists unresolved spans, context requests, and deferred relations. If a purge removed the response, `response_available` is false and the response-only lists are null; Merl does not claim the run had no unresolved work.

Application uses `merl.assertion-application/v1`. Each input has its own `outcome` and `reason`; the envelope names the policy evaluation, policy version, configuration digest, basis revision, and accepted revision when one exists. Outcomes are `accepted`, `candidate`, `rejected`, `duplicate`, or `conflict`. A conflict includes its failed guard when the final transaction detected it. Ordinary dispositions return a successful command result; invalid requests use the error envelope and a nonzero exit.

Only a recorded author's positive, unquoted `decision` with act `request`, basis `reported`, and a current `decision_author` grant qualifies for automatic acceptance. Here `request` means a directive to adopt that decision. Other supported semantic assertions remain candidates, including task requests from the same author. Provider and control predicates are rejected. The [architecture's permission matrix](architecture.md#applying-recorded-assertions) defines the mapping, including supported object kinds and retained payload references.

Add `--dry-run` to preview current policy without recording an evaluation, accepted state, or inbox entry. JSON previews use `merl.assertion-preview/v1` and include proposed object IDs. A preview grants no acceptance and does not reserve the request ID.

Reuse `--id` when retrying the same application. Merl returns the original outcomes even after authority changes. To reevaluate a candidate, submit a new request ID; Merl still deduplicates previously accepted run/index pairs. Reusing an ID with another run or actor returns `POLICY_INPUT_CONFLICT`. An empty successful run returns an empty input list and no evaluation or revision.

`ASSERTION_RUN_INELIGIBLE` means the run is not a completed live result without context requests. Replay, eval, and hindsight need a separate promotion workflow. Changed or erased evidence cannot create new current support. Reading assertions spends no compiler tokens and accepts no state.

### Reviewing candidates

Inspect the candidates produced by `source apply`, then act with a current `command_actor` grant:

```bash
merl candidate list --project P1 --database project.sqlite --json
merl candidate show C81 --project P1 --database project.sqlite --json
merl candidate accept C81 --project P1 --database project.sqlite \
  --actor reviewer --id review-1 --json
merl candidate reject C82 --project P1 --database project.sqlite \
  --actor reviewer --id review-2 --reason 'Capture S1 does not measure this effect'
merl candidate correct C83 --project P1 --database project.sqlite \
  --actor reviewer --id review-3 --subject Q41 --kind question --value none \
  --reason 'The author left this question open' --json
```

Use the candidate ID returned by `list`; the examples abbreviate it as `C81`. Lists include pending and resolved candidates, with up to 100 entries per page. Pass `next_after` as `--after` to continue. `show` displays the original assertion, exact source span, semantic axes, proposed effect, first policy reason, and current dependency state. It also returns up to 100 review attempts; pass `next_offset` as `--offset` for the next page. Inspection reads no unrelated prose and spends no compiler tokens.

Acceptance adopts the original proposal. Correction creates a new typed proposal linked to that candidate and assertion. `--kind` uses the same supported semantic predicates as assertion application; `--value` names an existing retained payload or `none`. A correction keeps the original Issue scope and cannot replace an existing object's kind or scope. `merl show Q41 --source --history` expands the accepted correction to the original interpretation and the reviewer's replacement, including after a later object update.

Each action runs policy under current command authority. A source author's `decision_author` grant alone cannot authorize review. Acceptance and correction conflict if evidence changed or disappeared, the original target changed, or another action already resolved the candidate. Rejection can close a stale candidate. An accepted rejection advances the revision with a control record but creates no semantic object from the rejected assertion. Review control records stay out of project semantic views.

Rejection and correction require `--reason`; acceptance permits it. Merl stores the note behind an erasable payload reference and limits it to 8 KiB. Inspection reports `reason_available=false` after erasure. The original assertion and review record remain available, and retrying the request does not restore erased bytes.

Reuse `--id` for an identical retry. Merl returns the original outcome even after grants change. Different content under the same ID returns `POLICY_INPUT_CONFLICT`; another request to resolve a completed candidate returns `conflict`. Later `source apply` calls cannot revive rejected or corrected candidates. Use `--dry-run` to preview current policy without writing a request, reason payload, revision, or inbox entry.

JSON uses `merl.candidates/v1`, `merl.candidate/v1`, `merl.candidate-review/v1`, and `merl.candidate-review-preview/v1`. A review result separates `action` from `outcome`: an authorized rejection has `action=reject` and `outcome=accepted`. Results name the reviewer, policy evaluation, and accepted revision when one exists. Policy rejection and conflict are successful command results with explicit dispositions. Invalid requests return the error envelope; an unknown candidate uses `CANDIDATE_NOT_FOUND`. Human output exposes the same dispositions and provenance.

### Rebuilding and replaying

```bash
merl project rebuild --project P17 --database project.sqlite
merl source replay --project P17 --database project.sqlite --run CR42 --json
```

Use `project rebuild` to reconstruct objects and relations from accepted events and the provider Issue mirror from accepted observations and sightings. Accepted views also need the immutable typed policy inputs referenced by those events; semantic command receipts supply task facets and question/finding resolution status. It never reads source prose or calls a compiler. The result includes the accepted revision and flags degraded provenance if payloads were erased. Use `source replay` to reconstruct one recorded compiler input with its original selector and renderer. Missing bytes produce `MISSING_EVIDENCE`; an unknown selector or renderer produces `UNSUPPORTED_REPLAY_VERSION`. Replay does not accept state.

An operator can run a configured process compiler against that causal input:

```bash
merl source replay --project P17 --database project.sqlite --run CR42 \
  --program ./configured-compiler --new-run CR99 \
  --compiler-version v2 --model chosen-model \
  --prompt-digest sha256:...
```

`CR99` is a new `replay` run. Its assertions remain separate from accepted project state until a later policy evaluation explicitly promotes them. A rerun may infer something different; projection recovery never depends on that model result.

### Purging retained source content

The local administrator can remove source bytes that Merl must no longer retain:

```bash
merl source purge --version SE771 --project P17 --database project.sqlite \
  --reason 'Credential posted in comment' \
  --dry-run

merl source purge --version SE771 --project P17 --database project.sqlite \
  --reason 'Credential posted in comment' \
  --actor administrator \
  --confirm-digest sha256:...

merl source purge-audit --version SE771 --project P17 \
  --database project.sqlite --json
```

The preview follows protected text through accepted objects into later compiler contexts. It names the affected payloads, runs, and accepted state. Merl hashes that set; if another derivation appears before confirmation, the operator must preview again. After erasure commits, a busy SQLite reader may delay the physical scrub. The command then returns a receipt with `completed=false`, and Merl retries the scrub on the next open. `purge-audit` shows the actor, reason, payload digests, and completion status. Merl stores the reason behind a protected payload reference. `--actor` records the local user's claim; it does not authenticate a process running under the same OS account.

After purge, `merl show D18 --source` returns `source_content_unavailable` with the tombstone reference. Derived state remains visible in redacted form unless a separate policy action invalidates it.

The first release covers the active Merl store. No managed replica or backup scope is configured yet. Provider systems, unmanaged backups, and prior exports remain outside the erasure claim.

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
  --impact-if-late 'Session two analysis cannot begin' \
  --note-file design-context.md
```

`--note-file` is captured atomically with the command. The resulting source records `semantic_origin=CMD44` and `supplements=T44` after acceptance. If the note is compiled later, Merl supplies those links and the represented task semantics so compilation can add constraints or evidence without creating another task.

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
merl inbox claim IN123 --session S18
merl inbox ack IN123
merl inbox watch
```

`inbox watch` talks to the Merl daemon. The invoking agent does not maintain a fragile shell watcher. If wake-up fails, the entry remains pending and appears in `merl project status`.

One active session owns a logical agent's actionable inbox cursor. Other sessions may inspect entries but cannot claim them or advance the cursor. An acknowledgement records receipt and cursor progress; it does not mean agreement, understanding, task acceptance, question resolution, or completion.

Before ending a session, an agent records a checkpoint:

```bash
merl session checkpoint \
  --agent project-a-tech-lead \
  --summary 'Session two planning is blocked on X17' \
  --ref T42 --ref X17 --ref D18
```

A later session resumes from accepted project state plus the checkpoint's references:

```bash
merl session resume --agent project-a-tech-lead --format json
```

The checkpoint does not copy full decisions, tasks, or artifacts. `resume` expands current versions and reports anything that changed after the checkpoint revision.

Each new model context receives one bootstrap line from its host adapter:

```text
Run `merl session resume --agent project-a-tech-lead --format json` before work; use `merl help <topic>` when needed.
```

The durable agent profile stores this behavior once. The host renders the short pointer into each disposable context because a fresh model cannot retrieve durable guidance until it knows how to resume. Merl does not require an installed skill or inject the command catalog.

Agent guidance survives process shutdown, restart, and model-context clearing:

```bash
merl agent practice add \
  --agent project-a-dev \
  --scope project:project-a \
  --statement 'Write a failing behavioral test before implementation'

merl agent lesson propose \
  --agent project-a-dev \
  --scope project:project-a \
  --statement 'Run capture compatibility tests before changing reader framing' \
  --evidence T44

merl agent guidance list --agent project-a-dev
merl agent guidance show AG18
merl agent guidance activate AG18
merl agent guidance retire AG18 --reason 'Superseded by project test policy'
```

Instructions record behavior required by an authorized human or project policy. Practices record adopted working methods. Lessons record observations proposed for reuse and may require approval. Each item has scope, provenance, priority, and lifecycle. Guidance changes how an agent works; it is not project knowledge and cannot override accepted requirements or safety policy.

`merl session resume --agent project-a-dev` assembles a bounded view from identity and role, relevant active guidance, actionable assignments, the latest checkpoint, changes since its cursor, and current referenced project state. Current state wins over stale checkpoint prose.

Context lifetime is configurable per agent:

```bash
merl agent context-policy set project-a-pm --mode continuous
merl agent context-policy set project-a-dev --mode task-scoped
merl agent context-policy set project-a-researcher --mode manual
merl agent context-policy show project-a-dev
```

With `task-scoped`, Merl closes the session after its bound task becomes completed, cancelled, deferred, or otherwise non-actionable. The Task outcome and any Assignment transition are accepted independently. Before asking the host to clear context, session closure commits a final checkpoint, cursor, proposed lessons, and lease releases.

```bash
merl task complete T44 \
  --result github:acme/project-a/pull/229

merl session close \
  --agent project-a-dev \
  --assignment A44 \
  --reason task-non-actionable \
  --propose-lesson 'Run compatibility tests before changing reader framing'
```

The result separates durable closure from the host side effect:

```text
SESSION CLOSED project-a-dev generation 18
CHECKPOINT CP91 committed at project-a@512
CONTEXT CLEAR pending through host adapter
```

If the adapter cannot reset context, Merl reports `context_reset_unsupported` and gives the user a manual next step. It does not claim that generation 18 was cleared. Starting the next task creates generation 19 with active guidance and current state for that task, without generation 18's transcript. Replacing a context while work remains creates a new session and new leases on the existing Assignment and workspace; it does not transfer generation 18's leases.

An agent with `continuous` policy stays in the same generation after completing a task. It still receives compact deltas and bounded views. A manual clear uses the same safe closure path:

```bash
merl session close --agent project-a-pm --checkpoint --clear-context
```

### Advertising runtime and choosing an agent

The host adapter should publish runtime information when a session starts. A manually integrated host can do the same through the CLI:

```bash
merl agent advertise \
  --agent project-a-senior-dev \
  --model anthropic:opus \
  --effort high \
  --cost-class high \
  --capability rust \
  --capability architecture-review \
  --available

merl agent advertise \
  --agent project-a-dev \
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
1. project-a-senior-dev
   model=anthropic:opus effort=high cost=high
   matches: rust, high reasoning; extra: architecture-review

Ineligible
- project-a-dev
  model=anthropic:sonnet effort=medium cost=medium
  missing: high reasoning
```

For a routine task with medium reasoning demand and `--budget prefer-cost`, both agents may be eligible and the Sonnet-backed developer ranks first. Ranking is advisory and gives reasons. The PM or another authorized actor commits the choice:

```bash
merl task assign T52 --to project-a-senior-dev \
  --reason 'High-risk parser change needs reasoning headroom'
```

A runtime advertisement cannot grant access or satisfy a review requirement. Merl records the requirements, candidate evidence, policy, and rationale in a StaffingDecision. The Assignment records the logical agent's durable responsibility, while the concrete session advertisement belongs to its execution lease and outcome. This history lets the team compare the staffing prediction with Task outcome and token use.

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
  --project project-a

merl delegation grant \
  --to project-a-pm \
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
Agent: project-a-dev-4
Workspace reservation: 10 GiB
Provisioning: pending through host adapter
```

The agent appears in `merl agent list --available` only after the host reports it ready and Merl records its current advertisement. A duplicate spawn ID returns the existing result. An ambiguous host response is reconciled before retry, so one request cannot start two agents.

The PM can inspect, drain, or stop agents covered by its delegation:

```bash
merl agent provisioning show SP19
merl agent drain project-a-dev-4
merl agent stop project-a-dev-4
```

Draining prevents new assignments and lets active work checkpoint. Stopping releases workspace and storage reservations only after the host confirms process exit. Attempts to exceed concurrency, select another model, grant a permission, or use an unapproved credential fail with `delegation_exceeded`.

An urgent stop does not bypass durable state:

```text
CANCELLATION REQUESTED T53
INTERRUPT REQUESTED S18
HOST ACTION HA91 pending
```

Merl attempts `HA91` immediately after commit. The UI reports interrupt requested, host delivery, confirmed process exit, and task cancellation separately. A delivery failure leaves the host action pending and retryable.

A human may attach a process started elsewhere:

```bash
merl agent attach --template software-dev-medium --task T53
```

The authority assigns or confirms the logical identity. It marks manually supplied runtime fields with their actual provenance.

The attach result prints the exact bootstrap line for the user to place in the external session. Host-managed sessions receive it automatically.

Clearing context in an agent host does not call a Merl mutation. Guidance stops affecting future sessions only through an explicit retirement or supersession:

```bash
merl agent guidance retire AG18 --reason 'No longer applies'
```

Retirement preserves provenance and audit history. Erasing retained project records for privacy or compliance is a separate administrative concern.

### Direct coordination

Most collaboration changes project state. A request that may be accepted, prioritized, or delayed must be a task. Direct prose is source evidence, not schedulable work:

```bash
merl task request --team software \
  --summary 'Review parser memory safety' \
  --needed-by 2026-10-01

merl message send \
  --to project-a-reviewer \
  --summary 'Please look at the unsafe block in the parser' \
  --ref T45 \
  --ref github:acme/project-a/pull/229
```

The structured task is the state change. It does not need an extraction model. The authored note becomes an immutable source event with one or more delivery records and follows the binding's compilation policy. Delivery itself accepts nothing. A PM defers `T45`, not the message.

Cold notes appear without their body in ordinary inbox and role views:

```text
M91 from project-a-researcher
refs: H4 E37
payload: 3.8k chars, not loaded
compilation: on_demand, not compiled
coverage: optional
```

The recipient can read the source without compiling it, or request semantic extraction:

```bash
merl message read M91
merl source compile --project P1 --database project.sqlite --version SE91 \
  --run CR91 --actor pm --reason 'Required for T42' \
  --program ./compiler-adapter --compiler-version v1 \
  --model MODEL --prompt-digest sha256:DIGEST
```

An authorized compile request records the effective policy, requester, reason, and chosen compiler. One project-scoped run serves every delivery and later session while its source and context remain applicable.

An optional note does not block a completeness claim, even when structural metadata links it to a task. If its semantics are required, an authorized actor promotes it for a recorded scope:

```bash
merl source require --project P1 --database project.sqlite --version SE91 \
  --scope task:T42 --actor pm --reason 'Required safety evidence'
merl source compile --project P1 --database project.sqlite --version SE91 \
  --run CR91 --actor pm --reason 'Required for T42' \
  --program ./compiler-adapter --compiler-version v1 \
  --model MODEL --prompt-digest sha256:DIGEST
```

Both commands pass through administrative policy. An untrusted actor cannot change coverage or start the compiler. An accepted requirement follows later versions of the same source. An accepted compile request records the actor, reason, source version, run, compiler, model, and prompt digest before external work begins. Coverage reports each current version as a required gap until the authority compiles it or an authorized policy action excludes it. Project-significant actions should normally use structured commands rather than rely on optional prose.

`in_reply_to` preserves conversation history but does not close a question, finding, or task. Those objects change only through semantic commands or accepted assertions such as `answers Q41`, `updates T45`, or `disputes C9`. One note may address several objects, and several notes may address one object.

Merl-generated inbox text carries its originating command or domain-event batch ID and never enters semantic compilation. One origin may create several recipient deliveries. Manually similar notes are not merged unless they share strong lineage. Cross-project work uses versioned requests and exports, not direct messages.

## Workspace safety

Merl treats agent workspaces as local operational state. Assignment can create one automatically:

```bash
merl task assign T52 --to project-a-dev --workspace auto

merl agent register project-a-reviewer --role reviewer
merl workspace check
merl workspace create \
  --agent project-a-dev \
  --task T52 \
  --source github:acme/project-a \
  --strategy worktree
merl workspace list
```

The default layout uses sibling worktrees under `MERL_HOME`:

```text
~/.merl/git/R1.git/
~/.merl/workspaces/project-a/T52-project-a-dev/
~/.merl/workspaces/project-a/T53-project-a-senior-dev/
```

The two worktrees share Git objects. They have different files, indexes, task branches, and current working directories. Merl does not create one agent's worktree inside another checkout.

`workspace check` canonicalizes paths and inspects Git worktree identity. It reports two active agents using the same writable checkout even when one path reaches it through a symbolic link. Commands that assign concurrent write work fail until Merl creates a separate workspace.

Read-only review uses a detached worktree at a pinned commit:

```bash
merl workspace create \
  --agent project-a-reviewer \
  --task T52 \
  --source github:acme/project-a \
  --at a83c1 \
  --read-only
```

Several readers may share that immutable checkout only when the host enforces read-only access. A reviewer never shares a writer's live checkout. A reviewer that may edit receives a separate branch and writable workspace.

Worktree is the default because it avoids duplicate Git objects. A full clone is an explicit or policy-selected fallback:

```bash
merl workspace create \
  --agent project-a-dev \
  --task T52 \
  --source github:acme/project-a \
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
  --to project-b \
  --kind infrastructure.change/v1 \
  --summary 'Provision storage for session two captures' \
  --needed-by 2026-10-01 \
  --impact-if-late 'Session two capture is blocked' \
  --snapshot D18@4 \
  --live T42
```

The result distinguishes local acceptance from remote delivery:

```text
ACCEPTED project-a:X17 at project-a@813
DELIVERY PENDING envelope XE52 to project-b
```

The target sees its own imported aggregate. Project B can defer triage without accepting responsibility:

```bash
merl request inbox --project project-b
merl request show project-b:XR91
merl request defer project-b:XR91 \
  --reason 'Higher-priority infrastructure work' \
  --review-at 2026-10-01
merl request need-info project-b:XR91 --question 'Required retention period?'
```

Alternatively, Project B can accept the request and schedule its work:

```bash
merl request accept project-b:XR91
merl request schedule project-b:XR91 --target-start 2026-10-15
merl request start project-b:XR91 --work T91
merl request fulfill project-b:XR91 --ref github:acme/infra#412
```

The origin observes those updates without gaining authority over Project B's work:

```text
project-a:X17
  local: submitted
  requested need: 2026-10-01
  Project B commitment: pending
  Project B scheduling: deferred; review 2026-10-01
  Project B reason: higher-priority infrastructure work
  Project B delivery commitment: none
```

If Project B accepts the request but schedules it later, the view says `commitment: accepted` and shows Project B's target dates. Transport receipt, acknowledgement, and a review date never render as acceptance or a delivery promise.

The origin can amend or withdraw its request:

```bash
merl request amend project-a:X17 --summary 'Need 30-day retention'
merl request withdraw project-a:X17 --reason 'Experiment cancelled'
```

Withdrawal asks the target to stop. It does not mark Project B's work cancelled.

Project administrators manage directional links:

```bash
merl link grant \
  --from project-a \
  --to project-b \
  --contract infrastructure.change/v1 \
  --export 'artifact:A81' \
  --export 'decision:D18'

merl link show project-a --to project-b
merl link revoke project-a --to project-b --contract infrastructure.change/v1
```

A target rejects unsupported contract versions and unauthorized exports without trying to infer their meaning.

## GitHub interface

Merl publishes little to GitHub.

Several projects may ingest one Issue, but a project administrator assigns each publication slot to one project:

```bash
merl publication slot assign \
  github:acme/project-a#204/merl \
  --project project-a

merl publication slot show github:acme/project-a#204/merl
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
Blocked: waiting on Project B infrastructure request X17

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
merl projection show github:acme/project-a#204/merl-status
merl projection restore github:acme/project-a#204/merl-status
```

## Evaluation

The evaluator-side `merl-eval` tool compares Merl with practical alternatives under one recorded model configuration. Its plan names the five methods, paired trials, external model and scorer adapters, and independently prepared Merl authorities:

```bash
merl-eval run --plan /path/to/frozen-evaluation-plan.json
```

The report records model and version, reasoning effort, prompts, available tools, sampling controls, per-trial answers, correctness, total token cost, and variance. It identifies the read count where each approach becomes cheaper than repeated raw-history consumption. A hindsight compilation cannot enter a causal Merl trial. See the [benchmark harness](evaluation/benchmark-harness.md) for the process protocol and held-out gate.

## Errors and non-interactive use

Human errors state what happened, what remained unchanged, and the next safe action. Machine errors have stable codes and structured details.

Non-interactive commands never prompt. Ambiguous context, missing authority, stale basis revisions, and required confirmation produce explicit results. Merl does not silently select a project, drop a command, or reinterpret an unsupported contract.

Exit success means Merl durably performed the action it reported. For a queued offline command, success means the command is safely queued. Callers that require accepted state use `--require-accepted`.

## Behavior-driven test contract

Gherkin is the initial behavior language. It is readable by users and gives the implementation room to change. We should not create another test language until repeated scenarios expose a concrete limitation.

Scenarios interact through public actions and observable results. They do not query SQLite tables or call private Rust modules. The same scenario can run through several internal test drivers:

- CLI subprocess against a local daemon
- application service in process
- client against a shared authority

Provider scenarios use controlled GitHub and artifact-store fakes at the external boundary. The runner supplies deterministic time and aliases for generated IDs while assertions use public output.

Useful tags include `@first_release`, `@local`, `@shared`, `@offline`, `@github`, `@cross_project`, and `@recovery`. The [first release plan](plans/first-release.md) owns which scenarios carry `@first_release`; this larger contract also describes deferred behavior.

### Project context is never guessed

```gherkin
Feature: Select project context

  Scenario: One repository is attached to two projects
    Given repository "acme/project-a" is attached to projects "project-a" and "project-b"
    And no local workspace context is selected
    When I run "merl project view" without an interactive terminal
    Then the command fails with code "project_context_ambiguous"
    And the result lists projects "project-a" and "project-b"
    And neither project is read or changed

  Scenario: An explicit project resolves the ambiguity
    Given repository "acme/project-a" is attached to projects "project-a" and "project-b"
    When I run "merl project view --project project-a"
    Then I see Project A's accepted revision
```

### Local mode states its trust boundary

```gherkin
@first_release
Feature: Explain the local trust boundary

  Scenario: Local mode does not claim to sandbox same-user processes
    Given project "project-a" uses a local authority
    When I inspect its security model
    Then Merl reports that unrestricted same-user processes are trusted
    And it identifies local policy as governance and audit
    And it does not claim that local files are protected from those processes
```

### Offline work reports queued state

```gherkin
Feature: Submit a command while offline

  Scenario: A disconnected client queues a mutation
    Given shared project "project-a" is accepted through revision 481
    And its authority is unavailable
    When I resolve question "Q32" with decision "D18"
    Then the result is "queued"
    And it includes a durable command ID
    And project revision 481 remains the latest accepted revision

  Scenario: Automation requires immediate acceptance
    Given shared project "project-a" is accepted through revision 481
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
    Given project "project-a" is accepted through revision 481
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
    Given project "project-a" has accepted decision "D18"
    When I view issue "github:acme/project-a#204" as a researcher
    Then the view contains the meaning of decision "D18"
    And the view does not contain the full source thread
    When I expand decision "D18" to its source
    Then I receive the captured source and provenance

  Scenario: A negative answer exposes incomplete semantic coverage
    Given issue "github:acme/project-a#204" has no accepted blocker
    And required source "SE804" is captured but uncompiled
    And optional source "SE900" is cold and linked to task "T42"
    When I view the issue as a researcher
    Then the view says there is no accepted blocker
    And it says one required source remains uncompiled
    And it lists "SE900" as an optional cold attachment
    And it does not count "SE900" in the coverage gap

  Scenario: An eager Issue is visibly caught up
    Given required eager Issue sources are observed through sequence 771
    And every required source through sequence 771 compiled successfully
    When I inspect Issue coverage
    Then the observation head is 771
    And the contiguous processed cutoff is 771
    And the view reports no required gaps

  Scenario: Optional cold evidence does not defeat completeness
    Given every required source through sequence 771 processed successfully
    And optional source "SE900" remains cold and is linked to task "T42"
    When I inspect task "T42" coverage
    Then the view reports required coverage as complete
    And it lists "SE900" as optional cold evidence
```

### Compiler output stays bounded

```gherkin
@first_release
Feature: Enforce the compiler protocol

  Scenario: An over-budget response fails without partial semantics
    Given a compilation run permits at most 12 assertions
    When its compiler returns 13 assertions
    Then the run fails with code "compiler_output_budget_exceeded"
    And no assertion from that response enters policy
    And Merl does not silently truncate the response

  Scenario: Explanatory prose is outside the normal response schema
    Given a compiler response contains a rationale essay or copied source text
    When Merl validates the response
    Then validation fails with code "compiler_response_invalid"
    And no chain-of-thought is requested or retained
```

### Evaluation compares practical alternatives

```gherkin
@first_release
Feature: Measure the value of compiled state

  Scenario: A held-out run reports paired baseline results
    Given a frozen held-out corpus
    And one recorded model, effort, prompt, tool, and sampling configuration
    When I run five paired trials for raw history, rolling summary, recent retrieval, summary plus retrieval, and Merl
    Then the report shows correctness, total token cost, and variance for each method
    And it shows the measured break-even read count
    And it labels hindsight runs and excludes them from causal replay results
```

### A new comment produces a pollable delta

```gherkin
@first_release
Feature: Read an incremental project change

  Scenario: An agent receives references instead of the full thread
    Given agent "project-a-dev" has read project revision 481
    And a new Issue comment produces accepted revision 482
    When the agent polls its inbox
    Then one entry identifies revision 482 and the changed object references
    And the entry does not contain the full Issue history
    When the agent acknowledges the entry
    Then its durable cursor advances through revision 482
```

### Inbox receipt does not imply semantic acceptance

```gherkin
Feature: Consume one logical agent inbox safely

  Scenario: One session owns the actionable cursor
    Given logical agent "project-a-dev" has active sessions "S18" and "S19"
    And session "S18" holds the inbox lease
    When session "S19" tries to claim entry "IN123"
    Then the command fails with code "inbox_lease_not_owned"
    And the agent cursor does not advance

  Scenario: Acknowledgement records receipt only
    Given inbox entry "IN123" refers to open task "T45" and question "Q41"
    When the consuming session acknowledges "IN123"
    Then the inbox records the acknowledgement
    And task "T45" remains open
    And question "Q41" remains unresolved
```

### Direct prose is compiled, not trusted

```gherkin
Feature: Derive state from an authored note

  Scenario: Supplemental prose enriches one structured action
    Given a task request and note were submitted atomically as command "CMD44"
    And the accepted task is "T45"
    When Merl later compiles the note
    Then the source links to command "CMD44" and task "T45"
    And compilation may add constraints, evidence, or corrections
    And policy does not create another task for the covered request

  Scenario: A note addresses several project objects
    Given an agent sends one note that reports evidence, answers "Q41", and requests work on "T45"
    When Merl captures and compiles the note
    Then the immutable source event has separate delivery records
    And its reply link records conversation provenance only
    And separate assertions address the evidence, question, and task
    And delivery accepts none of those assertions

  Scenario: Generated fan-out does not feed the compiler
    Given one accepted project change creates inbox entries for three subscribers
    When Merl renders the three notifications
    Then every notification references the same semantic origin
    And none becomes a new semantic source event

  Scenario: A sender cannot buy a compiler run
    Given direct notes use compilation mode "capture_only" and coverage "optional"
    When a sender marks a note as important or asks that it compile
    Then Merl captures the sender metadata
    And the binding policy remains "capture_only"
    And the coverage requirement remains "optional"
    And no compilation run is scheduled
```

### Compiler context is bounded and reproducible

```gherkin
@first_release
Feature: Compile an incremental Issue comment

  Scenario: Binding policy chooses eager compilation
    Given Issue comments on source "github:acme/project-a" use mode "eager" and coverage "required"
    When Merl captures a new human Issue comment
    Then the source event records the effective compilation policy
    And Merl schedules one project-scoped compilation run
    And no recipient-specific compilation run is created

  Scenario: Capture-only prose stays cold
    Given bot comments on source "github:acme/project-a" use mode "capture_only" and coverage "optional"
    When Merl captures a bot comment
    Then Merl retains its payload without scheduling compilation
    And ordinary project and inbox views omit the payload body
    And the bot comment does not create a required coverage gap

  Scenario: An authorized request compiles cold evidence
    Given source event "SE91" was captured without compilation
    When an authorized user requests compilation of "SE91"
    Then Merl records the requester, reason, effective policy, and compiler
    And later role views reuse the resulting derivation

  Scenario: A comment uses earlier context
    Given issue "204" has current decision "D18" and open question "Q32"
    And comment "SE771" says "Do that after session two"
    When Merl builds a compilation context for "SE771"
    Then the context records its interpretation-basis revision
    And it records the selected object revisions and recent source events
    And it records the selector, renderer, budget, and rendered-input digest
    And it does not contain the full Issue history

  Scenario: Late compilation separates interpretation from policy
    Given source "SE100" was observed at sequence 100 and project revision 72
    And the project is now at observation 900 and revision 500
    When an authorized user requests compilation of "SE100"
    Then the compilation context uses interpretation-basis revision 72
    And its source-observation cutoff is 100
    And it contains no later source or accepted state
    And the resulting policy evaluation uses `basis_project_revision` 500

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

  Scenario: Historical replay cannot see the future
    Given comment "SE10" says "Keep gain fixed"
    And later comment "SE12" says "Correction: sweep gain"
    When Merl replays comment "SE11" between them
    Then its context cutoff ends at "SE11"
    And its basis state does not contain information derived from "SE12"

  Scenario: Missing provider history is labeled as hindsight
    Given an edited comment has no retrievable prior version or observation order
    When Merl compiles the historical thread with the current comment body
    Then the run mode is "hindsight"
    And the run is excluded from causal replay results
    And its assertions cannot enter accepted state without promotion

  Scenario: One comment contains several semantic acts
    Given a comment reports an experiment failure, infers that a hypothesis weakened, and requests a software task
    When Merl compiles the comment
    Then it emits separate assertions for the report, inference, and request
    And each assertion records its speech act, epistemic basis, polarity, confidence, and source span
    And policy may give the assertions different dispositions

  Scenario: A relay does not transfer authority
    Given an agent comment attributes a destructive instruction to an authorized human
    And no authenticated source contains the human's instruction
    When Merl evaluates the extracted assertion
    Then the agent is the message author
    And the human attribution is unverified
    And policy does not treat the comment as the human's authorized command

  Scenario: Relative time preserves its basis
    Given a comment authored at a recorded time and timezone says "tomorrow"
    And the same comment says "after PR 229 merges"
    When Merl compiles the comment
    Then the date resolves from the recorded author context
    And the original expression remains in provenance
    And the pull request phrase becomes an event predicate rather than a guessed date
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

### Execute evidence revalidation

List pending evidence work after an edit, observed deletion, or purge:

```sh
merl project revalidation list --project P1 --database project.sqlite --json
```

Each entry names the impact, affected object and support event, earlier run,
trigger, changed source, replacement, and next action. It also reports unavailable
evidence and the latest reserved attempt, including pending or failed work. Pages
contain up to 100 entries. Pass `next_after` as `--after` to continue.

A command actor can execute one affected derivation:

```sh
merl project revalidation run --project P1 --database project.sqlite \
  --id impact_123 --actor reviewer --program ./compiler \
  --compiler-version v1 --model configured-model --prompt-digest sha256:<hex>
merl source assertions --project P1 --database project.sqlite --run impact_123
merl compilation context --project P1 --database project.sqlite --run impact_123
```

The run uses the latest versions of the recorded sources and inherits the original
limits. It records hindsight interpretation. Live coverage remains unchanged.
Retrying the run ID reuses its result or resumes its saved input after interruption.
A failed or stale attempt remains in history; pass a fresh `--run` to try again.
Use `compilation expand` for a recorded context request, then review the successful
successor. The compiler cannot resolve the impact itself.

Review the result through current command authority:

```sh
merl project revalidation resolve --project P1 --database project.sqlite \
  --id review_123 --impact impact_123 --actor reviewer \
  --action confirm --run impact_123 --assertion-index 0
```

| Action | Required evidence | Accepted effect |
| --- | --- | --- |
| `confirm` | Successful run and positive assertion matching subject, kind, and value reference | Fresh support for the same object |
| `weaken` | Successful run | Withdraw this support; keep the object active |
| `supersede` | Successful run and positive assertion naming a new subject of the same kind | New object and `supersedes` relation; old object becomes superseded |
| `invalidate` | Successful run | Old object becomes invalidated |
| `unavailable` | Erased or bodyless evidence; omit run and assertion index | Close the work as unsupported |

A run may be the original hindsight attempt or its completed expansion successor.
Confirmation and supersession require `--assertion-index`; the other actions omit
it. Weakening can leave an object partially supported if it has independent evidence.
An unavailable result does not restore erased content or claim successful compilation.

Resolution records use `merl.revalidation-resolution/v1`. They include the review
request, impact, action, policy evaluation, disposition, reason, and accepted
revision when present. The list and run schemas are `merl.revalidation-work/v1`
and `merl.revalidation-run/v1`. Reusing a request with changed content returns
`POLICY_INPUT_CONFLICT`; an exact retry returns its prior outcome, including a
rejection or conflict. A fresh request asks policy to reconsider under current state.
`COMPILER_UNAUTHORIZED` rejects execution without command authority.
`REVALIDATION_RUN_INELIGIBLE` rejects incomplete runs and runs outside the
impact's hindsight chain.

`show <object> --source --history` includes the reviewed impact and revised
compiler provenance. A support resolution advances the normal revision, delta,
and inbox together. Grant revocation, source changes or purge, target changes,
and a competing resolution prevent stale prepared work from committing.

### Changed evidence triggers reconsideration

```gherkin
@first_release
Feature: Maintain truth after source changes

  Scenario: An edited source requires support revalidation
    Given active decision "D18" derives from source event "SE10"
    And "D18" has support status "current"
    When source event "SE11" supersedes "SE10"
    Then observed assertions from "SE10" remain unchanged
    And Merl appends an evidence-impact record
    And decision "D18" remains active
    And its support status becomes "revalidation_pending"

  Scenario: A cosmetic edit preserves support
    Given decision "D18" is active with support awaiting revalidation
    When recompilation finds no material semantic change
    Then decision "D18" remains active
    And its support status becomes "current"

  Scenario: A material correction supersedes the old decision
    Given decision "D18" says to keep gain fixed
    And its supporting source is corrected to require a gain sweep
    When policy accepts the new assertion
    Then a new decision supersedes "D18"
    And the old source, assertion, evidence impact, and policy decision remain auditable
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
    And the deterministic provider observation advances the project revision
    And subscribed agents can read the change through the normal delta and inbox cursor

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
    And the result names every covered Merl-managed retention scope
    And the result excludes provider systems, unmanaged backups, and prior exports from its erasure claim

  Scenario: Equal payloads in separate scopes remain independently erasable
    Given two retention scopes store the same protected bytes
    When an administrator purges the first scope
    Then the first payload becomes unavailable
    And the second payload remains available
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
    Given projects "project-a" and "project-b" both ingest issue "204"
    And project "project-a" owns its Merl publication slot
    When project "project-b" attempts to publish to that slot
    Then the command fails with code "publication_slot_not_owned"
    And no GitHub write is attempted

  Scenario: Another authority recognizes Merl output
    Given project "project-a" published a verified Merl comment to issue "204"
    And project "project-b" also ingests issue "204"
    And Project B has a trusted mapping from Project A to its publication key or authority
    When Project B captures the comment
    Then Project B records the stable publication and publishing-project identities
    And Project B does not compile the generated prose as ordinary source
    And a shared provider actor or hidden marker by itself would not be sufficient verification
```

### Cross-project work preserves authority

```gherkin
Feature: Request work from another project

  Scenario: Origin state and envelope commit together
    Given project "project-a" may send "infrastructure.change/v1" to project "project-b"
    When Project A submits an infrastructure request
    Then Project A accepts an OutboundRequest
    And a pending envelope exists for the same origin transition
    And both appear in one Project A revision

  Scenario: The target owns its work
    Given Project B durably received Project A's request "X17"
    When Project B accepts the request and creates task "T91"
    Then task "T91" belongs to Project B
    And Project A observes a reference to "T91"
    And Project A cannot change the task owner or priority

  Scenario: The target defers without making a commitment
    Given Project A needs request "X17" by 2026-10-01
    And Project B has not accepted responsibility for its inbound request
    When Project B defers the request until review on 2026-10-15
    Then Project A sees Project B's commitment as "pending"
    And Project A sees Project B's scheduling as "deferred"
    And Project A sees the reason and review date
    And Project A does not see a delivery commitment
    And Project A sees that the current plan cannot meet its requested date

  Scenario: A repeated envelope does not duplicate work
    Given Project B processed envelope "XE52" into inbound request "XR91"
    When Project B receives envelope "XE52" again
    Then Project B returns the stored receipt
    And no second inbound request or task is created

  Scenario: A legitimate causal loop is allowed
    Given correlation "X17" traveled from Project A to Project B to Project C
    When Project C asks Project A a new question in correlation "X17"
    Then Project A accepts the question envelope
    And Merl does not reject it because Project A appears earlier in the lineage
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
    Given software agent "project-a-dev" is executing task "T44"
    When the project manager defers task "T44" without pausing it
    Then the command fails with code "task_in_progress"
    And task "T44" remains "in_progress"
```

### Session continuation uses current state

```gherkin
Feature: Resume an agent session

  Scenario: A fresh context receives a bounded bootstrap
    Given agent "project-a-dev" has active guidance and an actionable assignment
    When its host starts a fresh model context
    Then the injected instruction contains the agent's `merl session resume` command
    And it points to hierarchical help
    And it contains neither the durable guidance nor the command catalog
    When the agent runs the resume command
    Then the resume view contains its relevant guidance and assignment

  Scenario: Referenced state changed after checkpoint
    Given agent "project-a-tech-lead" checkpointed at revision 481 with reference "D18"
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
    Given agents "project-a-dev" and "project-a-reviewer" use the same writable directory
    When Merl checks workspace safety
    Then it reports a writable workspace collision
    And Merl refuses a concurrent write assignment
    And it offers to create a separate workspace

  Scenario: Two writers receive separate worktrees
    Given agents "project-a-dev" and "project-a-senior-dev" have writable assignments in repository "acme/project-a"
    When Merl creates their workspaces
    Then each agent receives a different working directory and index
    And each agent receives a unique task branch
    And both worktrees may share the same Git object store

  Scenario: A reviewer does not share a writer's live checkout
    Given agent "project-a-dev" holds a writable workspace
    When agent "project-a-reviewer" requests a read-only review at its current commit
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
    Given node "old" has agent "project-a-dev" with active TDD practice "AG18"
    And node "old" has a pending command "CMD91"
    When I export node "old" and import it on a new computer
    Then agent "project-a-dev" keeps its logical identity
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
    Given agent "project-a-dev" has an active practice to use TDD
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
    Given agent "project-a-dev" uses "task-scoped" context policy
    And generation 18 is bound to task "T44"
    And the agent has an active TDD practice
    When task "T44" is completed with its result references
    Then Merl accepts the Task outcome and closes its Assignment separately from the session
    And Merl durably closes generation 18 before requesting a context reset
    And session closure records the checkpoint, cursor, and lease releases
    When the agent starts unrelated task "T52"
    Then generation 19 contains the active TDD practice
    And generation 19 contains current state for task "T52"
    And generation 19 does not contain the transcript from generation 18

  Scenario: A project manager keeps continuous context
    Given agent "project-a-pm" uses "continuous" context policy
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
    Given "project-a-senior-dev" advertises model "anthropic:opus" at high effort and high cost
    And "project-a-dev" advertises model "anthropic:sonnet" at medium effort and medium cost
    And task "T52" requires Rust and high reasoning for a high-risk change
    When the project manager lists candidates for task "T52"
    Then "project-a-senior-dev" is eligible
    And "project-a-dev" is ineligible because it misses the reasoning requirement
    And neither advertisement grants new source access

  Scenario: Routine work favors the cheaper eligible runtime
    Given both developer sessions meet task "T53" requirements
    And task "T53" prefers lower cost
    When the project manager lists candidates for task "T53"
    Then "project-a-dev" ranks before "project-a-senior-dev"
    And the result explains the cost tradeoff
    And no assignment exists until an authorized actor chooses one

  Scenario: A stale advertisement is not eligible
    Given "project-a-senior-dev" last advertised availability before its current session
    When the project manager lists candidates for a new task
    Then "project-a-senior-dev" is excluded as stale
```

### A PM can provision only approved agents

```gherkin
Feature: Delegate agent provisioning

  Scenario: A PM spawns an approved developer
    Given a human delegated template "software-dev-medium" to "project-a-pm"
    And the template allows two active agents with 10 GiB each
    When "project-a-pm" spawns an agent for task "T53"
    Then Merl durably accepts one spawn request
    And Merl reserves the workspace and scratch allowance before calling the host
    And the agent is unavailable until the host reports a ready session

  Scenario: A PM cannot widen its delegation
    Given "project-a-pm" may spawn only template "software-dev-medium"
    When it requests another model or a third active agent
    Then the request fails with code "delegation_exceeded"
    And no host provisioning call occurs

  Scenario: An ambiguous host response does not duplicate an agent
    Given spawn request "SP19" may already have reached the host
    When provisioning retries after a lost response
    Then Merl reconciles using provisioning identity "SP19"
    And at most one host agent belongs to the request

  Scenario: An urgent stop survives host failure
    Given session "S18" is executing task "T53"
    When an authorized PM urgently stops the agent
    Then cancellation and interrupt requests commit with a durable host action
    And host interruption begins only after commit
    When host delivery fails
    Then the interrupt remains pending and retryable
    And Merl does not report the process stopped or task cancelled
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

Step definitions should call a small behavior-driver interface rather than shelling out from every step. A CLI driver renders actions as commands and parses public results. Internal drivers may call the corresponding application service to test the same behavior below the process boundary.

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
