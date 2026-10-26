# Messaging interface and behavior

The `merl` CLI is the public interface. `merl help` exposes the mail group; `merl help mail` lists operations; `merl help mail send --json` loads one command contract. A skill is optional. Detailed examples are in the [setup guide](agent-mail.md).

## Commands

All stateful commands require an explicit absolute `--home`. Identities and message IDs contain 1..64 ASCII letters, digits, dot, underscore or hyphen. Required flags and examples are available through command help.

| Command | Observable behavior |
| --- | --- |
| `mail register --agent NAME` | Register a durable name without inferring a session |
| `mail bind --agent NAME --session UUID --socket PATH --expected-generation N` | Explicitly bind an existing session and socket under a generation guard |
| `mail unbind --agent NAME --expected-generation N` | Disable notification without losing mail |
| `mail send --from NAME --to NAME --id ID --kind LABEL --body-file PATH` | Atomically publish bounded immutable mail and notification intent |
| `mail list --agent NAME` | Return paged headers; no body or read side effect |
| `mail show --agent NAME --id ID` | Inspect one addressed body without marking it read |
| `mail read --agent NAME --id ID` | Read and idempotently mark it read |
| `mail receipt --agent NAME --id ID` | Record application receipt and stop retries; no reply or completion |
| `mail complete --agent NAME --id ID [--reply ID]` | Explicitly complete one addressed coordination request |
| `mail status --agent NAME` | Report binding, unread, unreceipted, outstanding and last notification independently |
| `mail dispatch --codex PATH` | Perform one bounded due-notification pass |
| `mail watch --codex PATH` | Run the opt-in foreground dispatcher until stopped |

`send` labels are `assign`, `ready`, `review`, `blocked`, `question`, `reply` and `note`. The first five are coordination requests. A reply requires `--in-reply-to ID`, reverses the original endpoints and closes nothing by itself. Completion may link a matching reply. An exact send retry is `unchanged`; altered content under the same ID conflicts.

Lists accept at most one of `--unread`, `--pending` or `--outstanding`. Without a filter they show all addressed headers. `--limit` is 1..100. Follow `next_after` with `--after`; a cursor from another recipient is invalid. Pending includes interrupted reads without receipt, and outstanding includes receipted unfinished requests. Read and receipt timestamps preserve their first values on retry.

## Output and errors

Success uses `merl.mail/v1` with an action such as `mail.send`. Help uses `merl.help/v1`, including usage, argument names, outcomes, errors, examples, related topics and child names. Errors use `merl.error/v1` with stable `code` and `message`. Human errors go to stderr; JSON errors go to stdout. Failure exits 2. JSON mode never prompts. The last `--format human|json` or `--json` option controls formatting. Human mail records format the same fields for inspection.

Important errors include missing identity, incorrect recipient, changed immutable retry content, mismatched reply, stale binding generation, unsupported schema, invalid input, I/O/storage failure and scoped lock contention. Each leaf help topic lists its reachable codes. Retired or unknown command groups return `UNKNOWN_COMMAND` before opening any runtime store, including one supplied with an obsolete `--database` flag.

## Notification and lifecycle

Notification states include `pending`, `dispatching`, `offline`, `failed`, `stale_binding`, `pointer_limit` and `enqueued`. A superseded host attempt reports its host outcome while preserving a new pending batch. These are transport facts. Receipt and completed work remain separate.

Use an absolute Codex executable and an explicitly supplied existing session UUID/socket. A host restart or socket replacement requires explicit rebinding. No substitute context is started. The watcher prints startup status, reports transient contention and resumes, and leaves durable work for restart. Stop only the foreground process you started with Ctrl-C or SIGTERM. No service is installed and no unrelated watcher is stopped.

This local interface trusts cooperating same-user identity claims. It is not authentication or OS isolation. Mail is retained plaintext without purge or quota. Old runtime stores are never automatically opened, migrated, overwritten or removed. See the [architecture](architecture.md) for concurrency and retention invariants.
