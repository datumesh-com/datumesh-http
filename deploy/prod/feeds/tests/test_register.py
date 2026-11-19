"""The licence register (answer 3): the class `<host>/register/dataset(source, code)` and its eight entries."""
import copy
import hashlib
import json
import unicodedata

import pytest

import common
import register as reg
from common import ShapeError
from conftest import PRODUCTION, REHEARSAL, fixture
from helpers import HEX64, INSTANT, jcs

METHODS = ["name", "status", "licence_page", "licence_hash", "licence_clauses", "retrieved"]
NOW = "2026-11-19T12:00:00.000000Z"


def entries():
    return reg.parse(fixture("register.json"))


def kinds(lines):
    out = []
    for l in lines:
        assert isinstance(l, dict) and "\n" not in json.dumps(l, ensure_ascii=False)
        if "class" in l:
            out.append("class")
        else:
            (k,) = [k for k in ("descriptor", "catalog", "cell") if k in l]
            out.append(k)
    return out


def split(lines):
    ks = kinds(lines)
    rank = {"descriptor": 0, "class": 1, "catalog": 2, "cell": 3}
    assert [rank[k] for k in ks] == sorted(rank[k] for k in ks), ks
    return {k: [l for l, kk in zip(lines, ks) if kk == k] for k in rank}


def cells_of(lines):
    return split(lines)["cell"]


def test_parse_keeps_every_entry_with_its_verdict():
    rows = entries()
    assert sorted(r["source_domain"] for r in rows) == sorted(
        ["ecb.europa.eu/fx", "ecb.europa.eu/estr", "ecb.europa.eu/policy-rates", "chc.ucsb.edu/chirps",
         "home.treasury.gov", "bankofengland.co.uk", "worldbank.org/pink-sheet", "fao.org/giews/asis"])
    assert {r["verdict"] for r in rows} == {"RELAY", "SKIP"}


def test_parse_refuses_a_changed_shape():
    data = fixture("register.json")
    for bad in (data.replace(b'"sha256"', b'"sha"'), data.replace(b'"retrieved"', b'"when"'), b"{}", b"", b"[1]",
                data.replace(b"5c467e3b05d8e84a", b"zz"), data.replace(b'"SKIP"', b'"MAYBE"'),
                data.replace(b'"conditions"', b'"terms"')):
        with pytest.raises(ShapeError):
            reg.parse(bad)


def test_first_run_class_descriptors_catalogue_and_cells(names):
    sec = names["register"]
    host = REHEARSAL
    cls = f"{host}/register/dataset"
    lines, state = reg.emit(entries(), {}, names)
    parts = split(lines)

    # the six stored methods, as descriptor lines
    assert len(parts["descriptor"]) == 6
    keyargs = {"source": {"type": "string", "valid": False}, "code": {"type": "string", "valid": False}}
    descs = {}
    for m, line in zip(METHODS, parts["descriptor"]):
        assert set(line) == {"descriptor"}
        d = line["descriptor"]
        returns = "instant" if m == "retrieved" else "string"
        assert d == {"code": {"body": "", "lang": "stored"}, "inputs": [],
                     "signature": {"args": keyargs, "order": ["source", "code"], "returns": returns}}
        descs[m] = hashlib.sha256(jcs(d)).hexdigest()

    # the class: keys source then code, a steward, the methods by hash
    (cl,) = parts["class"]
    assert set(cl) == {"class", "descriptor"} and cl["class"] == cls
    none = {"type": "string", "key_normalisation": "none"}
    assert cl["descriptor"] == {
        "keys": {"source": none, "code": none}, "order": ["source", "code"], "methods": descs,
        "steward": {"namespace": f"{host}/register", "process": f"https://{host}/classes/change"}}
    assert list(cl["descriptor"]["keys"]) == ["source", "code"] and "retrieved" not in cl["descriptor"]["keys"]

    # the catalogue: the class (with dictionaries) and one entry per method
    cats = {c["catalog"]: c for c in parts["catalog"]}
    assert set(cats) == {cls} | {f"{cls}/{m}" for m in METHODS}
    c = cats[cls]
    assert c["kind"] == "class" and c["label"] and 0 < len(c["description"].encode()) <= 1024
    assert len(c["label"].encode()) <= 120
    assert [e["value"] for e in c["dictionary"]["source"]] == [
        "ecb.europa.eu", "treasury.gov", "bankofengland.co.uk", "chc.ucsb.edu", "worldbank.org", "fao.org"]
    assert [e["value"] for e in c["dictionary"]["code"]] == [d["code"] for d in sec["datasets"]]
    assert all(e["label"] for k in c["dictionary"] for e in c["dictionary"][k])
    for m in METHODS:
        e = cats[f"{cls}/{m}"]
        assert e["kind"] == "function" and e["label"] and e["description"] and set(e["args"]) == {"source", "code"}
        assert len(e["description"].encode()) <= 1024

    # 8 entries x 6 methods
    cells = parts["cell"]
    assert len(cells) == 48
    by = {}
    for cell in cells:
        assert set(cell) == {"cell", "value", "licence", "summary"}, "no source member: these are our own statements"
        fn = cell["cell"]["fn"]
        assert fn.startswith(cls + "/") and fn[len(cls) + 1:] in METHODS
        assert list(cell["cell"]["args"]) == ["source", "code"] and "retrieved" not in cell["cell"]["args"]
        args = cell["cell"]["args"]
        assert args["source"]["type"] == args["code"]["type"] == "string"
        assert cell["licence"] == sec["licence"]
        s = cell["summary"]
        assert s and "\n" not in s and len(s.encode()) <= 1024 and unicodedata.is_normalized("NFC", s)
        assert args["source"]["value"] in s and args["code"]["value"] in s
        by[(args["source"]["value"], args["code"]["value"], fn[len(cls) + 1:])] = cell
    assert len(by) == 48

    inputs = {r["source_domain"]: r for r in entries()}
    for d in sec["datasets"]:
        r = inputs[d["input"]]
        v = lambda m: by[(d["source"], d["code"], m)]["value"]
        assert v("name") == {"type": "string", "value": d["name"]} and d["name"]
        assert v("status") == {"type": "string", "value": d["status"]}
        assert v("licence_page") == {"type": "string", "value": r["licence_url"]}
        assert v("licence_hash") == {"type": "string", "value": r["sha256"]} and HEX64.match(r["sha256"])
        assert v("retrieved") == {"type": "instant", "value": r["retrieved"]} and INSTANT.match(r["retrieved"])
        clauses = v("licence_clauses")
        assert clauses["type"] == "string"
        for cond in r["conditions"]:
            assert cond in clauses["value"], (d["code"], cond)
        if r["attribution"]:
            assert r["attribution"] in clauses["value"]
        if d["status"] == "not_relayed":
            assert d["reason"] in clauses["value"] and r["verdict"] == "SKIP"
        else:
            assert r["verdict"] == "RELAY" and "reason" not in d

    # statuses of the eight, as in the S4 table
    status = {(d["source"], d["code"]): by[(d["source"], d["code"], "status")]["value"]["value"] for d in sec["datasets"]}
    assert status == {("ecb.europa.eu", "reference_rates"): "relayed", ("ecb.europa.eu", "estr"): "relayed",
                      ("ecb.europa.eu", "policy_rates"): "relayed", ("treasury.gov", "par_yield_curve"): "relayed",
                      ("bankofengland.co.uk", "bank_rate"): "relayed", ("chc.ucsb.edu", "chirps"): "computed_from",
                      ("worldbank.org", "commodity_prices"): "not_relayed", ("fao.org", "asis"): "not_relayed"}
    # the reasons are those of LICENCES.md
    wb = by[("worldbank.org", "commodity_prices", "licence_clauses")]["value"]["value"]
    fao = by[("fao.org", "asis", "licence_clauses")]["value"]["value"]
    assert "not clear enough" in wb.lower() or "does not state a licence" in wb.lower()
    assert "non-commercial" in fao.lower()
    assert state


def test_production_host_names_the_production_class():
    prod = common.names(PRODUCTION)
    lines, _ = reg.emit(entries(), {}, prod)
    parts = split(lines)
    assert parts["class"][0]["class"] == "datumesh.com/register/dataset"
    assert all(c["cell"]["fn"].startswith("datumesh.com/register/dataset/") for c in parts["cell"])
    assert "mesh-test" not in json.dumps(lines)


def test_a_rerun_with_the_same_licence_text_emits_nothing(names):
    _, state = reg.emit(entries(), {}, names)
    state = json.loads(json.dumps(state))
    assert reg.emit(entries(), state, names) == ([], state)
    # a new check that finds the same text is not a change
    later = copy.deepcopy(entries())
    for r in later:
        r["retrieved"] = "2026-11-20T05:00:00.000000Z"
    assert reg.emit(later, state, names)[0] == []


def test_a_changed_text_emits_new_versions_of_that_entry_only(names):
    cls = f"{REHEARSAL}/register/dataset"
    _, state = reg.emit(entries(), {}, names)
    state = json.loads(json.dumps(state))
    changed = copy.deepcopy(entries())
    (e,) = [r for r in changed if r["source_domain"] == "ecb.europa.eu/estr"]
    e["sha256"] = "ab" * 32
    e["retrieved"] = "2026-12-01T05:00:00.000000Z"
    e["conditions"] = e["conditions"] + ["A new clause."]
    lines, state2 = reg.emit(changed, state, names)
    assert kinds(lines) == ["cell"] * 6, "no descriptors, class or catalogue again"
    cells = cells_of(lines)
    assert {c["cell"]["fn"] for c in cells} == {f"{cls}/{m}" for m in METHODS}
    assert all(c["cell"]["args"] == {"source": {"type": "string", "value": "ecb.europa.eu"},
                                     "code": {"type": "string", "value": "estr"}} for c in cells)
    v = {c["cell"]["fn"].rsplit("/", 1)[1]: c["value"] for c in cells}
    assert v["licence_hash"] == {"type": "string", "value": "ab" * 32}
    assert v["retrieved"] == {"type": "instant", "value": "2026-12-01T05:00:00.000000Z"}
    assert "A new clause." in v["licence_clauses"]["value"]
    # and then it is stable again
    assert reg.emit(changed, json.loads(json.dumps(state2)), names)[0] == []


def test_a_shared_licence_page_does_not_tie_entries_together(names):
    """reference_rates and policy_rates have the same page text; a change in one input changes only its entry."""
    _, state = reg.emit(entries(), {}, names)
    changed = copy.deepcopy(entries())
    (e,) = [r for r in changed if r["source_domain"] == "ecb.europa.eu/fx"]
    e["sha256"] = "cd" * 32
    lines, _ = reg.emit(changed, state, names)
    codes = {c["cell"]["args"]["code"]["value"] for c in cells_of(lines)}
    assert codes == {"reference_rates"}


def test_run_writes_one_file_then_nothing(names, tmp_path):
    state_dir, out_dir = tmp_path / "s", tmp_path / "o"
    state_dir.mkdir(), out_dir.mkdir()
    path = tmp_path / "register.json"
    path.write_bytes(fixture("register.json"))
    out = reg.run(names, path=path, now=NOW, state_dir=state_dir, out_dir=out_dir)
    lines = [json.loads(l) for l in out.read_text(encoding="utf-8").splitlines()]
    assert len(lines) == 6 + 1 + 7 + 48
    assert reg.run(names, path=path, now=NOW, state_dir=state_dir, out_dir=out_dir) is None


def test_a_failed_publish_keeps_no_state(names, tmp_path):
    state_dir, out_dir = tmp_path / "s", tmp_path / "o"
    state_dir.mkdir(), out_dir.mkdir()
    path = tmp_path / "register.json"
    path.write_bytes(fixture("register.json"))

    def boom(_):
        raise RuntimeError("publish failed")
    with pytest.raises(RuntimeError):
        reg.run(names, path=path, now=NOW, state_dir=state_dir, out_dir=out_dir, publish=boom)
    assert list(out_dir.iterdir()) == [] and list(state_dir.iterdir()) == []
