# Live result delivery spike findings

## Recommendation

**Overall verdict: no-go for production live delivery at the tested host versions.** Keep Merl's existing plugin-only, durable pull workflow and do not proceed to a production wrapper, watcher, or daemon from this spike.

The transport-neutral Rust core and deterministic Codex boundary are useful partial evidence. They prove exact requester filtering, one-attempt-per-wrapper deduplication, fixed untrusted framing, board preservation, exact UUID argument construction, and stock-process `exec` launch behavior. A real queue attempt against the exact active Codex thread was rejected because direct app-server input is forbidden for multi-agent-v2 sessions. The Claude subprocess expresses the small documented wire shape, but Anthropic's current Channels documentation requires the TypeScript MCP SDK and a Node.js-compatible runtime; Rust is not a supported implementation route.

Neither host cleared exact live binding, safe-boundary delivery, concurrent isolation, and trust gates. U4 was therefore intentionally stopped and the four-way U5 matrix was not run. Calling this a partial-go would imply at least one viable host; the evidence does not support that claim.

## Decision matrix

| Direction | Status | Evidence and reason |
|---|---|---|
| Claude requester, Codex worker | **Attempted / fail** | Claude started, but managed policy reported `channelsEnabled` was not enabled and silently dropped inbound messages. |
| Codex requester, Claude worker | **Attempted / fail** | The exact active thread rejected `thread/queue/add`: direct app-server input is not allowed for multi-agent-v2 sessions. |
| Claude requester, Claude worker | **Attempted / fail** | The real TUI rejected development channels under organization policy, so no message could reach Claude. |
| Codex requester, Codex worker | **Attempted / fail** | Exact UUID targeting reached the app server, which rejected delivery for the active multi-agent-v2 session before safe-boundary behavior could be tested. |
| Requester absent, delivery unavailable, or delivery fails | **Pass, deterministic** | The core never acknowledges or mutates the request. Tests prove byte-for-byte preservation and continued `merl results` visibility after failure. Unwrapped sessions retain pull behavior because no production code changed. |

"Not run" is a gate result: the plan requires stopping instead of building broader matrix machinery when a host cannot first prove binding, safe delivery, and authorization boundaries. Fake-process tests do not support a production transport claim.

## Requirement audit

| Requirement | Status | Evidence or gap |
|---|---|---|
| R4 exact requester routing | **Partial** | Synthetic tests filter by `requester_session_id`; Codex accepts only an explicit UUID; Claude configuration is per process. Real same-repository isolation was not observed. |
| R5 no acknowledgment and bounded duplicates | **Pass, deterministic** | Adapters cannot mutate the board. Tests prove one attempt per request ID during one core lifetime, including after failure. Restart duplicates remain intentionally unsolved. |
| R6 durable fallback | **Pass, deterministic** | Successful and failed attempts leave the result unread and returned by the existing results command. `make check` verifies the production workflow remains green. |
| R7 all four directions | **Fail / not run** | Neither requester host passed its gate, so no direction has defensible live evidence. |
| R8 non-authorizing context | **Partial** | A fixed untrusted, `authorization=none` header precedes JSON-encoded adversarial payload. Claude exposes no tools or permission relay in the fake protocol. Neither real host's interpretation was observed. |
| R9 unwrapped plugin-only behavior | **Pass** | The spike is isolated under `spikes/live-delivery/`; production Python, packaging, and plugin manifests are unchanged. |
| R10 Rust experiment | **Pass as a stop result** | The spike is Rust. Claude's documented TypeScript SDK and Node-compatible requirement makes the planned Rust route unsupported; no JavaScript shim was introduced. |
| R11 versioned evidence | **Pass as a no-go record** | Versions, mechanisms, unobserved timing/consent, failure modes, security gaps, commands, and verdicts are recorded below. |

## Codex gate

**Host verdict: no-go for multi-agent-v2 sessions at the tested version.**

- **Tested version:** `codex-cli 0.153.4` on 2026-09-09.
- **Binding:** `codex resume <UUID>` and `codex queue --thread <UUID> --message <envelope>`. The spike rejects aliases such as `last` and never selects by repository, age, recency, or rollout timestamp.
- **Protocol evidence:** the experimental schema exposes `thread/start`, `thread/resume` with `threadId`, and `thread/queue/add` with `threadId`, `clientUserMessageId`, and `input`. CLI help exposes `resume [SESSION_ID]` and `queue --thread <THREAD>`.
- **Passed:** fake-process tests record exact arguments, inherited environment, unchanged user arguments, and child exit status. `exec` leaves terminal descriptors, signals, resizing, and final status directly owned by Codex after replacement.
- **Observed real-host result:** `codex queue --thread <exact-active-UUID> --message <fixed-envelope>` reached `thread/queue/add` and failed with JSON-RPC code `-32600`: direct app-server input is not allowed for multi-agent-v2 sub-agents. This is the session topology Merl is intended to coordinate, so the transport fails before delivery timing or trust behavior can be evaluated.
- **Still unobserved:** idle and active-turn queue timing, two-TUI same-repository isolation, consent or approval behavior, and adversarial-input classification. Schema/help availability and fixed framing cannot establish these behaviors.
- **Failure:** malformed bindings fail before launch; spawn and non-zero queue exits become delivery errors. The core retains the unread result and suppresses another attempt only for that wrapper lifetime.

## Claude Channel gate

**Host verdict: no-go in the tested organization, independently of the unsupported Rust route.**

- **Tested version:** `2.1.266 (Claude Code)` on 2026-09-09.
- **Binding:** one inline MCP configuration supplies one Claude process with one stdio server entry; `--dangerously-load-development-channels server:<name>` opts in only that entry. No heuristic lookup is used.
- **Implementation:** the raw Rust server uses existing `serde_json`, answers `initialize`, declares only `capabilities.experimental["claude/channel"]`, waits for `notifications/initialized`, and emits one channel notification. No Rust MCP crate or Node shim was added.
- **Support boundary:** Anthropic's Channels reference identifies `@modelcontextprotocol/sdk` and a Node.js-compatible runtime as the hard requirement. Mimicking the wire shape in Rust is not a documented supported route, so R10 and KTD4 require stopping here.
- **Permissions:** the server declares neither tools nor `claude/channel/permission`, implements no permission handler, and fixes `authorization=none` plus untrusted provenance. The launcher passes no permission-bypass mode.
- **Observed real-host result:** the stock TUI started and reported that `--dangerously-load-development-channels` was blocked by organization policy, inbound messages would be silently dropped, and an administrator must set `channelsEnabled: true` in managed settings. It also reported that no MCP server was configured with the selected name; this may be a separate selector or inline-configuration defect, but fixing it cannot overcome the policy block.
- **Still unobserved:** successful registration, preview consent, idle/busy timing, same-repository isolation, disconnect/exit handling, and adversarial-content treatment. Fake-client tests prove only protocol construction and ordering.

## Verification evidence

Final deterministic checks:

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo test --doc
cargo deny check                    # not run: cargo-deny is not installed
make check                         # repository root
```

The Rust tests cover reconciliation, exact requester selection, wake deduplication, malformed-board recovery, failed-delivery durability, adversarial envelope encoding, Codex UUID arguments and exit status, Claude capability minimization, notification ordering, and consent-flag construction.

`cargo fmt --check`, Clippy with warnings denied, all-feature tests, doctests, and `make check` passed. `cargo deny check` could not run because this environment has no `cargo-deny` subcommand; dependency-policy verification is therefore an explicit residual rather than a claimed pass.

Real delivery attempts were made against both hosts. Codex rejected direct app-server input for the exact active multi-agent-v2 thread. Claude stopped at managed policy before delivery. The full matrix was therefore not run. No test used a real `~/.merl`; board tests use temporary synthetic state.

## Smallest retained evidence

The retained crate contains the delivery core, existing-CLI board adapter, exact Codex UUID/queue boundary, minimal raw Rust Claude protocol probe, and focused tests. Each supports a deterministic result or stop condition. There is no watcher, production daemon, released `merl run`, plugin wiring, Python migration, JavaScript shim, or completed U4 lifecycle layer to remove.

## Reconsideration gates

- **Codex:** first obtain a supported injection surface for multi-agent-v2 sessions. Only then record idle and active-turn delivery, two-session isolation, exited-thread failure, permission behavior, adversarial content, and unchanged unread state.
- **Claude:** first obtain an officially supported Rust Channel route; then record consent/policy outcomes, idle and busy ordering, two sessions, disconnect/exit behavior, adversarial content, and unchanged unread state.
- **Matrix:** only after a requester host clears its gate, run applicable directions against a temporary `MERL_HOME`. A both-hosts go still requires all four directions.
