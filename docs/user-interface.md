# Messaging interface and behavior

The `merl` CLI is the public interface. `merl help` exposes usage, init, status and the team/mail/lane groups; group help lists operations, and leaf help loads one contract. A skill is optional. Detailed examples are in [installation](installation.md), [mail setup](agent-mail.md) and [reviewer lanes](reviewer-lanes.md).

`merl --version` and `merl -V` print `merl VERSION` and exit successfully without selecting a mailbox. These standalone binary flags report the root package version used by the release archive; command JSON envelopes remain unchanged. See [installation](installation.md) for supported platforms and installation behavior.

## Usage primer

`merl usage` returns concise setup, explicit scope/identity, join/resume and individual handoff guidance. It reads no user settings, receipts, databases, project documents, processes or sockets and creates nothing. `--json` / `--format json` returns `merl.usage/v1` with `action`, `text` and related leaf topics; the last output flag wins. `merl help usage` loads `merl.help/v1`. Unknown options return `USAGE_INPUT`, exit 2, with no runtime access.

## Commands

Mail and lane commands require an absolute mailbox home, supplied with `--home` or saved by `merl init`. Explicit home bypasses all settings reads, even with a corrupt `--config`. Otherwise `--config` selects one absolute settings file, or the known platform user location is checked. There is no project config discovery. Missing settings require setup or explicit home; invalid settings return `INIT_CONFIG` before mailbox access. Help, version and unknown commands remain pure. Identities and message IDs contain 1..64 ASCII letters, digits, dot, underscore or hyphen. Required flags and examples are available through command help.

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

## Teams

`team create --name NAME --scope-file PATH [--coordinator NAME] [--project NAME]` saves bounded shared scope. `team join --team NAME --member NAME --description-file PATH` saves starting guidance and creates an individual identity atomically. Optional free-form `--role` and repeatable inert `--guidance-ref` values persist. Exact retries are unchanged; changed inputs conflict. `team show --team NAME`, paged `team members --team NAME`, and `team member --team NAME --member NAME` inspect existing state without creation or binding. Coordinator and role metadata are optional and descriptive. A project reference names an existing scope for exact one-hop navigation only. Paged `team list [--project NAME]` discovers scope headers without loading member guidance. No edit/leave/removal lifecycle is included.

Team results use `merl.team/v1`. Existing mail identity/handling/binding commands resolve each explicit `scope/member` independently; `--team` supplies a scope only for unqualified actors, and add `resolved` metadata while retaining `merl.mail/v1`. Mail registration and mailbox-wide dispatch/watch do not accept the qualifier. Unqualified shapes remain unchanged. Message IDs stay global per home; qualified names do not authenticate users or replace bindings. The [team guide](teams.md) defines limits, errors, JSON fields and complete CLI onboarding.

## User setup

`merl init` guides user-global setup through the controlling terminal. `--global` makes the scope explicit for scripts. Repeat `--host codex|claude`, or select `none`; supply `--home`, optional `--config` and host-specific `--codex-skills-dir`/`--claude-skills-dir` overrides. `--dispatcher foreground|none` selects whether to show foreground start instructions. It never starts a process. No native host configuration or session binding is edited.

`--show` reports `configured`, `incomplete` or `not_configured`. `--dry-run` validates a preview. Both create nothing, including directories and locks. First unattended apply needs explicit hosts/home and `--yes`; a known setup can reuse saved choices. JSON never prompts. `--uninstall --yes` removes unchanged owned setup files; optional `--host` selects only that host. Borrowed skills, local edits and every mailbox directory/descendant survive. Edits produce `INIT_CONFLICT` before conflict-plan mutation. Repeating the same setup options resumes failed publication; `--show` distinguishes incomplete setup.

Human init preview/results/show and top-level status share one summary layout: setup state, selected host skills, mailbox, settings and dispatcher. Preview says **will install/create/save/remove**; it does not present proposed files as already installed. Unchanged/borrowed files and retained mail are explicit. Apply confirmation remains separate; cancellation creates nothing. Paths beneath the user home display with `~`, while Enter keeps the underlying absolute default. The short next step loads relevant help instead of printing unresolved binding templates. Explicit `init --json` preserves its full existing record and remains noninteractive.

Canonical setup uses receipt v2 and retains the known legacy skill association without rewriting its bytes; config v1 and explicit `merl.init/v1` output remain compatible. Incomplete v1 setup rejects migration without writes and offers safe new-binary uninstall/retry. Retained legacy status has `retained_legacy=true`; content/ownership/matches_bundle remain independent. See [canonical startup and recovery](installation.md#canonical-startup-and-legacy-recovery).

Results use `merl.init/v1`; errors use the common envelope with `INIT_INPUT`, `INIT_CONFIG`, `INIT_CONFLICT`, `INIT_BUSY` or `INIT_IO`. Dispatcher state is `not_requested`, `not_started` or `missing_codex`. A start command is shown only for a detected real absolute Codex executable. Claude skill installation does not enable automatic Claude wake. See [installation](installation.md) for ownership, recovery and installer pipeline behavior.

## Informational team announcements

`team announce --team S --from MEMBER_OR_SCOPE_MEMBER --id ID --all|--role ROLE --body-file PATH`
publishes fixed-kind ordinary notes to one frozen selection of 1..100 members.
Use exactly one selector; repeat exact case-sensitive roles as an OR set, at most
16 supplied occurrences before deduplication. All includes roleless members.
The author is excluded unless `--include-self` retains an already matching member.
Project references do not expand the target. Qualified authors resolve separately.

The nonblank UTF-8 body is limited to 8192 bytes. Selection filters run before a
SQL fetch of at most 101 headers. Zero/overflow selections fail with
`TEAM_SELECTION`; a child-ID collision fails with `TEAM_MESSAGE_CONFLICT`. One
transaction commits the snapshot, all notes, handling and notification intent.
Any pre-commit failure rolls back all of them. `published` means durable notes,
not enqueue, receipt or completed work. Exact full-input retries return
`unchanged` with original recipients/IDs before considering current membership;
changed immutable inputs return `TEAM_CONFLICT`.

`team announcement --team S --id ID` reads only accepted metadata and bounded
recipient/message-ID mappings, with no body/source reads, handling changes,
host probes or state creation. Missing/wrong-target IDs return
`TEAM_ANNOUNCEMENT_NOT_FOUND`. Use lookup/retry after a lost response or uncertain
commit error. Both leaves use `merl.team/v1`, common errors exit 2, and individual
mail handling remains separate. See [teams](teams.md#informational-announcements)
for examples, fields and limits. No group assignment or completion is provided.

## Setup status

`merl status` reads only bounded local setup metadata and selected skills. It creates no directory, lock, receipt, database or other file. It opens no mailbox store and probes no host, process or socket. This command differs from `mail status --agent NAME`, which reports one identity's mail/binding state.

`merl status --verbose`, `-v`, `--json` and `--format json` return the same full `merl.status/v1` record. These are JSON aliases, not verbose prose. The last output option wins, as elsewhere. Help/version still bypass settings.

| Field | Meaning |
| --- | --- |
| `setup.state` | `not_configured`, `configured`, `incomplete`, or `not_checked` |
| `setup.pending`, `setup.issues` | Unfinished receipt intent and bounded local problem metadata |
| `config.path`, `source`, `state` | Selected settings and its observation; source is `user_default`, `explicit`, or `bypassed` |
| `mailbox.path`, `source`, `state` | Path and directory type only; source is `saved_default`, `receipt`, `explicit`, or `none` |
| `skills[]` | Host, path, content state, receipt ownership, `matches_bundle` and `retained_legacy`; no file contents |
| `dispatcher.guidance`, `runtime` | Saved foreground/none/unknown guidance; runtime is always `not_checked` |
| `next_step` | One relevant CLI/help command |

Explicit `--home` bypasses settings and receipts entirely, even with corrupt `--config` or an unusable user environment. It reports the explicit directory and `setup.state=not_checked`, with no skill/default claims. Otherwise `--config` selects one file or the known user config path is checked. A receipt home without published settings is a **planned home, not an active saved default**. Invalid selected settings fail honestly with `INIT_CONFIG` and exit 2; raw config content is never echoed.

Skills distinguish installed, outdated recorded bytes, missing, locally modified, different unmanaged bytes, pending publication and path/read conflicts. Ownership is separately owned, borrowed, unmanaged or pending; matching the current bundle never establishes ownership. A modified/missing/conflicting skill or unfinished setup makes status incomplete while preserving files. An intact older recorded skill can be configured and outdated. Symlink targets and unrelated host configuration are not read. These are best-effort local observations; a configured setup does not mean a dispatcher is running, any session is connected or work has been assigned. Human output says **Not checked** for runtime and abbreviates paths, while full JSON keeps exact paths and stable state identifiers.

## Reviewer-lane commands

All lane commands use the same mailbox selection and output-format options. Lane names use 1..32 key characters. Required labels are an AND selector; authors are never treated as lane identity.

| Command | Observable behavior |
| --- | --- |
| `lane start --name NAME --repo OWNER/REPO --agent NAME_OR_SCOPE_MEMBER [--team S] --label LABEL ...` | Create a durable multi-PR subscription; exact active creation retries are unchanged |
| `lane start --name NAME --expected-generation N` | Resume saved configuration without changing recipient binding |
| `lane update --name NAME --expected-generation N --label LABEL ...` | Replace labels, restart membership discovery and retain prior intent/cursors |
| `lane stop --name NAME --expected-generation N` | Pause scans and unclaimed publication; retain all state |
| `lane status [--name NAME]` | Show bounded lane health, complete discovery time, partial work, delivery failures and source cooldown |
| `lane queue --name NAME [--after PR_NUMBER] [--limit N]` | Page per-PR state/cursors/gaps, with `next_after` |
| `lane scan --gh PATH [--max-requests N]` | Read bounded metadata and publish durable mail; no host dispatch |
| `lane watch --gh PATH --codex PATH [--max-requests N]` | Run due scans and the existing dispatcher in one foreground process |

New named subscriptions can target any existing qualified member, including a
roleless member. `--team` qualifies only a bare `--agent`; explicit scope/member
overrides even a nonexistent valid default, and only the selected actor is
looked up. Bare names without a team remain legacy mail identities. Missing
qualified actors fail without fallback or runtime creation. Equivalent
qualified/shorthand full retries preserve the stored key; another recipient
requires a new lane name. Full scoped creation adds `resolved["--agent"]` with
team/member/qualified/mail_key; legacy creation keeps its shape.

`--team` is limited to full creation. Resume uses only name and generation;
existing recipients, checkpoints and batches are never rewritten after joins or
binding changes. JSON status keeps raw keys and existing fields without optional
mail lookup. Human status may add `qualified_agent` via at most 16 read-only
header lookups; absent/unavailable metadata retains raw keys without failing
status or creating mail state. The source and `review` request kind remain
unchanged for every recipient role. See [qualified recipients](reviewer-lanes.md#team-qualified-recipients).

Lane results use `merl.lane/v1` with `lane.OPERATION` actions. Help uses `merl.help/v1`; errors retain `merl.error/v1` and exit 2. Provider/rate/protocol/cursor errors are explicit and leave checkpoints intact. Status distinguishes a completed discovery sweep from pending PR baselines/pages, and a frozen batch from receipt/review completion. The foreground watcher reports changing failures without creating a mail acknowledgment loop and continues bounded retry/dispatch.

Repository and recipient are immutable per lane. Fresh sessions use explicit `mail bind`; the lane never guesses one. Pause can race an already claimed send, which may finish. Published review-labelled mail remains outstanding until the recipient explicitly completes it. The [lane contract](reviewer-lanes.md) defines initial baselines, inline comment coverage, scope departure, cursor gaps and polling exclusions.

## Output and errors

Success uses `merl.mail/v1` with an action such as `mail.send`. Help uses `merl.help/v1`, including usage, argument names, outcomes, errors, examples, related topics and child names. Errors use `merl.error/v1` with stable `code` and `message`. Human errors go to stderr; JSON errors go to stdout. Failure exits 2. JSON mode never prompts. The last `--format human|json` or `--json` option controls formatting. Human mail records format the same fields for inspection.

Important errors include missing identity, incorrect recipient, changed immutable retry content, mismatched reply, stale binding generation, unsupported schema, invalid input, I/O/storage failure and scoped lock contention. Each leaf help topic lists its reachable codes. Retired or unknown command groups return `UNKNOWN_COMMAND` before opening any runtime store, including one supplied with an obsolete `--database` flag.

## Notification and lifecycle

Notification states include `pending`, `dispatching`, `offline`, `failed`, `stale_binding`, `pointer_limit` and `enqueued`. A superseded host attempt reports its host outcome while preserving a new pending batch. These are transport facts. Receipt and completed work remain separate.

Use an absolute Codex executable and an explicitly supplied existing session UUID/socket. A host restart or socket replacement requires explicit rebinding. No substitute context is started. The watcher prints startup status, reports transient contention and resumes, and leaves durable work for restart. Stop only the foreground process you started with Ctrl-C or SIGTERM. No service is installed and no unrelated watcher is stopped.

This local interface trusts cooperating same-user identity claims. It is not authentication or OS isolation. Mail is retained plaintext without purge or quota. Old runtime stores are never automatically opened, migrated, overwritten or removed. See the [architecture](architecture.md) for concurrency and retention invariants.
