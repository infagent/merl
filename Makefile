SHELL := /bin/bash

.PHONY: setup check test

setup:
	@command -v uv >/dev/null || { echo "uv is required" >&2; exit 1; }
	@uv sync

check: test

test:
	@uv run python -m compileall -q .
	@find . -type f -not -path "./.venv/*" -not -path "./.git/*" -print0 | while IFS= read -r -d '' file; do case "$$file" in *.sh|*.bash) shellcheck "$$file" ;; *) IFS= read -r first <"$$file" || true; case "$$first" in "#!/usr/bin/env bash"|"#!/bin/bash") shellcheck "$$file" ;; esac ;; esac; done
