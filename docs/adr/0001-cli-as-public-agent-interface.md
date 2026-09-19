# ADR 0001: Use the CLI as the public agent interface

Status: accepted

Date: 2026-09-18

## Context

Merl exists partly to reduce the context agents spend on coordination. Its interface must not recreate that cost by loading a large tool catalog or a second set of operation descriptions into every session.

Coding and research agents already run commands, inspect exit status, and parse standard output. They can discover a CLI incrementally when its help is concise and predictable. Merl also needs structured results, stable errors, non-interactive execution, and remote-authority support; none of those requires a separate agent protocol.

A required Merl skill would duplicate CLI documentation and consume context before the agent knows whether it needs Merl. Durable agent guidance avoids repeated configuration, but a fresh model context cannot retrieve that guidance until it knows how to resume its Merl identity.

We considered three public agent interfaces. MCP was the concrete protocol adapter under discussion:

- a CLI plus complete usage documentation;
- a separate protocol adapter with discoverable tool schemas;
- a required agent skill that wraps or teaches the CLI.

## Decision

The `merl` CLI is the sole public agent interface.

It provides:

- hierarchical human help;
- searchable command discovery;
- versioned machine-readable help for one command at a time;
- versioned JSON results;
- stable outcomes, error codes, and exit behavior;
- non-interactive and dry-run modes;
- the same commands for local and remote project authorities.

Merl does not require an installed agent skill. Each new host-managed model context receives one short bootstrap instruction:

```text
Run `merl session resume --agent <id> --format json` before work; use `merl help <topic>` when needed.
```

The logical agent's durable profile stores its guidance once. `session resume` renders the relevant guidance, assignments, checkpoint references, and project delta into the new context. An externally started agent receives the same bootstrap line from `merl agent attach`.

Internal application services remain independent of CLI parsing. This keeps the domain testable and allows graphical or administrative clients to reuse application behavior. It is not a commitment to publish another agent protocol.

## Consequences

Agents pay only for the help and results they request. Merl has one public command vocabulary, one machine-result contract, and one body of usage documentation to maintain. Shell scripts, CI, and agents exercise the same interface.

The CLI must compensate for the discovery and parsing features another protocol might provide. Incomplete help, unstable JSON, vague errors, or interactive surprises are release defects. Remote use also needs a CLI client that handles authentication without exposing long-lived credentials to the model process.

Non-terminal environments do not receive a first-class agent interface from Merl under this decision. A future adapter requires measured demand, a threat model, and evidence that its context and operational costs improve the intended workflow. It must reuse application services rather than define another domain model.

## Evidence

The [Tyk comparison of MCP and CLI](https://tyk.io/learning-center/mcp-vs-cli-for-ai-agents-enterprise-comparison-guide/) describes CLI as the better fit for developer-focused coding agents and token-constrained work, while assigning the other approach advantages in centralized authentication, non-developer access, and browser or mobile clients. Merl's initial users and evaluation goals fall on the CLI side of that boundary.
