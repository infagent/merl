# Agent-mail validation

This slice implements [#145](https://github.com/infagent/merl/issues/145). It was exercised through the public CLI and its clock-controlled public Rust entry point. Synthetic host executables and Unix sockets keep automated checks independent of models, credentials and the developer's home.

## Automated behavior

`cargo test --locked --test mail --test help_contract` verifies these distinct risks:

- A wrong recipient cannot read or complete another delivery; a receipt and a linked reply do not complete its request.
- Concurrent publishers commit complete messages; exact retries remain unchanged and altered retries conflict.
- Offline dispatch survives reopening the store, coalesces messages into one pointer and stops retrying after application receipt.
- An outdated binding generation or replaced socket cannot silently select another session.
- Read-but-unreceipted notes and requests stay recoverable; receipted unfinished requests remain in the outstanding folder; duplicate handling preserves original timestamps.
- A foreground watcher reports temporary dispatch-lock contention and resumes without restarting.
- Successful host enqueue has a receipt grace period rather than rapidly queuing duplicate model turns.
- Output-format precedence matches the existing CLI contract for help and errors.
- A socket-gated concurrent scenario proves that an older in-flight enqueue cannot overwrite the due time of a new batch after receipt.
- New mail joins the pending batch's notification window, while a fully receipted batch does not delay genuinely new work.

The initial scenarios failed because the interface did not exist. Interrupted read recovery, watcher contention, receipt grace, and new-message coalescing each had a failing regression before its fix. Root scenarios contain Given/When/Then steps; mechanics and assertions live in `tests/helpers/mail.rs`. Existing help-tree traversal discovers each new operation and checks both formats.

## Live host and three-role evidence

With installed Codex CLI 0.164.0, bounded shell-originated host probes first verified that `codex queue` could wake two existing idle sessions. Those bootstrap probes are separate from Merl delivery.

A dedicated local test mailbox then registered and explicitly bound three existing coordinator/developer/reviewer sessions. The foreground dispatcher was started and stopped. Two synthetic requests were published while it was stopped and remained pending. Restarting that same dispatcher enqueued bounded pointers; both idle recipients read the durable messages, recorded receipt, returned linked nonce replies through Merl and explicitly completed the original requests. A Merl reply woke the existing developer session after its turn ended. No human prompt, native-message nudge, fresh exec replacement or provider API integration drove those deliveries. Subsequent coordination used Merl.

The first live run also exposed repeated empty wake turns: short retry delays had queued duplicate notifications before normal receipt. That observation was retained as a finding, not hidden as successful exactly-once delivery. Two deterministic regressions drove the receipt-grace and pending-batch coalescing fixes. Application receipt still stops retries; a missing receipt remains recoverable. The final head requires independent exact-head review and green CI before handoff. The user has not authorized a merge.

Runtime thread IDs, socket paths, mailbox bodies, nonces and host receipts remain local rather than entering this public record. This is functional integration evidence, not a token-cost benchmark or proof of OS isolation. Host enqueue is not application receipt, and neither implies completion.

## Reproduce locally

Run the checks in `CONTRIBUTING.md`, including formatting, workspace Clippy/tests and rustdoc with warnings denied. Unix-socket and existing bubblewrap isolation checks require an environment that permits those OS primitives; they do not have an unrestricted runtime fallback.

For a real host test, follow [agent-mail setup](agent-mail.md): choose an isolated absolute mailbox home, explicitly bind consenting existing sessions, end recipient turns, publish synthetic mail while the foreground dispatcher is stopped, then restart it. Observe durable read/receipt/completion and matching replies through Merl. Stop only the foreground process you started. Keep private runtime evidence out of public commits and PR comments.
