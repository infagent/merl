# Visible package integration

The maintainer approved visible v2 for a first pass: 38 questions, 57 question-cutoff units, 43 distinct case/cutoff prefixes and eight cases. X01–X14 remain recorded exclusions, without success credit. Historical pending/zero fields in the frozen proposal stay unchanged; the receipt-bound scope overlay records effective approval. This integration does not close #131 or establish a benchmark result.

The package adapter implements the field names demonstrated in the planner's wholly synthetic interface sample. Only synthetic examples have been executed. The evaluator must verify the actual package and its optional field variants before #132. Natural captures and answers remain in the evaluator workspace; redistribution permission is separate from correctness approval.

## Artifact contract

`VerifiedPackage::open` requires out-of-band pins for `package.sha256`, the approved inventory, the approved rules and the expected unit count. A newly calculated manifest hash is not an authenticity check. The evaluator obtains the package-manifest pin from the package owner through a trusted channel. The approved v2 pins are:

| Artifact | SHA-256 |
| --- | --- |
| question-inventory.json | `94f9e4c2309e3db5fe710653c82d13f7dfde4676ce3699d354d42d882001ddb2` |
| scoring-rules.json | `d466d535d8395c80dc30f01558f4bd657acc0a2568374780fd5c95a2a2407e89` |
| proposal-v2.sha256 | `b757dd109cfee26b10acba890549ea7937539414ecf84049dc194a92a5ff5ae6` |

Package digest fields use bare lowercase hex; observation bodies and the existing preparation protocol use `sha256:` digests. The package manifest resolves POSIX paths from the package root. The nested proposal manifest resolves paths from its containing directory. Hashes cover original file bytes, not reserialized JSON.

Verification checks the manifest, receipt, approved artifacts, scope overlay, unit index, reader manifest, per-unit allowlists, oracle copies and method/rubric copies. It rejects unsafe paths, symlinks, unlisted/missing files, duplicate destinations, changed prompts, changed approved meanings, duplicate/missing units, future observations/evidence, inconsistent prefixes and denominator changes. Limits bound files, bytes, directories and nesting. Verified bytes are snapshotted; changing the source paths afterward cannot change staged inputs. Diagnostics and `Debug` omit source and oracle contents.

The unit ID is exactly `question_id@cutoff`. The reader question is exactly `approved_answer.question_at_cutoff`, never the descriptive inventory title. Every occurrence of a repeated question remains a separate unit. `verify_prefix` compares the original fixture JSON observations with the approved prefix before parsing can erase optional fields. `run_prefix` then truncates the fixture and clears gold states before rendering, retrieval or stateful summary preparation. Unavailable body bytes stay unavailable under the existing body-availability rules. Terminal provider facts are not disclosed to readers.

## Process contract

The approved package defines file allowlists and grading policy. It does **not** define an executable sandbox, checkpoint RPC, broker or registered categorical JSON response schema. The interfaces in this section are additions made by this harness integration.

On Linux, `IsolatedProgram` snapshots the executable and runs each invocation in a fresh bubblewrap user, mount, PID and network namespace. It clears the environment, disables networking, mounts the OS runtime at `/usr` read-only, supplies private `/tmp` and working directories, and exposes only explicitly selected files at `/input`. The runtime must contain OS software only; keep evaluation packages, adapters with private data, fixtures and authorities outside `/usr`. There are no evaluator-home, host temporary-directory or sibling-unit mounts. Executable, stdin, staged files, stdout and execution time have hard bounds. Missing or denied isolation fails execution; there is no unrestricted fallback. CI installs bubblewrap, gives only `/usr/bin/bwrap` a scoped AppArmor `userns` permission on Ubuntu, and runs the isolation tests. It leaves the system-wide user-namespace restriction enabled. The launcher explicitly requires its own user namespace and disables further user namespaces before executing workers. Ubuntu's [release notes](https://documentation.ubuntu.com/release-notes/24.04/#unprivileged-user-namespace-restrictions) explain the scoped permission; bubblewrap documents the [nested-namespace restriction](https://github.com/containers/bubblewrap/blob/main/bwrap.xml).

The trusted controller can read the four-file reader stage (`prompt.json`, `source.json`, `method.json`, `instructions.txt`). `source.json` is the eligible preparation/retrieval pool. **Model workers receive no mounted input files.** They see only the existing `merl.eval-adapter/v1` stdin request, containing the verified reader instructions and general method as labeled context, their method's disclosed evidence, and bounded tool results. The question string stays verbatim; model configuration remains shared and unchanged. This preserves the five-method comparison: summaries and Merl cannot bypass their disclosure or token budgets by reading the raw source pool. Requests do not identify the benchmark arm.

The categorical grader runs in another fresh namespace. Its five static files are exactly this unit's `prompt.json`, `source.json`, `method.json`, `oracle.json`, and `grading-instructions.txt`. A sixth runtime file, `response.json`, contains the same JSON request as stdin:

```json
{
  "schema": "merl.eval-categorical-request/v1",
  "unit_id": "SYN-Q1@1",
  "answer": "An amber heading.",
  "citations": ["O1"]
}
```

The new categorical parser implements the sample's proposed response shape:

```json
{"unit_id":"SYN-Q1@1","verdict":"correct","reason":"The required instructed color is present."}
```

The only verdicts are `correct`, `incomplete`, `incorrect`, and `not_scored`. The unit must match and the reason must be nonempty. Legacy numeric-score objects and unknown fields are rejected. The grader applies approved meanings, accepted alternatives, uncertainty/history guards and evidence scope from its unchanged oracle and method files. The harness does not replace semantic grading with object keys, support-state enums or string equality. It does not assign numeric weights to these categories. Grader output never enters a reader request or later preparation.

The implementation supports deterministic local workers. A network-denied worker cannot call a remote provider. An evaluator-owned transport or narrowly scoped broker still needs an interface and isolation review before remote execution; its endpoint, credentials and launch protocol are not supplied by the approved package. Do not enable general networking or use the legacy unrestricted subprocess path to bypass this gap.

## Preparation and accounting

Use one closed authority per paired trial, case and cutoff, with the existing `merl.eval-preparation/v1` receipts. Each authority must match its source head, candidate/preparer/compiler artifacts, causal runs and reported usage. The existing verifier rejects terminal provider mirrors in observation-only trials, unavailable body text, hindsight runs, missing calls, changed files and nonempty WALs. The harness verifies all supplied authorities before invoking readers or graders and checks authority bytes again afterward. Changed authorities invalidate grades while retaining known usage and answers. This command consumes prepared artifacts; it does not launch an untrusted candidate preparer with the evaluator package in scope.

A fresh rolling summary starts from an empty string for each cutoff/trial and receives only eligible observations in order. Same-cutoff questions reuse that preparation. The two summary arms share physical summary calls; their attributable costs remain separate comparisons. A different cutoff gets fresh summary state and a distinct Merl authority. No later summary, retrieval pool or database can serve an earlier cutoff.

`merl.eval-visible-report/v1` retains all approved unit IDs and native categorical counts per method. Its expected count is approved units multiplied by paired trials, including failed and missing slots. `not_scored`, failed and missing remain separate from correct; exclusions add no slots. Each submitted slot must belong to an approved prefix and trial, and duplicates fail validation.

Preparation records retain costs once per applicable method, trial and prefix, including known summary calls before failure. Per-answer usage and tool rounds stay separate. A failure does not transfer a preparation charge onto every question or erase known calls. An unsuccessful model call may have unknown additional usage; `usage_incomplete` or an incomplete preparation record makes that limitation explicit. Failed grading retains the submitted answer, citations and reported read usage. Failure of a shared summary skips only its dependent scoring slots. No break-even claim is made from incomplete categorical execution.

The execution envelope records the plan, evaluator binary, worker executable, preparation-record and database digests, plus runtime stdin/stdout and mounted-file hashes. Runtime answer-file hashes are separate from static package hashes. Keep detailed reports local: they contain answers and grader reasons and may contain natural text. Only non-sensitive counts and verification attestations should leave the evaluator workspace without redistribution approval.

## Evaluator commands

Run from the repository root with the pinned Rust toolchain on `PATH`, Python 3, `/usr/bin/timeout` and bubblewrap installed. Run isolation tests on a Linux host where bubblewrap can create its own namespaces. An outer sandbox may deny nested namespaces; that is an execution failure, not permission to fall back to an unrestricted process.

First run only synthetic integration and isolation checks:

```sh
rtk proxy cargo test --locked --test evaluation_isolation --test visible_package
rtk proxy cargo test --locked -p merl-eval --test visible_cli
rtk proxy cargo build --locked -p merl-eval
```

Then set `PACKAGE_ROOT` to the evaluator-owned visible package and `PACKAGE_MANIFEST_SHA256` to its independently supplied manifest pin. This command verifies integration without invoking a model or grader or reporting benchmark scores:

```sh
rtk proxy target/debug/merl-eval package verify \
  --package "$PACKAGE_ROOT" \
  --manifest-sha256 "$PACKAGE_MANIFEST_SHA256" \
  --inventory-sha256 94f9e4c2309e3db5fe710653c82d13f7dfde4676ce3699d354d42d882001ddb2 \
  --rules-sha256 d466d535d8395c80dc30f01558f4bd657acc0a2568374780fd5c95a2a2407e89 \
  --expected-units 57
```

Expected output lists all 57 exact unit IDs, 43 source prefixes, retained exclusion IDs and `scored_units: 0`. The package owner should confirm eight cases and 38 distinct question IDs from that list. Preserve any integrity error as a failed integration check; do not alter the approved artifacts to make the check pass.

A new evaluator-owned `merl.eval-visible-plan/v1` selects execution inputs. It is an integration plan, not a replacement for the approved package schemas. Required fields are:

| Field | Value |
| --- | --- |
| `schema` | `merl.eval-visible-plan/v1` |
| `package` | `root`, `manifest_sha256`, `inventory_sha256`, `rules_sha256`, `expected_units` |
| `config` | Existing `BenchmarkConfig`, shared across all five methods and prefixes |
| `model_program`, `model_program_sha256` | Isolated model worker path and byte digest |
| `grader_program`, `grader_program_sha256` | Separate categorical worker path and byte digest |
| `limits` | `request_bytes`, `response_bytes`, `program_bytes`, `seconds` |
| `prefixes` | Entries described below, one per distinct case/cutoff |

Each prefix entry has `case`, `cutoff`, `fixture`, `fixture_sha256`, `candidate_preparer_program`, `candidate_commit`, `compiler_artifacts`, and `merl`. The last two retain the fields shown in the [existing harness plan](benchmark-harness.md); `merl.trials` lists exactly one independently prepared authority per configured trial at this cutoff, in trial order, all with `capture_phase: false`. Fixture and worker digests accept bare hex or `sha256:`. The package route rejects held-out fixtures.

After constructing that plan and pinning its exact bytes externally, verify prepared-artifact integration without model/scorer execution:

```sh
rtk proxy target/debug/merl-eval package inspect \
  --plan "$VISIBLE_PLAN" --plan-sha256 "$VISIBLE_PLAN_SHA256"
```

Inspection verifies supplied authorities and returns every scoring slot as missing because nothing ran. It is not an OS isolation attestation: use the synthetic tests above. The synthetic sample's placeholder authorities are not SQLite and must fail this readiness check. Partial plans retain missing units explicitly; a failed preflight returns a versioned error and no scores.

The same plan can later be passed to `package run` by the evaluator during the separately arranged benchmark. An incomplete run writes its partial report to stdout and exits 1; a completed run exits 0, including legitimate incorrect/incomplete/not-scored verdicts. Transport integrity does not certify answer quality. No scored benchmark was run to implement this integration.

Remaining work belongs to the planner/evaluator: verify the actual 57-unit package and all field variants, confirm all 43 prepared prefix groups and provider usage, arrange a restricted remote transport if needed, choose and verify the independent semantic grader, reconcile wider corpus/split commitments and reviewer-role wording, and arrange #132. Held-out work and v4 are outside this integration.
