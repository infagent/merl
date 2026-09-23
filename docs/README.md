# Documentation

Start with the [product description](product-description.md). It explains the problem Merl solves and the product we intend to build.

The [roadmap](roadmap.md) shows the release sequence. Each plan owns the scope and acceptance criteria for its release:

- [First release](plans/first-release.md): compile GitHub Issue history into compact, provenance-backed state and measure the token break-even point.
- [Phase 4](plans/single-project-agent-operations.md): maintain an existing repository and operate manually started agents with assignments, worktrees, continuity, and pull request review.
- [Phase 5](plans/delegated-agent-management.md): let an authorized manager provision and control agents within human-approved limits.

Use the [UI behavior contract](user-interface.md) for observable commands and outcomes. The [architecture](architecture.md) defines the state model, authority boundaries, and transaction rules.

Supporting material lives in:

- [`evaluation/`](evaluation/) for the corpus and benchmark harness;
- [`reference/`](reference/) for wire formats and protocol details;
- [`adr/`](adr/) for decisions that constrain implementation.
