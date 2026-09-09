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

# Claude Channel gate findings

## Verdict

**No-go pending a supported implementation route and interactive proof for Claude Code 2.1.266.** The smallest Rust subprocess expresses the documented MCP wire contract, and deterministic fake-client and fake-launcher tests cover its intended behavior. Anthropic's current Channels reference, however, says the only hard requirement is `@modelcontextprotocol/sdk` with a Node.js-compatible runtime. Rust is therefore not a documented custom-channel implementation route. Per R10 and KTD4, this spike does not hide that boundary behind a JavaScript shim or claim a live pass from protocol-shaped output.

No real Claude TUI was launched in this run. Registration, consent, busy-turn ordering, same-repository isolation, organization-policy rejection, disconnect behavior, and the host's treatment of adversarial content are all **pending/unobserved**, not passing.

## Recorded evidence

- **Tested version:** `2.1.266 (Claude Code)` on 2026-09-09.
- **Binding mechanism:** an inline MCP configuration gives one stock Claude process one per-process stdio server entry. The launcher opts in only that exact entry with `--dangerously-load-development-channels server:<name>`; no repository, recency, or timestamp lookup exists.
- **Rust implementation:** dependency-free beyond the spike's existing `serde_json`; it reads newline-delimited JSON-RPC over stdio, answers `initialize`, declares `capabilities.experimental["claude/channel"] = {}`, waits for `notifications/initialized`, then emits one `notifications/claude/channel` event.
- **Library support:** no Rust MCP crate was added. The official reference names `@modelcontextprotocol/sdk` and a Node.js-compatible runtime as the hard requirement, so adopting a Rust MCP crate would not turn this into a documented/supported route. The raw Rust server is retained only as evidence that the small wire shape can be expressed.
- **Tools and permission relay:** the initialize response omits both `capabilities.tools` and `capabilities.experimental["claude/channel/permission"]`. The event metadata fixes `authorization=none` and `provenance=untrusted_agent_result`. The server implements no inbound permission-request handler and can emit no permission verdict.
- **Preview consent:** the wrapper passes the documented per-entry development flag but does not pass `--dangerously-skip-permissions`, a permissive permission mode, or any permission handler. Anthropic documents that this flag bypasses the preview allowlist only; the confirmation prompt and `channelsEnabled` organization policy remain in force. The fake launcher proves construction only; no consent screen was observed.
- **Arguments and terminal:** after its channel flags, the launcher appends user arguments unchanged and uses `exec`, leaving terminal descriptors, signals, resize behavior, and exit status to the stock Claude process. Fake coverage verifies the argument vector. No real PTY comparison was performed.
- **Notification timing:** not observed in a real TUI. The fake client proves notification ordering after MCP initialization, not Claude's busy-turn queue behavior.
- **Concurrent-session isolation:** the configuration is per launched process, but two real same-repository Claude TUIs were not observed. Runtime isolation remains unproven.
- **Durable fallback:** the Claude adapter has no board dependency and cannot acknowledge or mutate a result. U1 remains the evidence that failed delivery preserves unread board state; consent decline and disconnect were not exercised through a real host.

## Deterministic commands

```text
claude --version
# 2.1.266 (Claude Code)

claude --help
# exposes --mcp-config and --strict-mcp-config; the preview development flag is documented but hidden from this help output

cargo test --test launcher
# 6 passed
```

The U3 tests were added before the implementation. The first red attempt could not reach compilation because `cargo` was absent from `PATH`; the installed pinned toolchain was then invoked explicitly. After implementation, all 11 crate tests pass, including the 6 launcher/channel tests. Because the pre-implementation attempt was an infrastructure failure rather than the intended behavioral failure, it is recorded as incomplete red evidence rather than misreported as a clean red/green cycle.

## Manual evidence still required

First resolve the documented Node-only support boundary with Anthropic or an official Rust-supported path. If a Rust route becomes supported, run the wrapper in a real PTY against a synthetic board and retain the full preview warning and confirmation flow. Test idle delivery and ordered delivery during an active turn; two sessions in the same repository; consent decline, organization-policy rejection, subprocess disconnect, and session exit; and an adversarial envelope claiming approval. Confirm after every attempt that the durable result remains unread until explicit acknowledgment. Until then, the Claude path remains no-go.
