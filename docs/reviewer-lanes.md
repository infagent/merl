# Reviewer-lane PR activity

A lane is a durable PR activity subscription for one explicitly named recipient,
one GitHub repository and required labels. It discovers and follows many PRs over
time. The subscription belongs to the recipient identity, not one PR or one
Codex session. Any registered identity or existing team member can receive it;
roles are optional metadata and confer no routing policy or authority. GitHub
authors do not identify lanes: this team can share an account, so Merl leaves all
authors' activity visible. Notifications retain the existing `review` request
kind; generic recipients do not generalize the source or notification kind.

## Team-qualified recipients

A release containing #167 supports new subscriptions for existing team members:

```sh
merl lane start --home /absolute/team-mail --name epic-a-activity \
  --repo example/widgets --agent epic-a/member-a --label activity-ready --json
merl lane start --home /absolute/team-mail --name epic-b-activity \
  --repo example/widgets --team epic-b --agent member-a --label activity-ready --json
```

The recipient may have any role or none. `--team` qualifies only a bare `--agent`;
an explicit scope/member wins even when the supplied default names a different
or nonexistent team. Both names must have valid syntax; only the selected actor
is looked up. Without `--team`, bare actors retain legacy identity semantics.
Missing teams/members fail without raw-name fallback, registration or creation
of a mail/lane store. Prepare membership separately using [teams](teams.md).

Creation and full active retries add `resolved["--agent"]` with `team`, `member`,
`qualified` and `mail_key` to `merl.lane/v1`. Equivalent explicit/shorthand forms
return `unchanged` under existing retry rules. The raw key is the durable
recipient. Another member or scope under the old subscription name returns
`LANE_CONFLICT`; create a new named lane instead. Existing raw recipients, source
checkpoints and frozen batches are never repointed or adopted after joins.

Resume uses only `--name` and `--expected-generation`. `--team` is valid only with
full creation inputs; other lane operations gain no team default. Missing actors
return existing `TEAM_NOT_FOUND` or `TEAM_MEMBER_MISSING`; malformed names, missing
identities and schema failures retain their existing typed errors. Incomplete actor/default forms reject before creating runtime state and preserve
requested JSON/human error routing. Selected actor lookup uses existing mail
state read-only.

Human `lane status` optionally adds `qualified_agent` to each mapped record and
keeps raw `agent` visible. This is a best-effort read-only lookup of at most 16
saved keys, with no guidance or host reads. Missing mappings leave the key shown;
unavailable/unsupported metadata stops further optional lookups and never fails
status or creates mail state. JSON status retains its existing fields/raw keys
and does not access this optional metadata. Resume and dispatch use saved keys
without team resolution. Binding a replacement session remains explicit.

## Configure and run

Choose a persistent mailbox home and register the reviewer through `mail register`. Use an existing authenticated GitHub CLI with read access to the selected repository and PR metadata. Merl invokes only fixed metadata queries against `github.com`; it does not perform GitHub mutations or store credentials.

These names and paths are synthetic; replace them with explicit configuration:

```sh
merl mail register --home /absolute/team-mail --agent review-bot --json
merl lane start --home /absolute/team-mail --name backend-review \
  --repo example/widgets --agent review-bot --label review-ready --label backend --json
merl lane status --home /absolute/team-mail --name backend-review --json
merl lane queue --home /absolute/team-mail --name backend-review --limit 100 --json
```

Both labels are required. Names use 1..32 ASCII key characters; configure 1..8 nonempty labels, up to 64 bytes each. ASCII label case is ignored; other UTF-8 bytes compare exactly. The registry supports up to 16 lanes per home. Repository and recipient are immutable for a lane; use another name for a different repository or reviewer. Repeating an identical active creation is unchanged.

Bind `review-bot` to its explicitly supplied existing session/socket through `mail bind`; see [mail setup](agent-mail.md). Then run one foreground process:

```sh
merl lane watch --home /absolute/team-mail \
  --gh /absolute/gh --codex /absolute/codex --max-requests 16 --json
```

This process polls due GitHub work and reuses normal mail dispatch. It shares the existing `mail watch` lock, so do not run both watchers for the same home. Stop only your own foreground process with Ctrl-C or SIGTERM. Restart recovers checkpoints and unsent intent. The watcher calls no model while idle, starts no substitute session and installs no service. Both configured subprocesses run from the canonical mailbox directory, so deleting a development checkout cannot strand them.

For one bounded source pass without host dispatch:

```sh
merl lane scan --home /absolute/team-mail --gh /absolute/gh --max-requests 16 --json
```

This can publish activity into Merl mail, but does not itself enqueue a host wake. `mail watch` can deliver that mail; `lane watch` combines both parts. An unbound or stale reviewer binding retains mail and exposes the existing offline/stale state. It never reroutes the subscription to another session automatically.

## Initial work, activity and scope

The first paginated sweep discovers matching open PRs. Each PR starts at its observed timeline tail and builds an inline-thread baseline. `initializing` remains visible in the queue until all baseline thread pages finish. Merl then publishes a current-work pointer instead of replaying old discussions. A large baseline can delay this first notification; fair scheduling lets discovery and other PRs continue meanwhile.

For an active PR, Merl compares the current head SHA, open/closed/merged state and draft status. It follows selected timeline events: commits, issue comments, reviews, review threads, force pushes, closure/merge/reopen, draft/readiness, labels, review requests/removals and review dismissal. A recurring paginated thread sweep compares each thread's comment count, latest comment ID/update time and resolution state. This catches new replies on existing inline threads; a timeline tail alone is insufficient.

Activity summaries contain PR URLs, event kinds and the latest observed head/state. They contain no titles, discussion bodies, patches or author identities. Bursts coalesce into at most five PR pointers per immutable mail batch. More work forms separate batches; normal recipient-level transport coalescing bounds host wakes. Reviewers fetch authoritative current state rather than treating a pointer as a frozen PR snapshot.

Discovery continues after the first PR closes. Known closed/merged PRs drain their remaining observed timeline/thread pages before tracking stops. A known PR that no longer has the required labels receives a final `left-scope` pointer and stops tracking. Missing/inaccessible PRs or repositories are failures, not evidence of a scope exit. Neither an incomplete discovery sweep nor missing truncated labels retires existing work.

Changing selectors restarts membership discovery, preserves still-matching PR cursors and all prior intent, and lets explicit PR observations retire departures. Re-entry or reopening after tracking stopped creates a new current baseline and a visible `reentry-gap`. Activity while out of scope is not replayed as a lossless history. A scope exit with unfinished pages records `scope-exit-gap` rather than claiming those pages were consumed.

## Pause, resume and fresh sessions

Configuration changes use the generation returned by status:

```sh
merl lane stop --home /absolute/team-mail --name backend-review --expected-generation 1 --json
merl lane start --home /absolute/team-mail --name backend-review --expected-generation 2 --json
merl lane update --home /absolute/team-mail --name backend-review --expected-generation 3 \
  --label review-ready --label frontend --json
```

Stop pauses source reads and unclaimed publication. It retains cursors, staged events and frozen batches. A send already in flight may finish, and mail already published can still be delivered by transport. Resume continues saved progress; it does not clear server cooldowns. A generation change during external I/O prevents the old response from committing content or mail intent. Authenticated server retry deadlines still apply even if that response's configuration became stale.

A replacement reviewer session uses the same Merl identity and an explicit generation-guarded `mail bind`. Lane configuration and cursors remain untouched. At startup, drain `mail list --pending`, then inspect `mail list --outstanding`, `lane status` and the paged `lane queue`. Follow every `next_after` page before calling the queue complete.

The watcher registers sender identity `merl-watch` and publishes review-labelled requests with stable `lane.NAME.SEQUENCE` IDs. Receipt stops transport retries; it does not finish a review. Record findings or a no-change decision using project policy, checkpoint the handled PR/head, then explicitly `mail complete` the relevant request. Later activity can create another request for the same PR. Do not post GitHub acknowledgment comments just to receipt notifications: they create more activity. Merl makes no GitHub comments, reviews, ready/merge actions, assignment changes or code edits.

## Progress, failures and limits

Normal complete source units become due after 60 seconds. Partial pages remain due and rotate through durable scheduling tickets; a long PR history cannot monopolize later discovery. Each pass makes at most 16 requests, each with a ten-second deadline and two-MiB response cap. Discovery/timeline pages request 50 items, thread sweeps request 20 tails, and label metadata is bounded at 100 names. If truncated labels cannot prove membership, Merl records `LANE_INCOMPLETE` and retains progress. It never assumes omitted labels were absent.

`lane status` shows the last complete discovery sweep (`last_scan_at`), discovery continuation, provider cooldown, active/initializing/failed PRs, staged work and pending/failed frozen batches. That timestamp does not claim every PR connection is current. `lane queue` exposes independent timeline/thread cursors, `tracking`, last fully successful observation, failures and gap counts. `mail status` separately reports enqueue, receipt and outstanding work. None substitutes for review completion.

Network, authentication, deadline, protocol, partial GraphQL and unavailable-source failures retain checkpoints and use a shared source retry delay. HTTP 200 is not sufficient: GraphQL errors and rate metadata are checked. Repeated failures increase local retry intervals: transient failures back off from five to 60 seconds, and rate limits from one minute to one hour. Numeric or HTTP-date `Retry-After`, reset headers and GraphQL reset times can extend those waits without a local cap. Existing durable mail can still be delivered during a provider cooldown.

A rejected cursor records `LANE_CURSOR_GAP`. Discovery restarts its sweep with a retained gap count. An affected PR rebuilds its current tail/thread baseline and publishes a `cursor-gap` pointer. This is explicit resynchronization, not a claim that missing history was recovered. Corrupt/incomplete metadata never silently becomes an empty current page.

This polls current matching open work, not an archival event log. A PR that opens and closes wholly between discovery observations may be unseen. Intermediate head/state transitions can coalesce. Edits only to older issue comments, old review text or a non-tail inline comment may not create a selected timeline event or change a thread tail. A comment created and deleted between observations may also be unseen. These exclusions do not count as observed activity. There is no text diff, webhook, generalized query language, spawning or scheduling service.

Lane metadata, fingerprints, intent, batches and mail remain local plaintext without purge/quota. Preserve the runtime home when cleaning worktrees. Credentials stay with authenticated gh; executable paths and same-user filesystem access remain trusted capabilities.

## Verification

Relative retry delays start at response receipt, using the caller-supplied clock after I/O. An empty completed timeline page retains the last consumed cursor; GitHub's null `endCursor` does not reset history. Both boundaries have public clock/fixture regressions and the empty-page shape was checked against read-only real metadata.

Run the full checks in [CONTRIBUTING.md](../CONTRIBUTING.md). Focused public scenarios:

```sh
cargo test --locked --test lanes --test lane_integrity --test launch_directory --test mail --test product
```

Synthetic GitHub fixtures, supplied clocks, public CLI/API calls and controlled Unix sockets cover multiple PRs/later discovery, paging, inline replies, shared-account activity, unchanged polls, scope/pause/generation changes, long rate limits, cursor gaps, ambiguous publication/restart and deleted launch directories. They make no real GitHub or model calls. [Messaging validation](agent-mail-validation.md) distinguishes these checks from live delivery.

A live acceptance run must identify the exact application and dispatch binaries, observe the existing reviewer idle, and let watcher-originated read-only GitHub input cause the wake. Receipt/reply/completion must use Merl, with no native nudge after publication. Native coordination or a recovered manual-mail nonce does not prove lane delivery. Use controlled boundaries for multiple PRs and session replacement when public test writes would create litter; label live versus simulated evidence accurately.
