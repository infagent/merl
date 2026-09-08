from __future__ import annotations

import json
from pathlib import Path

from typer.testing import CliRunner

from merl.cli import app

runner = CliRunner()


def invoke(home: Path, *arguments: str):
    return runner.invoke(app, ["--home", str(home), *arguments])


def initialized(
    home: Path, project: Path, *, owns: str = "", capabilities: str = ""
) -> dict:
    project.mkdir(parents=True, exist_ok=True)
    result = invoke(
        home,
        "init",
        "--cwd",
        str(project),
        "--owns",
        owns,
        "--capabilities",
        capabilities,
        "--json",
    )
    assert result.exit_code == 0, result.output
    return json.loads(result.output)


def test_two_sessions_share_a_project_but_not_an_identity(tmp_path: Path) -> None:
    project = tmp_path / "app"
    project.mkdir()

    first = initialized(
        tmp_path / "board", project, owns="billing", capabilities="python"
    )
    second = initialized(tmp_path / "board", project)

    assert first["project_id"] == second["project_id"]
    assert first["session_id"] != second["session_id"]
    assert second["inbox_count"] == 0


def test_owner_sees_request_before_capable_project_and_claim_is_human_selected(
    tmp_path: Path,
) -> None:
    home = tmp_path / "board"
    requester = initialized(home, tmp_path / "app", capabilities="python")
    owner = initialized(home, tmp_path / "infra", owns="aws-prod")
    capable = initialized(home, tmp_path / "ops", capabilities="aws-prod")

    asked = invoke(
        home,
        "ask",
        "--session",
        requester["session_id"],
        "--title",
        "Add the worker role",
        "--outcome",
        "Terraform grants the worker access",
        "--evidence",
        "The application fails with AccessDenied",
        "--artifact",
        "services/worker/config.ts",
        "--needs",
        "aws-prod",
        "--blocking",
        "--json",
    )
    assert asked.exit_code == 0, asked.output
    request_id = json.loads(asked.output)["request_id"]

    owner_inbox = invoke(home, "inbox", "--session", owner["session_id"], "--json")
    capable_inbox = invoke(home, "inbox", "--session", capable["session_id"], "--json")
    assert json.loads(owner_inbox.output)[0]["match"] == "ownership"
    assert json.loads(capable_inbox.output)[0]["match"] == "capability"

    still_open = json.loads(
        invoke(home, "outgoing", "--session", requester["session_id"], "--json").output
    )
    assert still_open[0]["status"] == "open"

    claimed = invoke(
        home,
        "claim",
        "--session",
        owner["session_id"],
        "--request",
        request_id,
        "--json",
    )
    assert claimed.exit_code == 0, claimed.output
    assert json.loads(claimed.output)["status"] == "claimed"
    assert (
        json.loads(
            invoke(home, "inbox", "--session", capable["session_id"], "--json").output
        )
        == []
    )


def test_nonblocking_answer_survives_requesting_session(tmp_path: Path) -> None:
    home = tmp_path / "board"
    requester = initialized(home, tmp_path / "terraform", owns="platform")
    worker = initialized(home, tmp_path / "wiki", owns="docs")

    rejected = invoke(
        home,
        "ask",
        "--session",
        requester["session_id"],
        "--title",
        "Document the new role",
        "--outcome",
        "The runbook explains the worker role",
        "--evidence",
        "Terraform adds role worker-prod",
        "--artifact",
        "roles.tf",
        "--needs",
        "docs",
        "--non-blocking",
    )
    assert rejected.exit_code == 2
    assert "continuation" in rejected.output.lower()

    asked = invoke(
        home,
        "ask",
        "--session",
        requester["session_id"],
        "--title",
        "Document the new role",
        "--outcome",
        "The runbook explains the worker role",
        "--evidence",
        "Terraform adds role worker-prod",
        "--artifact",
        "roles.tf",
        "--needs",
        "docs",
        "--non-blocking",
        "--continuation",
        "When answered, link the runbook from deploy.md; no Terraform changes depend on it.",
        "--json",
    )
    request_id = json.loads(asked.output)["request_id"]
    assert (
        invoke(
            home, "claim", "--session", worker["session_id"], "--request", request_id
        ).exit_code
        == 0
    )

    answered = invoke(
        home,
        "answer",
        "--session",
        worker["session_id"],
        "--request",
        request_id,
        "--summary",
        "Added the worker role to the operations runbook.",
        "--evidence",
        "Markdown link checker passed",
        "--artifact",
        "runbooks/worker.md",
        "--integration",
        "Link runbooks/worker.md from deploy.md",
        "--json",
    )
    assert answered.exit_code == 0, answered.output

    successor = initialized(home, tmp_path / "terraform")
    results = json.loads(
        invoke(home, "results", "--session", successor["session_id"], "--json").output
    )
    assert results[0]["request_id"] == request_id
    assert results[0]["answer"]["summary"].startswith("Added")
    assert results[0]["continuation"].startswith("When answered")
    history = json.loads(
        invoke(home, "history", "--session", successor["session_id"], "--json").output
    )
    assert history[0]["status"] == "answered"
    acknowledged = invoke(
        home,
        "results",
        "--session",
        successor["session_id"],
        "--acknowledge",
        "--json",
    )
    assert json.loads(acknowledged.output)[0]["request_id"] == request_id
    assert (
        json.loads(
            invoke(
                home, "results", "--session", successor["session_id"], "--json"
            ).output
        )
        == []
    )
    assert (
        json.loads(
            invoke(
                home, "history", "--session", successor["session_id"], "--json"
            ).output
        )[0]["status"]
        == "answered"
    )


def test_duplicate_and_wrong_worker_are_rejected(tmp_path: Path) -> None:
    home = tmp_path / "board"
    requester = initialized(home, tmp_path / "app")
    worker = initialized(home, tmp_path / "infra", capabilities="aws")
    stranger = initialized(home, tmp_path / "other", capabilities="aws")
    ask_args = (
        "ask",
        "--session",
        requester["session_id"],
        "--title",
        "Fix AWS access",
        "--outcome",
        "Access works",
        "--evidence",
        "AccessDenied",
        "--artifact",
        "app.log",
        "--needs",
        "aws",
        "--blocking",
        "--json",
    )
    first = invoke(home, *ask_args)
    assert first.exit_code == 0
    duplicate = invoke(home, *ask_args)
    assert duplicate.exit_code == 2
    assert "duplicate" in duplicate.output.lower()

    request_id = json.loads(first.output)["request_id"]
    assert (
        invoke(
            home, "claim", "--session", worker["session_id"], "--request", request_id
        ).exit_code
        == 0
    )
    denied = invoke(
        home,
        "answer",
        "--session",
        stranger["session_id"],
        "--request",
        request_id,
        "--summary",
        "Done",
        "--evidence",
        "Test passed",
    )
    assert denied.exit_code == 2
    assert "claimant" in denied.output.lower()


def test_direct_claim_cannot_bypass_routing(tmp_path: Path) -> None:
    home = tmp_path / "board"
    requester = initialized(home, tmp_path / "app")
    initialized(home, tmp_path / "infra", owns="aws")
    unrelated = initialized(home, tmp_path / "docs", capabilities="writing")
    asked = invoke(
        home,
        "ask",
        "--session",
        requester["session_id"],
        "--title",
        "Grant synthetic access",
        "--outcome",
        "Access works",
        "--evidence",
        "Synthetic AccessDenied",
        "--artifact",
        "synthetic.log",
        "--needs",
        "aws",
        "--blocking",
        "--json",
    )

    denied = invoke(
        home,
        "claim",
        "--session",
        unrelated["session_id"],
        "--request",
        json.loads(asked.output)["request_id"],
    )

    assert denied.exit_code == 2
    assert "not routed" in denied.output.lower()


def test_originating_project_cannot_claim_its_own_global_request(
    tmp_path: Path,
) -> None:
    home = tmp_path / "board"
    requester = initialized(home, tmp_path / "app")
    asked = invoke(
        home,
        "ask",
        "--session",
        requester["session_id"],
        "--title",
        "Unknown owner",
        "--outcome",
        "Find the owner",
        "--evidence",
        "No ownership metadata",
        "--artifact",
        "synthetic.txt",
        "--needs",
        "unknown",
        "--blocking",
        "--json",
    )
    denied = invoke(
        home,
        "claim",
        "--session",
        requester["session_id"],
        "--request",
        json.loads(asked.output)["request_id"],
    )
    assert denied.exit_code == 2
    assert "originating project" in denied.output.lower()


def test_unmatched_request_appears_in_global_inbox(tmp_path: Path) -> None:
    home = tmp_path / "board"
    requester = initialized(home, tmp_path / "app")
    unrelated = initialized(home, tmp_path / "docs", capabilities="writing")
    invoke(
        home,
        "ask",
        "--session",
        requester["session_id"],
        "--title",
        "Find mystery owner",
        "--outcome",
        "Mystery change is made",
        "--evidence",
        "No registered owner",
        "--artifact",
        "synthetic.txt",
        "--needs",
        "mystery",
        "--blocking",
        "--json",
    )
    inbox = json.loads(
        invoke(home, "inbox", "--session", unrelated["session_id"], "--json").output
    )
    assert len(inbox) == 1
    assert inbox[0]["match"] == "global"
