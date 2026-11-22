"""CONTRACT-round-0.7 R13: sec.py / sec_class.py emit sec/company in ONE step. The company file holds the class line with its
methods inline (K7: {"stored": sig} and {"formula": src, "signature": sig}, annual_report and loaded kept as the hashes they
have), parent = the relay company class's hash, lookup from_label with key_normalisation label_fold and folded labels (K8), extent
keys = loaded, the steward's process (K12), no `fact` override (K5), and the view bodies reading siblings with `this.`; then the
catalogue (with facets and source_name, R6), the cells and the tests. A company that leaves the top N gets a withdrawal of
loaded(cik); a label no longer given gets a withdrawal of its lookup cell. The four-step path (the class line after the
views, sec_class.py's describe round trip) is gone.

The synthetic world of sec_class_helpers, with two more companies whose short names fold together."""
import hashlib
import json
import re
import shutil

import pytest

import register as licence_register
import relay
import sec
from conftest import FEEDS, PRODUCTION, REHEARSAL
from sec_class_helpers import (Fetcher, LABEL_WORLD, NOW, World, company_file, fold, key_encoding, run_world)
from sec_helpers import FakeClock, args_of, check_order_all, kinds, read_lines
from sec_helpers import names  # noqa: F401
from test_feed_facets import C6, check_bounds, check_source_name

STAMP = relay._stamp(NOW)
S = {"type": "string", "valid": False}
FORMULA_METHODS = ["fact_in", "reported", "latest", "revenue", "cost_of_revenue", "net_income", "operating_income",
                   "total_assets", "current_assets", "current_liabilities", "inventory", "ppe_net", "accounts_payable",
                   "accounts_receivable", "current_ratio", "return_on_assets", "inventory_turnover", "fixed_asset_turnover",
                   "days_payable_outstanding", "cash_conversion_cycle", "gross_margin", "revenue_growth"]
METHODS = ["annual_report", "loaded", "from_label"] + FORMULA_METHODS      # no `fact`: it is inherited (K5)
assert len(METHODS) == 25


def jcs(o):
    return licence_register.jcs(o)


def stored_descriptor(signature):
    return {"code": {"body": "", "lang": "stored"}, "inputs": [], "signature": signature}


ANNUAL_SIG = {"args": {"cik": S, "fiscal_year": {"type": "number", "unit": "1", "valid": False}},
              "order": ["cik", "fiscal_year"], "returns": "string"}
LOADED_SIG = {"args": {"cik": S}, "order": ["cik"], "returns": "boolean"}


@pytest.fixture(scope="module")
def ran(names, tmp_path_factory):
    fc = FakeClock()
    path, f, s, o = run_world(names, tmp_path_factory.mktemp("os"), world=World(cos=LABEL_WORLD), fc=fc)
    cp = company_file(o)
    return {"path": path, "cpath": cp, "s": s, "o": o, "names": names, "lines": read_lines(path),
            "clines": read_lines(cp) if cp.exists() else []}


def sub(names):
    return f"{names['meta']['host']}/sec/company"


def class_line(ran):
    cs = [l for l in ran["clines"] if "class" in l]
    assert len(cs) == 1, "exactly one class line, in the company file"
    return cs[0]


def label_cells(lines, names):
    return [l for l in lines if "cell" in l and l["cell"]["fn"] == f"{sub(names)}/from_label" and "value" in l]


# ---------- one file, one step ----------

def test_the_company_file_starts_with_the_one_class_line_and_has_no_function_or_descriptor_lines(names, ran):
    k = kinds(ran["clines"])
    assert len(k["class"]) == 1 and k["function"] == [] and k["descriptor"] == []
    assert ran["clines"][0] is k["class"][0] or ran["clines"][0] == k["class"][0], "the class line is the first line"
    check_order_all(ran["clines"])
    assert class_line(ran)["class"] == sub(names)


def test_the_relay_file_has_no_sec_company_line_and_the_company_file_says_nothing_of_a_describe(names, ran):
    own = sub(names) + "/"
    for l in ran["lines"]:
        name = l.get("class") or l.get("catalog") or l.get("test") or (l["cell"]["fn"] if "cell" in l else "")
        assert not name.startswith(sub(names)), name


def test_sec_class_py_no_longer_asks_a_mesh_for_hashes():
    p = FEEDS / "sec_class.py"
    if not p.exists():
        return
    text = p.read_text(encoding="utf-8")
    assert "urllib.request" not in text and "/describe/" not in text and "def describe" not in text, "the describe round trip goes"
    assert "--mesh" not in text


def test_the_views_file_has_no_class_method_formula_and_no_fact_override_line():
    lines = [json.loads(l) for l in (FEEDS / "sec_views.jsonl").read_text(encoding="utf-8").splitlines() if l.strip()]
    own = f"{REHEARSAL}/sec/company/"
    assert not [l for l in lines if l.get("formula", "").startswith(own)], "the methods are inline in the class line"
    assert not [l for l in lines if own + "fact" in (l.get("formula"), l.get("catalog"), l.get("test"))], "no `fact` override (K5)"
    assert {l["formula"] for l in lines if "formula" in l} == {
        f"{REHEARSAL}/{g}/{n}" for g, ns in (("ratio", ("margin", "percent_change", "cagr", "percent_of_total")),
                                              ("calendar", ("age_at", "days_between"))) for n in ns}


# ---------- the class line ----------

def relay_company_class_hash(ran, names):
    (cl,) = [l for l in ran["lines"] if l.get("class") == f"{names['sec']['namespace']}/company"]
    return hashlib.sha256(jcs(cl["descriptor"])).hexdigest()


def test_the_descriptor_has_the_parent_the_lookup_the_extent_and_the_steward(names, ran):
    d = class_line(ran)["descriptor"]
    assert set(d) == {"keys", "order", "parent", "methods", "lookup", "extent", "steward"}
    assert d["keys"] == {} and d["order"] == [], "the key cik is the parent's"
    assert d["parent"] == relay_company_class_hash(ran, names), "the relay company class's hash, taken from the relay file"
    assert d["lookup"] == "from_label"
    assert d["extent"] == {"keys": hashlib.sha256(jcs(stored_descriptor(LOADED_SIG))).hexdigest()}, "keys = loaded"
    assert d["steward"] == {"namespace": f"{names['meta']['host']}/sec", "process": f"https://{names['meta']['host']}/classes/change"}
    assert "fact" not in d["methods"], "fact is inherited from the relay class, not overridden (K5)"
    assert sorted(d["methods"]) == sorted(METHODS)


def test_the_stored_methods_are_inline_or_keep_their_hashes(names, ran):
    m = class_line(ran)["descriptor"]["methods"]
    assert m["annual_report"] == hashlib.sha256(jcs(stored_descriptor(ANNUAL_SIG))).hexdigest() == (
        "e4e8ef0df1db260e9f0f3bf25dc4bbe0d3c828cc5fca5fda618352d27b772af5"), "annual_report keeps its hash"
    assert m["loaded"] == hashlib.sha256(jcs(stored_descriptor(LOADED_SIG))).hexdigest() == (
        "e8feac09cf0cc71b6c149b498f1f51fcc1776d9b40f775464fadf8f2ae70e8fc"), "loaded keeps its hash"
    assert set(m["from_label"]) == {"stored"}
    sig = m["from_label"]["stored"]
    assert sig == {"args": {"label": {"type": "string", "valid": False, "key_normalisation": "label_fold"}},
                   "order": ["label"], "returns": {"type": "string", "classes": [sub(names)]}}


def test_the_views_are_inline_formula_methods_with_a_signature_and_a_source(names, ran):
    m = class_line(ran)["descriptor"]["methods"]
    for n in FORMULA_METHODS:
        v = m[n]
        assert isinstance(v, dict) and set(v) == {"formula", "signature"}, n
        assert isinstance(v["formula"], str) and v["formula"].strip(), n
        assert v["signature"]["order"][0] == "cik" and "cik" in v["signature"]["args"], n


def test_the_bodies_read_their_siblings_with_this_and_no_full_name_of_the_class_or_its_parent(names, ran):
    host = names["meta"]["host"]
    m = class_line(ran)["descriptor"]["methods"]
    for n in FORMULA_METHODS:
        src = m[n]["formula"]
        assert f"{host}/sec/company/" not in src, (n, "a sibling is read with this")
        assert f"{host}/relay/sec.gov/company/" not in src, (n, "the inherited fact is this.fact")
    assert "this.reported(" in m["revenue"]["formula"]
    assert "this.fact_in(" in m["reported"]["formula"] and "this.annual_report(" in m["reported"]["formula"]
    assert "this.fact(" in m["fact_in"]["formula"] and "this.fact(" in m["latest"]["formula"]
    assert "this.revenue(" in m["gross_margin"]["formula"] or "this.cost_of_revenue(" in m["gross_margin"]["formula"]
    assert f"{host}/relay/sec.gov/filing/period_end(" in m["reported"]["formula"], "the filing class is read by its name"


def test_a_formula_method_reads_only_methods_the_class_defines(names, ran):
    m = class_line(ran)["descriptor"]["methods"]
    known = set(m) | {"fact"}
    for n in FORMULA_METHODS:
        for callee in re.findall(r"this\.(\w+)\(", m[n]["formula"]):
            assert callee in known, (n, callee)
            assert callee != n, (n, "no method reads itself")


def test_the_class_line_has_a_steward_page_for_production_and_for_the_rehearsal_host():
    s = dict(sec.sec_table(REHEARSAL)["sec"])
    for host, want in [(PRODUCTION, "https://datumesh.com/classes/change"), (REHEARSAL, f"https://{REHEARSAL}/classes/change"),
                       ("scratch.invalid", "https://scratch.invalid/classes/change")]:
        t = dict(s, namespace=f"{host}/relay/sec.gov")
        line = sec.company_class_line(t, "0" * 64)
        assert line["class"] == f"{host}/sec/company"
        assert line["descriptor"]["steward"] == {"namespace": f"{host}/sec", "process": want}, host
        assert line["descriptor"]["parent"] == "0" * 64


# ---------- the lookup: folded labels ----------

def test_every_label_cell_is_folded_nfc_and_the_value_is_the_key_encoding(names, ran):
    cs = label_cells(ran["clines"], names)
    assert cs
    for c in cs:
        lab = args_of(c)["label"]
        assert lab == fold(lab) and lab, lab
        assert re.fullmatch(re.escape(names["meta"]["host"]) + r'/sec/company\(cik:"\d{10}"\)', c["value"]["value"])
    got = {args_of(c)["label"]: c["value"]["value"] for c in cs}
    host = names["meta"]["host"]
    assert got["apple inc."] == f'{host}/sec/company(cik:"0000320193")'
    assert got["aapl"] == got["apple inc."] == got["apple"]
    assert "Apple Inc." not in got and "AAPL" not in got, "an unfolded label would be refused (E_KEY_NORMALISATION)"
    assert got["jpmorgan chase & co"].endswith('"0000019617")') and got["jpm-pc"].endswith('"0000019617")')


def test_labels_that_fold_together_onto_two_companies_are_dropped_and_listed(names, ran):
    got = {args_of(c)["label"] for c in label_cells(ran["clines"], names)}
    for l in ("zebra", "delta", "orion", "sun"):
        assert l not in got, l
    assert "zebra corp" in got and "zebra inc" in got and "zebx" in got and "zeb" in got
    rep = json.loads((ran["o"] / f"sec-scope-{STAMP}.json").read_text(encoding="utf-8"))
    amb = rep["left_out"]["ambiguous_labels"]
    assert amb["zebra"] == ["0001000060", "0001000061"], "listed by its folded label, with its companies"
    assert amb["delta"] == ["0001000040", "0001000041"] and amb["orion"] == ["0001000042", "0001000043"]
    assert amb["sun"] == ["0001000044", "0001000045", "0001000046"], "SUN, SUN Inc and Sun Corp fold together"
    assert set(amb) == {"zebra", "delta", "orion", "sun"}


def test_a_company_with_the_same_folded_label_twice_has_it_once(names, ran):
    labs = [args_of(c)["label"] for c in label_cells(ran["clines"], names)]
    assert len(labs) == len(set(labs))


def test_label_tests_and_examples_use_folded_labels_only(names, ran):
    calls = [t["call"] for t in ran["clines"] if t.get("test") == f"{sub(names)}/from_label"]
    assert calls
    for c in calls:
        label = re.search(r'label: "((?:[^"\\]|\\.)*)"', c).group(1)
        assert label == fold(label), c
    (cat,) = [l for l in ran["clines"] if l.get("catalog") == f"{sub(names)}/from_label"]
    for x in cat["examples"]:
        label = re.search(r'label: "((?:[^"\\]|\\.)*)"', x["call"]).group(1)
        assert label == fold(label), x
    d = cat["description"].lower()
    assert "case-sensitive" not in d and "exact and case" not in d, "labels ignore case and extra spaces now"
    assert "case" in d and "space" in d


# ---------- the catalogue of the class ----------

def test_the_class_catalogue_entry_has_facets_and_source_name(names, ran):
    (c,) = [l for l in ran["clines"] if l.get("catalog") == sub(names)]
    assert c["kind"] == "class"
    assert c["facets"] == C6["sec/company"]
    check_bounds(c["facets"])
    assert c["source_name"] == "SEC filings"
    check_source_name(c["source_name"])
    assert 0 < len(c["description"].encode()) <= 1024 and len(c["label"].encode()) <= 120


def test_the_other_catalogue_lines_are_for_the_methods_and_name_no_facets(names, ran):
    cats = [l for l in ran["clines"] if "catalog" in l and l["catalog"] != sub(names)]
    assert {c["catalog"] for c in cats} >= {f"{sub(names)}/{m}" for m in ("annual_report", "loaded", "from_label")}
    assert all("facets" not in c for c in cats)


# ---------- extent: a company that leaves the top N, a label no longer given ----------

def second_run(names, ran, tmp_path, world, now):
    s, o = tmp_path / "state", tmp_path / "out"
    shutil.copytree(ran["s"], s), shutil.copytree(ran["o"], o)
    fc = FakeClock()
    p = sec.run(names, fetch=Fetcher(names, world, fc.clock), now=now, state_dir=s, out_dir=o, sleep=fc.sleep, clock=fc.clock)
    return p, s, o


def test_a_company_that_leaves_the_top_n_is_withdrawn_from_loaded_and_its_labels_from_the_lookup(names, ran, tmp_path):
    gone = next(c for c in LABEL_WORLD if c.title == "Differing Corp")
    later = "2026-11-21T12:00:00.000000Z"
    p, s, o = second_run(names, ran, tmp_path, World(cos=[c for c in LABEL_WORLD if c is not gone]), later)
    cp = company_file(o, later)
    assert cp.exists(), "the withdrawals are lines of a company file"
    lines = read_lines(cp)
    check_order_all(lines)
    assert not [l for l in lines if "class" in l], "the class line is written once"
    withdrawn = [l for l in lines if "withdrawn" in l]
    loaded = [l for l in withdrawn if l["cell"]["fn"] == f"{sub(names)}/loaded"]
    assert loaded == [{"cell": {"fn": f"{sub(names)}/loaded", "args": {"cik": {"type": "string", "value": gone.cik10}}},
                       "withdrawn": "the company left the top N"}]
    labs = sorted(l["cell"]["args"]["label"]["value"] for l in withdrawn if l["cell"]["fn"] == f"{sub(names)}/from_label")
    assert labs == sorted(["differing corp", "diffr", "differing"])
    for l in withdrawn:
        assert set(l) == {"cell", "withdrawn"} and isinstance(l["withdrawn"], str) and l["withdrawn"].strip()
    assert all(l["cell"]["fn"] in (f"{sub(names)}/loaded", f"{sub(names)}/from_label") for l in withdrawn), (
        "nothing else of the company is withdrawn here")


def test_a_withdrawal_is_written_once(names, ran, tmp_path):
    gone = next(c for c in LABEL_WORLD if c.title == "Differing Corp")
    world = World(cos=[c for c in LABEL_WORLD if c is not gone])
    later, third = "2026-11-21T12:00:00.000000Z", "2026-11-22T12:00:00.000000Z"
    p, s, o = second_run(names, ran, tmp_path, world, later)
    fc = FakeClock()
    sec.run(names, fetch=Fetcher(names, world, fc.clock), now=third, state_dir=s, out_dir=o, sleep=fc.sleep, clock=fc.clock)
    cp = company_file(o, third)
    assert not cp.exists() or not [l for l in read_lines(cp) if "withdrawn" in l]


def test_nothing_is_withdrawn_when_every_company_and_label_is_still_given(names, ran, tmp_path):
    later = "2026-11-21T12:00:00.000000Z"
    p, s, o = second_run(names, ran, tmp_path, World(cos=LABEL_WORLD), later)
    cp = company_file(o, later)
    assert not cp.exists() or not [l for l in read_lines(cp) if "withdrawn" in l]


def test_a_label_no_longer_given_is_withdrawn_while_the_company_stays(names, ran, tmp_path):
    """The ticker ZEB goes from Zebra Corp's listing: its lookup cell is withdrawn; the company and its other labels stay."""
    zebra = next(c for c in LABEL_WORLD if c.title == "Zebra Corp")
    world = World(cos=LABEL_WORLD)
    data = json.loads(world.tickers)
    data["data"] = [r for r in data["data"] if not (r[0] == zebra.cik and r[2] == "ZEB")]
    world.tickers = json.dumps(data, ensure_ascii=False).encode("utf-8")
    later = "2026-11-21T12:00:00.000000Z"
    p, s, o = second_run(names, ran, tmp_path, world, later)
    cp = company_file(o, later)
    assert cp.exists()
    withdrawn = [l for l in read_lines(cp) if "withdrawn" in l]
    assert [l["cell"] for l in withdrawn] == [
        {"fn": f"{sub(names)}/from_label", "args": {"label": {"type": "string", "value": "zeb"}}}]
