from __future__ import annotations

import os
import re
from pathlib import Path
from typing import ClassVar

from atomicwrites import atomic_write
from filelock import FileLock
from pydantic import BaseModel

from .documents import entity_parse, entity_render
from .entities import Project, Request, Session


class MissingEntity(ValueError):
    pass


class Repository:
    identifier_patterns: ClassVar = {
        "projects": re.compile(r"prj-[0-9a-f]{16}"),
        "sessions": re.compile(r"ses-[0-9a-f-]{36}"),
        "requests": re.compile(r"req-[0-9a-f-]{36}"),
    }

    def __init__(self, *, home: Path):
        self.home = home.expanduser()
        for collection in ("projects", "sessions", "requests", "locks"):
            (self.home / collection).mkdir(parents=True, exist_ok=True)

    @classmethod
    def environment(cls) -> Repository:
        return cls(home=Path(os.environ.get("MERL_HOME", "~/.merl")))

    def path(self, *, collection: str, identifier: str) -> Path:
        pattern = self.identifier_patterns.get(collection)
        if pattern is None or pattern.fullmatch(identifier) is None:
            singular = collection.removesuffix("s")
            raise ValueError(f"invalid {singular} identifier: {identifier}")
        return self.home / collection / f"{identifier}.md"

    def save(
        self, *, collection: str, identifier: str, entity: BaseModel, body: str
    ) -> None:
        path = self.path(collection=collection, identifier=identifier)
        with atomic_write(path, overwrite=True, encoding="utf-8") as handle:
            handle.write(entity_render(entity=entity, body=body))

    def load[Entity: BaseModel](
        self, *, collection: str, identifier: str, entity_type: type[Entity]
    ) -> tuple[Entity, str]:
        path = self.path(collection=collection, identifier=identifier)
        try:
            return entity_parse(
                text=path.read_text(encoding="utf-8"), entity_type=entity_type
            )
        except FileNotFoundError as error:
            raise MissingEntity(
                f"unknown {entity_type.__name__.lower()}: {identifier}"
            ) from error

    def iterate[Entity: BaseModel](self, *, collection: str, entity_type: type[Entity]):
        for path in sorted((self.home / collection).glob("*.md")):
            yield entity_parse(
                text=path.read_text(encoding="utf-8"), entity_type=entity_type
            )

    def request_lock(self, *, request_id: str) -> FileLock:
        self.path(collection="requests", identifier=request_id)
        return FileLock(self.home / "locks" / f"{request_id}.lock", timeout=2)

    def project_lock(self, *, project_id: str) -> FileLock:
        self.path(collection="projects", identifier=project_id)
        return FileLock(self.home / "locks" / f"{project_id}.lock", timeout=2)

    def publication_lock(self, *, project_id: str, dedupe_key: str) -> FileLock:
        self.path(collection="projects", identifier=project_id)
        safe_key = re.sub(r"[^a-z0-9-]", "-", dedupe_key)
        return FileLock(
            self.home / "locks" / f"publish-{project_id}-{safe_key}.lock", timeout=2
        )

    def project_get(self, *, project_id: str) -> Project:
        return self.load(
            collection="projects", identifier=project_id, entity_type=Project
        )[0]

    def session_get(self, *, session_id: str) -> Session:
        return self.load(
            collection="sessions", identifier=session_id, entity_type=Session
        )[0]

    def request_get(self, *, request_id: str) -> tuple[Request, str]:
        return self.load(
            collection="requests", identifier=request_id, entity_type=Request
        )
