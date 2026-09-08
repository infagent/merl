from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, Field, model_validator


class Project(BaseModel):
    kind: Literal["project"] = "project"
    project_id: str
    name: str
    root: str
    remote: str | None = None
    owns: list[str] = Field(default_factory=list)
    capabilities: list[str] = Field(default_factory=list)
    updated_at: str


class Session(BaseModel):
    kind: Literal["session"] = "session"
    session_id: str
    project_id: str
    created_at: str


class Answer(BaseModel):
    summary: str = Field(min_length=1)
    evidence: list[str] = Field(min_length=1)
    artifacts: list[str] = Field(default_factory=list)
    concerns: str = ""
    integration: str = ""
    answered_at: str


class Request(BaseModel):
    kind: Literal["request"] = "request"
    request_id: str
    status: Literal["open", "claimed", "answered"] = "open"
    title: str = Field(min_length=1)
    outcome: str = Field(min_length=1)
    evidence: list[str] = Field(min_length=1)
    artifacts: list[str] = Field(min_length=1)
    needs: list[str] = Field(min_length=1)
    blocking: bool
    continuation: str = ""
    details: str = ""
    origin_project_id: str
    requester_session_id: str
    dedupe_key: str
    created_at: str
    claimed_by: str | None = None
    claimed_at: str | None = None
    answer: Answer | None = None
    acknowledged_by_projects: list[str] = Field(default_factory=list)

    @model_validator(mode="after")
    def continuation_validate(self) -> Request:
        if not self.blocking and len(self.continuation.strip()) < 80:
            raise ValueError(
                "non-blocking requests require at least 80 characters of continuation context"
            )
        return self
