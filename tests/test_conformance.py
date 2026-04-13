"""Run conformance tests against the shared testdata/ fixtures."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

import phig

TESTDATA = Path(__file__).resolve().parent / "testdata"


def _collect_pass() -> list[str]:
    return sorted(
        p.stem for p in TESTDATA.glob("*.phig") if not p.stem.endswith("_FAIL")
    )


def _collect_fail() -> list[str]:
    return sorted(p.stem for p in TESTDATA.glob("*_FAIL.phig"))


@pytest.mark.parametrize("name", _collect_pass())
def test_pass(name: str) -> None:
    phig_text = (TESTDATA / f"{name}.phig").read_text()
    expected = json.loads((TESTDATA / f"{name}.json").read_text())
    result = phig.loads(phig_text)
    assert result == expected, f"{name}: {result!r} != {expected!r}"


@pytest.mark.parametrize("name", _collect_fail())
def test_fail(name: str) -> None:
    phig_text = (TESTDATA / f"{name}.phig").read_text()
    with pytest.raises(phig.PhigError):
        phig.loads(phig_text)
