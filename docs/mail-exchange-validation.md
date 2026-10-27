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

The script creates/removes only its own temporary synthetic homes. It records binary hashes, version, every command's stdout/stderr bytes, accepted notice content and checkpoint bytes. It opens no existing mailbox, invokes no real host/model and requires no credentials. Both packages still report 0.5.0 before release automation; the binary hash distinguishes the candidate.

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
| Accepted notification bytes | 512 | 1,657 |
| Checkpoint bytes | 156 | 156 |
| Total stdout + stderr + notices + checkpoints | 5,350 | 5,849 |
| Synthetic host-accepted events | 2 | 2 |
| Real model calls | 0 | 0 |
| Provider-reported usage | not invoked/measured | not invoked/measured |

The candidate removes four CLI operations by using complete inline bodies and compact explicit handling. Total recorded bytes increase by 499 because content and handling guidance now travel inline. These are CLI counts, not measured tool/model turns, tokens, cost or savings. Version/hash probes are excluded from the exchange counts. Setup, help and final verification remain visible; checkpoint bytes are not omitted. Temporary path/timestamp lengths can change byte totals on another run. No actual provider usage was available in this synthetic experiment.

## Duplicate and race evidence

`tests/mail.rs` separately exercises a controlled accepted-event queue: two notices for the same immutable ID are accepted before application receipt. After receipt no new dispatch occurs, while the already accepted duplicate remains available to the host. That reproduces one way a notice can be obsolete when eventually consumed; it does not establish the cause of the 14 real empty drains.

Another public scenario pauses one host call, receipts a later selected recipient, then resumes dispatch. The candidate returns `obsolete` and creates no host event for that now-empty selection. Existing public cases cover missing-receipt retry, partial/new pending work, full-receipt/new-batch publication during host I/O and binding replacement. Snapshot reads and lease creation share a bounded transaction; host I/O does not hold it. Already accepted queue cancellation is unsupported, and the successful-enqueue grace remains 60 seconds.
