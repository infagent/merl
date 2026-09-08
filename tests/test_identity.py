import subprocess
from pathlib import Path

import pytest

from merl.identity import IdentityError, project_detect, remote_normalize


def test_equivalent_ssh_and_https_remotes_have_one_identity_source() -> None:
    assert remote_normalize(
        remote="git@gitlab.com:team/service.git"
    ) == remote_normalize(remote="https://gitlab.com/team/service.git")


def test_unexpected_git_failure_is_not_treated_as_local_project(tmp_path: Path) -> None:
    def failed(command: list[str], *, cwd: Path) -> subprocess.CompletedProcess:
        return subprocess.CompletedProcess(command, 128, "", "fatal: unsafe repository")

    with pytest.raises(IdentityError, match="unsafe repository"):
        project_detect(cwd=tmp_path, runner=failed)


def test_expected_non_repository_failure_uses_local_identity(tmp_path: Path) -> None:
    def absent(command: list[str], *, cwd: Path) -> subprocess.CompletedProcess:
        return subprocess.CompletedProcess(
            command, 128, "", "fatal: not a git repository"
        )

    detected = project_detect(cwd=tmp_path, runner=absent)
    assert detected["root"] == str(tmp_path.resolve())
    assert detected["remote"] is None
