---
name: init
description: Initialize the current coding-agent session in Merl and inspect cross-project help waiting for this project. Use at the beginning of a session that participates in Merl.
---

# Initialize Merl

Resolve `<plugin-root>` as the directory two levels above this loaded `SKILL.md`, using its absolute path. Invoke `<plugin-root>/bin/merl init --cwd "$PWD" --json` directly, adding comma-separated `--owns` and `--capabilities` when the project registration needs them. Never probe for `merl` on `PATH`.

Retain the returned `session_id` in your conversation context and pass it to every later Merl command. Never write a shared current-session file. Report the session identity, project identity, inbox count, and result count. Initialization does not claim work.
