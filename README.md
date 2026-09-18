# Merl

Merl gives teams of coding and research agents a shared, durable understanding of their work.

GitHub threads, review comments, research notes, and agent conversations hold useful information, but agents pay to reconstruct that information each time they read them. Merl captures the source material, compiles it into provenance-backed project state, and sends each agent a small view of the changes relevant to its role. An agent can expand any decision, finding, claim, or task back to the evidence behind it.

Merl tracks software work such as issues, pull requests, requirements, contracts, and review findings. For research, it tracks hypotheses, experiments, claims, and evidence. A separate control plane handles assignments, inboxes, subscriptions, leases, checkpoints, and handoffs without mixing runtime details into project knowledge.

The first release will run locally as a Rust CLI and daemon backed by SQLite. GitHub is the first source integration. A thin MCP adapter will expose the same operations once the command-line interface and state model have settled.

Merl is under active design. Read the [full product description](docs/product-description.md) and the [state architecture notes](docs/agent-github-state-architecture.md).

## License

Merl is available under the [MIT License](LICENSE).
