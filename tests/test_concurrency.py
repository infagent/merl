from __future__ import annotations

import json
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from test_workflows import initialized, invoke

from merl.repository import Repository
from merl.service import WorkflowError, request_claim, request_create


def test_two_claimants_produce_exactly_one_winner(tmp_path: Path) -> None:
    home = tmp_path / "board"
    requester = initialized(home, tmp_path / "app")
    first = initialized(home, tmp_path / "infra-a", capabilities="aws")
    second = initialized(home, tmp_path / "infra-b", capabilities="aws")
    asked = invoke(
        home,
        "ask",
        "--session",
        requester["session_id"],
        "--title",
        "Synthetic access request",
        "--outcome",
        "Access succeeds",
        "--evidence",
        "Synthetic AccessDenied",
        "--artifact",
        "synthetic.log",
        "--needs",
        "aws",
        "--blocking",
        "--json",
    )
    request_id = json.loads(asked.output)["request_id"]
    repository = Repository(home=home)

    def claim(session_id: str) -> str:
        try:
            return (
                request_claim(
                    repository=repository, session_id=session_id, request_id=request_id
                ).claimed_by
                or ""
            )
        except WorkflowError:
            return "lost"

    with ThreadPoolExecutor(max_workers=2) as executor:
        outcomes = list(
            executor.map(claim, [first["session_id"], second["session_id"]])
        )

    assert outcomes.count("lost") == 1
    assert len([outcome for outcome in outcomes if outcome != "lost"]) == 1


def test_two_identical_asks_produce_one_open_request(tmp_path: Path) -> None:
    home = tmp_path / "board"
    requester = initialized(home, tmp_path / "app")
    repository = Repository(home=home)

    def ask() -> str:
        try:
            return request_create(
                repository=repository,
                session_id=requester["session_id"],
                title="Synthetic duplicate",
                outcome="One request exists",
                evidence=["Synthetic evidence"],
                artifacts=["synthetic.txt"],
                needs=["docs"],
                blocking=True,
                continuation="",
                details="",
            ).request_id
        except WorkflowError:
            return "duplicate"

    with ThreadPoolExecutor(max_workers=2) as executor:
        outcomes = list(executor.map(lambda _: ask(), range(2)))

    assert outcomes.count("duplicate") == 1
    assert len(list((home / "requests").glob("*.md"))) == 1
