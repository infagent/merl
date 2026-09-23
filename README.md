# Merl

Merl gives teams of coding and research agents a shared, durable understanding of their work.

Agents waste tokens rebuilding the same context from GitHub threads, research notes, and old conversations. Merl captures that material, compiles it into provenance-backed project state, and gives each agent a compact view of the changes relevant to its role. Any decision, finding, claim, or task can expand back to its evidence.

The state model covers software work and research. A separate control plane tracks assignments, inboxes, checkpoints, agent guidance, and runtime state. GitHub stays readable: Merl keeps routine machine activity internal, publishes significant events as ordinary prose, and maintains one bounded status view.

One project may span several repositories, and one repository may participate in several projects. Projects coordinate through permissioned requests and exports while retaining separate accepted histories.

Humans approve agent templates and resource limits. Project managers can staff work from those templates, choose between stronger and cheaper runtimes, and reset task-scoped agent context between unrelated assignments. Durable practices and checkpoints survive the reset.

On one computer, concurrent writers use separate Git worktrees and task branches. Merl gives each managed agent bounded, disk-backed scratch space instead of relying on the system `/tmp`. A portable manifest moves non-secret agent and workspace setup to another computer.

Merl ships as a Rust CLI and project authority. Local projects use SQLite; shared projects use the same application services behind a reachable authority. Agents discover commands through concise, hierarchical help and consume versioned JSON output when human-readable output is not appropriate.

Merl is under active design. Start with the [product description](docs/product-description.md), then read the [roadmap](docs/roadmap.md), [UI behavior contract](docs/user-interface.md), [architecture](docs/architecture.md), and [architecture decisions](docs/adr/).

## Contributing

Bug reports, design feedback, documentation fixes, and code contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) before opening an issue or pull request.

## License

Merl is available under the [MIT License](LICENSE).
