from __future__ import annotations

import re
import uuid
from datetime import UTC, datetime
from pathlib import Path

from .entities import Answer, Project, Request, Session
from .identity import project_detect
from .repository import MissingEntity, Repository
from .routing import inbox_rank, request_match


class WorkflowError(ValueError):
    pass


def now() -> str:
    return datetime.now(UTC).isoformat()


def tags_parse(*, value: str) -> list[str]:
    return sorted({item.strip().lower() for item in value.split(",") if item.strip()})


def session_initialize(
    *, repository: Repository, cwd: Path, owns: list[str], capabilities: list[str]
) -> dict:
    detected = project_detect(cwd=cwd)
    project_id = str(detected["project_id"])
    with repository.project_lock(project_id=project_id):
        try:
            project = repository.project_get(project_id=project_id)
            project.owns = sorted(set(project.owns) | set(owns))
            project.capabilities = sorted(set(project.capabilities) | set(capabilities))
            project.updated_at = now()
        except MissingEntity:
            project = Project(
                **detected, owns=owns, capabilities=capabilities, updated_at=now()
            )
        repository.save(
            collection="projects",
            identifier=project.project_id,
            entity=project,
            body=f"# {project.name}\n\nMerl project registration.",
        )
    session = Session(
        session_id=f"ses-{uuid.uuid4()}", project_id=project_id, created_at=now()
    )
    repository.save(
        collection="sessions",
        identifier=session.session_id,
        entity=session,
        body=f"# Session {session.session_id}\n\nProject: `{project.name}`",
    )
    return {
        "session_id": session.session_id,
        "project_id": project_id,
        "project": project.name,
        "inbox_count": len(
            inbox_list(repository=repository, session_id=session.session_id)
        ),
        "result_count": len(
            results_list(repository=repository, session_id=session.session_id)
        ),
    }


def request_create(
    *,
    repository: Repository,
    session_id: str,
    title: str,
    outcome: str,
    evidence: list[str],
    artifacts: list[str],
    needs: list[str],
    blocking: bool,
    continuation: str,
    details: str,
) -> Request:
    session = repository.session_get(session_id=session_id)
    dedupe_key = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")
    with repository.publication_lock(
        project_id=session.project_id, dedupe_key=dedupe_key
    ):
        for existing, _ in repository.iterate(
            collection="requests", entity_type=Request
        ):
            if (
                existing.origin_project_id == session.project_id
                and existing.dedupe_key == dedupe_key
                and existing.status != "answered"
            ):
                raise WorkflowError(
                    f"possible duplicate request: {existing.request_id}"
                )
        request = Request(
            request_id=f"req-{uuid.uuid4()}",
            title=title,
            outcome=outcome,
            evidence=evidence,
            artifacts=artifacts,
            needs=needs,
            blocking=blocking,
            continuation=continuation,
            details=details,
            origin_project_id=session.project_id,
            requester_session_id=session_id,
            dedupe_key=dedupe_key,
            created_at=now(),
        )
        body = f"# {title}\n\n## Requested outcome\n\n{outcome}"
        if details:
            body += f"\n\n## Details\n\n{details}"
        repository.save(
            collection="requests",
            identifier=request.request_id,
            entity=request,
            body=body,
        )
    return request


def inbox_list(*, repository: Repository, session_id: str) -> list[dict]:
    session = repository.session_get(session_id=session_id)
    project = repository.project_get(project_id=session.project_id)
    projects = [
        entity
        for entity, _ in repository.iterate(collection="projects", entity_type=Project)
    ]
    matches = []
    for request, _ in repository.iterate(collection="requests", entity_type=Request):
        if request.status != "open" or request.origin_project_id == project.project_id:
            continue
        match = request_match(request=request, project=project, projects=projects)
        if match:
            matches.append({**request.model_dump(mode="json"), "match": match})
    return sorted(
        matches, key=lambda item: (inbox_rank(match=item["match"]), item["created_at"])
    )


def request_claim(
    *, repository: Repository, session_id: str, request_id: str
) -> Request:
    repository.session_get(session_id=session_id)
    with repository.request_lock(request_id=request_id):
        request, body = repository.request_get(request_id=request_id)
        if request.status != "open":
            raise WorkflowError(f"request is already {request.status}")
        session = repository.session_get(session_id=session_id)
        if request.origin_project_id == session.project_id:
            raise WorkflowError("request cannot be claimed by its originating project")
        project = repository.project_get(project_id=session.project_id)
        projects = [
            entity
            for entity, _ in repository.iterate(
                collection="projects", entity_type=Project
            )
        ]
        if request_match(request=request, project=project, projects=projects) is None:
            raise WorkflowError("request is not routed to this project")
        request.status = "claimed"
        request.claimed_by = session_id
        request.claimed_at = now()
        repository.save(
            collection="requests", identifier=request_id, entity=request, body=body
        )
    return request


def request_answer(
    *,
    repository: Repository,
    session_id: str,
    request_id: str,
    summary: str,
    evidence: list[str],
    artifacts: list[str],
    concerns: str,
    integration: str,
) -> Request:
    repository.session_get(session_id=session_id)
    with repository.request_lock(request_id=request_id):
        request, body = repository.request_get(request_id=request_id)
        if request.status != "claimed" or request.claimed_by != session_id:
            raise WorkflowError("only the current claimant can answer this request")
        request.answer = Answer(
            summary=summary,
            evidence=evidence,
            artifacts=artifacts,
            concerns=concerns,
            integration=integration,
            answered_at=now(),
        )
        request.status = "answered"
        repository.save(
            collection="requests", identifier=request_id, entity=request, body=body
        )
    return request


def outgoing_list(*, repository: Repository, session_id: str) -> list[dict]:
    session = repository.session_get(session_id=session_id)
    return [
        request.model_dump(mode="json")
        for request, _ in repository.iterate(collection="requests", entity_type=Request)
        if request.origin_project_id == session.project_id
        and request.status != "answered"
    ]


def results_list(*, repository: Repository, session_id: str) -> list[dict]:
    session = repository.session_get(session_id=session_id)
    return [
        request.model_dump(mode="json")
        for request, _ in repository.iterate(collection="requests", entity_type=Request)
        if request.origin_project_id == session.project_id
        and request.status == "answered"
        and session.project_id not in request.acknowledged_by_projects
    ]


def results_acknowledge(*, repository: Repository, session_id: str) -> list[dict]:
    session = repository.session_get(session_id=session_id)
    results = results_list(repository=repository, session_id=session_id)
    for result in results:
        request_id = result["request_id"]
        with repository.request_lock(request_id=request_id):
            request, body = repository.request_get(request_id=request_id)
            if session.project_id not in request.acknowledged_by_projects:
                request.acknowledged_by_projects.append(session.project_id)
                repository.save(
                    collection="requests",
                    identifier=request_id,
                    entity=request,
                    body=body,
                )
    return results


def work_list(*, repository: Repository, session_id: str) -> list[dict]:
    repository.session_get(session_id=session_id)
    return [
        request.model_dump(mode="json")
        for request, _ in repository.iterate(collection="requests", entity_type=Request)
        if request.claimed_by == session_id and request.status == "claimed"
    ]


def history_list(*, repository: Repository, session_id: str) -> list[dict]:
    session = repository.session_get(session_id=session_id)
    return [
        request.model_dump(mode="json")
        for request, _ in repository.iterate(collection="requests", entity_type=Request)
        if request.origin_project_id == session.project_id
        or request.claimed_by == session_id
    ]
