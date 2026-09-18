# Product description

## What Merl is

Merl gives a group of humans and agents a shared, durable understanding of a project. It compiles activity from GitHub, agent work, research runs, and local artifacts into accepted project state. When that state changes, Merl sends each agent a small delta suited to its role.

Today, agents collaborate by rereading prose. A software engineer may need an entire issue thread to discover the current requirement. A reviewer may receive comments for findings that another revision has already fixed. A research agent may have to reconstruct which experiment weakened a hypothesis. Long-lived agents also lose useful context when their sessions end.

This costs tokens and produces mistakes. Summaries help for a while, but repeated summaries drift from their sources. Merl keeps the original evidence and derives typed state from it. Agents read the current state first, then retrieve the source when they need nuance.

## Who it is for

Merl is for teams that use several coding or research agents on the same project. A team might include a project lead, researchers, software engineers, and reviewers, with humans working through GitHub alongside them.

Merl does not require every participant to use the same model or agent host. Claude Code, Codex, shell scripts, CI jobs, and people can work against the same state through a CLI, a local service, or a small MCP interface.

## How it works

Merl processes project activity in five layers.

1. A `SourceEvent` records what Merl observed, including an immutable copy of the source content. An edited GitHub comment creates another source event instead of rewriting the first one.
2. A versioned `CompilationRun` interprets one or more source events and emits `ObservedAssertion` records. These records describe what that compiler believed the source meant.
3. A `PolicyEvaluation` considers assertions together with accepted state at a specific project revision. It applies authority rules, detects conflicts, and records why each assertion was accepted, rejected, or held for review.
4. Accepted evaluations produce an atomic batch of `DomainEvent` records. The batch updates materialized state, advances the project revision once, and creates inbox entries for subscribed agents in the same database transaction.
5. Merl renders the new state as role-specific views and deltas. Wake-up is best effort; the durable inbox remains correct if a process crashes or a host integration fails.

Every derived record points back to its source, compiler, and policy version. Merl can replay old sources through a new compiler without changing the live project. A user can compare the results and promote selected assertions through a fresh policy evaluation.

## Project knowledge and agent operations

Merl separates two kinds of state because they have different lifetimes and consistency needs.

The knowledge plane contains the project's accepted knowledge and work:

- issues, pull requests, and tasks
- decisions, requirements, questions, and blockers
- facts, artifacts, findings, claims, and evidence
- hypotheses and experiments
- software contracts

The control plane records how agents carry out that work:

- agents and assignments
- inbox entries and subscriptions
- cursors and acknowledgements
- leases and wait conditions
- checkpoints and handoffs

Control records refer to knowledge records rather than copying them. A task remains part of the project after an agent's lease expires. A session checkpoint records the project revision the agent understood and references the relevant tasks, decisions, and artifacts.

Some control state expires. Other records, including handoffs, assignment history, failed deliveries, and acknowledgements, remain available for recovery and diagnosis.

## What agents receive

Agents should rarely need a full transcript. Merl assembles context from the current project view, unseen changes, and any details the agent requests.

A researcher might see active hypotheses, conflicting evidence, open research questions, and experiments awaiting analysis. An engineer might see accepted requirements, contract changes, blockers, and artifacts needed for the next task. A reviewer gets the pull request's intent, changed contracts, open findings, test results, and merge status.

After an agent has read project revision 481, the next notification may contain only:

```text
project 481 -> 482
changed: D18 Q32 E42
```

The agent can request the delta, expand one object, or read its source. Merl preserves the path from a compact view to the captured evidence.

## GitHub and research work

GitHub Issues and pull requests are Merl's first major source integration. A mature thread may contain thousands of tokens, mix active decisions with stale discussion, and serve several roles at once. That gives Merl a concrete benchmark.

Merl incrementally ingests new descriptions, comments, reviews, edits, and status changes. It materializes the current issue or pull request state without discarding the original text. Human comments remain ordinary GitHub comments; users do not need to write a formal language for Merl.

Merl applies the same model to research. It records hypotheses, experiments, claims, evidence, and invalidations as linked objects. An experiment can support one claim, weaken another, and leave the captured dataset available as an artifact. Research agents can inspect those relationships without rebuilding them from paragraphs at the start of each session.

## Authority and safety

Extraction does not grant authority. Merl distinguishes what a compiler observed from what the project accepts.

A deterministic fact read from a machine artifact may pass policy without review. An explicit decision from an authorized project owner may also become active at once. An agent's interpretation of research direction, a conflicting requirement, or a request to approve a merge may remain a candidate until the right person or policy accepts it.

Policy evaluations record the assertions they considered and the project revision they used as their basis. Before Merl commits the resulting domain events, it checks that the project has not advanced. If another transaction changed the state, Merl reevaluates the policy rather than applying a decision made against stale assumptions.

Merl stores source content locally because external comments can change or disappear. That content may include private code and research data, so Merl uses user-only filesystem permissions, redacts logs, and sends no telemetry by default.

## Interfaces and deployment

Merl starts as a local-first Rust application with one daemon and one SQLite database. The daemon owns writes during normal operation. This keeps project revisions, materialized state, and inbox delivery within one transaction boundary.

The `merl` CLI is the first public interface. It supports humans, agent skills, shell scripts, CI, replay tools, and evaluation harnesses. A thin MCP server will call the same core library after usage shows which operations deserve tools. GitHub and model providers remain adapters outside the domain core.

Merl stores its data under `MERL_HOME`, which defaults to `~/.merl`. The directory holds configuration, databases, captured artifacts, logs, sockets, and agent workspaces. Project and agent identities do not depend on directory names, so users can move a checkout without changing its history.

Compiler integrations use a versioned, language-neutral boundary. Rust implementations can run in the Merl process; experimental extractors may run as separate programs or services. They produce assertions and never receive direct write access to accepted state.

## Product boundaries

Merl keeps GitHub useful for people. It does not replace issues, pull requests, or human-readable discussion. It reduces how often agents must consume the full history.

Merl is also not a general chat system. Agents can send direct messages when a state change cannot express the request, but durable project objects carry most collaboration. A researcher updates a hypothesis or records a claim; the subscribed engineer receives the resulting delta.

Merl does not choose a model, run arbitrary agent loops, or give an extractor permission to change the project. Agent hosts decide how and when to invoke agents. Merl maintains state, applies authority policy, and delivers committed changes.

## First useful release

The first vertical slice uses one difficult historical GitHub Issue:

```text
captured Issue history
    -> compiled assertions
    -> policy evaluation
    -> accepted decisions, questions, facts, and research objects
    -> materialized role views
    -> one new comment
    -> one project revision and compact delta
    -> durable inbox entry
    -> agent wake-up
```

Before implementation, maintainers will freeze a small evaluation corpus of long Issues, pull requests, and research traces. Each fixture will include a human-reviewed expected state. Merl will measure extraction accuracy, provenance, delta correctness, task success, and total induced token cost against the raw-history baseline.

A compact view only counts as an improvement if agents still reach the right conclusion. Merl will include expansion calls, clarification turns, extraction work, and corrections in its token accounting.

## Current status

Merl is in architectural design. The next artifacts are an architecture decision record, a concrete SQLite schema, the frozen evaluation corpus, and the first Issue vertical slice.
