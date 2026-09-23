# Roadmap

Status: draft

Merl's roadmap follows the product risks. The first release tests whether compiled project state saves tokens without losing meaning. It is a proof release, with deliberate Issue-by-Issue capture. Phase 4 is the first operational release: it maintains the active part of a repository and puts that state to work across a team that a human operates. Phase 5 lets an authorized manager staff and control the team.

The [product description](product-description.md) defines the full product. The [architecture](architecture.md) and [UI behavior contract](user-interface.md) define its semantics. The shorter plans below choose what to build and what evidence closes each phase.

## Release sequence

| Phase | Outcome | Plan |
| --- | --- | --- |
| 0 | Build, release, schema, and corpus foundations | [First release plan](plans/first-release.md) |
| 1 | One GitHub Issue reaches accepted state, views, and a pollable delta | [First release plan](plans/first-release.md) |
| 2 | Complete the public workflow and freeze the benchmark contract | [First release plan](plans/first-release.md) |
| 3 | Run held-out evaluation against the frozen candidate and make the release decision | [First release plan](plans/first-release.md) |
| 4 | Maintain one project and operate several manually started agents inside it | [Single-project agent operations](plans/single-project-agent-operations.md) |
| 5 | Let an authorized manager provision and control agents within human-approved limits | [Delegated agent management](plans/delegated-agent-management.md) |

Phases 0 through 3 form the first-release track. Phase 4 is the first multi-agent release and the first release intended for daily project use.

## Why Phase 4 comes before spawning

The original problem is repeated context, not process creation. A useful Phase 4 agent should enter a safe worktree, learn its assignment, receive a bounded view, and resume after a restart without rereading the project. Merl must also keep the active repository state current without asking a human to refresh every Issue. A human can still start the process.

This order tests Merl's multi-agent value before host provisioning complicates the result. It also gives Phase 5 one stable path to call after a host starts an agent.

```text
human selects logical agent
  -> assign accepted task
  -> prepare isolated workspace
  -> human starts process in that workspace
  -> attach session and resume bounded context
  -> receive compact deltas
  -> checkpoint and close
```

Phase 5 replaces the human launch with an authorized spawn request. Assignment, workspace setup, context assembly, delivery, and recovery stay the same.

## Evidence across releases

The first release measures repeated reading of Issue history. Phase 4 measures a working team. Its report should count context rendered for each assignment, expansions and source reads, restart cost, and repeated material shared across agents.

Phase 5 adds staffing evidence: provisioned runtime, declared capability and cost, reserved resources, measured usage, and task outcome. This history should help a manager choose agents from evidence instead of model branding.

## Work after Phase 5

The full product still includes work outside one local project authority:

- portable node setup and verified project authority transfer;
- shared or remote authorities;
- cross-project requests, contracts, exports, and authenticated transport;
- richer research and artifact integrations;
- optional graphical clients.

These areas need their own release plans once Phase 5 is working. The architecture describes them now so early schemas do not close off the later product.
