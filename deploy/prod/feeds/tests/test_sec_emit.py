"""sec.emit under the SEC rebuild (CONTRACT-sec-rebuild.md N1 to N3): the one relay function company_fact, the filing class
with six stored methods, and sec/annual_report (stored, signed by the operator, no source); their cells, receipts' source and
licence, summaries, catalogue and dictionaries. The cell value is the figure as filed; nothing is normalised under the
relay name."""
import copy
import hashlib
import json

import pytest

import register
import sec
from sec_helpers import names  # noqa: F401  (the SEC names: sec.names)
from helpers import HEX64, check_source_object, check_summary, host_of, sha256
from sec_helpers import (CIK_CAT, CIK_GE, CIK_JPM, CIKS, FACT_ORDER, FILING_METHODS, FILING_RETURNS, IN_SCOPE, NOW,
                         annual_fn, args_of, cf_url, check_order_all, fact_fn, facts_file, filing_cls, item, kinds,
                         mixed_company, source_of)

PUBLIC = "public:https://www.sec.gov/about/privacy-information"
S = {"type": "string", "valid": False}
D = {"type": "date", "valid": False}


def items(names):
    return [item(names, c) for c in CIKS]


@pytest.fixture(scope="module")
def first(names):
    its = items(names)
    lines, state = sec.emit(its, {}, names)
    return its, lines, state


def cells_of(lines, fn):
    return [l for l in lines if "cell" in l and l["cell"]["fn"] == fn]


def cat(lines, name):
    (c,) = [l for l in lines if l.get("catalog") == name]
    return c


def dcall(fn, order, vals, bare=()):
    """A call as the catalogue writes it: strings quoted, the arguments named in `bare` (dates, numbers) not."""
    return f"{fn}({', '.join(f'{a}: {vals[a]}' if a in bare else f'{a}: {chr(34)}{vals[a]}{chr(34)}' for a in order)})"


def fact_call(fn, vals):
    return dcall(fn, FACT_ORDER, vals, bare=("period_start", "period_end"))


# ---------- the names ----------

def test_names_are_the_contract_on_the_rehearsal_host(names):
    s = names["sec"]
    assert s["namespace"] == "mesh-test.datumesh.com/relay/sec.gov"
    assert s["function"] == "company_fact"
    assert s["filing_class"] == "filing" and s["filing_methods"] == FILING_METHODS
    assert (s["annual_report_namespace"], s["annual_report_function"]) == ("mesh-test.datumesh.com/sec", "annual_report")
    assert s["source_name"] == "sec.gov" and s["takeover"] == "https://mesh-test.datumesh.com/takeover/sec.gov"
    assert s["licence"] == PUBLIC
    assert s["companyfacts_url"] == "https://data.sec.gov/api/xbrl/companyfacts/CIK{cik}.json"
    assert s["submissions_url"] == "https://data.sec.gov/submissions/CIK{cik}.json"
    assert s["tickers_url"] == "https://www.sec.gov/files/company_tickers_exchange.json"
    assert s["frames_url"] == "https://data.sec.gov/api/xbrl/frames/{taxonomy}/{concept}/{unit}/{ccp}.json"
    for gone in ("shares_function", "per_share_function", "ratio_function", "filing_function", "filing_for_function"):
        assert gone not in s, f"{gone}: replaced by one company_fact, the filing class and sec/annual_report"
    assert fact_fn(names) == "mesh-test.datumesh.com/relay/sec.gov/company_fact"
    assert filing_cls(names) == "mesh-test.datumesh.com/relay/sec.gov/filing"
    assert annual_fn(names) == "mesh-test.datumesh.com/sec/annual_report"


def test_the_register_section_for_sec_is_named_in_the_table(names):
    assert names["sec"]["register_source"] == "sec.gov" and names["sec"]["register_code"] == "company_facts"


# ---------- lines of the first run ----------

def test_first_run_has_two_function_lines_six_descriptors_one_class_nine_catalogue_entries(names, first):
    _, lines, _ = first
    check_order_all(lines)
    k = kinds(lines)
    assert [len(k[x]) for x in ("function", "descriptor", "class", "catalog", "test")] == [2, 6, 1, 9, 0]
    assert len(k["cell"]) == 1488 + 18 * 6 + 18
    by = {l["function"]: l["signature"] for l in k["function"]}
    assert set(by) == {fact_fn(names), annual_fn(names)}, "one relay function, and the operator's annual_report"
    assert by[fact_fn(names)] == {
        "args": {"cik": S, "concept": S, "unit": S, "accession": S, "period_start": D, "period_end": D},
        "order": FACT_ORDER, "returns": {"type": "number", "unit": {"arg": "unit"}}}, (
        "no argument is a valid-time argument; the unit is the filed unit, returned through {arg: unit}")
    assert by[annual_fn(names)] == {
        "args": {"cik": S, "fiscal_year": {"type": "number", "unit": "1", "valid": False}},
        "order": ["cik", "fiscal_year"], "returns": "string"}
    assert not any("function" in l and "filing" in l["function"] for l in lines), "the old filing(accession, field) is gone"


def test_the_filing_class_follows_the_register_dataset_pattern(names, first):
    _, lines, _ = first
    k = kinds(lines)
    cls = filing_cls(names)
    want = {m: {"code": {"body": "", "lang": "stored"}, "inputs": [],
                "signature": {"args": {"accession": S}, "order": ["accession"], "returns": FILING_RETURNS[m]}}
            for m in FILING_METHODS}
    got = [l["descriptor"] for l in k["descriptor"]]
    assert got == [want[m] for m in FILING_METHODS], "six stored methods, each keyed by accession, in the contract's order"
    hashes = {m: hashlib.sha256(register.jcs(want[m])).hexdigest() for m in FILING_METHODS}
    (c,) = k["class"]
    assert c["class"] == cls
    d = c["descriptor"]
    assert d["keys"] == {"accession": {"type": "string", "key_normalisation": "none"}} and d["order"] == ["accession"]
    assert d["methods"] == hashes
    assert d["steward"]["namespace"] == names["sec"]["namespace"] and d["steward"]["process"].strip()
    assert set(d) == {"keys", "order", "methods", "steward"}


def test_cell_counts(names, first):
    _, lines, _ = first
    n = lambda f: len(cells_of(lines, f))
    assert n(fact_fn(names)) == sum(IN_SCOPE.values()) == 1488
    for m in FILING_METHODS:
        assert n(f"{filing_cls(names)}/{m}") == 18, m
    assert n(annual_fn(names)) == 18


# ---------- company_fact ----------

def test_company_fact_cell_is_the_figure_as_filed(names, first):
    _, lines, _ = first
    fq = fact_fn(names)
    cells = cells_of(lines, fq)
    key = lambda c: tuple(args_of(c)[k] for k in FACT_ORDER)
    by = {key(c): c for c in cells}
    assert len(by) == len(cells), "keys are unique"
    ge = by[(CIK_GE, "us-gaap:Assets", "USD", "0000040545-23-000023", "2022-12-31", "2022-12-31")]
    assert ge["value"] == {"type": "number", "value": "187788000000"}
    ge2 = by[(CIK_GE, "us-gaap:Assets", "USD", "0000040545-24-000027", "2022-12-31", "2022-12-31")]
    assert ge2["value"] == {"type": "number", "value": "188851000000"}
    for c in cells:
        assert set(c) == {"cell", "value", "licence", "summary", "source"}
        assert list(c["cell"]["args"]) == FACT_ORDER, "the arguments in the order of the signature"
        a = c["cell"]["args"]
        assert all(a[k]["type"] == "string" for k in ("cik", "concept", "unit", "accession"))
        assert a["period_start"]["type"] == "date" and a["period_end"]["type"] == "date"
        assert c["value"]["type"] == "number"
        assert a["concept"]["value"].startswith(("us-gaap:", "dei:")) and "taxonomy" not in a and "period" not in a
        assert a["period_start"]["value"] <= a["period_end"]["value"]


def test_instant_and_duration_periods_and_the_dei_concept(names, first):
    _, lines, _ = first
    cells = [args_of(c) for c in cells_of(lines, fact_fn(names))]
    inst = [a for a in cells if a["concept"] == "us-gaap:Assets"]
    assert inst and all(a["period_start"] == a["period_end"] for a in inst), "an instant has the same date in both"
    dur = [a for a in cells if a["concept"] == "us-gaap:Revenues" and a["cik"] == CIK_CAT]
    assert any((a["period_start"], a["period_end"]) == ("2023-01-01", "2023-12-31") for a in dur)
    dei = [a for a in cells if a["concept"].startswith("dei:")]
    assert dei and {a["concept"] for a in dei} == {"dei:EntityCommonStockSharesOutstanding"}
    assert {a["unit"] for a in dei} == {"shares"}


def test_every_unit_is_one_function_and_the_unit_is_a_key(names, first):
    _, lines, _ = first
    cells = cells_of(lines, fact_fn(names))
    units = {args_of(c)["unit"] for c in cells}
    assert units == {"USD", "shares", "USD/shares"}
    per = [c for c in cells if args_of(c)["unit"] == "USD/shares"]
    assert per and {args_of(c)["concept"] for c in per} <= {"us-gaap:EarningsPerShareBasic", "us-gaap:EarningsPerShareDiluted"}
    sh = {args_of(c)["concept"] for c in cells if args_of(c)["unit"] == "shares"}
    assert "dei:EntityCommonStockSharesOutstanding" in sh
    assert not [l for l in lines if "function" in l and any(n in l["function"] for n in ("_shares", "_per_share", "_ratio"))]


def test_every_relayed_cell_carries_the_public_licence_source_and_summary(names, first):
    its, lines, _ = first
    s = names["sec"]
    url_sha = {cf_url(names, i["parsed"]["cik"]): sha256(facts_file(i["parsed"]["cik"])) for i in its}
    relayed = cells_of(lines, fact_fn(names)) + [c for m in FILING_METHODS for c in cells_of(lines, f"{filing_cls(names)}/{m}")]
    assert len(relayed) == 1488 + 108
    for c in relayed:
        assert c["licence"] == PUBLIC, "N1/N2: public:<the SEC's terms page>, on every relayed cell"
        src = c["source"]
        check_source_object(src, host_of(s["namespace"]), name="sec.gov", url=src["url"], sha=src["sha256"],
                            retrieved=NOW, takeover="https://mesh-test.datumesh.com/takeover/sec.gov")
        assert src["url"] in url_sha and src["sha256"] == url_sha[src["url"]] and HEX64.match(src["sha256"])
        check_summary(c["summary"])
        assert "relayed by datumesh.com" in c["summary"] and s["takeover"] in c["summary"] and "sec.gov" in c["summary"]


def test_a_cells_source_is_the_file_of_its_company(names, first):
    its, lines, _ = first
    cik_of = {a: i["parsed"]["cik"] for i in its for a in i["parsed"]["filings"]}
    for c in lines:
        if "cell" not in c or "source" not in c:
            continue
        a = args_of(c)
        cik = a["cik"] if "cik" in a else cik_of[a["accession"]]
        assert c["source"]["url"] == cf_url(names, cik)


def test_company_fact_summary_names_company_form_filing_date_fy_and_period_as_filed(names, first):
    _, lines, _ = first
    c = next(l for l in cells_of(lines, fact_fn(names)) if args_of(l)["accession"] == "0000040545-24-000027"
             and args_of(l)["concept"] == "us-gaap:Assets" and args_of(l)["period_end"] == "2022-12-31")
    for part in ("GENERAL ELECTRIC COMPANY", "10-K", "2024-02-02", "fiscal year 2023", "2022-12-31", "Assets",
                 "188851000000", "0000040545-24-000027"):
        assert part in c["summary"], part
    d = next(l for l in cells_of(lines, fact_fn(names)) if args_of(l)["period_start"] == "2023-01-01"
             and args_of(l)["period_end"] == "2023-12-31" and args_of(l)["cik"] == CIK_CAT)
    assert "2023-01-01" in d["summary"] and "2023-12-31" in d["summary"]


# ---------- the filing class (N2) ----------

def test_filing_cells_are_the_methods_of_the_class_keyed_by_accession(names, first):
    _, lines, _ = first
    cls = filing_cls(names)
    a = "0000040545-24-000027"
    want = {"cik": {"type": "string", "value": CIK_GE}, "form": {"type": "string", "value": "10-K"},
            "filed": {"type": "date", "value": "2024-02-02"}, "fiscal_year": {"type": "number", "value": "2023"},
            "fiscal_period": {"type": "string", "value": "FY"}, "period_end": {"type": "date", "value": "2023-12-31"}}
    for m in FILING_METHODS:
        cs = cells_of(lines, f"{cls}/{m}")
        assert all(c["cell"]["args"] == {"accession": {"type": "string", "value": args_of(c)["accession"]}} for c in cs)
        got = {args_of(c)["accession"]: c["value"] for c in cs}
        assert got[a] == want[m], m
        assert len(got) == 18
    pe = {args_of(c)["accession"]: c["value"]["value"] for c in cells_of(lines, f"{cls}/period_end")}
    assert pe["0000019617-24-000225"] == "2023-12-31", "the period end of the annual statements, not a cover-page date"
    assert all(c["licence"] == PUBLIC and "source" in c for m in FILING_METHODS for c in cells_of(lines, f"{cls}/{m}"))


def test_filing_summaries_say_what_they_are(names, first):
    _, lines, _ = first
    cls = filing_cls(names)
    for m in FILING_METHODS:
        c = cells_of(lines, f"{cls}/{m}")[0]
        assert args_of(c)["accession"] in c["summary"] and "filing" in c["summary"], m


# ---------- sec/annual_report (N3) ----------

def test_annual_report_cells_are_stored_unsourced_and_say_it_is_our_selection(names, first):
    _, lines, _ = first
    fn = annual_fn(names)
    cs = cells_of(lines, fn)
    assert len(cs) == 18
    got = {(args_of(c)["cik"], args_of(c)["fiscal_year"]): c["value"] for c in cs}
    assert got[(CIK_GE, "2023")] == {"type": "string", "value": "0000040545-24-000027"}
    assert got[(CIK_CAT, "2019")] == {"type": "string", "value": "0000018230-20-000056"}
    for c in cs:
        assert "source" not in c, "no source: the trust is primary, the cell is not labelled relayed"
        assert set(c) == {"cell", "value", "licence", "summary"}
        assert c["licence"] == PUBLIC, "it restates public information"
        assert c["cell"]["args"]["fiscal_year"]["type"] == "number" and c["cell"]["args"]["cik"]["type"] == "string"
        assert list(c["cell"]["args"]) == ["cik", "fiscal_year"] and c["value"]["type"] == "string"
        t = c["summary"]
        check_summary(t)
        assert args_of(c)["cik"] in t and args_of(c)["fiscal_year"] in t and c["value"]["value"] in t and "latest" in t
        assert "selection" in t and "Datumesh" in t and "SEC" in t, "Datumesh's selection over the SEC's filings"
        assert "relayed by datumesh.com" not in t


# ---------- catalogue ----------

def test_company_fact_catalogue(names, first):
    its, lines, _ = first
    fq = fact_fn(names)
    c = cat(lines, fq)
    assert c["kind"] == "function" and c["label"].strip()
    d = c["description"]
    assert d.startswith("Relayed by datumesh.com from sec.gov")
    for part in ("as filed", "accession", "restate", "period", "not normalised", "unit", "shares", "pure", "instant"):
        assert part in d, part
    for gone in ("company_fact_shares", "company_fact_per_share", "company_fact_ratio"):
        assert gone not in d
    assert list(c["args"]) == FACT_ORDER or set(c["args"]) == set(FACT_ORDER)
    assert all(isinstance(v, str) and v.strip() for v in c["args"].values())
    assert "10 digits" in c["args"]["cik"] and "prefix" in c["args"]["concept"] and "us-gaap:" in c["args"]["concept"]
    assert "same date" in c["args"]["period_end"] or "instant" in c["args"]["period_end"]
    assert c["open_keys"] == ["accession"], "H7 of CONTRACT-0.4.9: accession is an open key"
    assert len(c["examples"]) >= 2
    cells = {}
    for l in cells_of(lines, fq):
        cells[fact_call(fq, args_of(l))] = l["value"]
    for e in c["examples"]:
        assert e["call"] in cells and e["expected"] == cells[e["call"]]
    dic = c["dictionary"]
    assert set(dic) == {"concept", "unit", "cik"}, "cik, concept and unit are dictionary keys; accession is open"
    used = {args_of(l)["concept"] for l in cells_of(lines, fq)}
    assert {e["value"] for e in dic["concept"]} == used and all(":" in v for v in used)
    labels = {}
    for i in its:
        for (t, n), v in i["parsed"]["labels"].items():
            labels[f"{t}:{n}"] = v
    for e in dic["concept"]:
        assert e["label"] == labels[e["value"]]["label"], "concept labels as filed"
        assert e.get("description") == labels[e["value"]]["description"]
    assert {e["value"] for e in dic["unit"]} == {"USD", "shares", "USD/shares"}
    assert all(e["label"].strip() and e["label"] != e["value"] for e in dic["unit"])
    assert {e["value"]: e["label"] for e in dic["cik"]} == {
        CIK_CAT: "CATERPILLAR INC", CIK_JPM: "JPMORGAN CHASE & CO", CIK_GE: "GENERAL ELECTRIC COMPANY"}
    assert json.dumps(dic, ensure_ascii=False).count("\n") == 0


def test_filing_class_and_method_catalogues(names, first):
    _, lines, _ = first
    cls = filing_cls(names)
    c = cat(lines, cls)
    assert c["kind"] == "class" and c["label"].strip() and "accession" in c["description"]
    assert set(c["args"]) == {"accession"} and c["open_keys"] == ["accession"]
    assert c["description"].startswith("Relayed by datumesh.com from sec.gov")
    for m in FILING_METHODS:
        e = cat(lines, f"{cls}/{m}")
        assert e["kind"] == "function" and e["label"].strip() and set(e["args"]) == {"accession"}
        assert e["open_keys"] == ["accession"], m
        assert e["description"].startswith("Relayed by datumesh.com from sec.gov") and "as filed" in e["description"]
        assert len(e["examples"]) >= 2, m
        cells = {args_of(l)["accession"]: l["value"] for l in cells_of(lines, f"{cls}/{m}")}
        for x in e["examples"]:
            assert x["call"].startswith(f'{cls}/{m}(accession: "') and x["call"].endswith('")')
            assert x["expected"] == cells[x["call"].split(chr(34))[1]]
    assert "period end" in cat(lines, f"{cls}/period_end")["description"] and "cover" in cat(lines, f"{cls}/period_end")["description"]
    assert "fiscal" in cat(lines, f"{cls}/fiscal_year")["description"]


def test_annual_report_catalogue_states_the_selection_rule(names, first):
    _, lines, _ = first
    fn = annual_fn(names)
    c = cat(lines, fn)
    assert c["kind"] == "function" and set(c["args"]) == {"cik", "fiscal_year"}
    d = c["description"]
    for part in ("10-K", "10-K/A", "latest", "filed", "accession", "tie", "fiscal year", "Datumesh"):
        assert part in d, part
    assert not d.startswith("Relayed"), "it is not relayed: our selection over the SEC's filings"
    assert "open_keys" not in c
    assert len(c["examples"]) >= 2
    cells = {(args_of(l)["cik"], args_of(l)["fiscal_year"]): l["value"] for l in cells_of(lines, fn)}
    for e in c["examples"]:
        assert e["call"].startswith(fn + '(cik: "') and e["call"].endswith(")") and "fiscal_year: 20" in e["call"]
        assert 'fiscal_year: "' not in e["call"]
        cik, fy = e["call"].split(chr(34))[1], e["call"].rstrip(")").split("fiscal_year: ")[1]
        assert e["expected"] == cells[(cik, fy)]
    assert {e["value"]: e["label"] for e in c["dictionary"]["cik"]} == {
        CIK_CAT: "CATERPILLAR INC", CIK_JPM: "JPMORGAN CHASE & CO", CIK_GE: "GENERAL ELECTRIC COMPANY"}


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


def test_a_new_report_emits_only_its_cells_and_moves_annual_report(names, first):
    its, _, state = first
    raw = json.loads(facts_file(CIK_GE))
    raw["facts"]["us-gaap"]["Assets"]["units"]["USD"].append(
        {"end": "2024-12-31", "val": 177_000_000_000, "accn": "0000040545-25-000099", "fy": 2024, "fp": "FY",
         "form": "10-K/A", "filed": "2025-06-02"})
    data = json.dumps(raw).encode()
    new = [dict(i) for i in its]
    new[CIKS.index(CIK_GE)] = item(names, CIK_GE, data=data)
    lines, state2 = sec.emit(new, state, names)
    k = kinds(lines)
    assert [len(k[x]) for x in ("function", "descriptor", "class", "catalog", "test")] == [0, 0, 0, 0, 0]
    cells = k["cell"]
    by = lambda f: [c for c in cells if c["cell"]["fn"] == f]
    assert [c["value"]["value"] for c in by(fact_fn(names))] == ["177000000000"]
    assert args_of(by(fact_fn(names))[0])["accession"] == "0000040545-25-000099"
    a = "0000040545-25-000099"
    assert {m: by(f"{filing_cls(names)}/{m}")[0]["value"]["value"] for m in FILING_METHODS} == {
        "cik": CIK_GE, "form": "10-K/A", "filed": "2025-06-02", "fiscal_year": "2024", "fiscal_period": "FY",
        "period_end": "2024-12-31"}
    assert all(args_of(by(f"{filing_cls(names)}/{m}")[0]) == {"accession": a} for m in FILING_METHODS)
    assert [(args_of(c)["cik"], args_of(c)["fiscal_year"], c["value"]["value"]) for c in by(annual_fn(names))] == [
        (CIK_GE, "2024", a)], "the latest report of fiscal 2024 is now the amendment"
    assert len(cells) == 1 + 6 + 1
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


# ---------- N1: the one function holds every unit ----------

def test_a_company_holding_every_unit_is_one_function_and_unsupported_units_are_left_out_and_counted(names):
    data = mixed_company()
    parsed = sec.parse_companyfacts(data)
    assert parsed["left_out_units"] == {"CAD/EUR": 1, "acre": 1, "USD-per-gal": 1}
    assert sec.count_cells(data) == 7, "the estimator counts only what is relayed"
    lines, state = sec.emit([item(names, "0000001234", data=data)], {}, names)
    k = kinds(lines)
    fq = fact_fn(names)
    assert {l["function"] for l in k["function"]} == {fq, annual_fn(names)}
    cs = cells_of(lines, fq)
    assert len(cs) == 7
    assert sorted((args_of(c)["concept"], args_of(c)["unit"]) for c in cs) == sorted([
        ("us-gaap:Assets", "USD"), ("us-gaap:Assets", "EUR"), ("us-gaap:Assets", "USN"),
        ("us-gaap:EarningsPerShareBasic", "EUR/shares"), ("us-gaap:EarningsPerShareDiluted", "USD/shares"),
        ("us-gaap:CommonStockSharesOutstanding", "shares"), ("us-gaap:GrossProfit", "pure")])
    (ratio,) = [c for c in cs if args_of(c)["unit"] == "pure"]
    assert ratio["value"] == {"type": "number", "value": "0.5"}
    assert (args_of(ratio)["period_start"], args_of(ratio)["period_end"]) == ("2023-01-01", "2023-12-31")
    assert {e["value"] for e in cat(lines, fq)["dictionary"]["unit"]} == {"USD", "EUR", "USN", "EUR/shares", "USD/shares",
                                                                         "shares", "pure"}
    assert all(c["licence"] == PUBLIC for c in cs)
    assert sec.emit([item(names, "0000001234", data=data)], state, names) == ([], state)


def test_a_unit_that_first_appears_in_a_later_run_emits_only_its_cell(names):
    from sec_helpers import facts_json, rec
    a, b = "0000001234-24-000001", "0000001234-25-000001"
    usd = facts_json(1234, "X CO", [("us-gaap", "Assets", "USD", "Assets", [rec("2023-12-31", "10", a, 2023)])])
    lines, state = sec.emit([item(names, "0000001234", data=usd)], {}, names)
    assert {l["function"] for l in kinds(lines)["function"]} == {fact_fn(names), annual_fn(names)}
    more = facts_json(1234, "X CO", [
        ("us-gaap", "Assets", "USD", "Assets", [rec("2023-12-31", "10", a, 2023)]),
        ("us-gaap", "CommonStockSharesOutstanding", "shares", "Shares", [rec("2024-12-31", "5", b, 2024)])])
    lines2, state2 = sec.emit([item(names, "0000001234", data=more)], state, names)
    k = kinds(lines2)
    assert [len(k[x]) for x in ("function", "descriptor", "class", "catalog")] == [0, 0, 0, 0], "no new function for a new unit"
    assert [(args_of(c)["unit"], c["value"]["value"]) for c in cells_of(lines2, fact_fn(names))] == [("shares", "5")]
    assert sec.emit([item(names, "0000001234", data=more)], state2, names) == ([], state2)


def test_the_scope_report_lists_the_units_left_out(names):
    p = sec.parse_companyfacts(mixed_company())
    ranking = {"ranked": [{"cik": "0000001234", "ticker": "X", "name": "X", "exchange": "Nasdaq", "value": "1"}],
               "listed_without_assets": 0, "frame_not_listed": 0, "frame_entities": 1}
    rep = sec.scope_report(ranking, {"n": 1, "estimated": 7}, unsupported_units=p["left_out_units"])
    assert rep["left_out"]["units_not_in_units_md"]["cells"] == {"CAD/EUR": 1, "acre": 1, "USD-per-gal": 1}
    assert "any other unit is left out" in rep["left_out"]["units_not_in_units_md"]["rule"]
