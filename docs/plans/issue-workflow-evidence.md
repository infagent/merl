# First-release Issue workflow evidence

Issue [#70](https://github.com/infagent/merl/issues/70) connects the first-release
commands in one acceptance scenario:
[`tests/first_release_workflow.rs`](../../tests/first_release_workflow.rs).
It starts without a database. Every project operation runs the built `merl`
binary in a fresh subprocess, including setup and inspection. The driver uses
JSON results and never opens SQLite or calls application crates.

## The workflow

| Stage | Public commands | Evidence |
| --- | --- | --- |
| Setup | `project init`, `project authority grant`, `inbox subscribe` | An administrator grants review authority. The captured source supplies the Issue author's identity for a separate decision-author grant. |
| Capture and review | `issue capture`, `source assertions`, `source apply`, `candidate list/show/accept` | Eager compilation records an interpretation. D1 remains absent until a reviewer accepts its candidate. |
| Provenance | `show D1 --source --history` | The decision names its policy evaluation and input, compiler run, source version, and exact byte span containing `Keep gain fixed.` |
| Incremental delivery | `issue view`, `inbox poll/ack`, `issue capture`, `source apply`, `project delta` | After the reader catches up, one new comment from the authorized author creates D2 in one accepted revision and one compact inbox entry. Capture and application retries add nothing. |
| Revalidation | `project revalidation list/run/resolve`, `show D2 --source --history` | An edit makes support pending. A hindsight run and explicit confirmation restore support without replacing D2 or changing its semantic history. |
| Recovery | `source replay`, `compilation show`, `project rebuild` | Replay produces a separate successful run. Accepted views, delta history, and D2's provenance remain unchanged. Rebuild dispatches no compiler work. |
| Structured action and note | `decision create --note`, `source show`, `source compile/apply`, `candidate accept` | D3 reaches accepted state without a compiler. Later compilation marks the repeated decision `duplicate`; review accepts the note's added finding F1. D3 retains one semantic transition. |
| Erasure | `source purge --dry-run`, `source purge`, `source purge-audit`, `source show`, `show F1 --source`, `source replay` | The optional note and derived evidence become unavailable. The audit preserves actor, reason, digests, and erasure scope. Replay of that input fails with `MISSING_EVIDENCE`. |
| Final read | `issue view`, `inbox poll/ack` | Required coverage reaches the observation head with no gaps, and the reader has acknowledged every accepted revision. |

The purge targets the optional supplemental note after all required Issue sources
have compiled. The final coverage assertion does not claim that erased evidence
is replayable or that F1 still has current support. A required-source purge would
instead need to remain visible as a coverage gap; focused purge tests cover that
case.

Live capture returns the accepted revision rather than the Issue object ID. The
driver obtains that ID from the corresponding `project delta` batch before calling
`issue view`. It acknowledges inbox entries in revision order. On-demand note
compilation uses the administrator; candidate review uses the command actor.

## Boundaries and local execution

The provider fake serves controlled GraphQL responses through `--github-program`.
The compiler fake reads Merl's process request through `--program` and returns
bounded assertions for fixed phrases. It takes source IDs and represented values
from the supplied context and computes byte spans from the source text. These
fakes exercise the adapters without network access or a model service.

Provider timestamps and source edits are fixed fixtures. Assertions compare
observation order, revisions, and retained receipts, not wall-clock durations or
specific local audit timestamps. Each subprocess gets an isolated home directory
and a cleared environment. A compiler dispatch log checks that direct commands
and projection rebuilds do not invoke the compiler; product assertions use public
JSON output.

Run the scenario on Unix with Python 3 at `/usr/bin/python3`:

```sh
cargo test --locked --test first_release_workflow
```

The existing Ubuntu CI job includes it through
`cargo test --locked --workspace --all-features`. No live credentials or additional
services are required. This evidence supports the
[#52 audit](https://github.com/infagent/merl/issues/52); it does not close the wider
audit, establish model quality, or replace a smoke test against a real repository.
