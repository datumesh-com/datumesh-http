"""K2 / D21 / N1: source-conformance of the relay method company/fact against the frames API (the SEC's own second channel
for the same concept, period and CIK). At least 20 tests a run, and a disagreement stops the run. Each test names the method
by its full name <host>/relay/sec.gov/company/fact (CONTRACT-sec-classes C1: the one method of every unit); N2: the six
methods of the filing class against the submissions API (C2: period_end is the submissions' own reportDate, so it has no
second copy: its test pins the relayed value; cik, form and filed are still compared); N3: the pinned samples of
<host>/sec/company/annual_report (C3)."""
import copy
import re

import pytest

import sec
from sec_helpers import names  # noqa: F401  (the SEC names: sec.names)
from common import ConformanceError
from sec_helpers import (CIK_CAT, CIK_GE, CIK_JPM, CIKS, FILING_METHODS, FILING_RETURNS, FRAMES_ASSETS, NOW, annual_fn,
                         fact_fn, facts_file, filing_cls, frames_json, item, sec_fixture, submissions_file,
                         synth_company, synth_submissions)


def fq(names):
    return fact_fn(names)


def call(names, cik, concept, unit, accession, start, end=None):
    """The call of a test: the concept with its prefix, the dates bare (an instant has the same date twice)."""
    end = end or start
    return (f'{fq(names)}(cik: "{cik}", concept: "{concept}", unit: "{unit}", accession: "{accession}", '
            f'period_start: {start}, period_end: {end})')


@pytest.fixture(scope="module")
def rows():
    return [r for c in CIKS for r in sec.parse_companyfacts(facts_file(c))["rows"]]


@pytest.fixture(scope="module")
def frame():
    return sec.parse_frames(sec_fixture(FRAMES_ASSETS))


def test_real_frame_against_real_companyfacts(names, rows, frame):
    tests = sec.conformance(rows, [frame], names, minimum=1)
    assert sorted(t["call"] for t in tests) == sorted([
        call(names, CIK_CAT, "us-gaap:Assets", "USD", "0000018230-25-000008", "2023-12-31"),
        call(names, CIK_GE, "us-gaap:Assets", "USD", "0000040545-25-000015", "2023-12-31")])
    for t in tests:
        assert set(t) == {"test", "call", "expect"} and t["test"] == fq(names)
    by = {t["call"]: t["expect"] for t in tests}
    assert by[call(names, CIK_GE, "us-gaap:Assets", "USD", "0000040545-25-000015", "2023-12-31")] == {"type": "number", "value": "173300000000"}
    assert by[call(names, CIK_CAT, "us-gaap:Assets", "USD", "0000018230-25-000008", "2023-12-31")] == {"type": "number", "value": "87476000000"}


def test_the_frames_value_is_the_latest_filed_so_a_restated_figure_is_checked_in_its_own_report(names, rows, frame):
    """GE: the frame's 2023-12-31 Assets is 173300000000 (FY2024 report). The FY2023 report's 163045000000 is a different
    cell and is not what the frame says, so the test is on the frame's accession, never on the other one."""
    tests = sec.conformance(rows, [frame], names, minimum=1)
    assert not any("0000040545-24-000027" in t["call"] for t in tests)


def test_a_frame_entry_whose_report_is_out_of_scope_is_skipped_not_failed(names, rows, frame):
    """JPMorgan's frame accession (0001628280-26-008131) is a fiscal 2025 report: not in the rows, not a test."""
    tests = sec.conformance(rows, [frame], names, minimum=1)
    assert not any(CIK_JPM in t["call"] for t in tests)


def test_default_minimum_is_20(names, rows, frame):
    with pytest.raises(ConformanceError) as e:
        sec.conformance(rows, [frame], names)
    assert "20" in str(e.value) and "2" in str(e.value)


def test_a_disagreement_stops_the_run(names, rows):
    data = sec_fixture(FRAMES_ASSETS)
    bad = data.replace(b'"val":87476000000', b'"val":87476000001')
    assert bad != data
    with pytest.raises(ConformanceError) as e:
        sec.conformance(rows, [sec.parse_frames(bad)], names, minimum=1)
    m = str(e.value)
    assert CIK_CAT in m and "87476000001" in m and "87476000000" in m and "0000018230-25-000008" in m


def test_a_frame_fact_of_an_in_scope_report_that_the_rows_lack_is_a_disagreement(names, rows, frame):
    fewer = [r for r in rows if not (r["cik"] == CIK_GE and r["concept"] == "us-gaap:Assets" and r["period_end"] == "2023-12-31"
                                     and r["accession"] == "0000040545-25-000015")]
    assert len(fewer) == len(rows) - 1
    with pytest.raises(ConformanceError) as e:
        sec.conformance(fewer, [frame], names, minimum=1)
    assert CIK_GE in str(e.value)


def test_companies_not_in_the_rows_are_not_tested(names, rows, frame):
    only_cat = [r for r in rows if r["cik"] == CIK_CAT]
    tests = sec.conformance(only_cat, [frame], names, minimum=1)
    assert len(tests) == 1 and CIK_CAT in tests[0]["call"]


def synth_world(n):
    entries = [(1000 + i, f"CO {i}", f"{1000 + i:010d}-24-000001", "2023-12-31", 10_000 + i) for i in range(n)]
    rows = []
    for c, name, accn, end, val in entries:
        rows += sec.parse_companyfacts(synth_company(c, name, accn, end, val))["rows"]
    return entries, rows


def test_twenty_five_pairs_pass_the_default_minimum(names):
    entries, rows = synth_world(25)
    tests = sec.conformance(rows, [sec.parse_frames(frames_json(entries))], names)
    assert len(tests) == 25
    assert all(t["call"].startswith(fq(names) + '(cik: "') and t["test"] == fq(names) for t in tests)
    assert len({t["call"] for t in tests}) == 25
    assert sec.conformance(rows, [sec.parse_frames(frames_json(entries))], names) == tests, "deterministic"


def test_nineteen_pairs_are_too_few(names):
    entries, rows = synth_world(19)
    with pytest.raises(ConformanceError):
        sec.conformance(rows, [sec.parse_frames(frames_json(entries))], names)


def test_pairs_from_several_frames_add_up_and_duration_frames_use_start_end(names):
    entries, rows = synth_world(12)
    assets = sec.parse_frames(frames_json(entries))
    rev = [(c, n, a, "2023-12-31", 1000) for c, n, a, _, _ in entries]
    data = frames_json(rev, concept="Revenues", ccp="CY2023")
    import json
    d = json.loads(data)
    for x in d["data"]:
        x["start"] = "2023-01-01"
    revenue = sec.parse_frames(json.dumps(d).encode())
    tests = sec.conformance(rows, [assets, revenue], names)
    assert len(tests) == 24
    assert any('concept: "us-gaap:Revenues"' in t["call"] and "period_start: 2023-01-01, period_end: 2023-12-31)" in t["call"]
               for t in tests)
    assert all(not re.search(r'period_(start|end): "', t["call"]) for t in tests), "the periods are dates, not strings"


def test_the_comparison_is_numeric_and_expect_is_canonical(names):
    entries, rows = synth_world(1)
    c = entries[0]
    frames = frames_json([(c[0], c[1], c[2], c[3], c[4])])
    tests = sec.conformance(rows, [sec.parse_frames(frames)], names, minimum=1)
    assert tests[0]["expect"] == {"type": "number", "value": str(c[4])}


def test_conformance_does_not_mutate_rows(names):
    entries, rows = synth_world(3)
    before = copy.deepcopy(rows)
    sec.conformance(rows, [sec.parse_frames(frames_json(entries))], names, minimum=1)
    assert rows == before


# ---------- N1: every unit is tested through the one company_fact ----------

def _mixed_rows(names):
    from sec_helpers import mixed_company
    return sec.parse_companyfacts(mixed_company())["rows"]


def _frame(concept, unit, end, val, start=None, uom=None):
    import json
    d = json.loads(frames_json([(1234, "MIXED CO", "0000001234-24-000001", end, val)], concept=concept, unit=uom or unit,
                               ccp="CY2023"))
    if start:
        d["data"][0]["start"] = start
    return sec.parse_frames(json.dumps(d).encode())


def test_money_shares_per_share_and_pure_tests_all_call_the_fact_method_by_its_full_name(names):
    rows = _mixed_rows(names)
    frames = [
        _frame("EarningsPerShareDiluted", "USD/shares", "2023-12-31", 1.25, start="2023-01-01", uom="USD-per-shares"),
        _frame("CommonStockSharesOutstanding", "shares", "2023-12-31", 100),
        _frame("Assets", "EUR", "2023-12-31", 9),
        _frame("GrossProfit", "pure", "2023-12-31", 0.5, start="2023-01-01"),
    ]
    tests = sec.conformance(rows, frames, names, minimum=1)
    assert {t["test"] for t in tests} == {fq(names)}, "one function: the unit is a key, not a function"
    by = {t["call"]: t["expect"] for t in tests}
    a = "0000001234-24-000001"
    c = lambda concept, unit, s, e: call(names, "0000001234", "us-gaap:" + concept, unit, a, s, e)
    assert by == {
        c("EarningsPerShareDiluted", "USD/shares", "2023-01-01", "2023-12-31"): {"type": "number", "value": "1.25"},
        c("CommonStockSharesOutstanding", "shares", "2023-12-31", "2023-12-31"): {"type": "number", "value": "100"},
        c("Assets", "EUR", "2023-12-31", "2023-12-31"): {"type": "number", "value": "9"},
        c("GrossProfit", "pure", "2023-01-01", "2023-12-31"): {"type": "number", "value": "0.5"}}
    assert not any(n in str(tests) for n in ("company_fact", "company_fact_shares", "company_fact_per_share", "company_fact_ratio"))
    assert all(t["call"].startswith(names["sec"]["namespace"] + "/company/fact(") for t in tests)


def test_a_frame_in_a_unit_that_is_not_relayed_is_skipped(names):
    rows = _mixed_rows(names)
    odd = _frame("Liabilities", "CAD/EUR", "2023-12-31", 7)
    good = _frame("Assets", "EUR", "2023-12-31", 9)
    tests = sec.conformance(rows, [odd, good], names, minimum=1)
    assert len(tests) == 1


# ---------- N2: the filing class against the submissions API ----------

def subs():
    return {c: sec.parse_submissions(submissions_file(c)) for c in CIKS}


METHOD_FNS = lambda names: [f"{filing_cls(names)}/{m}" for m in FILING_METHODS]


@pytest.fixture(scope="module")
def items3(names):
    return [item(names, c) for c in CIKS]


def mcall(names, method, accn):
    return f'{filing_cls(names)}/{method}(accession: "{accn}")'


def test_every_filing_method_is_tested_against_the_submissions_api(names, items3):
    tests = sec.filing_conformance(items3, list(subs().values()), names)
    assert len(tests) == 18 * 6 == 108
    for t in tests:
        assert set(t) == {"test", "call", "expect"} and t["test"] in METHOD_FNS(names)
        assert t["call"] == f'{t["test"]}(accession: "{t["call"].split(chr(34))[1]}")', "one key: the accession, quoted"
    by = {t["call"]: t["expect"] for t in tests}
    a = "0000040545-24-000027"
    assert by[mcall(names, "cik", a)] == {"type": "string", "value": CIK_GE}
    assert by[mcall(names, "form", a)] == {"type": "string", "value": "10-K"}
    assert by[mcall(names, "filed", a)] == {"type": "date", "value": "2024-02-02"}
    assert by[mcall(names, "period_end", a)] == {"type": "date", "value": "2023-12-31"}
    assert by[mcall(names, "fiscal_year", a)] == {"type": "number", "value": "2023"}
    assert by[mcall(names, "fiscal_period", a)] == {"type": "string", "value": "FY"}
    assert {t["test"].rsplit("/", 1)[1] for t in tests} == set(FILING_METHODS), "per method"
    assert all(sum(1 for t in tests if t["test"].endswith("/" + m)) == 18 for m in FILING_METHODS)
    assert sec.filing_conformance(items3, list(subs().values()), names) == tests, "deterministic"


def test_a_10k_amendment_is_tested_with_its_own_form_and_dates(names, items3):
    tests = sec.filing_conformance(items3, list(subs().values()), names)
    by = {t["call"]: t["expect"] for t in tests}
    assert by[mcall(names, "period_end", "0000019617-24-000225")] == {"type": "date", "value": "2023-12-31"}, (
        "the cover-page share count is dated January 2024; the second copy's report date is the period end")


def test_filings_the_submissions_do_not_have_are_skipped_not_failed(names, items3):
    only_cat = [subs()[CIK_CAT]]
    tests = sec.filing_conformance(items3, only_cat, names)
    assert len(tests) == 6 * 6 and all("0000018230-" in t["call"] for t in tests)
    recent_only = sec.parse_submissions(submissions_file(CIK_GE))
    del recent_only["filings"]["0000040545-20-000009"]
    tests = sec.filing_conformance(items3, [recent_only], names, minimum=1)
    assert len(tests) == 5 * 6 and not any("0000040545-20-000009" in t["call"] for t in tests)


@pytest.mark.parametrize("field,wrong,method", [("form", "10-K/A", "form")])
def test_a_disagreement_with_the_submissions_stops_the_run(names, items3, field, wrong, method):
    bad = sec.parse_submissions(submissions_file(CIK_GE))
    a = "0000040545-24-000027"
    right = bad["filings"][a][field]
    assert right != wrong
    bad["filings"][a][field] = wrong
    with pytest.raises(ConformanceError) as e:
        sec.filing_conformance(items3, [bad], names, minimum=1)
    m = str(e.value)
    assert a in m and wrong in m and right in m and method in m and "nothing is published" in m


def test_a_period_end_that_differs_from_the_companyfacts_is_no_disagreement_and_the_test_pins_the_submissions_value(names, items3):
    """C2: period_end has no second copy. The relayed value is the submissions' reportDate, whatever companyfacts' periods say."""
    sub = sec.parse_submissions(submissions_file(CIK_GE))
    a = "0000040545-24-000027"
    assert sub["filings"][a]["period_end"] == "2023-12-31"
    sub["filings"][a]["period_end"] = "2023-12-30"
    tests = sec.filing_conformance(items3, [sub], names, minimum=1)
    by = {t["call"]: t["expect"] for t in tests}
    assert by[mcall(names, "period_end", a)] == {"type": "date", "value": "2023-12-30"}
    assert by[mcall(names, "form", a)] == {"type": "string", "value": "10-K"}


def test_athene_amendment_gets_the_report_date_not_the_latest_one_year_period(names):
    """The regression of CONTRACT-sec-classes C2: Athene Holding's 10-K/A 0001527469-21-000032 has a one-year period that
    ends 2021-03-31 in its companyfacts, while its report date is 2020-12-31 (so has the 10-K -21-000025). No error, and
    2020-12-31 for both."""
    from sec_class_helpers import ATHENE_10K, ATHENE_10KA, ATHENE_CIK, athene_facts, athene_submissions
    its = [item(names, ATHENE_CIK, data=athene_facts())]
    assert {r["period_end"] for r in its[0]["parsed"]["rows"] if r["accession"] == ATHENE_10KA} >= {"2021-03-31"}, (
        "the fixture has the period that fooled the derivation")
    tests = sec.filing_conformance(its, [sec.parse_submissions(athene_submissions())], names, minimum=1)
    by = {t["call"]: t["expect"] for t in tests}
    for accn in (ATHENE_10K, ATHENE_10KA):
        assert by[mcall(names, "period_end", accn)] == {"type": "date", "value": "2020-12-31"}
    assert by[mcall(names, "form", ATHENE_10KA)] == {"type": "string", "value": "10-K/A"}
    assert by[mcall(names, "filed", ATHENE_10KA)] == {"type": "date", "value": "2021-04-20"}


def test_a_different_cik_for_an_accession_is_a_disagreement(names, items3):
    ge = sec.parse_submissions(submissions_file(CIK_GE))
    ge["cik"] = CIK_CAT  # the second copy says another company filed these accessions
    with pytest.raises(ConformanceError) as e:
        sec.filing_conformance(items3, [ge], names, minimum=1)
    assert "cik" in str(e.value) and CIK_GE in str(e.value)


def test_filing_conformance_has_a_minimum_of_20_tests(names, items3):
    assert sec.FILING_CONFORMANCE_MINIMUM == 20
    with pytest.raises(ConformanceError) as e:
        sec.filing_conformance(items3, [subs()[CIK_CAT]][:0], names)
    assert "20" in str(e.value) and "nothing is published" in str(e.value)
    assert len(sec.filing_conformance(items3, list(subs().values()), names)) >= 20


def test_filing_conformance_on_a_synthetic_company(names):
    accn = "0000001000-24-000001"
    its = [item(names, "0000001000", data=synth_company(1000, "CO 1000", accn, "2023-12-31", 10_000))]
    tests = sec.filing_conformance(its, [sec.parse_submissions(synth_submissions(1000, accn))], names, minimum=1)
    assert len(tests) == 2 * 6
    by = {t["call"]: t["expect"] for t in tests}
    assert by[mcall(names, "filed", accn)] == {"type": "date", "value": "2024-02-15"}
    assert by[mcall(names, "form", "0000001000-23-000001")] == {"type": "string", "value": "10-K"}


# ---------- N3: sec/annual_report is our selection: pinned samples ----------

def test_annual_report_tests_pin_the_earliest_and_latest_fiscal_year_of_each_company(names, items3):
    tests = sec.annual_report_tests(items3, names)
    fn = annual_fn(names)
    assert fn == "mesh-test.datumesh.com/sec/company/annual_report"
    assert all(set(t) == {"test", "call", "expect"} and t["test"] == fn for t in tests)
    by = {t["call"]: t["expect"] for t in tests}
    pin = lambda cik, fy, accn: by[f'{fn}(cik: "{cik}", fiscal_year: {fy})'] == {"type": "string", "value": accn}
    assert pin(CIK_GE, 2019, "0000040545-20-000009") and pin(CIK_GE, 2024, "0000040545-25-000015")
    assert pin(CIK_CAT, 2019, "0000018230-20-000056") and pin(CIK_CAT, 2024, "0000018230-25-000008")
    assert pin(CIK_JPM, 2019, "0000019617-20-000257") and pin(CIK_JPM, 2024, "0000019617-25-000270")
    assert len(tests) == 6 and all("fiscal_year: 20" in t["call"] and 'fiscal_year: "' not in t["call"] for t in tests), (
        "a number argument is written bare")
    assert sec.annual_report_tests(items3, names) == tests


def test_annual_report_tests_follow_the_amendment_rule_on_a_company_with_restated_years(names):
    from sec_helpers import RESTATE_ACCN, RESTATE_CIK, restated_company
    tests = sec.annual_report_tests([item(names, RESTATE_CIK, data=restated_company())], names)
    fn = annual_fn(names)
    by = {t["call"]: t["expect"]["value"] for t in tests}
    assert by == {f'{fn}(cik: "{RESTATE_CIK}", fiscal_year: 2020)': RESTATE_ACCN["E"],
                  f'{fn}(cik: "{RESTATE_CIK}", fiscal_year: 2023)': RESTATE_ACCN["D"]}, (
        "fiscal 2020 is the 10-K/A E (latest filed), not the 10-K A and not B, whose accession is higher")
