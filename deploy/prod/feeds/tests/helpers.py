"""Checks shared by the feed tests. Names never appear here: they come from names.toml."""
import hashlib
import json
import re
import unicodedata
from decimal import Decimal

INSTANT = re.compile(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{6}Z$")
DNS = re.compile(r"^(?=.{1,253}$)([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$")
NAME = re.compile(r"^[a-z][a-z0-9_]*$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
RETRIEVED = "2026-11-19T12:00:00.000000Z"


def sha256(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def canon(s: str) -> str:
    """The published decimal without insignificant trailing zeros: 0.83290 -> 0.8329, 137.00 -> 137."""
    return format(Decimal(s).normalize(), "f")


def host_of(namespace: str) -> str:
    return namespace.split("/")[0]


def source_for(data: bytes, section: dict, url: str | None = None, takeover: bool = True) -> dict:
    """The `source` argument of emit(): what run() knows about the fetch (no name, no takeover)."""
    return {"url": url or section["url"], "retrieved": RETRIEVED, "sha256": sha256(data)}


def check_source_object(src: dict, publisher_host: str, *, name, url, sha, retrieved=RETRIEVED, takeover=None):
    """spec-0.6.0 C2: exactly these members; name a DNS name that is not the publisher's host;
    url https, printable ASCII, at most 512; retrieved an instant; sha256 64 lowercase hex."""
    allowed = {"name", "url", "retrieved", "sha256", "takeover"}
    assert set(src) <= allowed and {"name", "url", "retrieved"} <= set(src), src
    assert DNS.match(src["name"]) and src["name"] != publisher_host, src["name"]
    assert src["name"] == name
    assert src["url"].startswith("https://") and src["url"].isascii() and src["url"].isprintable()
    assert len(src["url"]) <= 512 and src["url"] == url
    assert INSTANT.match(src["retrieved"]) and src["retrieved"] == retrieved
    assert HEX64.match(src["sha256"]) and src["sha256"] == sha
    if takeover is None:
        assert "takeover" not in src
    else:
        assert src["takeover"] == takeover and src["takeover"].startswith("https://") and len(takeover) <= 512


def check_summary(text: str):
    assert isinstance(text, str) and text and "\n" not in text
    assert len(text.encode("utf-8")) <= 1024
    assert unicodedata.is_normalized("NFC", text)


def split_lines(lines):
    """-> (functions, catalogs, cells, tests); every line is a dict that survives one line of JSON."""
    out = {"function": [], "catalog": [], "cell": [], "test": []}
    for l in lines:
        assert isinstance(l, dict)
        assert "\n" not in json.dumps(l, ensure_ascii=False)
        kinds = [k for k in out if k in l]
        assert len(kinds) == 1, f"line is not exactly one of function/catalog/cell/test: {list(l)}"
        out[kinds[0]].append(l)
    return out["function"], out["catalog"], out["cell"], out["test"]


def check_order(lines):
    """function lines, then catalog lines, then cells, then tests."""
    rank = {"function": 0, "catalog": 1, "cell": 2, "test": 3}
    ranks = [next(rank[k] for k in rank if k in l) for l in lines]
    assert ranks == sorted(ranks), ranks


def call_text(fq: str, argnames: list[str], values: dict) -> str:
    """The call syntax of a cell: ns/fn(currency: "USD", date: 2026-11-18); strings quoted, dates bare."""
    parts = []
    for a in argnames:
        v = values[a]
        parts.append(f'{a}: {v}' if re.match(r"^\d{4}-\d\d-\d\d$", v) else f'{a}: "{v}"')
    return f"{fq}({', '.join(parts)})"
