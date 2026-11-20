"""sec.emit: the cells, filing cells, filing_for cells, source, summaries, catalogue and dictionaries (K2, K6, D36).
The cell value is the figure as filed; nothing is normalised under the relay name."""
import copy
import json

import pytest

import sec
from helpers import (HEX64, check_order, check_source_object, check_summary, host_of, sha256, split_lines)
from sec_helpers import (CIK_CAT, CIK_GE, CIK_JPM, CIKS, IN_SCOPE, NOW, cf_url, facts_file, item, source_of)

FILING_FIELDS = ["cik", "form", "filed", "fy", "fp", "period_end"]


def items(names):
    return [item(names, c) for c in CIKS]


@pytest.fixture(scope="module")
def first(names):
    its = items(names)
    lines, state = sec.emit(its, {}, names)
    return its, lines, state


def fqs(names):
    s = names["sec"]
    ns = s["namespace"]
    return f"{ns}/{s['function']}", f"{ns}/{s['filing_function']}", f"{ns}/{s['filing_for_function']}"


def args_of(cell):
    return {k: v["value"] for k, v in cell["cell"]["args"].items()}


def call(fq, argnames, vals):
    return f"{fq}({', '.join(f'{a}: {chr(34)}{vals[a]}{chr(34)}' for a in argnames)})"


# ---------- the names ----------

def test_names_are_the_contract_on_the_rehearsal_host(names):
    s = names["sec"]
    assert s["namespace"] == "mesh-test.datumesh.com/relay/sec.gov"
    assert (s["function"], s["filing_function"], s["filing_for_function"]) == ("company_fact", "filing", "filing_for")
    assert s["source_name"] == "sec.gov" and s["takeover"] == "https://mesh-test.datumesh.com/takeover/sec.gov"
    assert s["licence"] == "https://www.sec.gov/about/privacy-information"
    assert s["companyfacts_url"] == "https://data.sec.gov/api/xbrl/companyfacts/CIK{cik}.json"
    assert s["tickers_url"] == "https://www.sec.gov/files/company_tickers_exchange.json"
    assert s["frames_url"] == "https://data.sec.gov/api/xbrl/frames/{taxonomy}/{concept}/{unit}/{ccp}.json"


def test_the_register_section_for_sec_is_named_in_the_table(names):
    assert names["sec"]["register_source"] == "sec.gov" and names["sec"]["register_code"] == "company_facts"


# ---------- lines of the first run ----------

def test_first_run_lines_shape_and_order(names, first):
    _, lines, _ = first
    funcs, cats, cells, tests = split_lines(lines)
    check_order(lines)
    assert len(funcs) == 3 and len(cats) == 3 and tests == []
    fq, ffq, gfq = fqs(names)
    by = {l["function"]: l["signature"] for l in funcs}
    assert by[fq] == {"args": {k: {"type": "string", "valid": False} for k in
                               ("cik", "taxonomy", "concept", "unit", "accession", "period")},
                      "order": ["cik", "taxonomy", "concept", "unit", "accession", "period"],
                      "returns": {"type": "number"}}
    assert by[ffq] == {"args": {"accession": {"type": "string", "valid": False}, "field": {"type": "string", "valid": False}},
                       "order": ["accession", "field"], "returns": {"type": "string"}}
    assert by[gfq] == {"args": {"cik": {"type": "string", "valid": False}, "fiscal_year": {"type": "string", "valid": False}},
                       "order": ["cik", "fiscal_year"], "returns": {"type": "string"}}
    assert [c["catalog"] for c in cats] == [fq, ffq, gfq]


def test_cell_counts(names, first):
    _, lines, _ = first
    fq, ffq, gfq = fqs(names)
    cells = [l for l in lines if "cell" in l]
    n = lambda f: sum(1 for c in cells if c["cell"]["fn"] == f)
    assert n(fq) == sum(IN_SCOPE.values()) == 1488
    assert n(ffq) == 18 * 6 and n(gfq) == 18
    assert len(cells) == 1488 + 108 + 18


def test_company_fact_cell_is_the_figure_as_filed(names, first):
    _, lines, _ = first
    fq = fqs(names)[0]
    cells = {tuple(args_of(c)[k] for k in ("cik", "taxonomy", "concept", "unit", "accession", "period")): c
             for c in lines if "cell" in c and c["cell"]["fn"] == fq}
    assert len(cells) == 1488, "keys are unique"
    ge = cells[(CIK_GE, "us-gaap", "Assets", "USD", "0000040545-23-000023", "2022-12-31")]
    assert ge["value"] == {"type": "number", "value": "187788000000"}
    ge2 = cells[(CIK_GE, "us-gaap", "Assets", "USD", "0000040545-24-000027", "2022-12-31")]
    assert ge2["value"] == {"type": "number", "value": "188851000000"}
    assert all(set(c["cell"]) == {"fn", "args"} and all(a["type"] == "string" for a in c["cell"]["args"].values())
               for c in cells.values())
    dei = [k for k in cells if k[1] == "dei"]
    assert dei and all(k[2] == "EntityCommonStockSharesOutstanding" and "/" not in k[5] for k in dei)
    eps = [k for k in cells if k[3] == "USD/shares"]
    assert eps


def test_every_cell_carries_licence_source_and_summary(names, first):
    its, lines, _ = first
    s = names["sec"]
    url_sha = {cf_url(names, i["parsed"]["cik"]): sha256(facts_file(i["parsed"]["cik"])) for i in its}
    cells = [l for l in lines if "cell" in l]
    for c in cells:
        assert c["licence"] == s["licence"] == "https://www.sec.gov/about/privacy-information"
        src = c["source"]
        check_source_object(src, host_of(s["namespace"]), name="sec.gov", url=src["url"], sha=src["sha256"],
                            takeover="https://mesh-test.datumesh.com/takeover/sec.gov")
        assert src["url"] in url_sha and src["sha256"] == url_sha[src["url"]] and HEX64.match(src["sha256"])
        assert src["retrieved"] == NOW
        check_summary(c["summary"])
        assert "relayed by datumesh.com" in c["summary"] and s["takeover"] in c["summary"] and "sec.gov" in c["summary"]


def test_a_cells_source_is_the_file_of_its_company(names, first):
    _, lines, _ = first
    for c in lines:
        if "cell" in c and "cik" in c["cell"]["args"]:
            assert c["source"]["url"] == cf_url(names, c["cell"]["args"]["cik"]["value"])


def test_company_fact_summary_names_company_form_filing_date_fy_and_period_as_filed(names, first):
    _, lines, _ = first
    fq = fqs(names)[0]
    c = next(l for l in lines if "cell" in l and l["cell"]["fn"] == fq and
             args_of(l)["accession"] == "0000040545-24-000027" and args_of(l)["concept"] == "Assets"
             and args_of(l)["period"] == "2022-12-31")
    t = c["summary"]
    for part in ("GENERAL ELECTRIC COMPANY", "10-K", "2024-02-02", "fiscal year 2023", "2022-12-31", "Assets",
                 "188851000000", "0000040545-24-000027"):
        assert part in t, part
    d = next(l for l in lines if "cell" in l and l["cell"]["fn"] == fq and args_of(l)["period"] == "2023-01-01/2023-12-31"
             and args_of(l)["cik"] == CIK_CAT)
    assert "2023-01-01/2023-12-31" in d["summary"] or "2023-01-01 to 2023-12-31" in d["summary"]


def test_filing_cells(names, first):
    _, lines, _ = first
    ffq = fqs(names)[1]
    got = {(args_of(c)["accession"], args_of(c)["field"]): c["value"]
           for c in lines if "cell" in c and c["cell"]["fn"] == ffq}
    a = "0000040545-24-000027"
    assert {f: got[(a, f)] for f in FILING_FIELDS} == {
        "cik": {"type": "string", "value": CIK_GE}, "form": {"type": "string", "value": "10-K"},
        "filed": {"type": "string", "value": "2024-02-02"}, "fy": {"type": "string", "value": "2023"},
        "fp": {"type": "string", "value": "FY"}, "period_end": {"type": "string", "value": "2023-12-31"}}
    assert got[("0000019617-24-000225", "period_end")]["value"] == "2023-12-31"
    assert {k[1] for k in got} == set(FILING_FIELDS)


def test_filing_for_cells(names, first):
    _, lines, _ = first
    gfq = fqs(names)[2]
    got = {(args_of(c)["cik"], args_of(c)["fiscal_year"]): c["value"]
           for c in lines if "cell" in c and c["cell"]["fn"] == gfq}
    assert got[(CIK_GE, "2023")] == {"type": "string", "value": "0000040545-24-000027"}
    assert got[(CIK_CAT, "2019")] == {"type": "string", "value": "0000018230-20-000056"}
    assert len(got) == 18


def test_filing_and_filing_for_summaries_say_what_they_are(names, first):
    _, lines, _ = first
    _, ffq, gfq = fqs(names)
    f = next(c for c in lines if "cell" in c and c["cell"]["fn"] == ffq and args_of(c)["field"] == "form")
    g = next(c for c in lines if "cell" in c and c["cell"]["fn"] == gfq)
    assert args_of(f)["accession"] in f["summary"]
    assert args_of(g)["cik"] in g["summary"] and args_of(g)["fiscal_year"] in g["summary"] and "latest" in g["summary"]


# ---------- catalogue (K6) ----------

def cat(lines, fq):
    return next(l for l in lines if l.get("catalog") == fq)


def test_company_fact_catalogue(names, first):
    _, lines, _ = first
    fq = fqs(names)[0]
    c = cat(lines, fq)
    assert c["kind"] == "function" and c["label"].strip()
    d = c["description"]
    assert d.startswith("Relayed by datumesh.com from sec.gov")
    for part in ("as filed", "accession", "restate", "period", "not normalised"):
        assert part in d, part
    order = ["cik", "taxonomy", "concept", "unit", "accession", "period"]
    assert set(c["args"]) == set(order) and all(isinstance(v, str) and v.strip() for v in c["args"].values())
    assert "10 digits" in c["args"]["cik"] and "YYYY-MM-DD" in c["args"]["period"]
    assert len(c["examples"]) >= 2
    cells = {}
    for l in lines:
        if "cell" in l and l["cell"]["fn"] == fq:
            cells[call(fq, order, args_of(l))] = l["value"]
    for e in c["examples"]:
        assert e["call"] in cells and e["expected"] == cells[e["call"]]


def test_filing_and_filing_for_catalogues(names, first):
    _, lines, _ = first
    _, ffq, gfq = fqs(names)
    f, g = cat(lines, ffq), cat(lines, gfq)
    assert set(f["args"]) == {"accession", "field"} and set(g["args"]) == {"cik", "fiscal_year"}
    assert [e["value"] for e in f["dictionary"]["field"]] == FILING_FIELDS
    assert all(e["label"].strip() for e in f["dictionary"]["field"])
    assert len(f["examples"]) >= 2 and len(g["examples"]) >= 2
    for e in f["examples"] + g["examples"]:
        assert e["expected"]["type"] == "string"
    ex = f["examples"][0]
    assert ex["call"].startswith(ffq + '(accession: "') and ', field: "' in ex["call"]
    assert g["examples"][0]["call"].startswith(gfq + '(cik: "') and ', fiscal_year: "' in g["examples"][0]["call"]
    assert "latest" in g["description"] and "10-K" in g["description"]
    assert {e["value"]: e["label"] for e in g["dictionary"]["cik"]} == {
        CIK_CAT: "CATERPILLAR INC", CIK_JPM: "JPMORGAN CHASE & CO", CIK_GE: "GENERAL ELECTRIC COMPANY"}


def test_dictionaries_concept_unit_taxonomy_cik(names, first):
    its, lines, _ = first
    fq = fqs(names)[0]
    dic = cat(lines, fq)["dictionary"]
    assert set(dic) == {"concept", "unit", "taxonomy", "cik"}
    used = {(l["cell"]["args"]["taxonomy"]["value"], l["cell"]["args"]["concept"]["value"]) for l in lines
            if "cell" in l and l["cell"]["fn"] == fq}
    assert {e["value"] for e in dic["concept"]} == {c for _, c in used}
    labels = {}
    for i in its:
        for k, v in i["parsed"]["labels"].items():
            labels[k[1]] = v
    for e in dic["concept"]:
        assert e["label"] == labels[e["value"]]["label"], "concept labels as filed"
        assert e.get("description") == labels[e["value"]]["description"]
    assert {e["value"] for e in dic["unit"]} == {"USD", "shares", "USD/shares"}
    assert all(e["label"].strip() and e["label"] != e["value"] for e in dic["unit"] if e["value"] != "shares")
    assert {e["value"] for e in dic["taxonomy"]} == {"us-gaap", "dei"}
    assert all(e["label"].strip() for e in dic["taxonomy"])
    assert {e["value"]: e["label"] for e in dic["cik"]} == {
        CIK_CAT: "CATERPILLAR INC", CIK_JPM: "JPMORGAN CHASE & CO", CIK_GE: "GENERAL ELECTRIC COMPANY"}
    assert json.dumps(dic, ensure_ascii=False).count("\n") == 0


def test_every_line_is_one_json_line_and_nfc(first):
    _, lines, _ = first
    for l in lines:
        s = json.dumps(l, ensure_ascii=False)
        assert "\n" not in s
        if "cell" in l:
            check_summary(l["summary"])


# ---------- a second run emits nothing new ----------

def test_second_run_emits_nothing_new_and_keeps_the_state(names, first):
    its, _, state = first
    lines2, state2 = sec.emit(its, state, names)
    assert lines2 == [] and state2 == state
    json.dumps(state)  # plain JSON
    later = [{"parsed": i["parsed"], "source": dict(i["source"], retrieved="2026-11-21T12:00:00.000000Z")} for i in its]
    assert sec.emit(later, state, names) == ([], state)


def test_a_new_report_emits_only_its_cells_and_moves_filing_for(names, first):
    its, _, state = first
    raw = json.loads(facts_file(CIK_GE))
    raw["facts"]["us-gaap"]["Assets"]["units"]["USD"].append(
        {"end": "2024-12-31", "val": 177_000_000_000, "accn": "0000040545-25-000099", "fy": 2024, "fp": "FY",
         "form": "10-K/A", "filed": "2025-06-02"})
    data = json.dumps(raw).encode()
    new = [dict(i) for i in its]
    new[CIKS.index(CIK_GE)] = item(names, CIK_GE, data=data)
    lines, state2 = sec.emit(new, state, names)
    funcs, cats, cells, tests = split_lines(lines)
    assert funcs == [] and cats == [] and tests == []
    fq, ffq, gfq = fqs(names)
    by = lambda f: [c for c in cells if c["cell"]["fn"] == f]
    assert [c["value"]["value"] for c in by(fq)] == ["177000000000"]
    assert args_of(by(fq)[0])["accession"] == "0000040545-25-000099"
    assert {args_of(c)["field"]: c["value"]["value"] for c in by(ffq)} == {
        "cik": CIK_GE, "form": "10-K/A", "filed": "2025-06-02", "fy": "2024", "fp": "FY", "period_end": "2024-12-31"}
    assert [(args_of(c)["cik"], args_of(c)["fiscal_year"], c["value"]["value"]) for c in by(gfq)] == [
        (CIK_GE, "2024", "0000040545-25-000099")], "the latest report of fiscal 2024 is now the amendment"
    assert sec.emit(new, state2, names) == ([], state2)


def test_emit_does_not_mutate_its_input(names):
    its = items(names)
    before = copy.deepcopy(its)
    sec.emit(its, {}, names)
    assert its == before


def test_emit_with_nothing_in_scope_emits_nothing(names):
    from sec_helpers import facts_json, rec
    data = facts_json(1234, "X", [("us-gaap", "Assets", "USD", "Assets", [rec("2023-12-31", "1", "0000001234-24-000001", 2023, form="10-Q", fp="Q1")])])
    assert sec.emit([item(names, "0000001234", data=data)], {}, names) == ([], {})
