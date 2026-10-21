# Restricted model transport

`merl-eval` can broker remote generation for network-denied reader, compiler and categorical-grader workers. This is execution plumbing for #132. It selects no provider and includes only a synthetic mock-provider adapter. The approved questions, answers and scoring rules are unchanged. No scored corpus calls were made to implement it.

The controller owns a pinned endpoint and named credential headers. Workers send text messages over their process pipes. A separate pinned provider adapter translates those messages into a JSON body, then translates the HTTP response into text, reported usage and provider identifiers. Both translation invocations run in fresh network-denied namespaces, with an empty `/input`. Only the controller performs HTTP. Neither workers nor translators receive credentials, endpoint configuration, evaluator files, filesystem capabilities or a general HTTP client.

Reader workers still receive only method-disclosed evidence. Graders retain their separate unit allowlist, including the oracle. A reader cannot select the grader transport. Each role has its own configuration and ledger. The controller never returns grader output to readers or preparation. All worker and translator invocations keep [the existing bubblewrap boundary](visible-package-integration.md#process-contract), including fresh writable state and disabled nested user namespaces. Keep evaluator material and private adapter data outside the shared OS runtime at `/usr`.

## Configuration

An evaluator-owned transport configuration contains no credential values:

```json
{
  "schema": "merl.remote-transport/v1",
  "role": "reader",
  "endpoint": "https://provider.invalid/generate",
  "credential_headers": [
    {"name": "Authorization", "env": "READER_PROVIDER_KEY", "prefix": "Bearer "}
  ],
  "adapter_program": "/evaluator/adapters/provider-translator",
  "adapter_program_sha256": "REPLACE_WITH_EXECUTABLE_SHA256",
  "settings": {
    "model": "evaluator-selected-model",
    "model_version": "evaluator-selected-version",
    "effort": "none",
    "temperature": 0,
    "max_output_tokens": 1024
  },
  "limits": {
    "request_bytes": 1048576,
    "response_bytes": 262144,
    "program_bytes": 67108864,
    "seconds": 30
  },
  "max_calls": 1
}
```

The controller resolves only the explicitly named environment variables. Set them in the trusted controller environment using the evaluator's credential mechanism. Missing/empty credentials fail preflight. Do not put a key in the endpoint, executable, prompt, plan or configuration file. No ambient proxy, credential helper, shell configuration or child environment is inherited. `Debug`, errors and receipts omit credential values. The broker checks raw response bytes and recursively checks decoded JSON strings and object keys for credentials, including JSON-escaped echoes, before invoking the provider decoder. Rejected responses cannot populate provider identifiers or reach the worker. This is an extra guard, not permission to send credentials in request bodies.

The endpoint is a fixed HTTPS URI with a path, without user information, query, fragment or whitespace. Plain HTTP is accepted only for a literal loopback IP, for mock/local endpoints. HTTPS uses certificate verification with the compiled public trust roots. Redirects and environment proxies are disabled explicitly using [ureq configuration](https://docs.rs/ureq/3.4.2/ureq/config/struct.ConfigBuilder.html). The broker submits one JSON POST per allowed generation, without automatic retries, streaming, cookies or provider tools. Workers and translators cannot choose a URL or headers. Configured header names cannot replace host, framing or content headers. Providers requiring query credentials, dynamic destinations, custom CA roots, streaming or interactive login need a separately reviewed extension; none is inferred here.

`limits` bound each translator invocation and HTTP request/response. HTTP response headers have a separate 32 KiB ceiling. The worker's own process limits bound its lifetime and cumulative stdout transcript. An HTTP exchange already in progress can finish its bounded attempt after a worker times out; its receipt is retained. `max_calls` is an explicit per-worker allowance from 1 through 16, including failed attempts. Use 1 unless a reviewed adapter requires more. Every new attempt gets a receipt; there are no hidden correction or retry calls.

Add these optional fields to the existing `merl.eval-visible-plan/v1`:

```json
{
  "model_transport": {"path": "/evaluator/reader-transport.json", "sha256": "CONFIG_SHA256"},
  "grader_transport": {"path": "/evaluator/grader-transport.json", "sha256": "SEPARATE_CONFIG_SHA256"}
}
```

The reader transport's model, version, effort and temperature must match the plan's shared `config`. Host-selected trial randomness is passed to the translator; workers cannot override it. The grader configuration must have `role: grader`, its own endpoint/credential references and explicitly chosen model settings. The evaluator supplies and pins a separate categorical-grader worker. Its semantic validity remains the evaluator's responsibility. Sharing an endpoint or account does not share model context: concrete translators must issue stateless requests without conversation IDs, stored history, remote tools or persistence.

Offline workers may omit transport pins and retain the original EOF protocol. A configured remote worker never falls back to that protocol or an unrestricted subprocess. Missing/denied bubblewrap, a bad pin, an unavailable provider, a protocol failure or a missing credential fails execution.

## Worker extension

Remote workers opt into JSON lines over the existing stdin/stdout pipes. Initial request and final response payloads retain the existing reader, grader or compiler JSON formats. This extension is a harness interface, not a schema claimed by the approved package.

1. Read the initial JSON request using one `readline`, rather than waiting for EOF.
2. Write and flush a generation frame. Only `schema` and `messages` are accepted; there must be 1–128 text messages with role `system`, `user` or `assistant`.

   ```json
   {"schema":"merl.model-call/v1","messages":[{"role":"user","text":"Synthetic question?"}]}
   ```

3. Read one result line from the controller:

   ```json
   {"schema":"merl.model-result/v1","kind":"success","text":"Synthetic answer.","usage":{"input":10,"output":5},"model":"resolved-model","request_id":"provider-id"}
   ```

4. Interpret the returned text under the worker's existing response contract, then emit one final line and exit successfully:

   ```json
   {"schema":"merl.transport-final/v1","response":{"schema":"merl.eval-adapter/v1","kind":"final","answer":"Synthetic answer.","citations":[],"usage":{"input":10,"output":5}}}
   ```

A failed generation aborts the worker and records the failure; there is no success-shaped error reply. At least one successful generation is required before a final frame. Extra fields, extra output, invalid JSON, wrong usage or excess calls fail the invocation. A reader response's usage must equal the sum of provider receipts for that worker invocation. A categorical grade keeps its existing three-field shape; grader usage lives in the separate transport ledger.

## Concrete provider adapter interface

A concrete adapter is one pinned executable implementing two pure JSON EOF operations. Every operation starts in a fresh isolated process and may use only stdin. It has no credential environment, evaluator mounts, endpoint argument or network. Unsupported model/version, effort, temperature, seed or output controls must fail before HTTP, rather than silently dropping a setting.

The encode request supplies host-selected controls and worker-selected text:

```json
{
  "schema":"merl.provider-adapter-request/v1", "kind":"encode",
  "settings":{"model":"chosen-model","model_version":"chosen-version","effort":"none","temperature":0,"max_output_tokens":1024},
  "randomness":{"seed":7},
  "messages":[{"role":"user","text":"Synthetic question?"}]
}
```

`randomness` is `null` when the caller has no paired-trial control; otherwise it uses the existing `RandomnessControl` representation (`{"seed":7}` or `{"unavailable":{"reason":"..."}}`). The response must be:

```json
{"schema":"merl.provider-adapter-response/v1","kind":"encoded","body":{"provider_specific":"JSON object"}}
```

The adapter must translate the configured output cap into the provider's actual output-token control, preserve the declared supported settings, disable provider persistence/tools, and include only these messages. It must not return destinations, headers, paths or commands. Nonzero exit, an unsupported response or malformed JSON prevents HTTP and records `provider_adapter`.

After the POST, the controller invokes the same executable afresh:

```json
{"schema":"merl.provider-adapter-request/v1","kind":"decode","status":200,"body":{"provider_specific":"JSON response"}}
```

Decode successful generation as:

```json
{"schema":"merl.provider-adapter-response/v1","kind":"success","text":"Synthetic answer.","usage":{"input":10,"output":5},"model":"resolved-model","request_id":"provider-id"}
```

Decode provider refusal, truncation, rate limiting or other unsuccessful completion as:

```json
{"schema":"merl.provider-adapter-response/v1","kind":"failure","code":"rate_limited","usage":{"input":10,"output":5},"model":"resolved-model","request_id":"provider-id"}
```

Failure usage may be `null` if the provider did not report it; success requires both nonnegative integer usage fields. Neither may be estimated from prompt/output length or replaced with zero when absent. The concrete adapter must document how provider input/output, cache and reasoning usage map to these totals, avoiding double counting. `model` and `request_id` may be `null` when unavailable; supplied values must be nonempty, at most 256 bytes and free of control characters. Failure codes use lowercase ASCII letters/underscores, at most 64 bytes, without provider prose. All response objects reject unknown fields. Decode receives non-2xx JSON too, so reported costs survive provider errors. Redirects, malformed/oversized bodies and transport failures retain status/hashes when available and unknown usage explicitly. Unsupported non-JSON provider errors fail without guessing their contents.

The adapter is trusted to extract usage and identify provider failures correctly. Hashing its code and HTTP bodies makes that translation auditable; the generic broker cannot independently interpret a provider's billing semantics. The repository's [synthetic translator](../../crates/merl-eval/tests/fixtures/provider_adapter.py) is a test fixture, not an integration for a real API.

## Compiler calls and accounting

For compiler generation, use the standalone command with `role: preparer` and an evaluator-owned pinned plan:

```json
{
  "schema":"merl.transport-plan/v1", "role":"preparer",
  "worker_program":"/evaluator/compiler-worker",
  "worker_program_sha256":"WORKER_SHA256",
  "transport":{"path":"/evaluator/preparer-transport.json","sha256":"CONFIG_SHA256"},
  "limits":{"request_bytes":1048576,"response_bytes":262144,"program_bytes":67108864,"seconds":30}
}
```

```sh
rtk proxy target/debug/merl-eval transport call \
  --plan "$TRANSPORT_PLAN" --plan-sha256 "$TRANSPORT_PLAN_SHA256" \
  < "$SYNTHETIC_COMPILER_REQUEST" > "$LOCAL_TRANSPORT_RECEIPT"
```

The input remains `merl.compiler-request/v1`. The broker uses the smaller of configured `max_output_tokens` and the run's positive `limits.output_tokens`. The worker's final payload remains `merl.compiler-response/v1`, without adding usage fields to that schema. The stdout envelope is `merl.transport-execution/v1` with `response`, `error`, worker process records and transport receipts. It exits 1 on failure while retaining receipts for calls already made. Preflight failures return a versioned error on stderr before any worker runs.

The candidate preparer's trusted compiler wrapper must persist that envelope before forwarding its `response` to the existing compiler process adapter. It must bind every receipt to the run ID, paired trial, prefix and phase, then construct the existing `merl.eval-preparation/v1` call ledger, including failed attempts/corrections. Unknown usage cannot support a complete preparation attestation. This PR supplies the restricted compiler-call seam, not a candidate authority builder or prepared artifacts. Never launch the entire authority-owning preparer inside a model worker or give it evaluator answers.

Package execution adds `model_transport` and `grader_transport` receipts to `merl.eval-visible-execution/v1`. Receipts bind configuration/adapter hashes, effective settings/randomness, initial worker request hash, unit/trial, normalized request and HTTP body hashes, status, attempted flag, reported model/request ID, usage and safe failure codes. Process records identify `json_eof` versus `json_lines_model_calls`; the latter hashes the initial JSON and final payload, excluding framing, while generation/translation receipts cover the intervening calls. They do not persist provider bodies or credentials. Detailed execution reports still contain answers and grader reasons; keep them in the evaluator workspace.

Known usage survives a provider error, invalid final response, wrong worker usage, worker exit or grading failure. Failed read costs enter aggregate method totals. Failed shared-summary generation contributes its known cost once per applicable method/trial/prefix, without multiplying that preparation by the number of questions. Unknown additional usage remains explicit through incomplete preparation or `usage_incomplete`. Grader costs remain a separate ledger. Failures and exclusions never become correct scoring units.

## Verification and remaining real-call work

Run these exact commands on a Linux host with working bubblewrap, Python 3, `/usr/bin/timeout` and the repository Rust toolchain on `PATH`:

```sh
rtk proxy cargo test --locked -p merl-eval --test restricted_transport
rtk proxy cargo test --locked -p merl-eval --test visible_cli
rtk proxy cargo test --locked --test evaluation_isolation --test visible_package
```

These use synthetic fixtures and loopback mock HTTP only. They check pinned requests, credential withholding/redaction, network/file denial, fresh worker/translator state, reader/grader separation, redirects, invalid frames/usage, call budgets, role/code pin mismatches, compiler caps and failed-call/shared-preparation accounting. Run the full checks in `CONTRIBUTING.md` before integrating. Retain bubblewrap throughout; an outer sandbox that forbids namespace creation must fail rather than weakening the tests.

Before one real end-to-end call, the human/evaluator must select one provider, endpoint, account, concrete adapter and supported model controls. That adapter still needs implementation/review against the interface above and validation of provider-reported usage/failures. The evaluator must also supply a reader/compiler worker that interprets model text, an independently configured semantic grader, and the trusted preparation wrapper/receipts. First verify a benign synthetic real call with that pinned configuration under the same isolation. No provider-specific choice has been made here, and this PR makes no actual-provider compatibility claim.

The first-pass annotation work is finished. The evaluator has already verified #142's exact head against the approved package and attested all 11 original synthetic integration/isolation tests with bubblewrap retained. The planner still owns wider corpus-contract reconciliation in #131; the evaluator owns grader validation and prepared trial/prefix artifacts. This transport work does not close #131, run the benchmark or authorize v4.
