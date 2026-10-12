# Merl

Merl is infrastructure for running a persistent team of humans and agents around a shared project.

Running a team of agents can mean copying findings between conversations, repeating instructions after a restart, and paying several models to reread the same history. Merl aims to cut that token spend and manual coordination. Each agent receives the current state relevant to its role and assignment, plus changes since it last participated. It can expand a decision or finding back to its evidence when it needs more detail.

The team coordinates through shared project objects. A researcher records a finding; the engineer working on the affected task receives the change. A project manager accepts or defers work, and a reviewer sees current requirements and unresolved findings. Humans continue to discuss work through GitHub and decide who may act on the project's behalf.

An agent keeps its logical identity across disposable model sessions. Its assignment, guidance, checkpoint, and inbox cursor survive a context reset. A fresh session resumes from current project state without loading the previous transcript. Teams can mix Codex, Claude Code, and other hosts, with different models, capabilities, and costs.

The full product includes assignments and handoffs, isolated workspaces, agent selection and spawning, and resource limits. Humans approve templates and delegation boundaries; project managers staff and operate the team within them. Projects may span repositories and coordinate with other projects through permissioned requests.

Shared state must be trustworthy enough for agents to act on it. Merl separates model interpretations from accepted decisions, checks authority, and retains links to evidence. These mechanisms support the team's work and make errors inspectable.

Merl is under development as a Rust CLI with a local SQLite authority. The first release tests one foundation of the team workflow: whether agents can use compact Issue state as accurately as raw history while spending fewer tokens over repeated reads. Phase 4 adds persistent team operations for manually started agents. Phase 5 adds delegated provisioning and control. The [roadmap](docs/roadmap.md) distinguishes those release scopes from the full product described here.

Start with the [product description](docs/product-description.md), then read the [UI behavior contract](docs/user-interface.md), [architecture](docs/architecture.md), and [architecture decisions](docs/adr/).

## Contributing

Bug reports, design feedback, documentation fixes, and code contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) before opening an issue or pull request.

## License

Merl is available under the [MIT License](LICENSE).
