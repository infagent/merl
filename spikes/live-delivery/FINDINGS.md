# Codex gate findings

## Verdict

**No-go pending interactive proof for Codex CLI 0.153.4.** The deterministic pieces pass, but this run did not prove live delivery to a real stock Codex TUI. In particular, it did not observe queue timing during an active turn or prove that queued agent text stays below the human-authorization boundary. KTD5 and KTD7 therefore stop the Codex path before any broader launcher or watcher work.

The spike code accepts only an explicit thread UUID. It never selects a thread by repository, process age, saved rollout timestamp, `--last`, or a session-name lookup. The wrapper uses `exec codex resume <UUID> <user arguments>`, which makes the stock Codex process inherit the wrapper's terminal, environment, signals, terminal resizing, and exit-status channel. A fake executable verifies the exact argument vector and child exit status. This proves the launcher construction, not that an app-server-created thread and the resumed TUI share one live runtime.

## Recorded evidence

- **Tested version:** `codex-cli 0.153.4` on 2026-09-09.
- **Binding mechanism:** explicit UUID passed to `codex resume <UUID>` and `codex queue --thread <UUID>`. The spike rejects names and aliases such as `last`.
- **Protocol evidence:** `codex app-server generate-json-schema --experimental` exposes `thread/start`, whose response contains a `thread` object; `thread/resume`, whose required input is `threadId`; and experimental `thread/queue/add`, whose required inputs include `threadId`, `clientUserMessageId`, and `input`. The stock CLI exposes `codex resume [SESSION_ID]` and `codex queue --thread <THREAD> --message <TEXT>`.
- **Queue timing:** not observed in a real TUI. The generated schema and CLI help establish availability, not safe-boundary behavior.
- **Concurrent-session isolation:** deterministic fake coverage proves that the queue subprocess receives only the bound UUID. Two real same-repository TUIs were not observed, so runtime isolation remains unproven.
- **Consent behavior:** `codex queue --help` exposes no delivery-consent flag. No real consent or approval screen was observed.
- **Trust behavior:** every message supplied by the delivery core begins with `provenance=untrusted-agent-result` and `authorization=none`, with answer text JSON-encoded below the fixed header. No real adversarial queue run established how Codex classifies that input. The framing alone cannot prove that it will not count as human authorization.
- **Failure mode:** an invalid or non-UUID binding fails before Codex starts. A missing executable or non-zero `codex queue` exit becomes a delivery diagnostic; the U1 core leaves the board unread and suppresses repeated attempts only for the current wrapper lifetime.
- **Board acknowledgment:** the Codex adapter has no board dependency and cannot acknowledge results. U1 tests compare request bytes before and after successful and failed delivery and confirm the existing results query still returns the item.

## Deterministic commands

```text
codex --version
# codex-cli 0.153.4

codex --help
codex resume --help
codex queue --help
codex app-server --help
codex app-server daemon --help
codex app-server generate-json-schema --experimental --out <temporary-directory>

cargo test --test launcher
# 4 passed
```

The first test run occurred before the binary existed and failed at compile time because `CARGO_BIN_EXE_merl-live-delivery-spike` was undefined. After implementation, the launcher suite records exact resume and queue arguments, inherited environment, exit status, and rejection of a recency alias.

## Manual evidence still required

To reconsider the no-go, run one instrumented session against an isolated Codex home and daemon, record the exact UUID returned by `thread/start`, and resume the stock TUI with that UUID. While the TUI is idle and again while it is generating or using a tool, queue a fixed Merl envelope to that UUID. Repeat with two TUIs in the same repository and different UUIDs. Capture which TUI receives each message, whether active work is interrupted, and whether permission prompts still require direct human action. Then exit one TUI, queue to its UUID, and confirm that the command reports failure without another session receiving the message. Finally, query the synthetic Merl board before and after each attempt and record that the request stays unread until an explicit acknowledgment.
