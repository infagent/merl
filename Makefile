SHELL := /bin/bash

.PHONY: setup check test

setup:
	@command -v uv >/dev/null || { echo "uv is required" >&2; exit 1; }
	@uv sync

check: test
	@uv run ruff check .
	@uv run ruff format --check .
	@bash -n bin/merl
	@shellcheck bin/merl

test:
	@uv run pytest
