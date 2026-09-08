from __future__ import annotations

from .entities import Project, Request


def request_match(
    *, request: Request, project: Project, projects: list[Project]
) -> str | None:
    needs = set(request.needs)
    if needs & set(project.owns):
        return "ownership"
    if needs & set(project.capabilities):
        return "capability"
    any_match = any(
        needs & (set(candidate.owns) | set(candidate.capabilities))
        for candidate in projects
    )
    return None if any_match else "global"


def inbox_rank(*, match: str) -> int:
    return {"ownership": 0, "capability": 1, "global": 2}[match]
