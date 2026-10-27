# Synthetic full-exchange comparison

Issue #172 compares a released 0.5.0 binary with the candidate through their public CLIs. Both use isolated synthetic mailboxes, a local Unix socket and a host executable that records accepted notices without invoking Codex or a model. The same short assignment and linked result pass through read/checkpoint/receipt and explicit request completion. This measures transport/CLI mechanics, not semantic model behavior or live idle wake.

## Reproduce

Build the candidate, then supply the explicit released binary path:

```sh
cargo build --locked --bin merl
python3 scripts/compare_mail_exchange.py \
  --baseline /absolute/released-0.5/merl --candidate target/debug/merl \
  --output /tmp/merl-exchange.json
```

The script creates/removes only its own temporary binary copies and synthetic homes. It verifies equal-length executable/home paths so embedded argv spelling cannot bias the comparison. It records source binary hashes, version, every command's stdout/stderr/argv bytes, accepted notices and disk artifacts. It opens no existing mailbox, invokes no real host/model and requires no credentials. Both packages still report 0.5.0 before release automation; the binary hash distinguishes the candidate.

## Observed comparison

Measured on Linux with the synthetic request/result in the script:

| Observation | Released 0.5 | Candidate |
| --- | ---: | ---: |
| Setup commands, reported separately | 4 | 4 |
| Cold usage-help commands | 1 | 1 |
| Assignment/result/completion commands | 11 | 7 |
| Final verification commands | 2 | 2 |
| All command stdout bytes | 4,682 | 4,036 |
| Usage-help stdout, already included above | 1,693 | 1,775 |
| Command stderr bytes | 0 | 0 |
| Accepted notification bytes | 518 | 1,325 |
| Checkpoint bytes | 156 | 156 |
| Subtotal stdout + stderr + notices + checkpoints | 5,356 | 5,517 |
| Authored body-file bytes | 58 | 58 |
| All command argv JSON bytes | 2,855 | 2,365 |
| Sum of these recorded representations | 8,269 | 7,940 |
| Synthetic host-accepted events | 2 | 2 |
| Real model calls | 0 | 0 |
| Provider-reported usage | not invoked/measured | not invoked/measured |

The candidate removes four CLI operations by using complete inline bodies and compact explicit handling. The stdout/notice/checkpoint subtotal increases by 161. Including authored files and the captured argv JSON representations, the accounting sum decreases by 329. Neither byte comparison establishes actual model input/output, token cost or savings. Version/hash probes are excluded from exchange counts; setup/help/verification remain visible. Both invocation and mailbox paths are 32 UTF-8 bytes here; the utility asserts equal lengths across variants. Other path lengths and actual protocol framing can change these totals on another run.

## Exposure surfaces

The scenario maps usage help, authored sends and recipient handling results to agents; setup/dispatch/final verification output goes to the operator. Notifications are agent-facing input data. Checkpoints and authored bodies are disk artifacts. This is an explicit synthetic mapping, not a captured human transcript, actual model context or API tool framing:

| Captured representation | Released 0.5 | Candidate |
| --- | ---: | ---: |
| Agent command stdout, including help | 3,162 | 2,516 |
| Agent command argv JSON | 1,539 | 1,049 |
| Host notice input data | 518 | 1,325 |
| Operator command stdout | 1,520 | 1,520 |
| Operator command argv JSON | 1,316 | 1,316 |
| Actual model input/output bytes and usage | unknown/not invoked | unknown/not invoked |

Inline metadata omits per-entry recipient duplication and volatile read/receipt/completion timestamps; the recipient is explicit once, and authoritative handling facts remain in CLI results. Short instructions and direct complete bodies reduce extra fetch operations without dropping durable IDs, sender/kind/reply linkage, creation time or exact overflow references. Both model-visible tool framing and natural-language model output remain unmeasured; command counts are not measured tool/model turns. All real-model-call counts are zero.

## Duplicate and race evidence

`tests/mail.rs` separately exercises a controlled accepted-event queue: two notices for the same immutable ID are accepted before application receipt. After receipt no new dispatch occurs, while the already accepted duplicate remains available to the host. That reproduces one way a notice can be obsolete when eventually consumed; it does not establish the cause of the 14 real empty drains.

Another public scenario pauses one host call, receipts a later selected recipient, then resumes dispatch. The candidate returns `obsolete` and creates no host event for that now-empty selection. Existing public cases cover missing-receipt retry, partial/new pending work, full-receipt/new-batch publication during host I/O and binding replacement. Snapshot reads and lease creation share a bounded transaction; host I/O does not hold it. Already accepted queue cancellation is unsupported, and the successful-enqueue grace remains 60 seconds.
