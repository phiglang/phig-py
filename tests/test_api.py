"""Tests for the public loads/dumps/load/dump API."""

from __future__ import annotations

import io

import pytest

import phig


class TestLoads:
    def test_simple(self) -> None:
        assert phig.loads("name foo\nport 8080") == {
            "name": "foo",
            "port": "8080",
        }

    def test_nested(self) -> None:
        assert phig.loads("server { host localhost; port 3000 }") == {
            "server": {"host": "localhost", "port": "3000"},
        }

    def test_list(self) -> None:
        assert phig.loads("tags [a b c]") == {"tags": ["a", "b", "c"]}

    def test_empty(self) -> None:
        assert phig.loads("") == {}

    def test_comments(self) -> None:
        assert phig.loads("# header\na 1 # inline") == {"a": "1"}


class TestDumps:
    def test_simple(self) -> None:
        assert phig.dumps({"name": "foo", "port": "8080"}) == "name foo\nport 8080\n"

    def test_nested(self) -> None:
        result = phig.dumps({"server": {"host": "localhost", "port": "3000"}})
        assert result == "server {\n  host localhost\n  port 3000\n}\n"

    def test_list(self) -> None:
        assert phig.dumps({"tags": ["a", "b"]}) == "tags [a b]\n"

    def test_empty(self) -> None:
        assert phig.dumps({}) == ""

    def test_quoted(self) -> None:
        assert phig.dumps({"msg": "hello world"}) == 'msg "hello world"\n'

    def test_escapes(self) -> None:
        assert phig.dumps({"msg": "a\nb"}) == 'msg "a\\nb"\n'

    def test_coerces_non_string_scalars(self) -> None:
        assert phig.dumps({"port": 8080, "debug": True}) == "port 8080\ndebug true\n"


class TestRoundtrip:
    def test_value_roundtrip(self) -> None:
        src = "name foo\ntags [a b c]\nnested { x 1; y 2 }"
        data = phig.loads(src)
        text = phig.dumps(data)
        assert phig.loads(text) == data

    def test_file_io(self) -> None:
        data = {"name": "app", "port": "3000"}
        buf = io.StringIO()
        phig.dump(data, buf)
        buf.seek(0)
        assert phig.load(buf) == data


class TestErrors:
    def test_unclosed_brace(self) -> None:
        with pytest.raises(phig.PhigError):
            phig.loads("x {")

    def test_unclosed_bracket(self) -> None:
        with pytest.raises(phig.PhigError):
            phig.loads("x [")

    def test_duplicate_key(self) -> None:
        with pytest.raises(phig.PhigError, match="duplicate key"):
            phig.loads("a 1\na 2")

    def test_unterminated_string(self) -> None:
        with pytest.raises(phig.PhigError):
            phig.loads('x "hello')

    def test_invalid_escape(self) -> None:
        with pytest.raises(phig.PhigError):
            phig.loads('x "\\q"')

    def test_dumps_non_dict(self) -> None:
        with pytest.raises(phig.PhigError):
            phig.dumps("hello")  # type: ignore[arg-type]


class TestUnicodeWhitespace:
    def test_nbsp_not_separator(self) -> None:
        with pytest.raises(phig.PhigError):
            phig.loads("name\u00a0foo")

    def test_nbsp_in_bare_value(self) -> None:
        with pytest.raises(phig.PhigError):
            phig.loads("name foo\u00a0bar")

    def test_em_space_in_bare_value(self) -> None:
        with pytest.raises(phig.PhigError):
            phig.loads("name foo\u2003bar")

    def test_nbsp_in_quoted_ok(self) -> None:
        assert phig.loads('name "foo\u00a0bar"') == {"name": "foo\u00a0bar"}

    def test_nbsp_in_raw_ok(self) -> None:
        assert phig.loads("name 'foo\u00a0bar'") == {"name": "foo\u00a0bar"}

    def test_dumps_quotes_nbsp(self) -> None:
        text = phig.dumps({"name": "foo\u00a0bar"})
        assert phig.loads(text) == {"name": "foo\u00a0bar"}
