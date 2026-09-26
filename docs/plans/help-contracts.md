# First-release help inventory

Issue [#69](https://github.com/infagent/merl/issues/69) covers the 52 executable
commands below. The inventory follows the shipped `merl` interface; the larger
product architecture includes commands for later releases.

| Topic | Commands |
| --- | --- |
| `project` | `init`, `revision`, `rebuild`, `view`, `delta`, `batch` |
| `project authority` | `list`, `grant`, `revoke` |
| `project revalidation` | `list`, `run`, `resolve` |
| `issue` | `capture`, `import-fixture`, `view` |
| `source` | `show`, `compile`, `assertions`, `apply`, `require`, `replay`, `purge`, `purge-audit` |
| `source compilation-policy` | inspect a binding, or `set` its policy |
| `compilation` | `list`, `show`, `context`, `expand` |
| `candidate` | `list`, `show`, `accept`, `reject`, `correct` |
| `decision`, `hypothesis`, `claim` | `create` |
| `question`, `finding` | `create`, `resolve` |
| `task` | `request`, `accept`, `defer`, `start`, `complete` |
| `inbox` | `subscribe`, `poll`, `show`, `ack` |
| `show` | inspect an object or relation |
| `security` | `explain` |

## Reading a contract

Run `merl help source compile` for a human example, or add `--json` for the
versioned record. Both formats use the same usage, summary, outcome and error
lists, example, and related commands. Examples assume the named project and
objects exist. Replace compiler paths and configuration, including the prompt
digest, with the values for your adapter.

Help describes errors after the CLI converts domain failures. For example,
`source compile` can report `COMPILER_CONTEXT_REQUIRED` after retaining a context
request. Inspect it with `compilation show` and resume with `compilation expand`.
The expansion command handles another context request as a recorded status.
Compiler read commands do not advertise errors that require executing a compiler.

For `source compile` retries, keep the run, actor, reason, and compiler
configuration unchanged. A different compiler version, model, prompt digest, or
adapter configuration under the same request identity returns
`POLICY_INPUT_CONFLICT`.

Policy commands can return `rejected` or `conflict` as successful command results.
Read-only commands describe their returned data or the statuses of the records
they inspect. Help does not add an `outcome` field to a result schema that lacks
one.

## Maintaining coverage

`tests/help_contract.rs` starts at public root help and follows `children` through
each group, including the executable binding-policy parent. It checks required
fields, examples for the requested command, related-topic resolution, and matching
human output. Focused examples cover changed compiler retries, adapter and response
failures, and erased compiler input. A binding-policy example erases the reason
payload before reading its metadata. Other checks exclude payload lookup errors
from metadata reads and policy commit errors from commands that return conflict
receipts. The tests compare fields and behavior without snapshotting prose.

## Auditing errors

[`help/errors.rs`](../../crates/merl-cli/src/help/errors.rs) gives each command an
explicit entry. Only argument parsing and store-open failures are shared. An
unknown executable topic fails help rendering until it has an error contract.

Follow each public path through error conversion and recovery. A callee's error
type alone cannot tell you which codes the command exposes:

| Boundary | Rule checked in this inventory |
| --- | --- |
| Binding-policy inspection, compilation list/show, batch and inbox pages | Read metadata and payload references; do not resolve protected bytes. |
| Project/Issue views, source inspection, compilation context | Resolve retained bytes. `compilation context` returns `INVALID_COMPILATION` for erased input; metadata inspection still succeeds. |
| Semantic commands, source apply, candidate review, revalidation resolution | Catch `PolicyConflict` from commit and return its recorded disposition. Changed content under the same request identity can still return `POLICY_INPUT_CONFLICT`. |
| Task commands | Task updates render reason bytes. A new task request has no reason payload. |
| Assertion application and candidate review | Treat absent proposed payloads as rejected inputs. Revalidation can separately read reason spans while comparing planning meaning. |
| Compiler execution | Convert compiler-wrapped store failures to `STORAGE_ERROR`. Direct CLI store calls retain their specific codes. Expansion and revalidation handle context requests as statuses. |
| Capture and fixture import | Map policy preparation failures to `POLICY_ERROR`; commit failures retain store codes. Live capture records provider and dispatched compiler failures in its result. |

The acceptance tests cover the distinctions most likely to drift. They do not
manufacture every storage failure. When changing a command, inspect its reads,
its error conversions, and any failures it catches before editing the inventory.

To add a command, register its child topic in `crates/merl-cli/src/help.rs`, supply
its usage and example in the owning CLI module, and review its outcome in
`help.rs` and errors in `help/errors.rs` against execution. The recursive test
includes new child topics without another scenario or prose snapshot. Keep child
and related entries as names; rendering a leaf must not render its siblings'
contracts.
