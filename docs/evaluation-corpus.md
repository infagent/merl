# Evaluation corpus

Merl needs issue histories where the current answer differs from an earlier answer. A long thread alone does not test that. The visible development set has two public captures awaiting review and three controlled histories that test different failure modes.

The corpus is split by complete issue, never by comment. Development material is visible to implementers. Adversarial cases remain separate. Five held-out cases were selected independently; their identities and answers stay outside this repository and implementation-agent access until the release candidate freezes. The sealed manifest commitment is `sha256:86a37c47cb0164dd99e9ff53c036d621569a3f2d2da9c500d420a736b699f8c0`. This digest identifies a manifest supplied by the curator; we cannot verify the unseen contents here. If a held-out identity leaks, move it to development, replace the case, and publish a new commitment before further tuning.

## Visible first slice

| ID | Source | Why it was selected | Status |
| --- | --- | --- | --- |
| DEV-N1 | [Rust uniform paths](https://github.com/rust-lang/rust/issues/55618) | Language decision with competing concerns and formal review | Captured locally; redistribution review pending |
| DEV-N2 | [Prometheus memory regression](https://github.com/prometheus/prometheus/issues/4254) | Hypotheses, evidence requests, and a later finding | Captured locally; redistribution review pending |
| DEV-C1 | Controlled gain-strategy discussion | Decision at observation 2 reversed at observation 4 | Versioned with temporal gold states |
| DEV-C2 | Controlled checksum-parser discussion | A research note requests work; the PM accepts it but defers its start until a PR merges | Versioned with temporal gold states |
| DEV-C3 | Controlled report-export discussion | An edited source leaves a decision active during revalidation, then a later edit replaces it | Versioned with temporal gold states |

The two natural captures remain untracked while a maintainer checks their full text, authorship and privacy. Their source URLs, license observations, capture metadata, and SHA-256 digests are recorded in the local fixtures. Their GitHub repositories reported Apache-2.0 licenses at capture; that alone does not establish permission to redistribute every comment. Do not commit the captures merely because they were public. A contributor with access to the issues can recapture them using the commands below, but a new capture is a new corpus version and needs its own review.

DEV-C2 separates the researcher's acts. The observation that the frame length was read once disputes the active double-read hypothesis. The researcher's conclusion that the hypothesis is weakened remains a candidate claim. The parser task exists after the request, but its commitment is pending. After the PM replies, the task is accepted, deferred, and not started. Its start condition is the PR merge; no agent session is waiting on it.

The captures retain stable IDs for Issues, comments, actors, and edits. When GitHub includes a creation edit, that edit is the first version; the capture does not invent another one. Later edits supersede the previous version. GitHub exposes edit metadata and a diff, but not necessarily the full earlier body. Such versions carry an explicit missing-body reason; the final body is never moved back to the Issue's creation time. Timestamp ties between source streams are marked ambiguous rather than resolved by sorting IDs. Exact causal replay stops at either kind of gap. The provider snapshot describes capture time, not historical Issue state. The controlled histories are `staged_exact` and support exact replay.

## Fixture contract

`merl.corpus-fixture/v1` contains source identity, capture and redistribution records, a terminal provider snapshot, source versions, and gold states at selected observation cutoffs. Gold objects have separate lifecycle and evidence-support states. A `candidate` records a proposal without treating it as accepted. Task gold states also record commitment, scheduling, and execution as separate facets. A task's PR-merge prerequisite is a durable predicate in its plan, not a runtime `WaitCondition`. Evidence can support or dispute an object. Typed relations record consequences such as one decision superseding another. Validation checks version identity and lineage, capture times, body hashes and gaps, the fixture digest, and gold citations against their causal cutoffs. This capture path does not claim historical coverage of provider-owned transitions such as close/reopen or label changes; those are deferred rather than represented by an always-empty event list.

New GitHub captures use `merl.corpus-fixture/v2`. They record the Issue's upstream update time and stable IDs for labels and assignees. Merl still reads v1 fixtures under their original rules. For a v1 capture without label IDs, those identities remain unknown. Keep frozen v1 files unchanged. A later recapture or augmentation needs its own versioned artifact and provenance.

The body and ordering checks are separate. Passing the body check does not turn a `terminal_snapshot_only` capture into exact historical observation. The validator also cannot establish that a human label is correct. Gold states for natural fixtures need independent labeling before they count toward correctness. A second annotator should label a useful subset without seeing the first annotation; keep both and record disagreements before adjudication.

## Capture dependency

`capture-github` invokes the [GitHub CLI](https://cli.github.com/) as an external program. Install `gh` separately and authenticate with [`gh auth login`](https://cli.github.com/manual/gh_auth_login), or provide [`GH_TOKEN`](https://cli.github.com/manual/gh_help_environment) in automation. Run `gh auth status` to check the account. This repository has no installer that supplies `gh`. Fixture validation and ordinary Rust builds do not need it.

Use the workspace tool from the repository root (replace the example UTC timestamp with the time of capture):

```sh
cargo run --locked -p merl-corpus -- validate corpus/development/DEV-C1.json
cargo run --locked -p merl-corpus -- validate corpus/development/DEV-C2.json corpus/development/DEV-C3.json
cargo run --locked -p merl-corpus -- capture-github DEV-N1 rust-lang/rust 55618 2026-09-19T02:00:58Z corpus/development/DEV-N1.json
cargo run --locked -p merl-corpus -- capture-github DEV-N2 prometheus/prometheus 4254 2026-09-19T02:00:58Z corpus/development/DEV-N2.json
```

The capture command freezes the returned snapshot and leaves redistribution status `pending`. Keep local captures private until reviewed. The held-out inputs and answer keys must never be placed in this repository, its agent workspaces, or a searchable connected drive used by implementers.
