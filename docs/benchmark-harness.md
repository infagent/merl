# Benchmark harness

`merl-eval` runs the five Issue-reading methods in [the first-release plan](first-release.md#evaluation-corpus). It is evaluator-side tooling. Keep natural captures that await redistribution review, scoring keys, and held-out files outside implementation-agent workspaces.

The current runner handles one question at one causal cutoff. Each paired trial gives the same question, system prompt, task prompt, model identity, effort, and sampling settings to all five readers. The methods differ in the context they receive:

| Reader | Initial context | Optional detail |
| --- | --- | --- |
| Raw history | Visible Issue body and comments through the cutoff | None |
| Rolling summary | Summary updated after each source observation | None |
| Recent retrieval | Opening source and a recent window | Search older visible sources |
| Summary plus retrieval | Rolling summary | Search visible sources |
| Merl | Accepted Issue view from a prepared authority | Expand an object or source reference |

An edit replaces the version shown by the raw and recent readers at that cutoff. The summary reader sees edits in observation order. Neither receives a later correction early. A fixture with missing historical bodies, ambiguous ordering, or insufficient causal fidelity cannot enter a causal trial.

## Running a development case

Prepare one Merl authority database per paired trial. Import and compile only the fixture observations through the question's cutoff, then evaluate policy and check required semantic coverage. Close and checkpoint each database before the run; the harness rejects a nonempty WAL and checks that the database bytes do not change while readers use it. Record the compiler adapter's actual provider-reported input and output tokens, including corrections and retries. The harness cannot infer that cost from the size of a stored context.

Run:

```sh
cargo run --locked -p merl-eval -- run --plan /path/to/development-plan.json
```

`merl-eval inspect --plan /path/to/plan.json` prints the exact artifact digests the freeze record needs. It does not invoke the model or scorer. Run it only in the evaluator environment when the plan names sealed material.

The plan uses `merl.eval-plan/v1`. It names the fixture, question, shared model configuration, model and scorer programs, process byte limits, and the prepared Merl trials. Each Merl trial names a database and a `merl.eval-merl-usage/v1` record. That record includes the project, source cutoff, compiler-run IDs, and measured token usage. The runner checks that the record agrees with the plan and records SHA-256 digests of the record and database in its report. It reads Merl views through the same public CLI used by agents.

A development plan looks like this, with paths supplied by the evaluator:

```json
{
  "schema": "merl.eval-plan/v1",
  "fixture": "corpus/development/DEV-C1.json",
  "question": {
    "id": "DEV-C1-Q1",
    "cutoff": 4,
    "text": "What receive-gain strategy is current for the baseline?"
  },
  "config": {
    "model": "configured-model",
    "model_version": "exact-version",
    "effort": "medium",
    "system_prompt": "Use only the context and tools supplied for this trial.",
    "task_prompt": "Answer the project question and cite the evidence you used.",
    "trials": 2,
    "recent_window": 2,
    "max_tool_rounds": 3,
    "search_results": 2,
    "temperature": 0,
    "seed": 7
  },
  "model_program": "/path/to/model-adapter",
  "scorer_program": "/path/to/development-scorer",
  "merl": {
    "project": "P1",
    "issue": "I1",
    "scope": "controlled:DEV-C1",
    "role": "researcher",
    "trials": [
      {
        "database": "/path/to/trial-1.sqlite",
        "usage_record": "/path/to/trial-1-usage.json",
        "preparation_usage": { "input": 1300, "output": 160 },
        "causal": true
      },
      {
        "database": "/path/to/trial-2.sqlite",
        "usage_record": "/path/to/trial-2-usage.json",
        "preparation_usage": { "input": 1320, "output": 155 },
        "causal": true
      }
    ]
  },
  "limits": { "max_request_bytes": 1048576, "max_response_bytes": 262144 },
  "freeze": null
}
```

Each usage record has this shape; token counts must come from the compiler adapter's provider response, not a character-count estimate:

```json
{
  "schema": "merl.eval-merl-usage/v1",
  "project": "P1",
  "source_cutoff": 4,
  "compiler_runs": ["CR1", "CR2", "CR3", "CR4"],
  "usage": { "input": 1300, "output": 160 }
}
```

The model program receives one JSON request on stdin and returns one JSON response on stdout. Both use `merl.eval-adapter/v1`. Requests have `kind: summarize` or `kind: answer`; answer requests include the method, question, disclosed context, and whether search or expansion is available. Responses have `kind: summary`, `final`, `search`, or `expand`, plus provider-reported `usage.input` and `usage.output`. An answer may request at most the plan's tool-round budget. Search results come from source bodies available at the causal cutoff; Merl expansions come from the prepared authority. The adapter never receives the answer key.

The scorer is a separate evaluator-owned program. It receives `merl.eval-score-request/v1` with the question ID, answer, and citations, then returns `merl.eval-score/v1` with correctness, provenance, stale-state, and missed-blocker grades. The harness passes `MERL_EVAL_SCORING_SPEC` only to this scorer when a frozen scoring specification is supplied. A scorer can leave ambiguous grades unadjudicated rather than awarding an automatic success.

Reports use `merl.eval-report/v1`. They retain each answer and its reported usage, mean and sample variance by method, and the first downstream read count at which each method's average preparation plus reading cost becomes lower than repeated raw-history reads. An absent break-even count means the measured per-read saving cannot recover preparation cost. The calculation uses exact integer token totals; floating-point values appear only in descriptive statistics.

## Held-out gate

The implementation team should run development and visible adversarial cases only. An independent evaluator keeps the approved held-out package and scorer outside this repository. Before a held-out run, the plan requires a freeze record naming the candidate commit and binding the evaluator binary, approved corpus manifest, fixture, question, model configuration, model program, scorer program, and scoring specification by SHA-256. The runner checks those artifact hashes before it invokes the model. The evaluator must also confirm that the fixture and question belong to the approved manifest and that the binary came from the named commit; the runner cannot prove either fact from an opaque manifest and a commit string.

The handoff quoted in the #24 discussion names a different manifest digest from [the one recorded with the corpus](evaluation-corpus.md). The curator should identify which approved manifest is current before anyone creates a held-out freeze record. Do not resolve that by recapturing or opening the sealed cases in an implementation workspace.

## Still needed for #24

This runner is an executable comparison contract, not a release result. The visible repository has three controlled development histories. Two natural captures remain local pending privacy and redistribution review. The staged adversarial cases are not yet versioned fixtures. The natural histories need independent labels and disagreements; the benchmark needs real model and scorer adapters, measured Merl preparation logs, and development/adversarial trial reports. A multi-question aggregate report must avoid charging one Issue's shared compilation separately for every question. None of this calls for opening the held-out set yet.
