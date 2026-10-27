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
release_tools=$(mktemp -d)
npm install --prefix "$release_tools" --ignore-scripts --no-audit --no-fund --save-exact release-please@17.6.0
node scripts/check_release_policy.cjs "$release_tools/node_modules/release-please"
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps
```

The release-policy check needs Node 20+ and npm for an isolated temporary dependency install. Its synthetic source has no GitHub client or credentials. Node is a contributor check dependency, not a Merl build or installation requirement.

`cargo test --locked --test mail --test product` runs the public messaging and product-boundary scenarios. A manual real-host smoke additionally needs consenting existing sessions and an explicit Codex executable/socket; it is separate from automated tests. Preserve the working application/dispatcher during a binary transition and identify both binaries in the evidence.

The mail suite includes bounded inline notifications, explicit atomic selected-ID handling and accepted-versus-obsolete host-event races for #172. [Full-exchange validation](docs/mail-exchange-validation.md) gives the reproducible released/candidate synthetic comparison; it invokes no model and reports command/byte counts without claiming token savings.

`cargo test --locked --test announcements --test teams --test mail --test product` checks persistent shared scope/roster, individually owned qualified handoffs, restart/binding safety and legacy mail behavior with synthetic homes. `cargo test --locked -p merl-mail` also checks atomic join rollback/retry and raw identity collisions. The announcements suite also covers strict child-ID rollback, concurrent/lost-response retries, filtered selection and maximum-size snapshots through public CLI/API boundaries. No live account, model or host call is required.

`cargo test --locked --test setup_upgrade --test onboarding` covers pure usage and the canonical/legacy receipt transition, preflight ownership, interrupted recovery and safe v1 uninstall. Setup-upgrade scenarios drive the real candidate installer through a synthetic controlling terminal and inject unlink/directory-removal interruptions through the public Plan callback, without real user settings. The optional installed-host check is `python3 scripts/probe_skill.py --codex /absolute/codex --output /tmp/merl-skill-capture.json`; it uses isolated child user directories and a rejecting credential-free loopback mock, with no inference. The artifact proves loading/argument preservation, not semantic execution or Claude wake. See [Codex custom-provider configuration](https://learn.chatgpt.com/docs/config-file/config-reference) for the mock transport fields.

`cargo test --locked --test init` checks setup with synthetic user directories, terminal input, cancellation and ownership through public CLI/API boundaries. Python is also required for its controlled-terminal driver; no real user setup occurs.

`cargo test --locked --test install_version` checks the installed-binary version seam. Python discovery also runs synthetic installer and release-publication tests with local assets/Git repositories; no release is created. The Package workflow builds archives and verifies clean user-directory installs on all supported native platforms. See [release distribution](docs/release-distribution.md) for local package smoke and maintainer-only draft recovery. Do not publish a tag or release as a side effect of testing.

## Pull requests and review

For reviewer-lane changes, run `cargo test --locked --test qualified_lanes --test lanes --test lane_integrity --test launch_directory --test mail --test product` during development, then the complete checks above. The qualified-lane scenarios also cover same-named members/legacy recipients, frozen-batch immutability, explicit actor precedence and optional human display with unavailable metadata. These scenarios use synthetic GitHub replies and supplied clocks; ordinary tests need no gh authentication or live host. Read-only live acceptance is separate and must identify the exact candidate application/dispatch binaries, existing idle reviewer and watcher-originated wake. See [reviewer lanes](docs/reviewer-lanes.md).

Explain the final user-visible behavior, linked issue, validation, data/compatibility effects and material limits. Keep unrelated changes out. Document which head an independent reviewer checked and whether a live smoke used the replacement binary. Include example human/JSON output when it changes. Source deletion does not authorize runtime deletion.

Use conventional commits such as `feat:`, `fix:` and `docs:`. Mark an incompatible product replacement with `!` so release automation can account for it. Release Please maintains the release PR and changelog; do not publish or merge a release as an incidental implementation step.

`Rust checks` and `Test conventions` must pass before a PR can merge, including release PRs. The branch must contain current `main`. See [release gates](docs/release-gates.md). Merge permission comes from the human's instructions for the task; this replacement is handed back as a reviewed PR, not merged automatically.

## License

Contributions must be yours to submit and may be distributed under the repository's [MIT License](LICENSE). Verify source rights before copying third-party implementation. Role/worktree/merge policy belongs in [project guidance](docs/agent-roles.md), not reusable transport rules.
