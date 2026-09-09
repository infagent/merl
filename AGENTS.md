## Engineering workflow

- Work test-first. Prefer behavior-focused tests and dependency-injected fakes over mocks. Use Cucumber or Gherkin when it improves shared understanding; otherwise prefer a behavioral DSL readable by non-developers.
- Prefer a well-maintained, popular open source library with a permissive license over building functionality from scratch. If you still think building it yourself is the better call, confirm with the developer first, explicitly enumerating why the existing option is worse here (e.g., missing features, licensing conflict, dependency bloat, poor maintenance, security concerns).
- Keep the functional core separate from side effects at system boundaries. Add descriptive errors and useful logging in the first pass.
- Inspect existing code and tests before introducing a new pattern. Existing behavior is the source of truth unless the developer says otherwise.
- Make atomic commits using Conventional Commits. Integrate to `main` frequently, and treat `main` as releasable at all times. Release automation cuts releases from merged commits and the commit type selects the version bump, so `feat` and `fix` reach users and other types do not trigger a release.
- Put one reviewable change in each pull request. Split when the parts are independently releasable; keep them together when splitting would leave `main` broken or force consumers through a broken intermediate state. Treat a `size/L` or `size/XL` label as a prompt to explain in the description why the change is not divisible, not as a limit to cut scope against.
- When a change is too large to review in one pull request but cannot be split without breaking `main`, make it splittable: put the unfinished work behind an abstraction or a flag and integrate it inactive, rather than growing the pull request.
- Write comments that explain why, not what. Keep style consistent so the code reads as though one developer wrote it.
- Preserve user changes and avoid destructive Git operations. Never weaken tests, bypass checks, swallow errors, expose secrets, or cut corners without explicit developer permission.
- Use synthetic data in tests and fixtures. Never copy live command output, credentials, account identifiers, or infrastructure state into the repository.
- Keep documentation small. Do not add broad documentation unless a developer explicitly confirms a public SDK need; agents are the primary code consumers.
- Put deterministic rules in tools, tests, or CI rather than prose. Add nested `AGENTS.md` files only when a subtree truly needs different instructions.

## Python coding style

- Use `uv` as the toolchain: `uv run` to execute, `uv add` to change dependencies, and a committed `uv.lock`. Repository setup generates CI that assumes it. Do not introduce pip, Poetry, or a hand-managed virtual environment.
- Name a module after the noun or type its functions operate on, not a gerund or broad domain name. Prefer `image.write(...)` to `imaging.write_image(...)`.
- Put nouns before the verb in compound function names, in Reverse Polish notation style: `subimage_bounds_plot`, not `plot_subimage_bounds`. Remove a noun already implied by its module or shared family prefix.
- Enforce keyword-only arguments with a leading bare `*`, including short two-parameter functions, so call sites remain self-documenting.
- Apply these defaults to new modules and functions. When adapting existing code, update every call site in the same pass, including source, tests, documentation examples, nested calls, and executed snippets.

## Rust coding style

- Use stable Rust through `rustup` and Cargo. Pin the toolchain in `rust-toolchain.toml`, declare the minimum supported Rust version in `Cargo.toml`, use `cargo add` and `cargo remove` for dependencies, and commit `Cargo.lock` for applications and binaries.
- Name modules after the noun or type their functions operate on, not a gerund or broad domain. Prefer `image::write(...)` to `imaging::write_image(...)`. Methods should omit the receiver noun already implied by their type.
- Follow standard Rust naming and conversion conventions. Use `as_`, `to_`, and `into_` according to ownership, implement standard conversion traits where they fit, and derive common traits such as `Debug`, `Clone`, `Eq`, and `Hash` when their semantics are sound.
- Borrow by default and take ownership when a value must be retained or consumed. Do not add `clone()` only to silence the borrow checker; make the ownership boundary explicit instead.
- Replace unclear positional arguments, especially multiple booleans or values of the same type, with named structs, enums, or builders. Use newtypes for identifiers and values that must not be mixed accidentally.
- Return `Result` for recoverable failures and reserve panics for bugs or violated internal invariants. Give library and domain code typed errors; add operational context at command, process, filesystem, and network boundaries. Do not use `unwrap()` or `expect()` outside tests unless the invariant is local and explained.
- Keep async tasks owned and cancellable. Do not hold a synchronous lock or perform blocking filesystem or process work on an async executor. Track spawned tasks, propagate their failures, and define how shutdown drains or cancels them.
- Avoid `unsafe`. When it is required, keep it behind a small safe API, document each safety invariant with a `SAFETY` comment, and add targeted tests. Do not weaken an `unsafe_code` lint to make a dependency or implementation convenient.
- Format with `cargo fmt`. Before committing, run `cargo clippy --all-targets --all-features -- -D warnings` and `cargo test --all-features`; run doctests separately when another test runner is configured. Use `cargo-deny` in CI to enforce allowed licenses, advisories, duplicate-version policy, and trusted dependency sources.
- Apply these defaults to new modules and functions. When changing a public type or function, update every call site in the same pass, including source, tests, examples, documentation, and executed snippets.

## Bash coding style

- Use Bash for repository automation only when a small, dependency-free script is clearer than another project language.
- Start executable scripts with `#!/usr/bin/env bash` and enable `set -euo pipefail` unless a documented control-flow requirement prevents it.
- Quote expansions, use arrays for argument lists, and pass user-controlled values as arguments instead of evaluating shell text.
- Keep side effects at command boundaries. Put parsing and decisions in small functions that can be exercised with dependency-injected command fakes.
- Under `set -e`, a failing command inside a trap handler aborts the handler, so later cleanup never runs. Pass the pending exit status to a cleanup function as an argument rather than restoring it with a command that can fail.
- Check scripts with `bash -n` and ShellCheck. Test observable exit status, stdout, stderr, and filesystem effects rather than implementation details.
