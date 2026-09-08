---
name: claim
description: Show Merl help requests relevant to the current project and claim only the request selected by the human coordinator. Never use for automatic claiming.
---

# Claim Merl work

Resolve `<plugin-root>` as the directory two levels above this loaded `SKILL.md`, using its absolute path. Use this agent's initialized `session_id` unless the human coordinator explicitly supplies an initialized target worker session ID. Invoke `<plugin-root>/bin/merl inbox --session <target-session-id> --json` directly and summarize the candidates, including each request ID and why it matched. Never probe for `merl` on `PATH`.

Stop for the human to select a request. Only after an explicit selection, invoke `<plugin-root>/bin/merl claim --session <target-session-id> --request <request-id> --json`. When claiming for this agent, load and complete the returned work packet. When a manager claims for a sibling worker, pass the packet to that worker instead. Do not choose a request or invent a target worker on the human's behalf.
