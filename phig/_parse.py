from __future__ import annotations

from typing import IO, Any, Union

from ._error import PhigError

# Value = str | list[Value] | dict[str, Value]
Value = Union[str, list, dict]


def parse(stream: IO[str]) -> dict[str, Value]:
    return _Parser(stream).toplevel()


class _Parser:
    __slots__ = ("_stream", "_peeked", "_has_peeked", "pos")

    def __init__(self, stream: IO[str]) -> None:
        self._stream = stream
        self._peeked: str | None = None
        self._has_peeked = False
        self.pos = 0

    def at_end(self) -> bool:
        return self.peek() is None

    def peek(self) -> str | None:
        if not self._has_peeked:
            ch = self._stream.read(1)
            self._peeked = ch or None
            self._has_peeked = True
        return self._peeked

    def advance(self) -> str | None:
        ch = self.peek()
        if ch is not None:
            self.pos += 1
            self._has_peeked = False
        return ch

    def err(self, msg: str) -> PhigError:
        return PhigError(msg, self.pos)

    # HSPACE = /[ \t]+/
    def hspace(self) -> bool:
        found = False
        while self.peek() in (" ", "\t"):
            self.advance()
            found = True
        return found

    # PAIRSEP = /(\r?\n)+|;/
    def pairsep(self) -> bool:
        if self.peek() == ";":
            self.advance()
            return True
        found = False
        while True:
            ch = self.peek()
            if ch == "\n":
                self.advance()
                found = True
            elif ch == "\r":
                self.advance()
                if self.peek() == "\n":
                    self.advance()
                found = True
            else:
                break
        return found

    # COMMENT = '#' /[^\n]*/
    def comment(self) -> bool:
        if self.peek() != "#":
            return False
        while self.peek() is not None and self.peek() != "\n":
            self.advance()
        return True

    _WS = frozenset("\r\n\t ")

    # _ = { WS | COMMENT }
    def wsc(self) -> None:
        while self.peek() is not None:
            if self.peek() == "#":
                self.comment()
            elif self.peek() in self._WS:
                self.advance()
            else:
                break

    # QSTRING = '"' { QCHAR } '"'
    def qstring(self) -> str:
        open_pos = self.pos
        self.advance()  # skip "

        result: list[str] = []
        while True:
            if self.at_end():
                raise PhigError("unterminated string", open_pos)
            ch = self.peek()
            if ch == '"':
                self.advance()
                return "".join(result)
            if ch == "\\":
                esc_start = self.pos
                self.advance()
                if self.at_end():
                    raise PhigError("unterminated escape", esc_start)
                esc = self.peek()
                if esc == "n":
                    result.append("\n")
                    self.advance()
                elif esc == "r":
                    result.append("\r")
                    self.advance()
                elif esc == "t":
                    result.append("\t")
                    self.advance()
                elif esc == "\\":
                    result.append("\\")
                    self.advance()
                elif esc == '"':
                    result.append('"')
                    self.advance()
                elif esc == "0":
                    result.append("\0")
                    self.advance()
                elif esc == "\n":
                    # line continuation
                    self.advance()
                elif esc == "u":
                    self.advance()
                    if self.at_end() or self.peek() != "{":
                        raise PhigError("expected '{' after \\u", esc_start)
                    self.advance()  # skip {

                    hex_chars: list[str] = []
                    while not self.at_end() and self.peek() != "}":
                        if self.peek() not in "0123456789abcdefABCDEF":  # type: ignore[operator]
                            raise PhigError("invalid unicode escape", esc_start)
                        hex_chars.append(self.advance())  # type: ignore[arg-type]

                    hex_str = "".join(hex_chars)
                    if (
                        not hex_str
                        or len(hex_str) > 6
                        or self.at_end()
                        or self.peek() != "}"
                    ):
                        raise PhigError("invalid unicode escape", esc_start)
                    self.advance()  # skip }

                    try:
                        cp = int(hex_str, 16)
                        c = chr(cp)
                    except (ValueError, OverflowError):
                        raise PhigError("unicode codepoint out of range", esc_start)
                    if cp > 0x10FFFF or (0xD800 <= cp <= 0xDFFF):
                        raise PhigError("unicode codepoint out of range", esc_start)
                    result.append(c)
                else:
                    raise PhigError(f"invalid escape '\\{esc}'", esc_start)
            else:
                result.append(self.advance())  # type: ignore[arg-type]

    # QRSTRING = "'" /[^']*/ "'"
    def qrstring(self) -> str:
        open_pos = self.pos
        self.advance()  # skip '
        result: list[str] = []
        while not self.at_end() and self.peek() != "'":
            result.append(self.advance())  # type: ignore[arg-type]
        if not self.at_end() and self.peek() == "'":
            self.advance()
            return "".join(result)
        raise PhigError("unterminated raw string", open_pos)

    _SPECIAL = frozenset("{}[]\"#';")

    # BARE = /[^\p{White_Space}{}[\]"#';]+/
    def bare(self) -> str | None:
        result: list[str] = []
        while self.peek() is not None:
            ch = self.peek()
            if ch.isspace() or ch in self._SPECIAL:  # type: ignore[union-attr]
                break
            result.append(self.advance())  # type: ignore[arg-type]
        return "".join(result) if result else None

    # string = QSTRING | QRSTRING | BARE
    def string(self) -> str | None:
        ch = self.peek()
        if ch == '"':
            return self.qstring()
        if ch == "'":
            return self.qrstring()
        return self.bare()

    # value = map | list | string
    def value(self) -> Value | None:
        ch = self.peek()
        if ch == "{":
            return self.map()
        if ch == "[":
            return self.list()
        return self.string()

    # pair = string HSPACE value [ HSPACE ] [ COMMENT ]
    def pair(self, seen: set[str]) -> tuple[str, Value] | None:
        start = self.pos
        key = self.string()
        if key is None:
            return None

        if key in seen:
            raise PhigError(f"duplicate key '{key}'", start)
        seen.add(key)

        self.hspace()

        val = self.value()
        if val is None:
            raise PhigError(f"expected value for key '{key}'", self.pos)

        self.hspace()
        self.comment()

        return (key, val)

    # pairs = pair { PAIRSEP _ pair }
    def pairs(self, closing: str | None) -> list[tuple[str, Value]]:
        pairs: list[tuple[str, Value]] = []
        seen: set[str] = set()

        while True:
            if self.at_end() or self.peek() == closing:
                break

            result = self.pair(seen)
            if result is not None:
                pairs.append(result)
            else:
                if not self.at_end() and self.peek() != closing:
                    raise self.err(f"unexpected '{self.peek()}'")
                break

            if self.at_end() or self.peek() == closing:
                break

            if not self.pairsep():
                raise self.err("expected newline or ';' after value")
            self.wsc()

        return pairs

    # map = '{' _ [ pairs ] _ '}'
    def map(self) -> dict[str, Value]:
        open_pos = self.pos
        self.advance()  # skip {
        self.wsc()
        pairs = self.pairs(closing="}")
        self.wsc()
        if self.peek() == "}":
            self.advance()
            return dict(pairs)
        raise PhigError("unclosed '{'", open_pos)

    # items = value { _ [ ';' ] _ value }
    def items(self) -> list[Value]:
        items: list[Value] = []

        while True:
            if self.at_end() or self.peek() == "]":
                break

            val = self.value()
            if val is not None:
                items.append(val)
            else:
                if not self.at_end() and self.peek() != "]":
                    raise self.err(f"unexpected '{self.peek()}'")
                break

            if self.at_end() or self.peek() == "]":
                break

            self.wsc()
            if self.peek() == ";":
                self.advance()
            self.wsc()

        return items

    # list = '[' _ [ items ] _ ']'
    def list(self) -> list[Value]:
        open_pos = self.pos
        self.advance()  # skip [
        self.wsc()
        items = self.items()
        self.wsc()
        if self.peek() == "]":
            self.advance()
            return items
        raise PhigError("unclosed '['", open_pos)

    # toplevel = _ [ pairs ] _ EOF
    def toplevel(self) -> dict[str, Value]:
        self.wsc()
        pairs = self.pairs(closing=None)
        self.wsc()
        if not self.at_end():
            raise self.err(f"unexpected '{self.peek()}'")
        return dict(pairs)
