# Contributing to Merl

Thanks for taking the time to contribute. Merl is early in development, so bug reports, design feedback, documentation fixes, test cases, and code are all useful.

## Before opening an issue

Read the [README](README.md) and search the [issue tracker](https://github.com/infagent/merl/issues) first. An existing issue may answer the question or provide a better place to add context.

Merl's current implementation scope lives in the [first release plan](docs/first-release.md). The [product description](docs/product-description.md), [UI behavior contract](docs/user-interface.md), and [architecture](docs/architecture.md) describe the intended system beyond that release.

## Ask a question

Open an issue with a short, specific title. Include the document, command, or behavior that prompted the question and explain what remains unclear.

Please do not include credentials, private repository content, customer data, or proprietary research in a public issue.

## Report a bug

A useful bug report gives another person enough information to reproduce the failure. Include:

- the Merl version or commit;
- your operating system and Rust version;
- the command or sequence that failed;
- the expected and actual behavior;
- the smallest reproducible example you can provide;
- relevant logs with secrets and private content removed.

Search existing bug reports before filing a new one. If the problem involves a security vulnerability or sensitive data exposure, do not post the details in a public issue. Use GitHub's private vulnerability reporting for this repository when it is available.

## Suggest a change

Feature requests should start with the problem. Describe who encounters it, what they are trying to accomplish, and why the current behavior is insufficient. Include alternatives or prior art when they clarify the tradeoff.

Changes to accepted-state semantics, provenance, revision rules, or the public CLI contract need design agreement before implementation. A maintainer may ask for an ADR or a narrower first step.

## Contribute documentation

Documentation fixes can be as small as a corrected command, a clearer example, or a repaired link. For larger edits, explain which reader was getting stuck and what the revision should help them do.

Keep private project names and source material out of examples. Use neutral names such as Project A and Project B.

## Contribute code

Pick an open issue or discuss the change before investing in a large patch. The issue should identify its dependencies and milestone. Work that crosses milestones may need to be split.

Install the stable Rust toolchain with [rustup](https://rustup.rs/). The CI workflow is the source of truth for required components and checks. Once the Rust workspace is present, the usual commands are:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps
```

If you are capturing GitHub fixtures, install and authenticate `gh` separately. See the [corpus capture instructions](docs/evaluation-corpus.md#capture-dependency). Ordinary builds and tests do not require it.

Repository-specific engineering rules live in [AGENTS.md](AGENTS.md). They apply to human- and agent-authored changes alike.

## Open a pull request

Keep pull requests focused. Separate feature work from dependency updates, formatting sweeps, and unrelated cleanup.

In the pull request description:

- link the issue the change addresses;
- explain the user-visible result;
- describe how you tested it;
- call out schema, migration, compatibility, security, or token-cost effects;
- include sample human and JSON CLI output when either changes.

Use a conventional commit subject such as `feat:`, `fix:`, `docs:`, `test:`, `refactor:`, or `chore:`. Release Please uses these prefixes to prepare release notes and versions.

Maintainers may ask for revisions or split work before merging. Review focuses on observable behavior, compatibility, correctness, and fit with the current milestone.

## License

By contributing, you confirm that you have the right to submit the work and agree that it may be distributed under the repository's [MIT License](LICENSE).
