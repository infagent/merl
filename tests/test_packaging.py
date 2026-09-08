from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).parents[1]


def test_bundled_entrypoint_runs_with_a_temporary_board(tmp_path: Path) -> None:
    environment = {
        **os.environ,
        "MERL_HOME": str(tmp_path / "board"),
        "UV_CACHE_DIR": str(tmp_path / "uv-cache"),
    }
    result = subprocess.run(
        [ROOT / "bin" / "merl", "init", "--cwd", str(tmp_path), "--json"],
        cwd=ROOT,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert result.stderr == ""
    assert json.loads(result.stdout)["session_id"].startswith("ses-")


def test_bundled_entrypoint_invokes_uv_quietly_without_dev_dependencies(
    tmp_path: Path,
) -> None:
    executable_directory = tmp_path / "bin"
    executable_directory.mkdir()
    arguments_path = tmp_path / "arguments"
    fake_uv = executable_directory / "uv"
    fake_uv.write_text(
        '#!/usr/bin/env bash\nprintf "%s\\n" "$@" >"$MERL_TEST_ARGUMENTS"\n'
    )
    fake_uv.chmod(0o755)
    environment = {
        **os.environ,
        "PATH": f"{executable_directory}:{os.environ['PATH']}",
        "MERL_TEST_ARGUMENTS": str(arguments_path),
    }

    result = subprocess.run(
        [ROOT / "bin" / "merl", "init", "--json"],
        cwd=ROOT,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode == 0
    assert result.stderr == ""
    assert arguments_path.read_text().splitlines() == [
        "run",
        "--quiet",
        "--no-dev",
        "--project",
        str(ROOT),
        "merl",
        "init",
        "--json",
    ]


@pytest.mark.parametrize("skill", ["answer", "ask", "claim", "init"])
def test_skill_resolves_bundled_entrypoint_without_path_probe(skill: str) -> None:
    skill_path = ROOT / "skills" / skill / "SKILL.md"
    plugin_root = skill_path.parents[2]
    instructions = skill_path.read_text()

    assert plugin_root == ROOT
    assert (plugin_root / "bin" / "merl").is_file()
    expected_commands = {
        "answer": ("answer", "work"),
        "ask": ("ask",),
        "claim": ("claim", "inbox"),
        "init": ("init",),
    }
    for command in expected_commands[skill]:
        assert f"<plugin-root>/bin/merl {command}" in instructions
    assert "merl is not on PATH" not in instructions


def test_claim_skill_supports_manager_directed_worker_claims() -> None:
    instructions = (ROOT / "skills" / "claim" / "SKILL.md").read_text()

    assert "target worker session ID" in instructions
    assert "manager claims for a sibling worker" in instructions


def test_both_plugin_manifests_share_the_four_skills() -> None:
    for manifest_path in (
        ROOT / ".codex-plugin" / "plugin.json",
        ROOT / ".claude-plugin" / "plugin.json",
    ):
        assert json.loads(manifest_path.read_text())["name"] == "merl"
    assert {path.name for path in (ROOT / "skills").iterdir()} == {
        "init",
        "ask",
        "claim",
        "answer",
    }

    codex_marketplace = json.loads(
        (ROOT / ".agents" / "plugins" / "marketplace.json").read_text()
    )
    claude_marketplace = json.loads(
        (ROOT / ".claude-plugin" / "marketplace.json").read_text()
    )
    assert codex_marketplace["plugins"][0]["name"] == "merl"
    assert claude_marketplace["plugins"][0]["name"] == "merl"


def test_public_marketplace_installation_uses_github_repository() -> None:
    readme = (ROOT / "README.md").read_text()
    expected = "plugin marketplace add infagent/merl"
    assert readme.count(expected) == 2
