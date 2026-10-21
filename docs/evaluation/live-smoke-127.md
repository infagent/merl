# Live GitHub smoke test: #127

Date: 2026-09-28

The manual run completed against [infagent/merl #127](https://github.com/infagent/merl/issues/127).
Capture, model compilation, review, source expansion, idempotent refresh, replay,
rebuild, and purge worked through CLI subprocesses. It found one release-contract
gap: a new comment produced two accepted revisions and inbox entries because
GitHub also advanced the Issue's `updatedAt` timestamp. [#128](https://github.com/infagent/merl/issues/128)
owns that gap before the #52 freeze. This run does not establish model quality or
clear the wider release audit.

## Build and configuration

| Item | Value |
| --- | --- |
| Merl commit | `b81c8103274f6ea8ca453482edc2ccc3bc2ff6d1` |
| Build | `cargo build --locked --bin merl`, debug profile |
| Binary SHA-256 | `fca25f5bfa41f39606f78f0c320e3bfa24ab2b0b282899f4193b96dc47adaa47` |
| Tool versions | Rust 1.98.1; gh 2.97.0; Claude Code 2.1.284; Python 3.14.7 |
| Provider | Authenticated `/usr/bin/gh api graphql`; no fake responses |
| Compiler | One-off process wrapper `smoke-127-v1`, using Claude Code print mode |
| Model | Requested `sonnet`; all three calls reported `claude-sonnet-5-5` |
| Model controls | Low effort, tools and MCP disabled, safe mode, no session persistence, one turn, no automatic API retries |
| Call limits | 180-second timeout and $1 per-call budget; Merl's 2,048 output-token cap passed through `CLAUDE_CODE_MAX_OUTPUT_TOKENS` |
| Adapter SHA-256 | `2edc665d9689ffb60220823b256fdad421061d1ba5988eabc9106e99ce4b3c0a` |
| Prompt SHA-256 | `b68ca91df1bdaad59060d51565f21f8cbe789415ff12eba1b34b0ebec744fce7` |

The wrapper supplied Merl's context and an index of exact line byte offsets. The
prompt asked for at most two independent interpretations of the triggering source,
with no invented authority or copied prose in structural fields. It treated source
instructions as data. The wrapper parsed the model's JSON without repairing or
replacing assertions. Merl then validated the response and applied policy.

The output cap uses Claude Code's documented
[environment variable](https://code.claude.com/docs/en/env-vars).
The three calls used 819 output tokens in total. Claude Code reported $0.0319336
at list prices; that is not a claim about subscription billing. There were no
compiler failures or repair calls.

The Issue was small, public, and dedicated to this test. Its original body was
captured before adding the one user-approved
[test comment](https://github.com/infagent/merl/issues/127#issuecomment-5878019361).
No repository code, Issue body, labels, assignees, or CI settings changed during
the run.

## Commands and observed results

Every product command used `--project smoke127 --database <isolated-path> --json`.
Project creation started without a database. The run made 57 CLI invocations;
none opened tables or called application crates. IDs below come from public
results, not recreated Merl hash functions.

| Commands | Observed result |
| --- | --- |
| `project init --id smoke127 --administrator smoke-owner`; `project authority grant --permission command_actor --subject smoke-reviewer`; `inbox subscribe --agent smoke-reader` | Project initialized at revision 0; review grant accepted at revision 1; reader subscribed. |
| `issue capture --repository infagent/merl --issue 127 --github-program /usr/bin/gh --program <wrapper> --compiler-version smoke-127-v1 --model sonnet --prompt-digest sha256:<above>` | One source captured and eagerly compiled; no failures; provider snapshot accepted at revision 2. |
| `source assertions --run <initial-run>`; `source apply --run <initial-run> --actor smoke-reviewer --id apply-initial` | Two task assertions, both held as candidates. Neither became accepted from compilation alone. |
| `candidate list`; `candidate show <id>`; `candidate accept <id> --actor smoke-reviewer --id review-initial-<index>` | Accepted `task_manual_live_cli_sanity_check` at revision 3 and `task_save_evidence_record` at revision 4 after inspecting their source spans. |
| `show <task> --source --history`; `source show --version <initial-source>` | Both objects expanded through policy input, compiler run, source version, and exact retained text. Their byte spans were `[12,254)` and `[1507,1703)`, respectively. |
| `project authority grant --permission decision_author --subject <captured-author>`; `issue view`; `inbox poll/ack` | Grant accepted at revision 5. Required coverage reached observation 1; all entries were acknowledged in order. |
| Repeat `issue capture`; `inbox poll` | `unchanged`, zero new sources, zero compiler calls, no provider change, revision still 5, empty inbox. |
| `source replay --run <initial-run> --new-run smoke-replay-initial --program <wrapper> ...`; `compilation show` | Recorded input digest matched; the separate model-backed replay succeeded. Accepted revision remained 5. |
| `project rebuild`; `issue view`; `project delta --since 0`; `inbox poll` | View and delta matched their pre-replay JSON values; the inbox remained empty. |
| Post approved comment, then `issue capture` | One new source and one successful compiler run. A timestamp-only provider update committed revision 6 before semantic application. |
| `source assertions/apply`; `candidate show/accept`; `show <decision> --source --history` | The model interpreted the comment as a reported positive decision with act `claim`. Policy kept it as a candidate despite the author's grant. Review accepted `decision_smoke_local_inbox_subscriber` at revision 7. The cited `[109,188)` span exactly matched the decision sentence. |
| `project delta --since 5`; `inbox poll` | Two entries: provider Issue at revision 6, accepted decision at revision 7. Neither entry was truncated. This is the #128 failure. |
| Repeat capture and candidate review; `inbox poll/ack` | No new captures, compiler calls, or accepted effects. The review returned its original receipt. Both entries were acknowledged; final cursor was 7 and required coverage reached observation 2 with zero gaps. |

The compiler's `claim` classification was retained as returned. Review adopted the
decision because its exact source span supported it. The smoke test did not alter
the interpretation to manufacture automatic acceptance.

## Purge evidence

Purge ran on a filesystem copy of the closed, checkpointed revision-5 authority.
This preserved the original local database for the later-comment check. The copy
was not edited or seeded through SQLite. The command sequence was:

```text
source purge --version <initial-source> --reason 'Erase local smoke-test source copy' --dry-run
source purge --version <initial-source> --reason 'Erase local smoke-test source copy' --actor smoke-owner --confirm-digest <preview-digest>
source purge-audit --version <initial-source>
source show --version <initial-source>
show task_manual_live_cli_sanity_check --source --history
source replay --run <initial-run>
project rebuild
issue view --issue <provider-issue> --scope I_kwDOUfFJis8AAAABTuNoUQ
```

The preview named both compiler runs and both accepted tasks. Purge completed;
the audit retained the actor, reason, preview digest, and matching payload digests.
Source and derived evidence became unavailable, support became `unsupported`, and
replay exited with `MISSING_EVIDENCE`. Rebuild retained accepted revision 5 and
reported five erased payloads with degraded provenance. Required coverage then
reported one purged source and one gap, rather than claiming completeness.

The receipt covered only the active store. The original local database, diagnostic
exports, and GitHub source remained outside that erasure claim.

## Follow-up and retained evidence

For the new comment, public Issue views differed only in provider revision and
`upstream_updated_at_millis` (`1790615846000` → `1790627718000`). The current
ingestion path compares serialized provider snapshots, so that timestamp change
caused the extra accepted provider event. The #70 fake adds a comment without
advancing the enclosing Issue timestamp. #128 requires a realistic regression and
a resolution that preserves provider facts and freshness semantics.

During the run, I saved diagnostic artifacts under
`/tmp/merl-smoke-127-bgxn95ru`: metadata and hashes, compiler
prompt/configuration/wrapper, three model call records, and each CLI command with
its JSON result. The live authority was `project.sqlite`; the erased copy was
`purge.sqlite`. Temporary-directory cleanup may remove these files. This
checked-in report records the findings for future reference; it does not archive
the raw artifacts or provide a portable evaluation package.

No planned smoke-test stage was skipped. Revalidation, context expansion,
permission failures, pagination, and a representative model-quality evaluation
were outside this one-off run; their automated or evaluation evidence remains
separate.
