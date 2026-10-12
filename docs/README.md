# Documentation

Start with the [product description](product-description.md): persistent teams of humans and heterogeneous agents, with less manual context copying and repeated token spend. It covers shared project understanding, agent continuity, and the operations needed to run the team.

The [roadmap](roadmap.md) shows the release sequence. Each plan owns the scope and acceptance criteria for its release:

- [First release](plans/first-release.md): test whether agents can use compact shared Issue state without losing correctness, and measure when repeated reads recover its preparation cost.
- [Phase 4](plans/single-project-agent-operations.md): maintain an existing repository and operate manually started agents with assignments, worktrees, continuity, and pull request review.
- [Phase 5](plans/delegated-agent-management.md): let an authorized manager provision and control agents within human-approved limits.

Use the [UI behavior contract](user-interface.md) for observable commands and outcomes. The [architecture](architecture.md) defines the state model, authority boundaries, and transaction rules.

Supporting material lives in:

- [`evaluation/`](evaluation/) for the corpus and benchmark harness;
- [`reference/`](reference/) for wire formats and protocol details;
- [`adr/`](adr/) for decisions that constrain implementation.
