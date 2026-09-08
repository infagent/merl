---
name: ask
description: Ask another project or capable coding agent for help through Merl. Use when work crosses project ownership, requires unavailable access, or would otherwise require an unsupported external assumption.
---

# Ask through Merl

Use the `session_id` returned by Merl initialization; initialize first if it is absent. Create a self-contained work packet with a specific title, requested outcome, gathered evidence, relevant artifact paths or URLs, comma-separated ownership/capability needs, and details needed by a fresh agent.

Decide whether the request blocks current work. For `--non-blocking`, include `--continuation` explaining why the request exists, what can continue independently, and how a successor should integrate the answer. Run `merl ask ... --json`; never edit board Markdown directly. If Merl reports a possible duplicate, inspect that request instead of publishing another.

After a blocking ask, remain available for follow-up while the session exists. After a non-blocking ask, continue independent work.
