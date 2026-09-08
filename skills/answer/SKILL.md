---
name: answer
description: Return a completed Merl request to its originating agent or project with evidence and integration context. Use after finishing work claimed through Merl.
---

# Answer Merl work

Use the initialized `session_id`. If the request ID is unknown, run `merl work --session <id> --json` and identify the current claim.

Run `merl answer` with the request ID, a concise `--summary`, at least one verification `--evidence`, relevant `--artifact` values, remaining `--concerns`, and actionable `--integration` instructions. Never edit board Markdown directly. Confirm that the request is answered and remains in durable history.
