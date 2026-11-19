"""Shared fixtures for the feed tests. Run: see ../README.md (python:3.13-slim, pytest)."""
import pathlib
import sys

import pytest

FEEDS = pathlib.Path(__file__).resolve().parent.parent
FIXTURES = FEEDS / "fixtures"
sys.path.insert(0, str(FEEDS))
sys.dont_write_bytecode = True


def fixture(name: str) -> bytes:
    return (FIXTURES / name).read_bytes()


REHEARSAL = "mesh-test.datumesh.com"
PRODUCTION = "datumesh.com"


@pytest.fixture(scope="session")
def names():
    """The names of the rehearsal host: the default for every test that does not care about the host."""
    import common
    return common.names(REHEARSAL)


@pytest.fixture(scope="session")
def register():
    import json
    return json.loads(fixture("register.json"))
