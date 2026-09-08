import json
from pathlib import Path

ROOT = Path(__file__).parents[1]


def test_github_check_runs_repository_gate() -> None:
    workflow = (ROOT / ".github" / "workflows" / "check.yml").read_text()
    assert "run: make check" in workflow


def test_release_please_starts_at_zero_and_updates_plugin_versions() -> None:
    manifest = json.loads((ROOT / ".release-please-manifest.json").read_text())
    configuration = json.loads((ROOT / "release-please-config.json").read_text())
    paths = {
        extra_file["path"]
        for extra_file in configuration["packages"]["."]["extra-files"]
    }

    assert manifest == {".": "0.0.0"}
    assert paths == {
        ".claude-plugin/plugin.json",
        ".claude-plugin/marketplace.json",
        ".codex-plugin/plugin.json",
        "src/merl/__init__.py",
    }
