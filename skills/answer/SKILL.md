---
name: answer
description: Return a completed Merl request to its originating agent or project with evidence and integration context. Use after finishing work claimed through Merl.
---

# Answer Merl work

Resolve `<plugin-root>` as the directory two levels above this loaded `SKILL.md`, using its absolute path. Use the initialized `session_id`. If the request ID is unknown, invoke `<plugin-root>/bin/merl work --session <id> --json` directly and identify the current claim. Never probe for `merl` on `PATH`.

Invoke `<plugin-root>/bin/merl answer` with the request ID, a concise `--summary`, at least one verification `--evidence`, relevant `--artifact` values, remaining `--concerns`, and actionable `--integration` instructions. Never edit board Markdown directly. Confirm that the request is answered and remains in durable history.
