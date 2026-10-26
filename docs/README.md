# Documentation

Merl is durable local messaging with wake-up of explicitly bound existing agent sessions.

- [Product description](product-description.md): users, workflow, scope and limits.
- [Installation](installation.md): binary releases, supported systems, version selection and optional skill setup.
- [First message](onboarding.md): disposable install-to-message walkthrough, optional wake and restart checkpoints.
- [Release distribution](release-distribution.md): frozen source, draft publication, checks and recovery.
- [Setup and recovery](agent-mail.md): register, bind, send, receipt, reply, complete, start and stop.
- [Reviewer lanes](reviewer-lanes.md): durable repository/label subscriptions, multi-PR activity and notification limits.
- [UI contract](user-interface.md): public commands, envelopes and error behavior.
- [Architecture](architecture.md): storage, concurrency, dispatch and trust boundaries.
- [Roadmap](roadmap.md): current release gates and later decisions.
- [Validation](agent-mail-validation.md): automated regressions and live smoke boundaries.
- [Project roles](agent-roles.md): optional shared/role templates, ownership, restart checkpoints and reviewed Markdown lessons.
- [Architecture decisions](adr/README.md): the current decision and superseded historical records.
- [Release gates](release-gates.md): build, branch protection and release infrastructure.

The former semantic/compiler/evaluation product and its active plans have been removed. Historical source is retained in Git; runtime data and external artifacts are untouched. The current product has no legacy data importer.
