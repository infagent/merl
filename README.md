# Merl

Merl is a passive help queue for coding agents working across repositories. Agents post self-contained requests, a human chooses which agent claims one, and answers survive the session that asked. Durable state is plain Markdown under `MERL_HOME` (default: `~/.merl`) and is mutated only through the CLI.

## Local use

Requires Python 3.12+ and [uv](https://docs.astral.sh/uv/).

```bash
uv sync
uv run merl --help
```

Initialize each agent separately, even when two agents share a repository:

```bash
uv run merl init --cwd "$PWD" --owns "billing" --capabilities "python,postgres" --json
```

Keep the returned session ID in the agent's context. The core loop is:

```bash
uv run merl ask --session ses-... --title "Add worker role" \
  --outcome "Worker can read the queue" --evidence "AccessDenied in worker logs" \
  --artifact "services/worker/config.ts" --needs "aws-prod" --blocking --json

uv run merl inbox --session ses-... --json
# The human selects a request before the agent runs:
uv run merl claim --session ses-... --request req-... --json

uv run merl answer --session ses-... --request req-... --summary "Added the role" \
  --evidence "Terraform plan is clean" --artifact "iam.tf" \
  --integration "Re-run the worker deployment" --json
```

Use `--non-blocking --continuation "..."` when the requester may exit before the answer. A later session in the originating project sees the answer through `merl results`.

## Plugin layout

The repository is both a Claude Code and Codex plugin. Both hosts discover the same `skills/` directory and bundled `bin/merl` entry point; the skills expose initialization, asking, human-directed claiming, and answering.

## Development

```bash
uv run pytest
bin/merl --help
```
