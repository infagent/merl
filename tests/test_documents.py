from __future__ import annotations

from pathlib import Path

import pytest

from merl.documents import DocumentError, entity_parse, entity_render
from merl.entities import Session
from merl.repository import Repository
from merl.service import session_initialize


def test_markdown_entity_round_trips_multiline_body() -> None:
    session = Session(
        session_id="ses-synthetic",
        project_id="prj-synthetic",
        created_at="2026-01-01T00:00:00+00:00",
    )

    rendered = entity_render(entity=session, body="# Context\n\nSynthetic notes.")
    parsed, body = entity_parse(text=rendered, entity_type=Session)

    assert parsed == session
    assert body == "# Context\n\nSynthetic notes."
    assert rendered.startswith("---\n")


def test_malformed_document_is_rejected_without_rewrite(tmp_path: Path) -> None:
    repository = Repository(home=tmp_path)
    session_id = "ses-00000000-0000-0000-0000-000000000000"
    path = repository.path(collection="sessions", identifier=session_id)
    path.write_text("---\nkind: wrong\n---\nOriginal", encoding="utf-8")

    with pytest.raises(DocumentError):
        repository.session_get(session_id=session_id)

    assert path.read_text(encoding="utf-8").endswith("Original")


def test_environment_home_override_is_respected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("MERL_HOME", str(tmp_path / "shared"))
    repository = Repository.environment()
    assert repository.home == tmp_path / "shared"


def test_identifiers_cannot_escape_the_board(tmp_path: Path) -> None:
    repository = Repository(home=tmp_path / "board")

    with pytest.raises(ValueError, match="invalid session identifier"):
        repository.session_get(session_id="../../outside")

    assert not (tmp_path / "outside.md").exists()


def test_initialization_does_not_replace_a_corrupt_project(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repository = Repository(home=tmp_path / "board")
    project_id = "prj-0000000000000000"
    path = repository.path(collection="projects", identifier=project_id)
    original = "---\nkind: invalid\n---\nDo not replace"
    path.write_text(original, encoding="utf-8")
    monkeypatch.setattr(
        "merl.service.project_detect",
        lambda *, cwd: {
            "project_id": project_id,
            "name": "synthetic",
            "root": str(cwd),
            "remote": None,
        },
    )

    with pytest.raises(DocumentError):
        session_initialize(
            repository=repository,
            cwd=tmp_path,
            owns=[],
            capabilities=[],
        )

    assert path.read_text(encoding="utf-8") == original
