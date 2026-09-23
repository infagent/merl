# Benchmark harness

`merl-eval` runs the five Issue-reading methods in [the first-release plan](first-release.md#evaluation-corpus). It is evaluator-side tooling. Keep natural captures that await redistribution review, scoring keys, and held-out files outside implementation-agent workspaces.

The runner handles several questions per Issue, including questions at different cutoffs. Within a paired trial, all five readers get the same question, system prompt, task prompt, model identity, effort, and sampling settings. Only their context and available tools differ:

| Reader | Initial context | Optional detail |
| --- | --- | --- |
| Raw history | Visible Issue body and comments through the cutoff | None |
| Rolling summary | Summary updated after each source observation | None |
| Recent retrieval | Opening source and a recent window | Search older visible sources |
| Summary plus retrieval | Rolling summary | Search visible sources |
| Merl | Accepted Issue view from a prepared authority | Expand an object or source reference |

An edit replaces the version shown by the raw and recent readers at that cutoff when its order is known. Missing historical bodies appear as gaps. A question names an observation cutoff and separately says whether terminal capture is included; the latter is permitted only after the final observation. The summary reader receives terminal-capture text only in that phase. For timestamp-tied observations with unresolved upstream order, the reader sees the retained versions and an explicit uncertainty marker. The report records when a cutoff splits such a group; the model does not see later observations. Exact replay still refuses ambiguous order or missing historical bytes.

## Running a development case

Prepare one Merl authority database per paired trial and source cutoff with the candidate preparer named in the plan. Import only observations through that cutoff. Historical compilation withholds bodies known only from terminal capture, even at the final observation; it must not retroactively interpret old prose with newly captured bytes. The terminal reader can see those bytes at capture time, while Merl reports the resulting semantic gap. Compile and evaluate policy against admissible evidence, then close and checkpoint the database. The preparer writes a receipt containing its own executable digest and the digest of the closed database. The harness checks both before any reader runs. It also rejects a nonempty WAL and checks that the database bytes do not change while readers use it. Keep the provider-reported input and output tokens for every compiler call, including retries and corrections. The harness cannot infer that cost from the size of a stored context.

Run:

```sh
cargo run --locked -p merl-eval -- run --plan /path/to/development-plan.json
```

`merl-eval inspect --plan /path/to/plan.json` prints the exact artifact digests the freeze record needs. It does not invoke the model or scorer. Run it only in the evaluator environment when the plan names sealed material.

The plan uses `merl.eval-plan/v1`. It names the fixture, questions, shared model configuration, model and scorer programs, the candidate preparer executable, compiler artifacts, process byte limits, and prepared Merl trials. Each preparation record must match the candidate commit, candidate preparer, closed authority database, exact compiler program/prompt/rules/configuration hashes, and every recorded compiler run. The run modes must be causal (`live` or `replay`), never `hindsight` or `eval`. Its measured call ledger includes retries and corrections. The evaluator binary has a separate freeze digest because it verifies and reads the authority; it does not claim to have produced it.

Questions at the same cutoff and capture phase share one rolling summary and Merl preparation within a paired trial. Their answer calls remain separate. The report charges each shared preparation once, not once per question. A different cutoff needs a different prepared authority and summary. One plan cannot mix observation-only and terminal-capture questions at the same cutoff; split them into separate plans so neither phase inherits the other's evidence.

A development plan looks like this, with paths supplied by the evaluator:

```json
{
  "schema": "merl.eval-plan/v1",
  "fixture": "corpus/development/DEV-C1.json",
  "questions": [{
    "id": "DEV-C1-Q1",
    "cutoff": 4,
    "capture_phase": false,
    "text": "What receive-gain strategy is current for the baseline?"
  }],
  "config": {
    "model": "configured-model",
    "model_version": "exact-version",
    "effort": "medium",
    "system_prompt": "Use only the context and tools supplied for this trial.",
    "task_prompt": "Answer the project question and cite the evidence you used.",
    "trials": [
      { "id": "pair-a", "randomness": { "seed": 7 } },
      { "id": "pair-b", "randomness": { "seed": 11 } }
    ],
    "recent_window": 2,
    "max_tool_rounds": 3,
    "search_results": 2,
    "temperature": 0
  },
  "model_program": "/path/to/model-adapter",
  "scorer_program": "/path/to/development-scorer",
  "candidate_preparer_program": "/path/to/frozen-merl-preparer",
  "candidate_commit": "40-character-candidate-git-sha",
  "compiler_artifacts": {
    "program": "/path/to/compiler-adapter",
    "prompt": "/path/to/compiler-prompt",
    "rules": "/path/to/compiler-rules",
    "config": "/path/to/compiler-model-config"
  },
  "merl": {
    "project": "P1",
    "issue": "I1",
    "scope": "controlled:DEV-C1",
    "role": "researcher",
    "trials": [
      {
        "trial_id": "pair-a",
        "source_cutoff": 4,
        "capture_phase": false,
        "database": "/path/to/trial-1.sqlite",
        "preparation_record": "/path/to/trial-1-preparation.json"
      },
      {
        "trial_id": "pair-b",
        "source_cutoff": 4,
        "capture_phase": false,
        "database": "/path/to/trial-2.sqlite",
        "preparation_record": "/path/to/trial-2-preparation.json"
      }
    ]
  },
  "limits": { "max_request_bytes": 1048576, "max_response_bytes": 262144 },
  "freeze": null
}
```

The candidate preparer writes one `merl.eval-preparation/v1` record for each prepared authority. It names the paired trial, project, cutoff, candidate commit, preparer executable digest, and closed database digest. Its `compiler` section gives the compiler ID/version/model, SHA-256 of each of the four artifacts, and a domain-separated contract digest. `merl-eval inspect` prints that digest; it must be the `prompt_digest` recorded on every contributing compiler run. The `runs` array lists every run in durable attempt order with its mode, cutoff, selector, and renderer. The `calls` array lists each provider call with a stable call ID, run ID, phase (`compile`, `retry`, `correction`, or `revalidation`), and reported input/output tokens. `total_usage` is their sum. Do not estimate usage from text length.

```json
{
  "schema": "merl.eval-preparation/v1",
  "trial_id": "pair-a",
  "project": "P1",
  "source_cutoff": 4,
  "capture_phase": false,
  "candidate_commit": "40-character-candidate-git-sha",
  "candidate_preparer_binary_sha256": "sha256:...",
  "authority_sha256": "sha256:...",
  "compiler": {
    "id": "configured-compiler",
    "version": "v1",
    "model": "configured-model",
    "program_sha256": "sha256:...",
    "prompt_sha256": "sha256:...",
    "rules_sha256": "sha256:...",
    "config_sha256": "sha256:...",
    "contract_sha256": "sha256:..."
  },
  "runs": [{
    "id": "CR1",
    "mode": "replay",
    "source_cutoff": 1,
    "selector_version": "issue_context_v1",
    "renderer_version": "json_v1"
  }],
  "calls": [{
    "id": "compiler-call-1",
    "run_id": "CR1",
    "phase": "compile",
    "usage": { "input": 1300, "output": 160 }
  }],
  "total_usage": { "input": 1300, "output": 160 }
}
```

The model program receives one JSON request on stdin and returns one JSON response on stdout. Both use `merl.eval-adapter/v1`. Requests have `kind: summarize` or `kind: answer`, the paired trial ID and randomness setting, and the shared model configuration. Answer requests include the question, disclosed context, and available search or expansion capabilities. They do not name the benchmark method. Responses have `kind: summary`, `final`, `search`, or `expand`, plus provider-reported `usage.input` and `usage.output`. An answer may request at most the plan's tool-round budget. Search results come from source bodies available at the cutoff; Merl expansions come from the prepared authority. The adapter never receives the answer key.

The scorer is a separate evaluator-owned program. It receives `merl.eval-score-request/v1` with the question ID, answer, and citations, then returns `merl.eval-score/v1` with correctness, provenance, stale-state, and missed-blocker grades. The harness passes `MERL_EVAL_SCORING_SPEC` only to this scorer when a frozen scoring specification is supplied. A scorer can leave ambiguous grades unadjudicated rather than awarding an automatic success.

Reports use `merl.eval-report/v1`. They retain every summary call and answer/tool round with its provider-reported usage, each question's source fidelity, mean and sample variance by method, failure categories, and the first downstream suite read count at which a method's preparation plus reading cost falls below repeated raw-history reads. The summary is generated once in the harness and reused by both summary-based methods; each method is charged that preparation in its own counterfactual cost comparison. Merl preparation calls stay in the attested record. An absent break-even count means the measured per-read saving cannot recover preparation cost. The calculation uses exact integer token totals; floating-point values appear only in descriptive statistics.

## Held-out gate

The implementation team should run development and visible adversarial cases only. An independent evaluator keeps the held-out package and scorer outside this repository. Before a held-out run, the plan requires a freeze record naming the candidate commit and binding both the evaluator and candidate preparer binaries, approved corpus manifest, fixture, questions, model configuration, model program, scorer program, scoring specification, preparation records, and prepared databases by SHA-256. The freeze contains one receipt and database digest for every trial/cutoff authority. The runner checks those hashes before it invokes the model. The evaluator must also confirm that the fixture and questions belong to the approved manifest and that both binaries came from the named commit; the runner cannot prove either fact from an opaque manifest and a commit string.

Fresh GitHub captures mark retained terminal bodies `at_capture`. A v4 migration may promote a body to `at_observation` only when the evaluator can document that the exact bytes were retained at that historical position. Otherwise the body stays terminal-only and earlier readers see a gap.

Held-out v3 remains evaluator-only archival material. The independent evaluator will build, review, and freeze v4 against the final corpus and harness contracts. No implementation agent should inspect v3 or v4 cases while finishing #24.

## Still needed for #24

This runner is an executable comparison contract, not a release result. Three controlled adversarial histories cover relayed authority, ambiguous replies, and relative time with a PR prerequisite. Purged evidence and optional-to-required promotion are exercised as authority-state acceptance scenarios because neither is an event in the shared Issue history. Approved natural captures still need independent labels. Real model/scorer trials and the development/adversarial report also remain #24 work. None calls for opening the held-out set.

Do not build v4 while Merl's public workflow is still changing. First, run the complete first-release path through public commands and fix every gap that does not depend on held-out results. Then freeze Merl, the corpus format, and this harness. The evaluator builds and reviews v4 against that frozen version, and #24 can close.

Next, freeze the exact candidate and evaluation settings before #32 runs v4. The last audit collects the evidence and makes the release decision. It cannot change the candidate that #32 tested.
