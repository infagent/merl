# Agent instructions

These instructions apply to the whole repository.

## Start with the contract

Read the documents that govern the work before changing code:

- `README.md` for the product boundary;
- `docs/architecture.md` for state semantics and invariants;
- `docs/user-interface.md` for observable behavior;
- `docs/first-release.md` for current scope and acceptance criteria;
- `docs/adr/` for decisions that should not be reopened casually.

Treat the assigned GitHub issue as the unit of work. Do not pull deferred features into an implementation because the architecture mentions them. If code and a governing document disagree, stop and surface the conflict.

## Build behavior first

Use red-green-refactor:

1. Write a failing example of the behavior.
2. Run it and confirm that it fails for the intended reason.
3. Add the smallest implementation that makes it pass.
4. Refactor while the tests stay green.

Test through public boundaries. Prefer a CLI invocation, public Rust API, or other user-visible seam over a private function or SQLite table. A refactor should not require test changes unless behavior changes.

Write acceptance tests so a non-developer can understand the rule they protect. Gherkin is welcome. A small domain-specific test language is also fine when it reads more clearly and keeps implementation details out of the scenario. Use `Given`, `When`, and `Then` to describe state and outcomes, not setup mechanics.

Keep integration tests under `tests/` when they exercise only public APIs. Use unit tests for narrow algorithms and invariants that need closer access. Every bug fix starts with a regression test.

Use the fewest tests that prove the behavior and its distinct risks. Before adding a test, name the failure it would catch that the existing suite would miss. Prefer one table-driven test or readable scenario over several copies with different inputs. Do not repeat the same assertion at unit, integration, and CLI layers unless each layer protects a separate contract. Do not test the Rust compiler, a dependency's documented behavior, or private implementation steps. Add cases when boundaries carry different risk, not to make the suite look thorough.

Make time, IDs, randomness, filesystem access, subprocesses, and network calls controllable at test boundaries. Tests must not depend on wall-clock timing, execution order, the developer's home directory, or a live service unless the test is explicitly marked as an external integration test.

## Rust rules

Follow these repository rules, adapted from Microsoft's [Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/guidelines/index.html) and the upstream Rust API and style guidelines. This file carries the working rules; agents do not need to fetch the source guide before each task. Apply their purpose rather than working around their wording.

### Project and checks

- Keep shared package metadata, dependencies, profiles, and lints in the workspace `Cargo.toml`.
- Put workspace crates beside one another under one crate directory. Split a crate when a component has a useful independent boundary; do not split only to satisfy a pattern.
- Use the latest stable Rust edition for new crates. Change the minimum supported Rust version deliberately and document the reason.
- Run formatting, Clippy, tests, and documentation checks before handing work off. Use the same commands as CI.
- Use `#[expect(..., reason = "...")]` for local lint exceptions. Reserve `#[allow]` for generated code or a documented case where expectation tracking cannot work.
- Keep features additive. A feature may add behavior or dependencies; enabling it must not remove another feature's API.

### Types and APIs

- Encode domain invariants in types. A fallible newtype must validate at construction and expose `TryFrom`, `FromStr`, or another fallible constructor rather than a permissive `From`.
- Use the strongest standard type at boundaries, such as `Path` and `PathBuf` for filesystem paths.
- Keep names short and specific. Avoid empty suffixes such as `Manager`, `Service`, and `Factory`; name the role the type performs.
- Implement `Debug` for public types. Redact secrets in custom `Debug` implementations and test the redaction.
- Implement `Display` for errors and values people are expected to read.
- Do not leak dependency types through a public API unless that dependency is part of the deliberate contract.
- Prefer concrete types over generics and generics over trait objects until runtime polymorphism earns its cost.
- Keep essential operations as inherent methods. Use free functions for work that does not belong to a receiver.
- Do not add a prelude or glob re-export public items. Give each public item one obvious import path.
- Validate interdependent builder fields in `build()` and return a typed error.

### Errors, panics, and unsafe code

- Return typed errors for expected failures. Application boundaries may add context with an application error type, but domain crates keep errors structured and inspectable.
- Use `From` for canonical error conversion. Do not scatter `map_err` calls that only rename the same failure.
- Treat a panic as a request to stop the process. Panic only for programming errors or violated internal invariants, and include a message that helps diagnose the bug.
- Do not catch a panic and continue ordinary work.
- Avoid `unsafe`. If it is unavoidable, isolate it behind a safe abstraction, document the safety argument beside the block, add adversarial tests, and run Miri. Unsound code is never acceptable.

### I/O, state, and diagnostics

- Keep domain logic independent of the CLI, SQLite, GitHub, clocks, and the filesystem. Pass those capabilities through narrow interfaces that tests can replace.
- Avoid global mutable state and correctness-sensitive statics.
- Use structured telemetry with stable event names and named fields. Production libraries do not use `println!` or `dbg!`; the CLI writes intentional user output to stdout and diagnostics to stderr.
- Do not format log messages eagerly when structured fields preserve the data without allocation.
- Bound work over external input. Prefer streaming or chunking when a payload can grow without a trustworthy limit.
- Profile before optimizing. Any `unsafe` performance optimization needs a benchmark that shows the safe code is insufficient.

## CLI behavior

The CLI is Merl's public agent interface. Keep human help concise and hierarchical. Keep `--json` non-interactive, versioned, and free of decorative prose. Preserve stable error meanings and exit behavior.

Do not make an MCP server or a mandatory skill part of a feature unless a later architecture decision changes the interface contract.

## Comments and documentation

Write comments and documentation in a natural, human voice. Use plain language, sentence-case headings, active subjects, and enough specificity to let a reader act. Remove filler, inflated claims, canned conclusions, and repetitive structure. [Tagore](https://github.com/apurvrdx1/tagore) is a recommended editing pass, not a required tool.

Comments and docstrings explain when a rule applies and why it exists. They record invariants, tradeoffs, surprising constraints, safety arguments, and consequences of changing a value. They do not translate the code into English or narrate how an obvious loop works.

Public Rust items need useful rustdoc. Start with a short, one-line summary. Add `# Errors`, `# Panics`, `# Safety`, and examples when the contract calls for them. Module documentation explains the boundary and the assumptions neighboring modules may rely on.

Name magic values. Document why the value was chosen, what breaks when it changes, and which external limit or measurement supports it.

Update the governing document when behavior or an invariant changes. Do not add speculative architecture to a code comment.

## Definition of done

Work is ready for review when:

- the behavior test failed first and now passes;
- relevant unit, integration, and acceptance tests pass;
- formatting, Clippy, and rustdoc checks pass with no unexplained exceptions;
- public behavior and JSON fixtures reflect the change;
- comments explain only non-obvious rationale and constraints;
- documentation and the assigned issue agree with the implementation;
- the diff contains no unrelated cleanup or generated debris.
