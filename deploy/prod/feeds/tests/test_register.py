"""F4: the licence register, stored cells under datumesh.com from register.json."""
import json

import pytest

import register as reg
from conftest import fixture
from helpers import check_order, check_source_object, check_summary, host_of, split_lines
from urllib.parse import urlparse


def entries():
    return reg.parse(fixture("register.json"))


def test_parse_keeps_the_relay_entries_only():
    rows = entries()
    assert sorted(r["source_domain"] for r in rows) == sorted(
        ["ecb.europa.eu/fx", "ecb.europa.eu/estr", "ecb.europa.eu/policy-rates", "chc.ucsb.edu/chirps",
         "home.treasury.gov", "bankofengland.co.uk"])  # the two SKIP verdicts are not loaded
    assert all(r["verdict"] == "RELAY" for r in rows)


def test_parse_refuses_a_changed_shape():
    from common import ShapeError
    data = fixture("register.json")
    for bad in (data.replace(b'"sha256"', b'"sha"'), data.replace(b'"retrieved"', b'"when"'), b"{}", b"", b"[1]",
                data.replace(b"5c467e3b05d8e84a", b"zz")):
        with pytest.raises(ShapeError):
            reg.parse(bad)


def test_lines(names, register):
    sec = names["register"]
    lines, state = reg.emit(entries(), {}, names)
    check_order(lines)
    fns, cats, cells, tests = split_lines(lines)
    ft, fp = f"{sec['namespace']}/{sec['text_function']}", f"{sec['namespace']}/{sec['page_function']}"
    assert [f["function"] for f in fns] == [ft, fp] and tests == []
    for f in fns:
        assert f["signature"] == {"args": {"source": {"type": "string", "valid": False},
                                           "retrieved": {"type": "date", "valid": True}},
                                  "order": ["source", "retrieved"], "returns": "string"}
    assert sorted(c["catalog"] for c in cats) == sorted([ft, fp])
    relay = [r for r in register if r["verdict"] == "RELAY"]
    for c in cats:
        assert c["kind"] == "function" and set(c["args"]) == {"source", "retrieved"} and c["label"]
        assert {e["value"] for e in c["dictionary"]["source"]} == {r["source_domain"] for r in relay}
        assert all(e["label"] for e in c["dictionary"]["source"])
    assert len(cells) == 2 * len(relay)
    for r in relay:
        day = r["retrieved"][:10]
        args = {"source": {"type": "string", "value": r["source_domain"]}, "retrieved": {"type": "date", "value": day}}
        t = [c for c in cells if c["cell"] == {"fn": ft, "args": args}]
        p = [c for c in cells if c["cell"] == {"fn": fp, "args": args}]
        assert len(t) == len(p) == 1
        assert t[0]["value"] == {"type": "string", "value": r["sha256"]}
        assert p[0]["value"] == {"type": "string", "value": r["licence_url"]}
        for c in (t[0], p[0]):
            assert set(c) == {"cell", "value", "licence", "summary", "source"}
            assert c["licence"] == sec["licence"]
            check_summary(c["summary"])
            assert r["source_domain"] in c["summary"] and "RELAY" in c["summary"]
            if r["attribution"]:
                assert r["attribution"] in c["summary"]
            # the register records what the licence page said when datumesh.com fetched it
            check_source_object(c["source"], host_of(sec["namespace"]), name=urlparse(r["licence_url"]).hostname,
                                url=r["licence_url"], sha=r["sha256"], retrieved=r["retrieved"], takeover=None)
    assert reg.emit(entries(), json.loads(json.dumps(state)), names)[0] == []
