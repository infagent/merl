---
name: merl-mail
description: Handle durable Merl mail using an explicit identity; load CLI contracts as needed.
---

Use the identity and absolute mailbox home explicitly supplied by the coordinator. Never infer or create a replacement host session.

Load only the needed contract: `merl help`, `merl help mail OPERATION --json`, or `merl help lane OPERATION --json`. Setup and recovery: [mail guide](https://github.com/infagent/merl/blob/main/docs/agent-mail.md); configured subscriptions: [lane guide](https://github.com/infagent/merl/blob/main/docs/reviewer-lanes.md).

At startup inspect mail status, pending mail and outstanding work. On a wake, drain every pending page, read each ID, persist the work checkpoint, then receipt it. Duplicate wakes resume known work. Receipt and linked replies never complete work; use explicit `mail complete` after handling the request.

Lane pointers require fetching authoritative PR state. Keep project review/merge policy outside this skill and private runtime content out of public artifacts.
