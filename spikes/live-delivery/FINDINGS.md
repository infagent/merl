# Live result delivery spike findings

## Recommendation

**Current verdict: partial-go for the Claude transport; no-go for Codex.** A real Rust Claude Channel registered, delivered, and preserved host-visible provenance. Codex delivered to the exact TUI but represented the result as ordinary user input. Production work still requires the remaining Claude lifecycle, timing, and isolation gates.

The transport-neutral Rust core and deterministic Codex boundary prove exact requester filtering, one-attempt-per-wrapper deduplication, fixed untrusted framing, board preservation, exact UUID argument construction, and stock-process launch behavior. A real queue attempt reached the exact top-level Codex TUI, while the nested multi-agent-v2 thread correctly rejected direct input. The delivered envelope appeared as an ordinary user message, however, so its `authorization=none` text is not host-enforced provenance and fails the authorization boundary. In contrast, the real Claude TUI labeled the message with its channel source and retained structured `authorization=none` and `provenance=untrusted_agent_result` attributes.

Claude cleared registration, trust, and real post-startup delivery despite Rust not being a documented implementation route. The U4 wrapper delivered an answer written after launch to the exact per-process channel and left it unread on the board. The four-way U5 matrix and production plugin identity reuse remain incomplete.

## Decision matrix

| Direction | Status | Evidence and reason |
|---|---|---|
| Claude requester, Codex worker | **Pass for idle delivery** | An answer written after startup reached the exact wrapped Claude channel with host-visible non-authorizing provenance and remained unread. |
| Codex requester, Claude worker | **Delivered / security fail** | Exact top-level routing worked, but the result arrived as an ordinary user message; nested multi-agent-v2 delivery was rejected. |
| Claude requester, Claude worker | **Pass** | Registration, post-startup delivery, busy-turn queuing, and two-session same-repository isolation worked with exact per-process bindings. |
| Codex requester, Codex worker | **Delivered / security fail** | Exact top-level routing worked, but the host provided no trusted distinction between queued agent context and human input. |
| Requester absent, delivery unavailable, or delivery fails | **Pass, deterministic** | The core never acknowledges or mutates the request. Tests prove byte-for-byte preservation and continued `merl results` visibility after failure. Unwrapped sessions retain pull behavior because no production code changed. |

"Not run" is a gate result: the plan requires stopping instead of building broader matrix machinery when a host cannot first prove binding, safe delivery, and authorization boundaries. Fake-process tests do not support a production transport claim.

## Requirement audit

| Requirement | Status | Evidence or gap |
|---|---|---|
| R4 exact requester routing | **Partial** | Synthetic tests filter by `requester_session_id`, and a real exact top-level Codex UUID received its message. Two-session isolation and live Claude routing were not observed. |
| R5 no acknowledgment and bounded duplicates | **Pass, deterministic** | Adapters cannot mutate the board. Tests prove one attempt per request ID during one core lifetime, including after failure. Restart duplicates remain intentionally unsolved. |
| R6 durable fallback | **Pass, deterministic** | Successful and failed attempts leave the result unread and returned by the existing results command. `make check` verifies the production workflow remains green. |
| R7 all four directions | **Fail / not run** | Neither requester host passed its gate, so no direction has defensible live evidence. |
| R8 non-authorizing context | **Pass for Claude; fail for Codex** | Claude retained channel source plus structured `authorization=none` and untrusted provenance. Codex represented the same boundary as an ordinary user message. |
| R9 unwrapped plugin-only behavior | **Pass** | The spike is isolated under `spikes/live-delivery/`; production Python, packaging, and plugin manifests are unchanged. |
| R10 Rust experiment | **Pass as a stop result** | The spike is Rust. Claude's documented TypeScript SDK and Node-compatible requirement makes the planned Rust route unsupported; no JavaScript shim was introduced. |
| R11 versioned evidence | **Pass as a no-go record** | Versions, mechanisms, unobserved timing/consent, failure modes, security gaps, commands, and verdicts are recorded below. |

## Codex gate

**Host verdict: no-go because queued input has user-message authority at the tested version.**

- **Tested version:** `codex-cli 0.153.4` on 2026-09-09.
- **Binding:** `codex resume <UUID>` and `codex queue --thread <UUID> --message <envelope>`. The spike rejects aliases such as `last` and never selects by repository, age, recency, or rollout timestamp.
- **Protocol evidence:** the experimental schema exposes `thread/start`, `thread/resume` with `threadId`, and `thread/queue/add` with `threadId`, `clientUserMessageId`, and `input`. CLI help exposes `resume [SESSION_ID]` and `queue --thread <THREAD>`.
- **Passed:** fake-process tests record exact arguments, inherited environment, unchanged user arguments, and child exit status. `exec` leaves terminal descriptors, signals, resizing, and final status directly owned by Codex after replacement.
- **Observed real-host routing:** an exact top-level UUID received the complete queued envelope. An exact nested multi-agent-v2 UUID failed with JSON-RPC code `-32600` because direct app-server input is not allowed for sub-agents. Merl can therefore target the owning TUI, but not inject directly into its workers.
- **Observed trust failure:** the queued envelope entered the conversation as a normal user-role message. The host did not attach a trusted agent-result role or authorization metadata, so a textual `authorization=none` prefix cannot enforce the R8 boundary.
- **Still unobserved:** active-turn queue timing, two-TUI same-repository isolation, consent or approval behavior, and exited-thread behavior. These cannot reverse the trust failure.
- **Failure:** malformed bindings fail before launch; spawn and non-zero queue exits become delivery errors. The core retains the unread result and suppresses another attempt only for that wrapper lifetime.

## Claude Channel gate

**Host verdict: partial-go after a successful real Rust-channel delivery.**

- **Tested version:** `2.1.266 (Claude Code)` on 2026-09-09.
- **Binding:** one inline MCP configuration supplies one Claude process with one stdio server entry; `--dangerously-load-development-channels server:<name>` opts in only that entry. No heuristic lookup is used.
- **Implementation:** the raw Rust server uses existing `serde_json`, answers `initialize`, declares only `capabilities.experimental["claude/channel"]`, waits for `notifications/initialized`, and emits one channel notification. No Rust MCP crate or Node shim was added.
- **Support boundary:** Anthropic's Channels reference identifies `@modelcontextprotocol/sdk` and a Node.js-compatible runtime as the hard requirement. Mimicking the wire shape in Rust is not a documented supported route, so R10 and KTD4 require stopping here.
- **Permissions:** the server declares neither tools nor `claude/channel/permission`, implements no permission handler, and fixes `authorization=none` plus untrusted provenance. The launcher passes no permission-bypass mode.
- **Observed real-host result:** after `channelsEnabled` was enabled in managed policy, the stock TUI registered the Rust subprocess and delivered its message. Claude displayed the channel source and treated the structured `authorization=none` and untrusted provenance as data rather than a user request. The TUI also printed `no MCP server configured with that name`, but that warning did not prevent registration or delivery.
- **Observed post-startup result:** against a fresh synthetic `MERL_HOME`, a second Merl session claimed and answered a request after the wrapped Claude TUI was already open. The polling watcher reconciled the canonical results view and Claude received the expected request ID, requester ID, answer, evidence, source, and non-authorizing provenance without a pull command. A subsequent `merl results` query returned the same unacknowledged result.
- **Observed busy-turn result:** a second answer was written at 12:11:23 local time while Claude was running an unrelated foreground command from 12:11:15 through 12:11:45. The channel event queued visibly, did not interrupt the command or turn, and was processed only after the turn completed normally.
- **Observed isolation result:** two wrapped Claude sessions in the same repository used separate synthetic boards and exact session bindings. The first received only `ISOLATION-B`; the second received only `ISOLATION-C`. Neither result crossed channels.
- **Identity presentation fix:** both Claude agents initially called their own result foreign because the opaque Merl session ID was not explained in model-visible context. The IDs exactly matched their wrapper-created bindings. The live channel now includes its bound Merl session ID in trusted server instructions so the model can compare it with each result instead of guessing.
- **Still unobserved on a real host:** disconnect/exit handling, explicit consent behavior, and a stronger adversarial-content probe. The undocumented Rust support status is a compatibility risk, not an observed blocker.
- **U4 deterministic lifecycle evidence:** the wrapper invokes exactly one existing CLI `init --json` against the selected synthetic home, passes the returned session ID and `MERL_HOME` into both the stock TUI environment and its private channel command, then replaces itself with Claude via `exec`. The channel performs startup reconciliation and uses `notify` 8.2.0 (CC0-1.0/Artistic-2.0) filesystem events only as wake hints before rereading the canonical CLI results view. Duplicate wakeups inject a request only once per channel lifetime.
- **Transparency and fallback:** fake-host tests preserve user argument order, working directory, inherited descriptors/environment, normal and abnormal exit status, and the operating system's direct signal/PTY/resize ownership after `exec`. If initialization fails, the wrapper prints pull guidance and still `exec`s the unmodified stock host without a channel. Closing channel stdin exits the bridge; Claude owns the only bridge subprocess, so no independent supervisor remains.
- **R3 blocker:** `MERL_SESSION_ID=ses-wrapper` is demonstrably inherited by the stock TUI and the exact identity is embedded in its per-process channel configuration. The production `merl init` command always creates a fresh session and plugin instructions explicitly invoke it; they do not consume `MERL_SESSION_ID`. KTD1 forbids changing production Python or plugin manifests/instructions in this spike. Therefore repeated plugin `init` cannot yet reuse the wrapper identity, and R3 is unresolved rather than simulated.

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

The Rust tests cover reconciliation, exact requester selection, wake deduplication, malformed-board recovery, failed-delivery durability, adversarial envelope encoding, Codex UUID arguments and exit status, Claude capability minimization, notification ordering, consent-flag construction, wrapper initialization/identity propagation, post-startup filesystem reconciliation, transparent abnormal exit, and pull-only fallback.

`cargo fmt --check`, Clippy with warnings denied, all-feature tests, doctests, and `make check` passed. `cargo deny check` could not run because this environment has no `cargo-deny` subcommand; dependency-policy verification is therefore an explicit residual rather than a claimed pass.

Real delivery attempts were made against both hosts. Codex delivered to the exact top-level TUI but represented the result as ordinary user input. After managed policy was enabled, Claude registered the raw Rust channel, preserved its non-user provenance, and received an answer created after startup while the durable result stayed unread. The full matrix is not yet complete. No board test used a real `~/.merl`; synthetic tests use temporary state.

## Smallest retained evidence

The retained crate contains the delivery core, existing-CLI board adapter, exact Codex UUID/queue boundary, minimal raw Rust Claude protocol, a per-session `notify` watcher, transparent wrapper, and focused tests. Each supports a deterministic result or stop condition. There is no production daemon, released `merl run`, plugin wiring, Python migration, or JavaScript shim.

## Reconsideration gates

- **Codex:** first obtain a host-enforced non-user provenance or authorization boundary for queued input. Direct delivery to nested workers is unnecessary if the owning TUI can route work through native agent messaging, but advisory envelope text is insufficient.
- **Claude:** first resolve R3 by defining a production-authorized way for plugin `init` to reuse the inherited wrapper session, and obtain an officially supported Rust Channel route; then record consent/policy outcomes, idle and busy ordering, two sessions, disconnect/exit behavior, adversarial content, and unchanged unread state.
- **Matrix:** only after a requester host clears its gate, run applicable directions against a temporary `MERL_HOME`. A both-hosts go still requires all four directions.
