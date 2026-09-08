from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

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
    assert json.loads(result.stdout)["session_id"].startswith("ses-")


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
