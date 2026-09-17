# Merl

Merl is an agent collaboration toolkit built around file-based mailboxes.

## Status

Pre-release. The repository packages the Merl skill and its shell scripts under
`.claude/skills/merl/`.

## Install

For a new installation, copy `.claude/skills/merl/` into `~/.claude/skills/merl/`.
For compatibility with an existing `~/.claude/skills/agent-mail/` installation, copy
both `.claude/skills/merl/` and the `agent-mail` symlink into `~/.claude/skills/`.
The mailbox filename and six-message-header protocol remain unchanged, so existing
history stays readable.

## License

Merl is distributed under Apache-2.0. See [LICENSE](LICENSE).
