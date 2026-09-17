---
name: merl
description: >
  File-based messaging between named agents (a lead and a builder, a research-lead
  and a researcher) through a shared mailbox directory, with a format that lets each
  side run a background watcher for the other's messages. Use when the user gives you
  an identity ("your identity is X"), asks you to leave, check, watch for, or reply
  to messages for another agent, or when a message addressed to you arrives. Covers
  sending, listing what is unanswered, watching, and how the protocol may change.
  On load, always: read the inbox, then arm the watcher, before any other work.
---

Messages between agents are files in one directory. The protocol is small on purpose
so a shell loop can watch it, and it may change; the last section says how.

## Every time this skill loads, do these two things first

Agents consistently do the first and forget the second. Without a watcher, a message
from the other side sits unread until someone happens to look, and the pair stalls.

1. **Read what is unread, then list what is open.**
   `~/.claude/skills/merl/scripts/read.sh` prints every inbound message you have
   not read, oldest first, and marks it read; `inbox.sh` then lists the requests still
   open. Both, every time, in that order. `read.sh` is what catches the second of two
   messages that landed together and the `ack` or `reply` that never shows as open;
   `inbox.sh` is what shows you what you owe. Act in filename order.
2. **Arm the watcher, immediately, even if the inbox was empty.** Use the one-wake-up
   form, as a background Bash command:
   ```
   Bash(run_in_background: true,
        command: "~/.claude/skills/merl/scripts/watch.sh --once --interval 15 --timeout 28800")
   ```
   It exits, and wakes you, on the first new message; the 8-hour timeout is a backstop
   (exit 3), and it is reachable: a watcher armed this way has run the full eight hours
   under Merl and exited 3 on its own. When it wakes you: run `read.sh`, then
   `inbox.sh`, handle what is open, then arm it again. Never handle a wake-up by reading
   "the newest file": two messages can land in one poll, and a watcher exits after the
   first poll that finds anything. That re-arm is part of handling
   a message, which is why it does not get forgotten. A watcher that expired with no
   message is also a wake-up, and the same rule applies.

   Arming a new watcher replaces your old one, so a restart is just arming again. If
   you must stop one by hand, use `watch.sh --stop` or the task id. **Never `pkill` or
   `killall` by script name.** Every agent's watcher is the same file in a shared
   directory, and one `pkill watch.sh` has already taken every agent's watcher down at
   once. See "Stopping a watcher" below.

   Do not default to the `Monitor` tool for this. It caps at 30 minutes and needs
   re-arming on every expiry, sixteen chances a night to forget, with nothing else
   happening at the moment you must remember. Reserve it for when you want per-message
   notifications during one long stretch of work, and treat its expiry notice as a
   defect to fix by re-arming at once.

Both steps need an identity; the next section says where it comes from. Do not send
anything before step 1: an answer you were about to write may already be moot.

## Identity and mailbox

Your identity is a short kebab-case name such as `lumen-tech-lead` or `researcher`.
It belongs to the agent, not to the repository, and it is resolved in this order:

1. The `AGENT_MAIL_IDENTITY` environment variable. This is the durable home. The user
   sets it when launching the session (`AGENT_MAIL_IDENTITY=lumen-dev claude`), and
   every script here defaults `--me` and `--from` to it, so nothing needs repeating.
2. The user saying so in the conversation ("your identity is X"). Use it for the
   session and suggest the variable so it survives the next one.
3. Nothing. Ask. Do not invent one, and do not guess from the repository name.

Do not rely on memory for identity. Merl session memory is keyed by working directory,
so two agents sharing a checkout would overwrite each other's, and an agent that moves
between checkouts would lose it. A memory note may say "this checkout is usually driven
by X", which is a hint to confirm, not an identity.

The mailbox is `~/projects/messages/` unless `AGENT_MAIL_DIR` says otherwise. All
teams share it; filenames carry the sender and recipient, so identities must be unique
across teams and must not contain `-to-`. An identity may be a hyphen-suffix of another
(`dev` beside `lumen-dev`); the scripts anchor on the timestamp so the two never match
each other's files. Every script in `scripts/` takes `--dir` to override.

## The contract

1. **One file per message**, named `<YYYY-MM-DD>T<HHMMSS>-<from>-to-<to>.md` in local
   time, for example `2026-09-16T204600-lumen-tech-lead-to-lumen-dev.md`. Files sort
   by time; the suffix says who it is for; a watcher for identity X matches `*-to-X.md`.
2. **Atomic write.** Write to a dotfile in the same directory, then rename into place.
   Watchers key on the rename, never on create, so no one reads a partial message.
   Anything beginning with a dot is not a message (`.seen-<me>` state files live there).
3. **Immutable.** Never edit or delete a message once renamed. A correction is a new
   message. The directory is the record.
4. **Header block**, one field per line, then a blank line, then free prose:
   ```
   From: <identity>
   To: <identity>
   Date: <YYYY-MM-DD>T<HH:MM:SS>
   Type: assign | ready | review | blocked | question | reply | ack
   Re: <space-separated issue and PR numbers, or none>
   In-Reply-To: <filename this answers, or none>
   ```
5. **Requests stay open until answered; answers close.** `assign`, `ready`, `review`,
   `blocked` and `question` are requests: each is *open* until a file exists whose
   `In-Reply-To` names it. `reply` and `ack` are answers and need no response of
   their own, so a thread ends without a final round of receipts. If an answer also
   asks something, send it as a `question`, not a `reply`. "What is outstanding" is
   then a set difference any script can compute, with no shared state.
   `In-Reply-To` must name a message written by the agent you are sending to, because
   naming it closes it; `send.sh` refuses anything else. To pass a third party's
   message along, reference it in `Re` and say so in the prose.
6. **First prose line stands alone.** For `ready` it is the PR URL; for `review` the
   verdict (`approve` or `changes requested`) before the list; for `blocked` the
   blocker; for `question` the question. A reader skimming filenames and first lines
   should know what to do next.

Types: `assign` hands over work. `ready` says a PR is up. `review` is the verdict on
one. `blocked` means the sender cannot proceed. `question` needs an answer before
continuing. `reply` answers one. `ack` is a receipt when there is nothing else yet.

## Sending

```bash
~/.claude/skills/merl/scripts/send.sh --to THEM --type TYPE \
    [--from ME] [--re "#170 #167"] [--in-reply-to <filename>] <<'BODY'
first line that stands alone
the rest of the prose
BODY
```

It stamps the header, writes to a dotfile, renames, and prints the path. It refuses an
unknown type and an `In-Reply-To` that names a file not in the mailbox. Write the
body yourself, in plain prose: why, then what, then what you left out. Point at the
GitHub issue or PR for anything that belongs there; a message is a pointer and a
judgment, not a copy of the artifact.

Reply to the specific message you are answering. If several are open, answer each
with its own file, or answer one and `ack` the others naming which file carries the
answer. Do not ack an ack or a reply; they are already closed.

## Reading

```bash
~/.claude/skills/merl/scripts/read.sh              # print every unread inbound message, mark read
~/.claude/skills/merl/scripts/read.sh --peek       # print without marking
~/.claude/skills/merl/scripts/inbox.sh             # open requests only
~/.claude/skills/merl/scripts/inbox.sh --all       # everything addressed to me
~/.claude/skills/merl/scripts/inbox.sh --from X    # only what X sent me
# --me ME overrides AGENT_MAIL_IDENTITY on all of them
```

Two different questions. `read.sh` answers "what have I not seen", using its own marker
`.read-<identity>`, which moves only when a message is printed to you. `inbox.sh`
answers "what do I owe": an unanswered request; inbound `reply` and `ack` files never
show there. The watcher's `.seen-<identity>` is neither of these; it only decides what
wakes you. Run `read.sh` then `inbox.sh` on every wake-up and at the start of every
session, and after any long piece of work.

## Watching

`watch.sh` polls the directory and prints each inbound filename it has not printed
before, remembering them in `.seen-<identity>`. Two forms:

- **One wake-up, the default:** `Bash` with `run_in_background` and `--once`. Exits 0
  on the first new message (printing every new filename found in that poll), 3 on
  `--timeout`. Either exit wakes the session.
  ```
  ~/.claude/skills/merl/scripts/watch.sh --once --interval 15 --timeout 28800
  ```
  If the harness enforces its own ceiling on a background command, the kill is still
  an exit and still a wake-up, so the worst case is an earlier re-arm, never a missed
  message: `inbox.sh` on wake-up catches anything that landed in between.
- **Per-arrival notifications while you keep working:** the `Monitor` tool with the
  unbounded form. Each new message is one event. Expires after 30 minutes and must be
  re-armed on every expiry notice; treat "no watcher armed" as a defect in your own
  session, not a default state.
  ```
  Monitor(command: "~/.claude/skills/merl/scripts/watch.sh --interval 15",
          description: "messages for <identity>", timeout_ms: 1800000)
  ```

### Stopping a watcher

One watcher per identity. `watch.sh` records its pid in `.watch-<identity>.pid` in the
mailbox, and arming a new watcher stops the old one for the same identity before it
starts, so restarting never needs a kill. To stop without restarting:

```
~/.claude/skills/merl/scripts/watch.sh --stop      # this identity's watcher only
```

or stop the background task by its id in Merl. Both touch only your
own process. `pkill watch.sh`, `killall`, or any kill by script name is forbidden: the
script is shared by every agent on the machine, and a name-based kill stops all of
them, silently, including watchers belonging to agents that are mid-review. If you did
that once, re-arm yours and tell the others by message so they re-arm too.

Polling is deliberate: `inotifywait` is not installed everywhere and a 15-second poll
on a local directory costs nothing. Where it is installed,
`inotifywait -m -e moved_to --include '.*-to-<identity>\.md$' DIR` is an equivalent
event source.

## Several counterparts at once

The inbox and the watcher are per recipient, not per pair. A lead talking to a builder
and to another lead has one inbox, sorted by time, with the sender on each line, and one
watcher covers all of it. Nothing is configured per pair.

- Filter with `inbox.sh --from X` when you want one conversation.
- Threads are per sender. A `ready` from the builder and a `question` from the other
  lead are separate open items and each gets its own answer.
- Forwarding: write a new message to the third party, put the original filename in
  `Re`, and summarize or quote what matters. Do not name it in `In-Reply-To`; that would
  mark the original answered when its author has heard nothing.
- A decision that affects two counterparts is two messages, one to each. There is no
  broadcast, on purpose: each recipient's open list should be exactly what they owe.

## Roles, checkouts and who merges

A team on this channel has a lead, one or more builders, and a reviewer, who may also be
a builder for the hard work. The channel carries the handoffs; these rules keep the
repository sane while it does.

- **One checkout per agent.** Every agent works in its own local clone. Never two
  agents in one directory: a reviewer checking out a branch would move the builder's
  working tree under it, and there is no message that can undo that. Review from
  `gh pr diff` and a throwaway `git worktree` in your own clone or scratch space.
- **One issue in flight at a time, whatever it touches.** The lead arbitrates by never
  holding two open work `assign`s at once, code or not: a design, a rename map, a
  document count the same as a PR. Reviewing the one issue in flight is not a second
  issue; a review that pushes fixes is, so a reviewer requests changes and the builder
  makes them. Branch from `main` after every merge, never from another open branch.
- **Every `assign` names the coder and the reviewer.** The builder sends `ready` to the
  named reviewer, not to the lead. The reviewer gets its own `assign` saying "review
  and merge when the ready lands", and reports completion to the lead as a `reply` to
  that assign, so the lead hears once per task. Work the lead marks risky in the assign,
  the lead reviews itself.
- **The reviewer merges what it approves**, squash, branch deleted, the moment the
  acceptance criteria hold. An approval left unmerged is a handoff back to the lead
  that nobody asked for. Formal GitHub approval is unavailable when agents share one
  account; the review comment plus the `review` message is the record.
- **Review against the issue's acceptance criteria, not taste**, and verify rather
  than read: run the checks, reproduce a claimed failure once, say in the `review`
  which criterion each requested change fails.
- **Escalate by type.** `question` when the answer changes what you build, `blocked`
  when you cannot proceed, both to the lead, and keep working on whatever does not
  depend on the answer. A builder that disagrees with a review says so in one
  paragraph with the alternative; the reviewer decides or escalates.

## Other harnesses

The contract is files and headers, so it works from any agent harness. Only the
watching differs. The scripts run anywhere with bash and coreutils.

- **Merl**: the managed shell session for the one-shot watcher,
  `Monitor` for per-arrival events, `TaskStop` to stop by task id. As above.
- **Merl CLI** and anything without a background wake-up: when you are waiting on the
  other side, run the watcher in the foreground, `watch.sh --once --interval 15
  --timeout 1800`, and treat its exit as the wake-up; when you are working, run
  `read.sh` then `inbox.sh` between tasks and before ending a turn. Never leave a turn
  with an unanswered request you have already read.
- Set `AGENT_MAIL_IDENTITY` in the environment that launches the harness, whichever it
  is. Identity is per agent, not per tool.

## Etiquette

- Before sending `ready`, run the project's full check and put its output in the PR,
  not in the message. The message carries the URL and anything the reviewer must know
  that the PR body does not say.
- A `review` names each requested change once and points at the file and line where
  it can.
- `blocked` says what was tried. `question` says what you will assume if no answer
  comes, and by when.
- Do not send a message to say you are starting. Send `ready`, `blocked` or
  `question`. Silence means working.
- Disagreement goes in the message, in one paragraph, with the alternative. The
  recipient decides or escalates to the user; neither side stalls waiting.

## Changing the protocol

Both agents read this same file. To change the contract, send a `question` proposing
the change and the reason; the other side replies. Once agreed, one of you edits this
SKILL.md and the scripts, and the user reloads skills on both sides. Keep changes
backward compatible where possible: old messages are never rewritten, so a parser must
keep reading the header block above even after fields are added.

## Scripts

- `scripts/send.sh` writes one message atomically.
- `scripts/read.sh` prints unread inbound messages and marks them read.
- `scripts/inbox.sh` lists inbound messages, open ones by default.
- `scripts/watch.sh` polls and prints new inbound filenames; `--once` (the default
  form) exits on the first, for a single background wake-up; `--stop` stops this
  identity's watcher and nothing else.

All take `--dir DIR` or honor `AGENT_MAIL_DIR`, and `--me`/`--from` or honor
`AGENT_MAIL_IDENTITY`.
