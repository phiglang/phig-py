"""phig configuration language"""

from __future__ import annotations

import io
from typing import IO, Any

from ._phig import (
    PhigError,
    dump as _dump,
    dumps as _dumps,
    load as _load,
    loads as _loads,
)

__all__ = ["loads", "dumps", "load", "dump", "PhigError"]


def load(fp: IO[str]) -> dict[str, Any]:
    """Read and parse phig from a file object."""
    return _load(fp)


def loads(s: str) -> dict[str, Any]:
    """Parse a phig string into a dict.

    >>> loads('name foo\\nport 8080')
    {'name': 'foo', 'port': '8080'}
    """
    return _loads(s)


def dump(data: Any, fp: IO[str]) -> None:
    """Serialize and write phig to a file object."""
    _dump(_to_dict(data), fp)


def dumps(data: Any) -> str:
    """Serialize a dict to a phig string.

    >>> dumps({'name': 'foo', 'port': '8080'})
    'name foo\\nport 8080\\n'
    """
    return _dumps(_to_dict(data))


def _to_dict(obj: Any) -> Any:
    if hasattr(obj, "__dataclass_fields__"):
        import dataclasses

        return dataclasses.asdict(obj)
    if isinstance(obj, dict):
        return {k: _to_dict(v) for k, v in obj.items()}
    if isinstance(obj, list):
        return [_to_dict(v) for v in obj]
    if isinstance(obj, bool):
        return str(obj).lower()
    if isinstance(obj, (int, float)):
        return str(obj)
    return obj
