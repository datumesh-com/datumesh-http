"""Shared fixtures for the feed tests. Run: see ../README.md (python:3.13-slim, pytest)."""
import pathlib
import sys
import tomllib

import pytest

FEEDS = pathlib.Path(__file__).resolve().parent.parent
FIXTURES = FEEDS / "fixtures"
sys.path.insert(0, str(FEEDS))
sys.dont_write_bytecode = True


def fixture(name: str) -> bytes:
    return (FIXTURES / name).read_bytes()


@pytest.fixture(scope="session")
def names():
    with open(FEEDS / "names.toml", "rb") as f:
        return tomllib.load(f)


@pytest.fixture(scope="session")
def register():
    import json
    return json.loads(fixture("register.json"))
