"""sec_views.jsonl (Build 2 Part 7, K4 to K6): structural checks of the normalised views and the ratio library.

The behaviour is proved in the http crate (tests/sec_views.rs publishes the file with the public test rule on
and runs every test line). Here: the shape of the file, with no network and no mesh.
"""
import json
import re

from conftest import FIXTURES, FEEDS

HOST = "mesh-test.datumesh.com"
RELAY = HOST + "/relay/sec.gov/"


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
K5_GENERIC = ["margin", "percent_change", "cagr", "percent_of_total", "age_at", "days_between"]
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
    for n in want:
        assert n in f, n
    assert HOST + "/sec/fact_in" in f  # the building block of reported and latest


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
    sig = lambda n: f[HOST + "/ratio/" + n]["signature"]
    assert sig("margin")["order"] == ["part", "whole"]
    assert sig("percent_change")["order"] == ["from", "to"]
    assert sig("cagr")["order"] == ["start", "end", "years"]
    assert sig("percent_of_total")["order"] == ["part", "total"]
    assert sig("age_at")["order"] == ["birth", "at"] and sig("age_at")["args"]["birth"]["type"] == "date"
    assert sig("days_between")["order"] == ["from", "to"] and sig("days_between")["returns"]["unit"] == "d"


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
    assert "restate" in d(HOST + "/sec/reported") and "restatement wins" in d(HOST + "/sec/latest")
    assert "filing_for" in d(HOST + "/sec/reported")  # D36: the helper is named
    for n in ("margin", "percent_change", "cagr", "percent_of_total"):
        assert "zero" in d(HOST + "/ratio/" + n), n
    for n in K5_COMPANY:
        assert "absent" in d(HOST + "/sec/ratio/" + n) or "no value" in d(HOST + "/sec/ratio/" + n), n  # a missing input is never a guess


def test_dictionaries_for_the_keyed_arguments():
    c = catalogs()
    concepts = set(re.findall(r'^name = "([^"]+)"', (FEEDS / "sec_concepts.toml").read_text(encoding="utf-8"), re.M))
    for name in (HOST + "/sec/reported", HOST + "/sec/latest", HOST + "/sec/fact_in"):
        d = c[name]["dictionary"]
        assert {"concept", "unit", "period_kind"} <= set(d), name
        vals = [x["value"] for x in d["concept"]]
        assert len(vals) == len(set(vals)) and set(vals) <= concepts and "Revenues" in vals and "Assets" in vals
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
            assert set(x) <= {"test", "call", "inputs", "expect"}, x
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


def test_worked_examples_come_from_the_stub_relay_of_two_companies():
    cells = {}
    for l in stub():
        if "cell" in l:
            a = {k: v["value"] for k, v in l["cell"]["args"].items()}
            cells[(l["cell"]["fn"], tuple(sorted(a.items())))] = l["value"]["value"]
    ciks = {dict(k[1])["cik"] for k in cells if k[0] == RELAY + "company_fact"}
    assert ciks == {"0000018230", "0000019617"}
    # the restated figure of the worked example: JPMorgan's 2019 revenue in three reports
    rev = {dict(k[1])["accession"]: v for k, v in cells.items()
           if k[0] == RELAY + "company_fact" and dict(k[1])["concept"] == "Revenues" and dict(k[1])["cik"] == "0000019617"
           and dict(k[1])["period"] == "2019-01-01/2019-12-31"}
    assert rev == {"0000019617-20-000257": "115627000000", "0000019617-21-000236": "115399000000", "0000019617-22-000272": "115720000000"}
    ex = {x["call"]: x["expected"]["value"] for e in catalogs().values() for x in e["examples"]}
    assert ex['%s/sec/reported(cik: "0000019617", concept: "Revenues", unit: "USD", fiscal_year: 2019, period_kind: "annual")' % HOST] == "115627000000"
    assert ex['%s/sec/latest(cik: "0000019617", concept: "Revenues", unit: "USD", fiscal_year: 2019, period_kind: "annual")' % HOST] == "115720000000"


def test_the_stub_relay_has_the_three_relay_signatures_the_views_read():
    sig = {l["function"]: l["signature"] for l in stub() if "function" in l}
    assert set(sig) == {RELAY + "company_fact", RELAY + "filing", RELAY + "filing_for"}
    assert sig[RELAY + "company_fact"]["order"] == ["cik", "taxonomy", "concept", "unit", "accession", "period"]
    assert sig[RELAY + "filing"]["order"] == ["accession", "field"]
    assert sig[RELAY + "filing_for"]["order"] == ["cik", "fiscal_year"] and sig[RELAY + "filing_for"]["args"]["fiscal_year"]["type"] == "number"
    # every read of a relay name in a view uses that name's own arguments
    for name, line in formulas().items():
        for fn, args in re.findall(r"(" + re.escape(RELAY) + r"\w+)\(([^()]*(?:\([^()]*\)[^()]*)*)\)", line["source"]):
            used = set(re.findall(r"(?:^|, |\()([a-z_]+): ", "(" + args))
            assert used == set(sig[fn]["args"]), (name, fn, used)


def test_callees_are_defined_before_their_callers():
    seen = set()
    for l in lines():
        if "formula" not in l:
            continue
        for callee in re.findall(re.escape(HOST) + r"/(?:sec|ratio)/[a-z_/]+(?=\()", l["source"]):
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
