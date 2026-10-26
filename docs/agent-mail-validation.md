# Messaging validation

[#145](https://github.com/infagent/merl/issues/145) and [PR #146](https://github.com/infagent/merl/pull/146) replace the former product with durable agent messaging. The current validation suite covers this product; removed semantic/compiler/evaluation behavior is retained in Git history, not maintained as active code or release gates.

## Automated behavior

`cargo test --locked --test mail --test product` exercises twelve public scenarios. Synthetic executables, explicit local paths, Unix-socket handshakes and a caller-controlled clock avoid model/account dependencies.

The ten mail scenarios protect these distinct risks:

- Wrong recipient/reply handling; receipt and linked reply do not complete a request.
- Concurrent complete publication and immutable idempotent send retries.
- Offline recovery, coalesced bounded pointers and stopping retries on receipt.
- Stale generation and replaced-socket binding rejection.
- Interrupted reads without receipt; outstanding work after receipt; idempotent timestamps.
- Foreground watcher contention followed by resumption without restart.
- Receipt grace after successful host enqueue, while missing receipts stay retryable.
- New messages joining a pending batch; immediately due work after full receipt.
- Last output-format option controlling help and errors.
- Old in-flight host finalization preserving a new batch published during its I/O.

Two product scenarios protect the current boundary: root/help exposes the documented mail and lane operations with matching human/JSON help; retired command groups reject before modifying an opaque old runtime file or creating any replacement store there. The original replacement scenarios failed on the additive implementation; ADR 0005 extends the supported help tree with reviewer lanes. The ten messaging scenarios remain unchanged in meaning.

Root scenarios use Given/When/Then steps and helpers hold mechanics/assertions. Run the full workspace checks and Python convention checks from [Contributing](../CONTRIBUTING.md). There is no evaluator runtime, paid provider or live Codex call in the automated suite.

## Live evidence and binary provenance

The [lane suite](reviewer-lanes.md) adds public scenarios for durable multi-PR discovery, source pages/errors/deadlines, inline replies, unchanged polls, pause/selector changes, cursor gaps, frozen publication retry and configuration races. Controlled subprocess handshakes cover removing a launch checkout during both host delivery and GitHub polling. A narrow retry-date unit test covers UTC/HTTP dates and HTTP 200 rate metadata. These tests are independent of live delivery.

The initial host feasibility gate used the installed Codex queue interface to wake existing idle sessions. That native/bootstrap evidence is separate from Merl application delivery.

The additive implementation then completed a three-role Merl roundtrip: synthetic requests persisted while the scoped foreground dispatcher was stopped, reached existing idle recipients after restart, and received linked replies and explicit completion. A reply woke the ended developer session. That run also exposed queued duplicate wakes before receipt, which drove deterministic receipt-grace/coalescing regressions. Independent review drove the socket-gated finalization-race regression. These findings were retained rather than counted as exactly-once success.

The product replacement additionally requires a relevant three-agent smoke using the replacement application binary. Its local record identifies the exact head/hash, each send/read/receipt/reply/complete binary, the binary owning host dispatch, existing session bindings, and any deliberate scoped dispatcher transition. A pass using only the former binary cannot establish the replacement application boundary. No old runtime store is migrated or deleted during smoke.

Runtime IDs, socket paths, message bodies, nonces and raw host receipts remain local. Public evidence reports the boundary and result without disclosing them. Live functional evidence is not a token-cost benchmark, OS-isolation proof or exactly-once execution guarantee. Final readiness requires the replacement smoke, complete reduced checks, green CI and independent review of the final head. The human has not authorized a merge.

## Reproduce safely

Reviewer-lane acceptance additionally requires read-only GitHub input to generate mail and wake an observed idle reviewer through the candidate combined watcher, without a native nudge. Record actual application/dispatch provenance. A manually authored transport nonce proves recovery only. Use controlled boundaries for multi-PR/restart/rebinding coverage where public test writes would create litter, and report those boundaries without calling them live evidence.

Follow [setup and recovery](agent-mail.md). Select a dedicated mailbox home and explicitly bind consenting existing sessions. Identify the application/dispatch binary revisions, end recipient turns, publish synthetic mail while the owned dispatcher is stopped, and restart only that dispatcher. Observe durable read/receipt/completion and matching replies through Merl. Preserve external data and keep a working binary available throughout transition. Stop only the foreground process you started.
