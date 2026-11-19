"""F1, F2, F5 for each relay source: parse, refuse a changed shape, emit, catalogue, idempotence."""
import importlib
import json
import re

import pytest

from conftest import fixture
from helpers import (RETRIEVED, canon, check_order, check_source_object, check_summary, host_of, sha256,
                     split_lines, call_text)
from specs import SPECS


def load(key):
    mod = importlib.import_module(key)
    return mod


def parsed(key, names):
    """-> (module, [(rows, source_arg, bytes)] one per input, spec)"""
    mod, spec = load(key), SPECS[key]
    out = []
    for fname, url in spec["inputs"](names):
        data = fixture(fname)
        out.append((mod.parse(data), {"url": url, "retrieved": RETRIEVED, "sha256": sha256(data)}, data))
    return mod, out, spec


def emit_all(mod, inputs, state, names, keep=lambda r: True):
    lines = []
    for rows, src, _ in inputs:
        l, state = mod.emit([r for r in rows if keep(r)], state, names, src)
        lines += l
    return lines, state


def rowkey(spec, r):
    return tuple(r[c] for c in spec["keycols"]) + (r["date"],)


@pytest.mark.parametrize("key", SPECS)
def test_parse_exact_decimals_absent_days_absent(key, names):
    mod, inputs, spec = parsed(key, names)
    rows = [r for rs, _, _ in inputs for r in rs]
    assert len(rows) == spec["nrows"]
    by = {rowkey(spec, r): r for r in rows}
    assert len(by) == len(rows), "one row per key and date"
    for k, v in spec["present"].items():
        assert by[k]["value"] == v, (k, by[k]["value"], v)  # as published: trailing zeros kept, nothing rescaled
    for k in spec["absent"]:
        assert k not in by
    for r in rows:
        assert set(r) == {"date", "value", *spec["keycols"]}
        assert re.match(r"^\d{4}-\d\d-\d\d$", r["date"]) and isinstance(r["value"], str)
        assert re.match(r"^-?\d+(\.\d+)?$", r["value"])


@pytest.mark.parametrize("key", SPECS)
def test_refuses_changed_shape(key, names):
    from common import ShapeError
    mod, spec = load(key), SPECS[key]
    for fname, _ in spec["inputs"](names):
        data = fixture(fname)
        for mutate in spec["mutations"]:
            bad = mutate(data)
            assert bad != data
            with pytest.raises(ShapeError) as e:
                mod.parse(bad)
            assert str(e.value), "a clear message"


@pytest.mark.parametrize("key", SPECS)
def test_first_run_lines(key, names):
    mod, inputs, spec = parsed(key, names)
    sec = names[key]
    lines, state = emit_all(mod, inputs, {}, names)
    check_order(lines)
    fns, cats, cells, tests = split_lines(lines)
    fq = f"{sec['namespace']}/{sec['function']}"
    argnames = spec["keycols"] + ["date"]

    # one stored function, typed args, the unit from the table
    assert len(fns) == 1 and fns[0]["function"] == fq
    sig = fns[0]["signature"]
    assert sig["order"] == argnames
    for a in spec["keycols"]:
        assert sig["args"][a] == {"type": "string", "valid": False}
    assert sig["args"]["date"] == {"type": "date", "valid": True}
    assert set(sig["args"]) == set(argnames)
    assert sig["returns"] == {"type": "number", "unit": sec["unit"]}
    assert "bound" not in fns[0], "rule A: the run's clock decides"

    # one cell per row, nothing else in it
    allrows = [(r, src, data) for rows, src, data in inputs for r in rows]
    assert len(cells) == len(allrows)
    published = {}
    for (r, src, data), c in zip(allrows, cells):
        assert set(c) == {"cell", "value", "licence", "summary", "source"}, "no tx/asof: rule A"
        assert c["cell"]["fn"] == fq
        want = {a: {"type": "string", "value": r[a]} for a in spec["keycols"]}
        want["date"] = {"type": "date", "value": r["date"]}
        assert c["cell"]["args"] == want
        assert c["value"] == {"type": "number", "value": canon(r["value"])}  # the published decimal, nothing else
        assert c["licence"] == sec["licence"]
        check_summary(c["summary"])
        for must in (sec["source_name"], "relayed by", "datumesh.com", sec["takeover"], canon(r["value"]), r["date"]):
            assert must in c["summary"], must
        check_source_object(c["source"], host_of(sec["namespace"]), name=sec["source_name"], url=src["url"],
                            sha=sha256(data), takeover=sec["takeover"])
        published[tuple(r[a] for a in spec["keycols"]) + (r["date"],)] = c
    assert json.loads(json.dumps(state)) == state, "the state is plain JSON"
    assert state != {}


@pytest.mark.parametrize("key", SPECS)
def test_catalogue_line(key, names):
    mod, inputs, spec = parsed(key, names)
    sec = names[key]
    lines, _ = emit_all(mod, inputs, {}, names)
    _, cats, _, _ = split_lines(lines)
    fq = f"{sec['namespace']}/{sec['function']}"
    assert len(cats) == 1
    c = cats[0]
    assert set(c) == {"catalog", "kind", "label", "description", "args", "examples", "dictionary"}
    assert c["catalog"] == fq and c["kind"] == "function"
    assert c["label"] and len(c["label"].encode()) <= 120
    assert 0 < len(c["description"].encode()) <= 1024
    assert sec["source_name"] in c["description"] and "datumesh.com" in c["description"]
    argnames = spec["keycols"] + ["date"]
    assert set(c["args"]) == set(argnames) and all(isinstance(t, str) and t for t in c["args"].values())

    # examples: calls with the published value, checked against the parsed file
    by = {rowkey(spec, r): r for rows, _, _ in inputs for r in rows}
    assert len(c["examples"]) >= 2
    for ex in c["examples"]:
        assert set(ex) == {"call", "expected"}
        m = re.match(r"^(.+?)\((.*)\)$", ex["call"])
        assert m and m.group(1) == fq
        vals = {a: (s or d) for a, s, d in re.findall(r'(\w+): (?:"([^"]*)"|(\d{4}-\d\d-\d\d))', m.group(2))}
        assert list(vals) == argnames
        r = by[tuple(vals[a] for a in argnames[:-1]) + (vals["date"],)]
        assert ex["expected"] == {"type": "number", "value": canon(r["value"])}
        assert ex["call"] == call_text(fq, argnames, vals)

    # dictionary: every key argument, labels for every value
    assert set(c["dictionary"]) == set(spec["keycols"])
    for k, entries in c["dictionary"].items():
        assert all(set(e) >= {"value", "label"} and e["label"] for e in entries)
        have = {e["value"]: e["label"] for e in entries}
        assert len(have) == len(entries)
        assert {r[k] for rows, _, _ in inputs for r in rows} <= set(have)
        for v, label in spec["dictionary"][k].items():
            assert have[v] == label
        if spec["dictionary_complete"]:
            assert set(have) == set(spec["dictionary_complete"][k])


@pytest.mark.parametrize("key", SPECS)
def test_second_run_emits_nothing_new(key, names):
    mod, inputs, spec = parsed(key, names)
    first, state = emit_all(mod, inputs, {}, names)
    state = json.loads(json.dumps(state))  # through the state file
    again, state2 = emit_all(mod, inputs, state, names)
    assert again == []
    assert state2 == state


@pytest.mark.parametrize("key", SPECS)
def test_only_dates_after_the_state_are_emitted(key, names):
    mod, inputs, spec = parsed(key, names)
    dates = sorted({r["date"] for rows, _, _ in inputs for r in rows})
    cut = dates[len(dates) // 2 - 1]
    old, state = emit_all(mod, inputs, {}, names, keep=lambda r: r["date"] <= cut)
    state = json.loads(json.dumps(state))
    new, state2 = emit_all(mod, inputs, state, names)
    fns, cats, cells, tests = split_lines(new)
    assert not fns and not cats and not tests, "catalogue lines only the first time"
    allrows = [r for rows, _, _ in inputs for r in rows]
    # policy rates keep the date per rate: a rate's later dates are new even if another rate has them
    expect = [r for r in allrows if r["date"] > cut]
    assert len(cells) == len(expect)
    got = {tuple(c["cell"]["args"][a]["value"] for a in spec["keycols"] + ["date"]) for c in cells}
    assert got == {rowkey(spec, r) for r in expect}
    # and the two runs together are the whole first run
    all_at_once, _ = emit_all(mod, inputs, {}, names)
    _, _, c_all, _ = split_lines(all_at_once)
    _, _, c_old, _ = split_lines(old)
    assert len(c_old) + len(cells) == len(c_all)
    assert emit_all(mod, inputs, state2, names)[0] == []
