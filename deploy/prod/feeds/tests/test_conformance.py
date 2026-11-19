"""F2: source-conformance tests built from a second, independent copy (ECB SDMX vs daily XML; Treasury CSV vs XML)."""
import importlib
import json
import re

import pytest

from conftest import fixture
from helpers import RETRIEVED, call_text, canon, check_order, sha256, split_lines
from specs import SPECS

CASES = {
    "ecb_fx": dict(parse2="parse_sdmx", file2="ecb_sdmx_20261118.csv", n=29,
                   mutations=[(b"OBS_VALUE", b"OBS_VAL"), (b"CURRENCY_DENOM", b"CCY_DENOM"), (b"1.6294", b"abc")]),
    "treasury": dict(parse2="parse_csv", file2="treasury_202611.csv", n=168,
                     mutations=[(b'"10 Yr"', b'"10 Year"'), (b"4.10", b"abc"), (b"Date,", b"Day,")]),
}


def setup(key, names):
    mod, spec = importlib.import_module(key), SPECS[key]
    (fname, url), = spec["inputs"](names)
    data = fixture(fname)
    rows = mod.parse(data)
    rows2 = getattr(mod, CASES[key]["parse2"])(fixture(CASES[key]["file2"]))
    src = {"url": url, "retrieved": RETRIEVED, "sha256": sha256(data)}
    return mod, spec, rows, rows2, src


@pytest.mark.parametrize("key", CASES)
def test_second_copy_parses_to_the_same_rows(key, names):
    mod, spec, rows, rows2, _ = setup(key, names)
    assert len(rows2) == CASES[key]["n"]
    k = lambda r: tuple(r[c] for c in spec["keycols"]) + (r["date"],)
    a = {k(r): canon(r["value"]) for r in rows}
    b = {k(r): canon(r["value"]) for r in rows2}
    assert a == b, "the two copies agree on every value (as numbers; trailing zeros differ)"


@pytest.mark.parametrize("key", CASES)
def test_second_copy_changed_shape_refused(key, names):
    from common import ShapeError
    mod = importlib.import_module(key)
    data = fixture(CASES[key]["file2"])
    for old, new in CASES[key]["mutations"]:
        assert old in data
        with pytest.raises(ShapeError):
            getattr(mod, CASES[key]["parse2"])(data.replace(old, new))


@pytest.mark.parametrize("key", CASES)
def test_conformance_lines(key, names):
    mod, spec, rows, rows2, _ = setup(key, names)
    sec = names[key]
    fq = f"{sec['namespace']}/{sec['function']}"
    argnames = spec["keycols"] + ["date"]
    tests = mod.conformance(rows, rows2, names)
    assert len(tests) == CASES[key]["n"]
    k = lambda r: tuple(r[c] for c in spec["keycols"]) + (r["date"],)
    want = {k(r): canon(r["value"]) for r in rows2}
    seen = set()
    for t in tests:
        assert set(t) == {"test", "call", "expect"}
        assert t["test"] == fq
        vals = {a: (s or d) for a, s, d in re.findall(r'(\w+): (?:"([^"]*)"|(\d{4}-\d\d-\d\d))', t["call"])}
        assert t["call"] == call_text(fq, argnames, vals)
        key = tuple(vals[a] for a in argnames)
        assert t["expect"] == {"type": "number", "value": want[key]}
        seen.add(key)
    assert seen == set(want)


@pytest.mark.parametrize("key", CASES)
def test_disagreeing_copies_are_refused(key, names):
    from common import ConformanceError
    mod, spec, rows, rows2, _ = setup(key, names)
    bad = [dict(r) for r in rows2]
    bad[0]["value"] = "99999.1"
    with pytest.raises(ConformanceError):
        mod.conformance(rows, bad, names)
    # a copy that has other keys or dates than the first is not a disagreement
    extra = rows2 + [dict(rows2[0], date="1999-01-01")]
    assert len(mod.conformance(rows, extra, names)) == CASES[key]["n"]


@pytest.mark.parametrize("key", CASES)
def test_emit_carries_tests_first_run_only(key, names):
    mod, spec, rows, rows2, src = setup(key, names)
    lines, state = mod.emit(rows, {}, names, src, second_rows=rows2)
    check_order(lines)
    fns, cats, cells, tests = split_lines(lines)
    assert len(tests) == CASES[key]["n"] and len(cells) == len(rows)
    assert tests == mod.conformance(rows, rows2, names)
    # without a second copy nothing is claimed
    l2, _ = mod.emit(rows, {}, names, src)
    assert split_lines(l2)[3] == []
    # second run: nothing at all
    assert mod.emit(rows, json.loads(json.dumps(state)), names, src, second_rows=rows2)[0] == []
