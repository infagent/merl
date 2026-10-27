# First message and project setup

This walkthrough uses a disposable directory and synthetic identities. It changes no real home configuration or startup service. The first binary release must finish before the public installer URL works; until then, maintainers use the local-artifact check below. See [installation](installation.md) for supported systems and exact-version downloads.

## Existing-agent startup

With a release containing #163, `merl usage` gives a short primer without reading setup/runtime state. Optional matching setup installs the short canonical skill: invoke `$merl scope/member starting guidance` in Codex or `/merl scope/member starting guidance` in Claude. Use a supplied mailbox home or an explicitly selected saved default. New members supply guidance once; resume loads it. Missing scope/access is requested rather than inferred. See [teams](teams.md) for arbitrary/roleless scopes and cross-scope handoffs, and [installation](installation.md#canonical-startup-and-legacy-recovery) for the retained `merl-mail` transition. V0.3.0 does not include this canonical startup.

## Install and send

After a binary release is published, run in one shell:

```sh
demo_root=$(mktemp -d)
bin_dir="$demo_root/user-bin"
mail_home="$demo_root/mail"
curl -fsSL https://github.com/infagent/merl/releases/latest/download/install.sh \
  | sh -s -- --bin-dir "$bin_dir"
merl_bin="$bin_dir/merl"
"$merl_bin" --version
"$merl_bin" help mail send --json
"$merl_bin" mail register --home "$mail_home" --agent alice --json
"$merl_bin" mail register --home "$mail_home" --agent bob --json
printf 'Please check the synthetic greeting: hello.\n' > "$demo_root/handoff.txt"
"$merl_bin" mail send --home "$mail_home" --from alice --to bob \
  --id handoff-1 --kind assign --body-file "$demo_root/handoff.txt" --json
"$merl_bin" mail list --home "$mail_home" --agent bob --pending --json
"$merl_bin" mail read --home "$mail_home" --agent bob --id handoff-1 --json
```

The pending list should contain `handoff-1`; the read body contains the synthetic greeting. Follow every `next_after` page with `--after` until null. Save a [work checkpoint](agent-roles.md#restart-checkpoint) recording the assignment, read ID and next action before receipt:

```sh
"$merl_bin" mail receipt --home "$mail_home" --agent bob --id handoff-1 --json
"$merl_bin" mail list --home "$mail_home" --agent bob --outstanding --json
```

The assignment remains outstanding. After checking the greeting, record the result in the checkpoint and explicitly finish it:

```sh
"$merl_bin" mail complete --home "$mail_home" --agent bob --id handoff-1 --json
"$merl_bin" mail status --home "$mail_home" --agent bob --json
```

Expect zero unreceipted messages and zero outstanding requests. These commands work without a host binding; no model was asked to poll. For replies and exact send retries, load `merl help mail send --json` or the [mail guide](agent-mail.md).

## Optional skill and wake

The CLI works without a skill. If wanted, obtain and verify the archive for the installed version, then copy its matching canonical `skills/merl/SKILL.md` (or legacy `merl-mail` in older archives) to an explicitly chosen supported host location. Inspect an existing destination before replacing it. The [manual installation steps](installation.md#manual-download-and-optional-skill) cover verification and setup. Keep project roles, checkpoints and lessons in [project guidance](agent-roles.md), outside that short skill.

Automatic wake is currently verified with Codex CLI 0.164.0 and its existing local control socket. Other coding hosts can invoke the CLI, but automatic Claude-to-Codex or Codex-to-Claude wake/restart has not been established. This is local same-user messaging, not a remote transport or authentication boundary.

For a real wake, obtain the recipient's explicitly supplied existing session UUID, socket and absolute Codex executable. Set `existing_session`, `control_socket` and `codex_bin` to those values; never substitute a newly created or guessed session. Check the installed host's `codex queue --help`. Inspect the current binding generation first; this fresh example uses generation 0:

```sh
"$merl_bin" mail status --home "$mail_home" --agent bob --json
"$merl_bin" mail bind --home "$mail_home" --agent bob \
  --session "$existing_session" --socket "$control_socket" \
  --expected-generation 0 --json
```

In a dedicated terminal, set `merl_bin`, `mail_home` and `codex_bin` to the same absolute values and run the foreground dispatcher:

```sh
"$merl_bin" mail watch --home "$mail_home" --codex "$codex_bin" --json
```

From the original shell, send a new ID such as `wake-1` using the same `mail send` command/body. The bound recipient drains pending pages, reads each ID, saves its checkpoint and receipts it. Enqueued means only that the host accepted a pointer; receipt and completed work are separate. The recipient must be able to execute the absolute Merl path in that pointer.

## Stop, restart and recover

Stop only your foreground dispatcher with Ctrl-C or SIGTERM to its known process. Restart the same command with the same mailbox home. Do not delete the binary or mailbox while a dispatcher or recipient still needs them. An upgrade does not restart a running dispatcher; deliberately stop/restart your process when changing its binary. See [mail recovery](agent-mail.md#status-and-recovery).

Before resuming work, load the saved assignment/checkpoint, inspect `mail status`, drain `mail list --pending`, and inspect `mail list --outstanding`. A receipt stops retries but preserves unfinished work. Resume by stable IDs rather than repeating completed actions. If the host/socket/session changed, inspect status and explicitly `mail bind` the same identity to its supplied replacement using the returned generation. A dispatcher restart does not select a replacement session or repair a stale socket fingerprint. Reviewer-lane users also inspect the saved [lane queue](reviewer-lanes.md#pause-resume-and-fresh-sessions).

## Verification before the first release

On a clean committed checkout, build/package the native binary with the current Cargo version using [release distribution](release-distribution.md#verify-locally). Run `scripts/smoke_package.py` against that archive: it exercises the existing installer, checksums and version/help in a disposable directory with local download fixtures. Then run the first-message commands above using the locally installed binary, saving the checkpoint before receipt and checking that completion remains explicit. Verify the optional skill against that same archive. Do not invent a release tag or change Cargo versions for the exercise.

This is controlled local-artifact evidence. Public release download/latest resolution, published asset bytes and the first real install still need checking after publication. Prior [Codex live validation](agent-mail-validation.md) and public binding/recovery tests cover different boundaries; this walkthrough does not claim a new live idle wake or another host adapter. No global installation, model experiment, configuration rewrite or mailbox migration is needed for the local check.
