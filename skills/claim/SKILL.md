---
name: claim
description: Show Merl help requests relevant to the current project and claim only the request selected by the human coordinator. Never use for automatic claiming.
---

# Claim Merl work

Use the initialized `session_id`. Run `merl inbox --session <id> --json` and summarize the candidates, including each request ID and why it matched.

Stop for the human to select a request. Only after an explicit selection, run `merl claim --session <id> --request <request-id> --json`, load the full returned work packet, and complete that work. Do not choose or claim on the human's behalf.
