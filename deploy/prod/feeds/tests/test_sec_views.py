"""sec_views.jsonl (Build 2 Part 7, K4 to K6, as changed by CONTRACT-sec-rebuild.md N4): structural checks of the normalised
views and the ratio library: concepts with their prefix ("us-gaap:Revenues"), dates (no string parsing) in sec/fact_in,
sec/reported through sec/annual_report and relay/sec.gov/filing/period_end, an exact sec/latest by last_by over the filing
class's filed(), and calendar/age_at and calendar/days_between in place of ratio/age_at and ratio/days_between.

The behaviour is proved in the http crate (tests/sec_views.rs publishes the file with the public test rule on
and runs every test line). Here: the shape of the file, with no network and no mesh.
"""
import json
import re

from conftest import FIXTURES, FEEDS

HOST = "mesh-test.datumesh.com"
RELAY = HOST + "/relay/sec.gov/"
FILING = RELAY + "filing/"
ANNUAL = HOST + "/sec/annual_report"
FILING_METHODS = ["cik", "form", "filed", "fiscal_year", "fiscal_period", "period_end"]


def lines():
    return [json.loads(l) for l in (FEEDS / "sec_views.jsonl").read_text(encoding="utf-8").splitlines() if l.strip()]


def stub():
    return [json.loads(l) for l in (FIXTURES / "sec_views_stub_relay.jsonl").read_text(encoding="utf-8").splitlines() if l.strip()]


def formulas():
    return {l["formula"]: l for l in lines() if "formula" in l}


def catalogs():
    out = {}
    for l in lines():
        if "catalog" in l:
            assert l["catalog"] not in out, "two catalogue lines for " + l["catalog"]
            out[l["catalog"]] = l
    return out


def grouped_tests():
    out = {}
    for l in lines():
        if "test" in l:
            out.setdefault(l["test"], []).append(l)
    return out


K4 = ["reported", "latest", "revenue", "net_income", "total_assets", "current_assets", "current_liabilities", "inventory",
      "ppe_net", "cost_of_revenue", "accounts_payable", "accounts_receivable", "operating_income"]
K5_GENERIC = ["margin", "percent_change", "cagr", "percent_of_total"]
CALENDAR = ["age_at", "days_between"]
K5_COMPANY = ["current_ratio", "return_on_assets", "inventory_turnover", "fixed_asset_turnover", "days_payable_outstanding",
              "cash_conversion_cycle", "gross_margin", "revenue_growth"]


def test_the_file_is_only_formulas_catalogue_and_test_lines():
    kinds = {"formula", "catalog", "test"}
    for l in lines():
        assert len(kinds & set(l)) == 1, l


def test_every_name_is_under_mesh_test_and_none_is_a_relay_name():
    raw = (FEEDS / "sec_views.jsonl").read_text(encoding="utf-8")
    # every mention of the public host is the rehearsal host
    assert not re.search(r"(?<!mesh-test\.)datumesh\.com", raw)
    assert "relay/" in raw  # the views read the relay ...
    for name in list(formulas()) + list(catalogs()) + list(grouped_tests()):
        assert name.startswith(HOST + "/"), name
        assert not name.startswith(RELAY) and "/relay/" not in name, name  # ... and define none of it (D36)
    assert not any("function" in l or "cell" in l or "descriptor" in l for l in lines())


def test_the_k4_and_k5_functions_are_all_there_under_their_names():
    f = formulas()
    want = [HOST + "/sec/" + n for n in K4] + [HOST + "/ratio/" + n for n in K5_GENERIC] + [HOST + "/sec/ratio/" + n for n in K5_COMPANY]
    want += [HOST + "/calendar/" + n for n in CALENDAR]
    for n in want:
        assert n in f, n
    assert HOST + "/sec/fact_in" in f  # the building block of reported and latest
    for gone in ("age_at", "days_between"):
        assert HOST + "/ratio/" + gone not in f and HOST + "/ratio/" + gone not in catalogs(), "moved to calendar/"
        assert HOST + "/ratio/" + gone not in grouped_tests()


def test_signatures_follow_the_contract():
    f = formulas()
    for n in ("reported", "latest"):
        args = f[HOST + "/sec/" + n]["signature"]["args"]
        assert f[HOST + "/sec/" + n]["signature"]["order"] == ["cik", "concept", "unit", "fiscal_year", "period_kind"]
        assert args["fiscal_year"]["type"] == "number" and args["cik"]["type"] == "string"
    for n in K4[2:]:
        assert f[HOST + "/sec/" + n]["signature"]["order"] == ["cik", "fiscal_year"], n
    for n in K5_COMPANY:
        assert f[HOST + "/sec/ratio/" + n]["signature"]["order"] == ["cik", "fiscal_year"], n
    fi = f[HOST + "/sec/fact_in"]["signature"]
    assert fi["order"] == ["cik", "concept", "unit", "accession", "period_end", "period_kind"]
    assert fi["args"]["period_end"] == {"type": "date", "valid": False}, "a date: nothing is parsed from a string"
    assert fi["args"]["concept"]["type"] == "string" and fi["returns"] == {"type": "number", "unit": {"arg": "unit"}}
    sig = lambda n: f[HOST + "/ratio/" + n]["signature"]
    assert sig("margin")["order"] == ["part", "whole"]
    assert sig("percent_change")["order"] == ["from", "to"]
    assert sig("cagr")["order"] == ["start", "end", "years"]
    assert sig("percent_of_total")["order"] == ["part", "total"]
    cal = lambda n: f[HOST + "/calendar/" + n]["signature"]
    assert cal("age_at")["order"] == ["birth", "at"] and cal("age_at")["args"]["birth"]["type"] == "date"
    assert cal("age_at")["returns"] == {"type": "number", "unit": "1"}
    assert cal("days_between")["order"] == ["from", "to"] and cal("days_between")["returns"]["unit"] == "d"


def test_every_function_has_one_catalogue_entry_with_args_examples_and_a_definition():
    f, c = formulas(), catalogs()
    assert set(f) == set(c)
    for name, line in f.items():
        e = c[name]
        assert e["kind"] == "function"
        assert 0 < len(e["label"].encode()) <= 120
        assert 40 < len(e["description"].encode()) <= 1024, name
        assert set(e["args"]) == set(line["signature"]["args"]), name
        assert all(isinstance(t, str) and t for t in e["args"].values()), name
        assert e["examples"], name
        for x in e["examples"]:
            assert x["call"].startswith(name + "("), (name, x["call"])
            assert x["expected"]["type"] in ("number", "string", "date")


def test_descriptions_state_their_definitions():
    c = catalogs()
    d = lambda n: c[n]["description"]
    for n in ("inventory_turnover", "fixed_asset_turnover", "return_on_assets", "days_payable_outstanding", "cash_conversion_cycle"):
        assert "average" in d(HOST + "/sec/ratio/" + n), n  # D37: the average of the opening and closing balances
        assert "prior fiscal year" in d(HOST + "/sec/ratio/" + n) or "prior year" in d(HOST + "/sec/ratio/" + n), n
    assert "365" in d(HOST + "/sec/ratio/days_payable_outstanding") and "365" in d(HOST + "/sec/ratio/cash_conversion_cycle")
    ccc = d(HOST + "/sec/ratio/cash_conversion_cycle")
    assert "DIO + DSO - DPO" in ccc
    assert "year-end" in d(HOST + "/sec/ratio/current_ratio")
    rev = d(HOST + "/sec/revenue")
    order = ["Revenues", "RevenueFromContractWithCustomerExcludingAssessedTax", "RevenueFromContractWithCustomerIncludingAssessedTax",
             "SalesRevenueNet", "SalesRevenueGoodsNet", "SalesRevenueServicesNet", "RevenuesNetOfInterestExpense"]
    pos = [rev.index("%d. %s" % (i + 1, t)) for i, t in enumerate(order)]
    assert pos == sorted(pos), "the tag order is stated"
    assert "do not tag alike" in rev
    assert "restate" in d(HOST + "/sec/reported") and "restate" in d(HOST + "/sec/latest")
    lat = d(HOST + "/sec/latest")
    assert "latest filing" in lat and "filed" in lat and "last_by" in lat and "exact" in lat, "N4: latest is exact"
    assert "annual_report" in d(HOST + "/sec/reported")  # the selection it reads is named
    assert "filing_for" not in "".join(d(n) for n in catalogs())
    for n in ("age_at", "days_between"):
        assert d(HOST + "/calendar/" + n).strip(), n
    for n in ("margin", "percent_change", "cagr", "percent_of_total"):
        assert "zero" in d(HOST + "/ratio/" + n), n
    for n in K5_COMPANY:
        assert "absent" in d(HOST + "/sec/ratio/" + n) or "no value" in d(HOST + "/sec/ratio/" + n), n  # a missing input is never a guess


def test_dictionaries_for_the_keyed_arguments():
    c = catalogs()
    import tomllib
    listed = tomllib.loads((FEEDS / "sec_concepts.toml").read_text(encoding="utf-8"))["concept"]
    concepts = {f"{e['taxonomy']}:{e['name']}" for e in listed}
    for name in (HOST + "/sec/reported", HOST + "/sec/latest", HOST + "/sec/fact_in"):
        d = c[name]["dictionary"]
        assert {"concept", "unit", "period_kind"} <= set(d), name
        vals = [x["value"] for x in d["concept"]]
        assert len(vals) == len(set(vals)) and set(vals) <= concepts, "concepts with their prefix, as company_fact takes them"
        assert "us-gaap:Revenues" in vals and "us-gaap:Assets" in vals and "Revenues" not in vals
        assert all(x["label"] for k in d for x in d[k])
        assert [x["value"] for x in d["period_kind"]] == ["annual", "instant"]
        assert "USD" in [x["value"] for x in d["unit"]]


def test_every_function_has_tests_for_its_derived_cases():
    f, t = formulas(), grouped_tests()
    assert set(t) <= set(f)
    for name, line in f.items():
        mine = t.get(name, [])
        assert len(mine) >= 4, name
        for x in mine:
            assert set(x) <= {"test", "call", "inputs", "expect", "asof"}, x  # asof: the spec test object (SPEC 7.3)
            assert x["call"].startswith(name + "("), x["call"]
            e = x["expect"]
            assert ("fault" in e and e["fault"].startswith("E_FORMULA_")) or e["type"] in ("number", "string", "date")
        faults = [x for x in mine if "fault" in x["expect"]]
        values = [x for x in mine if "fault" not in x["expect"]]
        assert values, name  # a value answer is what exercises the branch arms
        numbers = [a for a, d in line["signature"]["args"].items() if d["type"] == "number"]
        for a in numbers:
            # a negative value of every number argument, as the public rule derives it
            assert any(re.search(r"\b%s: -[0-9]" % a, x["call"]) for x in mine), (name, a)
        bare = re.sub(r"[a-z0-9.-]+\.[a-z]+/[a-z_/.]+", "NAME", line["source"])  # a qualified name has slashes
        if "/" in bare:
            for a in numbers:
                # and a zero, for the arguments a divisor can hold
                if re.search(r"/[^\n]*\b%s\b" % a, bare):
                    assert any(re.search(r"\b%s: 0\b" % a, x["call"]) for x in mine), (name, a)
        # a source that reads a cell has a test whose answer is the absent fault (or the empty one for slices)
        if "relay/sec.gov" in line["source"] or "/sec/" in line["source"]:
            assert any(x["expect"].get("fault") in ("E_FORMULA_ABSENT", "E_FORMULA_EMPTY") for x in faults), name
        # the calls name only the function's arguments
        for x in mine:
            used = re.findall(r"(?:^|\(|, )([a-z_]+): ", x["call"][len(name):])
            assert set(used) == set(line["signature"]["args"]), (name, x["call"])


def test_worked_examples_come_from_the_stub_relay_of_two_companies_and_the_restated_one():
    cells = {}
    for l in stub():
        if "cell" in l:
            a = {k: v["value"] for k, v in l["cell"]["args"].items()}
            cells[(l["cell"]["fn"], tuple(sorted(a.items())))] = l["value"]["value"]
    ciks = {dict(k[1])["cik"] for k in cells if k[0] == RELAY + "company_fact"}
    assert ciks == {"0000018230", "0000019617", "0000000777"}
    # the restated figure of the worked example: JPMorgan's 2019 revenue in three reports
    rev = {dict(k[1])["accession"]: v for k, v in cells.items()
           if k[0] == RELAY + "company_fact" and dict(k[1])["concept"] == "us-gaap:Revenues" and dict(k[1])["cik"] == "0000019617"
           and (dict(k[1])["period_start"], dict(k[1])["period_end"]) == ("2019-01-01", "2019-12-31")}
    assert rev == {"0000019617-20-000257": "115627000000", "0000019617-21-000236": "115399000000", "0000019617-22-000272": "115720000000"}
    # the synthetic company: fiscal 2020 assets in A, E (10-K/A), B and D; C does not report them
    assets = {dict(k[1])["accession"]: v for k, v in cells.items()
              if k[0] == RELAY + "company_fact" and dict(k[1])["cik"] == "0000000777"
              and dict(k[1])["concept"] == "us-gaap:Assets" and dict(k[1])["period_end"] == "2020-12-31"}
    assert assets == {"0000000777-21-000001": "100", "0000000777-21-000005": "110", "0000000777-22-000002": "120",
                      "0000000777-24-000004": "130"}
    ex = {x["call"]: x["expected"]["value"] for e in catalogs().values() for x in e["examples"]}
    assert ex['%s/sec/reported(cik: "0000019617", concept: "us-gaap:Revenues", unit: "USD", fiscal_year: 2019, period_kind: "annual")' % HOST] == "115627000000"
    assert ex['%s/sec/latest(cik: "0000019617", concept: "us-gaap:Revenues", unit: "USD", fiscal_year: 2019, period_kind: "annual")' % HOST] == "115720000000"


def test_sec_latest_is_exact_on_the_restated_company():
    """N4: the figure for the same period from the latest filing, by filed date, that reports it (last_by); the
    fiscal-2020 assets are 100, 110, 120 and 130 in four filings, a filing in between reports nothing of them, and the
    latest is the fourth, three fiscal years on (the old fallback chain looked two ahead and gave 120)."""
    want = {("sec/latest", "us-gaap:Assets", 2020, "instant"): "130", ("sec/latest", "us-gaap:Revenues", 2020, "annual"): "70",
            ("sec/reported", "us-gaap:Assets", 2020, "instant"): "110", ("sec/reported", "us-gaap:Revenues", 2020, "annual"): "55",
            ("sec/latest", "us-gaap:Assets", 2021, "instant"): "150", ("sec/latest", "us-gaap:Assets", 2022, "instant"): "200"}
    got = {}
    for l in lines():
        m = re.match(re.escape(HOST) + r'/(sec/(?:latest|reported))\(cik: "0000000777", concept: "([^"]+)", unit: "USD", '
                     r'fiscal_year: (\d+), period_kind: "(\w+)"\)', l.get("call", ""))
        if m:
            got[(m[1], m[2], int(m[3]), m[4])] = l["expect"]["value"]
    for k, v in want.items():
        assert got.get(k) == v, k


def test_the_views_read_the_new_relay_and_parse_no_strings():
    f = formulas()
    fact_in, reported, latest = (f[HOST + "/sec/" + n]["source"] for n in ("fact_in", "reported", "latest"))
    for n, line in f.items():
        src = line["source"]
        for old in ("filing_for", "company_fact_shares", "company_fact_per_share", "company_fact_ratio", 'taxonomy:'):
            assert old not in src, (n, old)
        assert not re.search(r"relay/sec\.gov/filing\(", src), (n, "the old filing(accession, field)")
        assert HOST + "/ratio/age_at" not in src and HOST + "/ratio/days_between" not in src, n
    # fact_in: dates in, dates out (an instant is company_fact(..., period_start: pe, period_end: pe))
    assert "substr(" not in fact_in and "find(" not in fact_in and "len(" not in fact_in, "no string is parsed"
    assert RELAY + "company_fact(" in fact_in and "period_start: period_end, period_end: period_end" in fact_in
    assert "period_start: *" in fact_in and "350" in fact_in and "380" in fact_in
    # reported: our selection, and the filing class's period_end
    assert ANNUAL + "(cik: cik, fiscal_year: fiscal_year)" in reported
    assert FILING + "period_end(accession: " in reported and HOST + "/sec/fact_in(" in reported
    # latest: last_by over the members over accession, ordered by the filing class's filed()
    assert "last_by(" in latest and "accession: *" in latest and FILING + "filed(accession: *)" in latest
    assert RELAY + "company_fact(" in latest and "otherwise(" not in latest, "no fallback chain: it is exact"
    assert "period_end" in latest and ANNUAL in latest and FILING + "period_end(" in latest


def test_the_stub_relay_is_the_fetchers_own_output_over_the_fixtures():
    """D38: the stub is generated by sec.emit (fixtures/make_sec_stub.py), never written by hand, so it cannot diverge from
    what sec.py emits; the same for the forms fixture the relay test publishes."""
    import sys
    sys.path.insert(0, str(FIXTURES))
    import make_sec_stub
    for name, build in (("sec_views_stub_relay.jsonl", make_sec_stub.build_views_stub),
                        ("sec_relay_forms.jsonl", make_sec_stub.build_forms)):
        on_disk = (FIXTURES / name).read_text(encoding="utf-8").splitlines()
        assert on_disk == build(), f"{name} is stale: run python -B fixtures/make_sec_stub.py"


def test_the_stub_relay_has_the_relay_signatures_the_views_read():
    sig = {l["function"]: l["signature"] for l in stub() if "function" in l}
    # one relay function for every unit, and the operator's annual_report (N1, N3); the filing class is its own lines (N2)
    assert set(sig) == {RELAY + "company_fact", ANNUAL}
    assert sig[RELAY + "company_fact"]["order"] == ["cik", "concept", "unit", "accession", "period_start", "period_end"]
    assert sig[RELAY + "company_fact"]["returns"] == {"type": "number", "unit": {"arg": "unit"}}
    assert sig[RELAY + "company_fact"]["args"]["period_start"] == {"type": "date", "valid": False}
    assert sig[ANNUAL]["order"] == ["cik", "fiscal_year"] and sig[ANNUAL]["returns"] == "string"
    assert sig[ANNUAL]["args"]["fiscal_year"] == {"type": "number", "unit": "1", "valid": False}
    classes = [l for l in stub() if "class" in l]
    assert [c["class"] for c in classes] == [RELAY + "filing"]
    descriptors = [l["descriptor"] for l in stub() if "descriptor" in l and "class" not in l]
    assert [d["signature"]["returns"] for d in descriptors] == ["string", "string", "date", {"type": "number", "unit": "1"},
                                                                 "string", "date"]
    assert all(d["signature"]["order"] == ["accession"] for d in descriptors)
    # every read of a relay name in a view uses that name's own arguments
    args = {RELAY + "company_fact": set(sig[RELAY + "company_fact"]["args"]), ANNUAL: {"cik", "fiscal_year"}}
    args.update({FILING + m: {"accession"} for m in FILING_METHODS})
    for name, line in formulas().items():
        for fn, a in re.findall(r"(" + re.escape(RELAY) + r"[\w/]+|" + re.escape(ANNUAL) + r")\(([^()]*(?:\([^()]*\)[^()]*)*)\)",
                                line["source"]):
            used = set(re.findall(r"(?:^|, |\()([a-z_]+): ", "(" + a))
            assert used == args[fn], (name, fn, used)


def test_the_views_declare_the_units_they_compute_in():
    """D38: money views read company_fact with unit USD and return USD; ratios say percent, 1 or d; the generic building
    blocks (fact_in, reported, latest) return the unit they were asked for, as the relay does."""
    f = formulas()
    returns = lambda n: f[n]["signature"]["returns"]
    for n in ("fact_in", "reported", "latest"):
        assert returns(HOST + "/sec/" + n) == {"type": "number", "unit": {"arg": "unit"}}, n
    for n in ("revenue", "net_income", "total_assets", "current_assets", "current_liabilities", "inventory", "ppe_net",
              "cost_of_revenue", "accounts_payable", "accounts_receivable", "operating_income"):
        assert returns(HOST + "/sec/" + n) == {"type": "number", "unit": "USD"}, n
        assert '"1"' not in f[HOST + "/sec/" + n]["source"]
    for n in ("margin", "percent_change", "cagr", "percent_of_total"):
        assert returns(HOST + "/ratio/" + n) == {"type": "number", "unit": "percent"}, n
    assert returns(HOST + "/calendar/days_between") == {"type": "number", "unit": "d"}
    for n in ("return_on_assets", "gross_margin", "revenue_growth"):
        assert returns(HOST + "/sec/ratio/" + n) == {"type": "number", "unit": "percent"}, n
    for n in ("current_ratio", "inventory_turnover", "fixed_asset_turnover"):
        assert returns(HOST + "/sec/ratio/" + n) == {"type": "number", "unit": "1"}, n
    for n in ("days_payable_outstanding", "cash_conversion_cycle"):
        assert returns(HOST + "/sec/ratio/" + n) == {"type": "number", "unit": "d"}, n
    # the named money views read through sec/reported with a literal USD and a concept with its prefix
    assert all('unit: "USD"' in f[HOST + "/sec/" + n]["source"] for n in ("revenue", "total_assets", "net_income"))
    assert 'concept: "us-gaap:Revenues"' in f[HOST + "/sec/revenue"]["source"]
    assert 'concept: "us-gaap:Assets"' in f[HOST + "/sec/total_assets"]["source"]
    for n, line in f.items():
        if n.startswith(HOST + "/sec/") and n.rsplit("/", 1)[1] in K4[2:]:
            assert not re.search(r'concept: "(?!us-gaap:)', line["source"]), (n, "every concept is written with its prefix")


def test_callees_are_defined_before_their_callers():
    seen = set()
    for l in lines():
        if "formula" not in l:
            continue
        for callee in re.findall(re.escape(HOST) + r"/(?:sec|ratio|calendar)/[a-z_/]+(?=\()", l["source"]):
            if callee == ANNUAL:
                continue  # stored, in the relay stub, not a formula of this file
            assert callee in seen, (l["formula"], callee)
        seen.add(l["formula"])


def test_the_test_lines_follow_their_functions_catalogue_and_stay_unique():
    seen = set()
    for l in lines():
        if "test" in l:
            key = json.dumps(l, sort_keys=True)
            assert key not in seen, "a duplicate test line"
            seen.add(key)
    total = sum(len(v) for v in grouped_tests().values())
    assert total >= 200, total
