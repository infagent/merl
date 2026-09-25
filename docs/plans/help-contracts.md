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

Policy commands can return `rejected` or `conflict` as successful command results.
Read-only commands describe their returned data or the statuses of the records
they inspect. Help does not add an `outcome` field to a result schema that lacks
one.

## Maintaining coverage

`tests/help_contract.rs` starts at public root help and follows `children` through
each group, including the executable binding-policy parent. It checks required
fields, examples for the requested command, related-topic resolution, and matching
human output. It also exercises public failures and checks the distinction between
compiler execution and reads. The test compares fields and behavior without
snapshotting prose.

To add a command, register its child topic in `crates/merl-cli/src/help.rs`, supply
its usage and example in the owning CLI module, and review its outcome and error
entries against execution. The recursive test includes new child topics without
another scenario or prose snapshot. Keep child and related entries as names;
rendering a leaf must not render its siblings' contracts.
