from __future__ import annotations

from typing import IO, Any

from ._error import PhigError

_SPECIAL = frozenset("{}[]\"#'; \t\n\r")


def _can_be_bare(s: str) -> bool:
    if not s:
        return False
    return not any(c in _SPECIAL or c.isspace() for c in s)


def _write_string(s: str, fp: IO[str]) -> None:
    if _can_be_bare(s):
        fp.write(s)
        return
    fp.write('"')
    for c in s:
        if c == '"':
            fp.write('\\"')
        elif c == "\\":
            fp.write("\\\\")
        elif c == "\n":
            fp.write("\\n")
        elif c == "\r":
            fp.write("\\r")
        elif c == "\t":
            fp.write("\\t")
        elif c == "\0":
            fp.write("\\0")
        elif c.isprintable() or not c.isascii():
            fp.write(c)
        else:
            fp.write(f"\\u{{{ord(c):x}}}")
    fp.write('"')


def _write_value(v: Any, indent: int, fp: IO[str]) -> None:
    if isinstance(v, bool):
        fp.write(str(v).lower())
    elif isinstance(v, str):
        _write_string(v, fp)
    elif isinstance(v, (int, float)):
        fp.write(str(v))
    elif isinstance(v, list):
        _write_list(v, indent, fp)
    elif isinstance(v, dict):
        _write_map(v, indent, fp, top_level=False)
    else:
        raise PhigError(f"unsupported type: {type(v).__name__}")


def _write_list(items: list[Any], indent: int, fp: IO[str]) -> None:
    if not items:
        fp.write("[]")
        return

    has_compound = any(isinstance(v, (dict, list)) for v in items)

    if has_compound:
        inner = indent + 1
        pad = "  " * inner
        close_pad = "  " * indent
        fp.write("[\n")
        for v in items:
            fp.write(pad)
            _write_value(v, inner, fp)
            fp.write("\n")
        fp.write(close_pad)
        fp.write("]")
    else:
        fp.write("[")
        for i, v in enumerate(items):
            if i > 0:
                fp.write(" ")
            _write_value(v, indent, fp)
        fp.write("]")


def _write_map(
    pairs: dict[str, Any], indent: int, fp: IO[str], *, top_level: bool
) -> None:
    if not pairs:
        if not top_level:
            fp.write("{}")
        return

    inner = indent if top_level else indent + 1
    pad = "  " * inner

    if not top_level:
        fp.write("{\n")

    for k, v in pairs.items():
        fp.write(pad)
        _write_string(k, fp)
        fp.write(" ")
        _write_value(v, inner, fp)
        fp.write("\n")

    if not top_level:
        fp.write("  " * indent)
        fp.write("}")


def serialize(data: Any, fp: IO[str]) -> None:
    if not isinstance(data, dict):
        raise PhigError("top-level value must be a dict")
    _write_map(data, 0, fp, top_level=True)
