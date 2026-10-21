# Release gates

Issue [#130](https://github.com/infagent/merl/issues/130) closes the gap between
running CI and requiring it before a merge. Verification below was recorded on
2026-10-21 UTC.

## Main protection

The active [protect main ruleset](https://github.com/infagent/merl/rules/23709525)
requires these exact checks from GitHub Actions, integration ID `15368`:

| Required check | What it runs |
| --- | --- |
| `Rust checks` | Formatting, Clippy, workspace tests, and rustdoc, with warnings denied |
| `Test conventions` | Acceptance scenario conventions and the convention checker's tests |

The names and integration ID were verified through the
[check runs on #136](https://github.com/infagent/merl/actions/runs/39526085175).
The `size` check remains advisory. The existing pull request requirement,
deletion protection, force-push protection, and review settings remain intact.
The bypass list is empty, including for release automation.

The rule uses strict checks: a branch must include the current `main` before
merging. If `main` advances, update the PR branch and wait for its required checks
again. This avoids relying on results from an older base. GitHub documents this
as [requiring branches to be up to date](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets#require-status-checks-to-pass-before-merging).

The applied API payload is
[`.github/rulesets/main.json`](../.github/rulesets/main.json). Editing that file
does not change GitHub's configuration. Read the live rule and reconcile any
later protection changes before applying an update:

```sh
gh api repos/infagent/merl/rulesets/23709525
gh api repos/infagent/merl/rules/branches/main
gh api --method PUT repos/infagent/merl/rulesets/23709525 \
  --input .github/rulesets/main.json
```

Readback matched the saved payload. Comparing the rules before and after the
update confirmed that only the required-check rule was added; conditions,
existing rules, and the empty bypass list were preserved.

## Enforcement evidence

Temporary [PR #139](https://github.com/infagent/merl/pull/139) exercised the normal
merge path without a bypass. Its first commit, `066c98c`, changed only spacing
in `fn main( )` to make `cargo fmt --all -- --check` fail.

| State | Required checks | Result |
| --- | --- | --- |
| Pending at `066c98c` | `Test conventions` passed; `Rust checks` was running | `mergeStateStatus=BLOCKED`; the normal merge command exited 1 |
| Failed at `066c98c` | `Test conventions` passed; `Rust checks` failed formatting | `mergeStateStatus=BLOCKED`; the normal merge command exited 1 |
| Passed at `0a8188d` | Both required jobs passed after restoring the original formatting | `mergeStateStatus=CLEAN`; the same PR became eligible |

Both refusal checks ran
`gh pr merge 139 --squash --match-head-commit 066c98c3101176c05637493286187cfff67eda00`.
The CLI reported: "the base branch policy prohibits the merge."
The [failed Check run](https://github.com/infagent/merl/actions/runs/39526532669)
retains the intended formatting failure and successful conventions job.
The [passing Check run](https://github.com/infagent/merl/actions/runs/39526640162)
retains the successful second phase. The probe was closed without merging; its
local and remote branch and temporary worktree were removed.

## Release automation

[Check](../.github/workflows/ci.yml) still runs on pull requests and pushes to
`main`, with `contents: read` and checkout credential persistence disabled.
[Release Please](../.github/workflows/release-please.yml) still creates or publishes
a release only after a successful Check run from a push to the default branch.
Its manual path validates configuration without publishing.

The [post-merge Check run for #136](https://github.com/infagent/merl/actions/runs/39526363539)
passed, then [Release Please](https://github.com/infagent/merl/actions/runs/39526509484)
updated [release PR #36](https://github.com/infagent/merl/pull/36) to `ab3d4458`.
The existing `infagent-release-automation` app triggered the
[release PR's Check run](https://github.com/infagent/merl/actions/runs/39526588503)
automatically. Both required jobs passed and the PR reported
`mergeStateStatus=CLEAN` under the active rule. No manual workflow approval,
personal token, or bot bypass was added. The app's installation token remains
scoped to this repository, with
contents, issues, and pull-request write permissions. GitHub documents
[app installation tokens](https://docs.github.com/en/actions/concepts/security/github_token#when-github_token-triggers-workflow-runs)
as a way for automation-created PRs to trigger workflows without manual approval.

## Local checks

Run the commands in [Contributing](../CONTRIBUTING.md#contribute-code), which match
both required jobs. Passing local checks does not satisfy the remote rule;
GitHub must receive both results from the expected Actions integration.

Inspect a PR with:

```sh
gh pr checks PR_NUMBER --required
gh pr view PR_NUMBER --json headRefOid,mergeStateStatus,statusCheckRollup
```
