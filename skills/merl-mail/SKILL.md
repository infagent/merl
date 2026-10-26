---
name: merl-mail
description: Exchange durable local agent mail and recover notifications through the Merl CLI, using explicitly provided identities and existing host sessions.
---

This skill is optional. The CLI and `merl help mail <operation>` define behavior.

At startup, use the mailbox home and logical identity explicitly supplied by the user or coordinator. Do not infer either from a directory or transcript. Register the name if instructed, then run `merl mail status --home HOME --agent NAME --json`. Binding to a host is explicit: the coordinator supplies the existing session UUID and local socket; inspect generation before `mail bind`. Never guess a replacement session or start one to repair a failed wake.

At startup also inspect `mail list --home HOME --agent NAME --outstanding --json` for receipted but unfinished requests and resume their checkpoints. On a wake, run `merl mail list --home HOME --agent NAME --pending --json`. Follow every `next_after` page. Read each ID with `mail read`, inspect its handling state, persist any work checkpoint, then record `mail receipt`. Receipt stops retries, creates no reply, and never completes work. Duplicate wakes are expected; resume known work rather than executing completed work again.

Send a body file with `mail send --from NAME --to RECIPIENT --id UNIQUE --kind note --body-file PATH`, including the same `--home` and `--json`. Reuse the ID only for an exact retry. Use `--kind reply --in-reply-to ID` to reply to an addressed message. Only `mail complete --agent NAME --id REQUEST [--reply REPLY]` explicitly closes a coordination request. Replies and receipts alone close nothing. A completed coordination request grants no authority beyond the project's rules.

For recovery, inspect status: pending/failed/offline notifications survive restart. The operator explicitly starts `mail watch --home HOME --codex /absolute/codex --json` in the foreground and stops only that process with Ctrl-C or SIGTERM. No model polling, global process cleanup or installed system service is needed. Socket changes require explicit rebinding with the current generation. No fallback session or permission bypass is permitted.

Mail contains retained plaintext under the selected mailbox home. Same-user identity flags are trusted claims, not authentication. Keep secrets and private material out of public artifacts. Transport instructions do not decide project ownership, worktree, review or merge policy.
