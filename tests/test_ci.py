import json
import tomllib
from pathlib import Path

ROOT = Path(__file__).parents[1]


def test_github_check_runs_repository_gate() -> None:
    workflow = (ROOT / ".github" / "workflows" / "check.yml").read_text()
    assert "run: make check" in workflow


def test_release_please_keeps_package_and_plugin_versions_in_sync() -> None:
    manifest = json.loads((ROOT / ".release-please-manifest.json").read_text())
    configuration = json.loads((ROOT / "release-please-config.json").read_text())
    project = tomllib.loads((ROOT / "pyproject.toml").read_text())
    claude_plugin = json.loads((ROOT / ".claude-plugin" / "plugin.json").read_text())
    claude_marketplace = json.loads(
        (ROOT / ".claude-plugin" / "marketplace.json").read_text()
    )
    codex_plugin = json.loads((ROOT / ".codex-plugin" / "plugin.json").read_text())
    paths = {
        extra_file["path"]
        for extra_file in configuration["packages"]["."]["extra-files"]
    }

    assert {
        manifest["."],
        project["project"]["version"],
        claude_plugin["version"],
        claude_marketplace["plugins"][0]["version"],
        codex_plugin["version"],
    } == {project["project"]["version"]}
    assert paths == {
        ".claude-plugin/plugin.json",
        ".claude-plugin/marketplace.json",
        ".codex-plugin/plugin.json",
        "src/merl/__init__.py",
    }
