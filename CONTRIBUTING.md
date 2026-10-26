# Contributing to Merl

Merl is a Rust CLI for durable local agent messaging. Start with the [README](README.md), [product description](docs/product-description.md), [UI contract](docs/user-interface.md), [architecture](docs/architecture.md) and [roadmap](docs/roadmap.md). Architecture decisions are indexed in [docs/adr](docs/adr/README.md).

## Issues and changes

Search existing issues before opening one. Describe the concrete workflow, expected behavior and smallest reproducible example. Use synthetic content and remove credentials, private project information and runtime identifiers from public reports. Use GitHub's private reporting mechanism for sensitive security reports.

Treat the assigned issue as the unit of work. Agree on changes to the public interface or persistence/dispatch guarantees before implementation. Record a governing decision when product scope changes. Do not infer authorization to migrate or delete old runtime data from a source-code replacement.

## Contribute code

Use the toolchain pinned in `rust-toolchain.toml`. Follow [AGENTS.md](AGENTS.md), including red-green public-boundary scenarios. Root acceptance scenarios use Given/When/Then steps; mechanics and assertions live under `tests/helpers`. Tests use synthetic local hosts, controllable clocks and explicit paths. Ordinary checks require no live model, account credentials or Codex installation. Unix-socket scenarios run on Unix hosts.

Run the same complete checks as CI:

```sh
python3 scripts/check_test_conventions.py
python3 -m unittest discover -s scripts -p 'test_*.py'
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps
```

`cargo test --locked --test mail --test product` runs the public messaging and product-boundary scenarios. A manual real-host smoke additionally needs consenting existing sessions and an explicit Codex executable/socket; it is separate from automated tests. Preserve the working application/dispatcher during a binary transition and identify both binaries in the evidence.

`cargo test --locked --test install_version` checks the installed-binary version seam. Python discovery also runs synthetic installer and release-publication tests with local assets/Git repositories; no release is created. The Package workflow builds archives and verifies clean user-directory installs on all supported native platforms. See [release distribution](docs/release-distribution.md) for local package smoke and maintainer-only draft recovery. Do not publish a tag or release as a side effect of testing.

## Pull requests and review

Explain the final user-visible behavior, linked issue, validation, data/compatibility effects and material limits. Keep unrelated changes out. Document which head an independent reviewer checked and whether a live smoke used the replacement binary. Include example human/JSON output when it changes. Source deletion does not authorize runtime deletion.

Use conventional commits such as `feat:`, `fix:` and `docs:`. Mark an incompatible product replacement with `!` so release automation can account for it. Release Please maintains the release PR and changelog; do not publish or merge a release as an incidental implementation step.

`Rust checks` and `Test conventions` must pass before a PR can merge, including release PRs. The branch must contain current `main`. See [release gates](docs/release-gates.md). Merge permission comes from the human's instructions for the task; this replacement is handed back as a reviewed PR, not merged automatically.

## License

Contributions must be yours to submit and may be distributed under the repository's [MIT License](LICENSE). Verify source rights before copying third-party implementation. Role/worktree/merge policy belongs in [project guidance](docs/agent-roles.md), not reusable transport rules.
