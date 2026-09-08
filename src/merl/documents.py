from __future__ import annotations

import frontmatter
from pydantic import BaseModel


class DocumentError(ValueError):
    pass


def entity_render(*, entity: BaseModel, body: str) -> str:
    document = frontmatter.Post(body.strip(), **entity.model_dump(mode="json"))
    return frontmatter.dumps(document, sort_keys=True) + "\n"


def entity_parse[Entity: BaseModel](
    *, text: str, entity_type: type[Entity]
) -> tuple[Entity, str]:
    try:
        document = frontmatter.loads(text)
        return entity_type.model_validate(document.metadata), document.content
    except Exception as error:
        raise DocumentError(
            f"invalid {entity_type.__name__.lower()} document: {error}"
        ) from error
