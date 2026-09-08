from __future__ import annotations

import hashlib
import subprocess
from pathlib import Path
from typing import Protocol
from urllib.parse import urlsplit


class GitRunner(Protocol):
    def __call__(
        self, command: list[str], *, cwd: Path
    ) -> subprocess.CompletedProcess: ...


class IdentityError(RuntimeError):
    pass


def subprocess_run(command: list[str], *, cwd: Path) -> subprocess.CompletedProcess:
    return subprocess.run(command, cwd=cwd, capture_output=True, text=True, check=False)


def git_read(*, cwd: Path, arguments: list[str], runner: GitRunner) -> str | None:
    result = runner(["git", *arguments], cwd=cwd)
    value = result.stdout.strip()
    if result.returncode == 0:
        return value or None
    expected = ("not a git repository", "no such remote")
    if any(message in result.stderr.lower() for message in expected):
        return None
    raise IdentityError(f"git {' '.join(arguments)} failed: {result.stderr.strip()}")


def remote_normalize(*, remote: str) -> str:
    value = remote.strip()
    if "://" not in value and ":" in value:
        host, path = value.split(":", 1)
        value = f"ssh://{host}/{path}"
    parsed = urlsplit(value)
    host = (parsed.hostname or "").lower()
    path = parsed.path.strip("/")
    path = path.removesuffix(".git")
    return f"{host}/{path}".lower()


def project_detect(
    *, cwd: Path, runner: GitRunner = subprocess_run
) -> dict[str, str | None]:
    root_value = git_read(
        cwd=cwd, arguments=["rev-parse", "--show-toplevel"], runner=runner
    )
    root = Path(root_value).resolve() if root_value else cwd.resolve()
    remote = git_read(
        cwd=root, arguments=["remote", "get-url", "origin"], runner=runner
    )
    source = remote_normalize(remote=remote) if remote else str(root)
    digest = hashlib.sha256(source.encode()).hexdigest()[:16]
    return {
        "project_id": f"prj-{digest}",
        "name": root.name,
        "root": str(root),
        "remote": remote,
    }
