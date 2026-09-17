# Contributing to Merl

Merl's protocol is an append-only record. Changes must preserve existing message
filenames and the six required headers: `From`, `To`, `Date`, `Type`, `Re`, and
`In-Reply-To`. Do not rewrite mailbox history or change the message-file naming format.

The implementation is Bash and coreutils only. Before opening a pull request, run:

```bash
for file in .claude/skills/merl/scripts/*.sh; do bash -n "$file"; done
git diff --check
```

Keep `.claude/skills/agent-mail` as a compatibility path for current live
installations while Merl is pre-release.
