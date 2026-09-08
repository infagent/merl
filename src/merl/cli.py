from __future__ import annotations

import json as json_module
from pathlib import Path
from typing import Annotated

import typer
from pydantic import ValidationError

from .repository import Repository
from .service import (
    WorkflowError,
    history_list,
    inbox_list,
    outgoing_list,
    request_answer,
    request_claim,
    request_create,
    results_acknowledge,
    results_list,
    session_initialize,
    tags_parse,
    work_list,
)

app = typer.Typer(help="A passive, Markdown-backed cross-project agent help queue.")


def repository_get(*, context: typer.Context) -> Repository:
    return context.obj


def emit(*, value: object, structured: bool) -> None:
    if structured:
        typer.echo(json_module.dumps(value, indent=2, sort_keys=True))
    elif isinstance(value, list):
        if not value:
            typer.echo("No items.")
        for item in value:
            typer.echo(f"{item['request_id']} [{item['status']}] {item['title']}")
    elif isinstance(value, dict):
        typer.echo("\n".join(f"{key}: {content}" for key, content in value.items()))
    else:
        typer.echo(value)


def fail(*, error: Exception) -> None:
    typer.echo(f"Error: {error}", err=True)
    raise typer.Exit(2)


@app.callback()
def main(
    context: typer.Context,
    home: Annotated[
        Path | None,
        typer.Option(help="Coordination root; defaults to MERL_HOME or ~/.merl."),
    ] = None,
) -> None:
    context.obj = Repository(home=home) if home else Repository.environment()


@app.command("init")
def initialize(
    context: typer.Context,
    cwd: Annotated[Path, typer.Option()] = Path("."),
    owns: Annotated[str, typer.Option()] = "",
    capabilities: Annotated[str, typer.Option()] = "",
    json: Annotated[bool, typer.Option("--json")] = False,
) -> None:
    try:
        result = session_initialize(
            repository=repository_get(context=context),
            cwd=cwd,
            owns=tags_parse(value=owns),
            capabilities=tags_parse(value=capabilities),
        )
        emit(value=result, structured=json)
    except (ValueError, OSError) as error:
        fail(error=error)


@app.command()
def ask(
    context: typer.Context,
    session: Annotated[str, typer.Option()],
    title: Annotated[str, typer.Option()],
    outcome: Annotated[str, typer.Option()],
    evidence: Annotated[list[str], typer.Option()],
    artifact: Annotated[list[str], typer.Option()],
    needs: Annotated[str, typer.Option()],
    blocking: Annotated[bool | None, typer.Option("--blocking/--non-blocking")] = None,
    continuation: Annotated[str, typer.Option()] = "",
    details: Annotated[str, typer.Option()] = "",
    json: Annotated[bool, typer.Option("--json")] = False,
) -> None:
    try:
        if blocking is None:
            raise WorkflowError("declare --blocking or --non-blocking")
        request = request_create(
            repository=repository_get(context=context),
            session_id=session,
            title=title,
            outcome=outcome,
            evidence=evidence,
            artifacts=artifact,
            needs=tags_parse(value=needs),
            blocking=blocking,
            continuation=continuation,
            details=details,
        )
        emit(value=request.model_dump(mode="json"), structured=json)
    except (ValueError, ValidationError, OSError) as error:
        fail(error=error)


def view_emit(
    *, context: typer.Context, session: str, view: str, structured: bool
) -> None:
    try:
        operation = {
            "inbox": inbox_list,
            "outgoing": outgoing_list,
            "results": results_list,
            "work": work_list,
            "history": history_list,
        }[view]
        emit(
            value=operation(
                repository=repository_get(context=context), session_id=session
            ),
            structured=structured,
        )
    except (ValueError, OSError) as error:
        fail(error=error)


@app.command()
def inbox(
    context: typer.Context,
    session: Annotated[str, typer.Option()],
    json: Annotated[bool, typer.Option("--json")] = False,
) -> None:
    view_emit(context=context, session=session, view="inbox", structured=json)


@app.command()
def outgoing(
    context: typer.Context,
    session: Annotated[str, typer.Option()],
    json: Annotated[bool, typer.Option("--json")] = False,
) -> None:
    view_emit(context=context, session=session, view="outgoing", structured=json)


@app.command()
def results(
    context: typer.Context,
    session: Annotated[str, typer.Option()],
    acknowledge: Annotated[bool, typer.Option("--acknowledge")] = False,
    json: Annotated[bool, typer.Option("--json")] = False,
) -> None:
    if not acknowledge:
        view_emit(context=context, session=session, view="results", structured=json)
        return
    try:
        emit(
            value=results_acknowledge(
                repository=repository_get(context=context), session_id=session
            ),
            structured=json,
        )
    except (ValueError, OSError) as error:
        fail(error=error)


@app.command("work")
def active_work(
    context: typer.Context,
    session: Annotated[str, typer.Option()],
    json: Annotated[bool, typer.Option("--json")] = False,
) -> None:
    view_emit(context=context, session=session, view="work", structured=json)


@app.command()
def history(
    context: typer.Context,
    session: Annotated[str, typer.Option()],
    json: Annotated[bool, typer.Option("--json")] = False,
) -> None:
    view_emit(context=context, session=session, view="history", structured=json)


@app.command()
def claim(
    context: typer.Context,
    session: Annotated[str, typer.Option()],
    request: Annotated[str, typer.Option()],
    json: Annotated[bool, typer.Option("--json")] = False,
) -> None:
    try:
        value = request_claim(
            repository=repository_get(context=context),
            session_id=session,
            request_id=request,
        )
        emit(value=value.model_dump(mode="json"), structured=json)
    except (ValueError, OSError) as error:
        fail(error=error)


@app.command()
def answer(
    context: typer.Context,
    session: Annotated[str, typer.Option()],
    request: Annotated[str, typer.Option()],
    summary: Annotated[str, typer.Option()],
    evidence: Annotated[list[str], typer.Option()],
    artifact: Annotated[list[str] | None, typer.Option()] = None,
    concerns: Annotated[str, typer.Option()] = "",
    integration: Annotated[str, typer.Option()] = "",
    json: Annotated[bool, typer.Option("--json")] = False,
) -> None:
    try:
        value = request_answer(
            repository=repository_get(context=context),
            session_id=session,
            request_id=request,
            summary=summary,
            evidence=evidence,
            artifacts=artifact or [],
            concerns=concerns,
            integration=integration,
        )
        emit(value=value.model_dump(mode="json"), structured=json)
    except (ValueError, ValidationError, OSError) as error:
        fail(error=error)
