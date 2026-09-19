# Evaluation corpus

Merl needs issue histories where the current answer differs from an earlier answer. A long thread alone does not test that. The first visible set has two public histories and one staged correction. We will use this set to settle the fixture format before collecting the remaining cases.

The corpus is split by complete issue, never by comment. Development material is visible to implementers. Adversarial cases remain separate. Five held-out cases were selected independently; their identities and answers stay outside this repository and implementation-agent access until the release candidate freezes. The sealed manifest commitment is `sha256:86a37c47cb0164dd99e9ff53c036d621569a3f2d2da9c500d420a736b699f8c0`. This digest identifies a manifest supplied by the curator; we cannot verify the unseen contents here. If a held-out identity leaks, move it to development, replace the case, and publish a new commitment before further tuning.

## Visible first slice

| ID | Source | Why it was selected | Status |
| --- | --- | --- | --- |
| DEV-N1 | [Rust uniform paths](https://github.com/rust-lang/rust/issues/55618) | Language decision with competing concerns and formal review | Captured locally; redistribution review pending |
| DEV-N2 | [Prometheus memory regression](https://github.com/prometheus/prometheus/issues/4254) | Hypotheses, evidence requests, and a later finding | Captured locally; redistribution review pending |
| DEV-C1 | Controlled gain-strategy discussion | Decision at observation 2 reversed at observation 4 | Versioned with temporal gold states |

The two natural captures remain untracked while a maintainer checks their full text, authorship and privacy. Their source URLs, license observations, capture metadata, and SHA-256 digests are recorded in the local fixtures. Their GitHub repositories reported Apache-2.0 licenses at capture; that alone does not establish permission to redistribute every comment. Do not commit the captures merely because they were public. A contributor with access to the issues can recapture them using the commands below, but a new capture is a new corpus version and needs its own review.

The captures retain stable IDs for Issues, comments, actors, and edits. Each edit has its own position in the observation stream and supersedes the previous version of that Issue or comment. GitHub exposes edit metadata and a diff, but not necessarily the full earlier body. Such versions carry an explicit missing-body reason; the final body is never moved back to the Issue's creation time. Exact causal replay stops at a missing body. The provider snapshot describes capture time, not historical Issue state. The staged fixture is `staged_exact` and supports exact replay.

## Fixture contract

`merl.corpus-fixture/v1` contains source identity, capture and redistribution records, a terminal provider snapshot, ordered source versions, and gold states at selected observation cutoffs. Gold objects have separate lifecycle and evidence-support states. Evidence can support or dispute an object, and typed relations record consequences such as one decision superseding another. Validation checks source-version lineage, body hashes and gaps, the fixture digest, and gold citations against their causal cutoffs. This capture path does not claim historical coverage of provider-owned transitions such as close/reopen or label changes; those are deferred rather than represented by an always-empty event list.

The validator cannot establish that a human label is correct. Gold states for natural fixtures need independent labeling before they count toward correctness. A second annotator should label a useful subset without seeing the first annotation; keep both and record disagreements before adjudication.

Use the workspace tool from the repository root (replace the example UTC timestamp with the time of capture):

```sh
cargo run --locked -p corpus -- validate corpus/development/DEV-C1.json
cargo run --locked -p corpus -- capture-github DEV-N1 rust-lang/rust 55618 2026-09-19T02:00:58Z corpus/development/DEV-N1.json
cargo run --locked -p corpus -- capture-github DEV-N2 prometheus/prometheus 4254 2026-09-19T02:00:58Z corpus/development/DEV-N2.json
```

The capture command calls an authenticated `gh` CLI. It freezes the returned snapshot and leaves redistribution status `pending`. Keep local captures private until reviewed. The held-out inputs and answer keys must never be placed in this repository, its agent workspaces, or a searchable connected drive used by implementers.
