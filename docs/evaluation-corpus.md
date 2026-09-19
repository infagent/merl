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

GitHub supplied 101 observations for DEV-N1 and 78 for DEV-N2, including the initial Issue bodies. The captures include stable provider IDs, current bodies, authors, creation and update times, edit diffs when exposed, and the current provider snapshot. They do **not** reconstruct every old edited body. Both are marked `diff_only`, and their provider snapshots describe capture time, not past states. Do not use them for exact causal replay across edits without independently verifying the old versions. The staged fixture is `staged_exact` and supports that test.

## Fixture contract

`merl.corpus-fixture/v1` contains a source identity, a capture record, redistribution status, a current provider snapshot, ordered observations, optional provider events, and gold states at selected observation cutoffs. Each gold object cites the observations supporting it. Validation checks schema, contiguous observation numbers, capture count, the digest of provider-owned state and source material, and whether a gold state cites an observation from its future.

The validator cannot establish that a human label is correct or that prose written later has not leaked through an edited terminal snapshot. Gold states for natural fixtures need independent labeling before they count toward correctness. A second annotator should label a useful subset without seeing the first annotation; keep both and record disagreements before adjudication.

Use the workspace tool from the repository root (replace the example UTC timestamp with the time of capture):

```sh
cargo run --locked -p merl-corpus -- validate corpus/development/DEV-C1.json
cargo run --locked -p merl-corpus -- capture-github DEV-N1 rust-lang/rust 55618 2026-09-19T02:00:58Z corpus/development/DEV-N1.json
cargo run --locked -p merl-corpus -- capture-github DEV-N2 prometheus/prometheus 4254 2026-09-19T02:00:58Z corpus/development/DEV-N2.json
```

The capture command calls an authenticated `gh` CLI. It freezes the returned snapshot and leaves redistribution status `pending`. Keep local captures private until reviewed. The held-out inputs and answer keys must never be placed in this repository, its agent workspaces, or a searchable connected drive used by implementers.

## Next work under #24

The remaining natural development candidates are [pandas copy/view semantics](https://github.com/pandas-dev/pandas/issues/36195) (DEV-N3), [Kubernetes local persistent storage](https://github.com/kubernetes/kubernetes/issues/7562) (DEV-N4), [VS Code extension policy](https://github.com/microsoft/vscode/issues/84756) (DEV-N5), and [Arrow nightly-wheel hosting](https://github.com/apache/arrow/issues/40216) (DEV-N6). Review each for suitability and redistribution before freezing it. This spread tests design discussion, debugging, requirements, and operational follow-up rather than six similar threads.

Staged development cases still needed are: a compound note that reports failed evidence, weakens but does not rule out a hypothesis, requests follow-up work, and waits for a PR (DEV-C2); an edited source that reverses an accepted decision and puts its support into revalidation (DEV-C3); and a maintainer's claim of completion before the provider actually closes the Issue (DEV-C4). Staged adversarial cases cover unverified quoted authority (ADV-C1), an ambiguous referent (ADV-C2), a relative date combined with an event condition (ADV-C3), purged evidence with degraded support (ADV-C4), and an optional cold bot diagnostic promoted to required coverage (ADV-C5). These cases are fictional and should contain no real credentials.

Complete privacy review and label intermediate cutoffs and final states before scoring. Two annotators should independently label a useful subset, retaining disagreements and acceptable answer variants. Then build the five-path benchmark runner: raw history, rolling summary, recent retrieval, summary plus retrieval, and Merl. Record the model, effort, prompt, tools, sampling settings, paired trials, task correctness, provenance, token costs, and break-even read count. The sealed held-out set belongs only to the release evaluator after the release candidate freezes.
