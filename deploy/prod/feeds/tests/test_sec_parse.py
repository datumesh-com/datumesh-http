"""sec.parse_companyfacts: companyfacts file -> rows, by the scope rule (K3) and the key rule (K2).
One row per (cik, taxonomy, concept, unit, accession, period); 10-K and 10-K/A only, fy 2019 to 2024, listed concepts."""
import json

import pytest

import common
import sec
from sec_helpers import CIK_CAT, CIK_GE, CIK_JPM, CIKS, IN_SCOPE, facts_file, facts_json, rec

KEY = ("cik", "taxonomy", "concept", "unit", "accession", "period")


def key(r):
    return tuple(r[k] for k in KEY)


def rows_of(cik):
    return sec.parse_companyfacts(facts_file(cik))["rows"]


def parsed_one(entries, cik=1234, name="TEST CO"):
    return sec.parse_companyfacts(facts_json(cik, name, entries))


# ---------- the concept list ----------

def test_concepts_file_has_45_us_gaap_and_the_one_dei_concept_each_with_a_reason():
    cs = sec.load_concepts()
    assert len(cs) == 46
    assert sum(1 for t, _ in cs if t == "us-gaap") == 45
    assert [k for k in cs if k[0] != "us-gaap"] == [("dei", "EntityCommonStockSharesOutstanding")]
    for (t, c), reason in cs.items():
        assert isinstance(reason, str) and len(reason.strip()) >= 10, (t, c)
    for c in ("Assets", "AssetsCurrent", "LiabilitiesCurrent", "InventoryNet", "PropertyPlantAndEquipmentNet",
              "AccountsPayableCurrent", "AccountsReceivableNetCurrent", "CostOfRevenue", "NetIncomeLoss",
              "OperatingIncomeLoss"):
        assert ("us-gaap", c) in cs
    # the seven revenue tags of K4, in the list
    for c in ("Revenues", "RevenueFromContractWithCustomerExcludingAssessedTax",
              "RevenueFromContractWithCustomerIncludingAssessedTax", "SalesRevenueNet", "SalesRevenueGoodsNet",
              "SalesRevenueServicesNet", "RevenuesNetOfInterestExpense"):
        assert ("us-gaap", c) in cs


def test_scope_constants():
    assert sec.FORMS == ("10-K", "10-K/A")
    assert (sec.FIRST_FY, sec.LAST_FY) == (2019, 2024)
    assert (sec.BUDGET, sec.STEP) == (300_000, 25)


# ---------- the real files ----------

@pytest.mark.parametrize("cik", CIKS)
def test_fixture_rows_by_the_scope_rule(cik):
    p = sec.parse_companyfacts(facts_file(cik))
    assert p["cik"] == cik and len(p["rows"]) == IN_SCOPE[cik] == sec.count_cells(facts_file(cik))
    concepts = sec.load_concepts()
    for r in p["rows"]:
        assert r["cik"] == cik and (r["taxonomy"], r["concept"]) in concepts
        assert r["form"] in ("10-K", "10-K/A") and 2019 <= int(r["fy"]) <= 2024
    assert len({key(r) for r in p["rows"]}) == len(p["rows"]), "one row per key"
    assert p["conflicts"] == []
    assert not any(r["concept"] in ("DebtAndEquitySecuritiesGainLoss", "DeferredTaxAssetsGross") for r in p["rows"])


def test_entity_names_and_ciks():
    assert sec.parse_companyfacts(facts_file(CIK_CAT))["entity"] == "CATERPILLAR INC"
    assert sec.parse_companyfacts(facts_file(CIK_JPM))["entity"] == "JPMORGAN CHASE & CO"
    assert sec.parse_companyfacts(facts_file(CIK_GE))["entity"] == "GENERAL ELECTRIC COMPANY"


def test_no_quarterly_form_no_other_year_no_later_report():
    accs = {r["accession"] for r in rows_of(CIK_JPM)}
    assert "0001628280-26-008131" not in accs, "a fiscal 2025 report (fy 2025) is out of scope"
    assert {r["accession"] for r in rows_of(CIK_JPM)} == {
        "0000019617-20-000257", "0000019617-21-000236", "0000019617-22-000272",
        "0000019617-23-000231", "0000019617-24-000225", "0000019617-25-000270"}
    assert {r["fy"] for r in rows_of(CIK_JPM)} == {"2019", "2020", "2021", "2022", "2023", "2024"}


def row(cik, concept, unit, accession, period, taxonomy="us-gaap"):
    hit = [r for r in rows_of(cik) if (r["taxonomy"], r["concept"], r["unit"], r["accession"], r["period"]) ==
           (taxonomy, concept, unit, accession, period)]
    assert len(hit) == 1, hit
    return hit[0]


def test_restated_figure_is_a_separate_cell_ge():
    a = row(CIK_GE, "Assets", "USD", "0000040545-23-000023", "2022-12-31")
    b = row(CIK_GE, "Assets", "USD", "0000040545-24-000027", "2022-12-31")
    assert (a["value"], b["value"]) == ("187788000000", "188851000000")
    c = row(CIK_GE, "Assets", "USD", "0000040545-24-000027", "2023-12-31")
    d = row(CIK_GE, "Assets", "USD", "0000040545-25-000015", "2023-12-31")
    assert (c["value"], d["value"]) == ("163045000000", "173300000000")
    cogs19a = row(CIK_GE, "CostOfGoodsAndServicesSold", "USD", "0000040545-20-000009", "2019-01-01/2019-12-31")
    cogs19b = row(CIK_GE, "CostOfGoodsAndServicesSold", "USD", "0000040545-21-000011", "2019-01-01/2019-12-31")
    assert (cogs19a["value"], cogs19b["value"]) == ("70029000000", "66911000000")
    shares = [row(CIK_GE, "CommonStockSharesOutstanding", "shares", a_, "2020-12-31")["value"]
              for a_ in ("0000040545-21-000011", "0000040545-22-000008")]
    assert shares == ["8765493000", "1095686581"]


def test_the_same_figure_in_two_reports_is_two_cells_even_when_equal():
    a = row(CIK_CAT, "Assets", "USD", "0000018230-24-000009", "2023-12-31")
    b = row(CIK_CAT, "Assets", "USD", "0000018230-25-000008", "2023-12-31")
    assert a["value"] == b["value"] == "87476000000" and a["fy"] == "2023" and b["fy"] == "2024"


def test_jpm_assets_restated_in_the_fy2021_report():
    got = {a: row(CIK_JPM, "Assets", "USD", a, "2019-12-31")["value"]
           for a in ("0000019617-20-000257", "0000019617-21-000236", "0000019617-22-000272")}
    assert got == {"0000019617-20-000257": "2687379000000", "0000019617-21-000236": "2687379000000",
                   "0000019617-22-000272": "2686477000000"}


def test_period_formats_instant_and_duration_and_dei():
    inst = row(CIK_CAT, "Assets", "USD", "0000018230-24-000009", "2023-12-31")
    assert inst["period"] == "2023-12-31"
    durs = [r for r in rows_of(CIK_CAT) if r["concept"] == "Revenues" and "/" in r["period"]]
    assert durs and all(len(r["period"]) == 21 and r["period"][10] == "/" for r in durs)
    assert any(r["period"] == "2023-01-01/2023-12-31" for r in durs)
    dei = [r for r in rows_of(CIK_CAT) if r["taxonomy"] == "dei"]
    assert dei and all(r["concept"] == "EntityCommonStockSharesOutstanding" and r["unit"] == "shares"
                       and "/" not in r["period"] for r in dei)


def test_units_as_filed():
    units = {r["unit"] for cik in CIKS for r in rows_of(cik)}
    assert units == {"USD", "shares", "USD/shares"}


def test_labels_as_filed_and_descriptions():
    p = sec.parse_companyfacts(facts_file(CIK_GE))
    raw = json.loads(facts_file(CIK_GE))
    for (t, c), lab in p["labels"].items():
        assert lab["label"] == raw["facts"][t][c]["label"] and lab["description"] == raw["facts"][t][c]["description"]
    assert p["labels"][("dei", "EntityCommonStockSharesOutstanding")]["label"] == "Entity Common Stock, Shares Outstanding"
    assert p["labels"][("us-gaap", "EarningsPerShareDiluted")]["label"] == "Earnings Per Share, Diluted"
    assert ("us-gaap", "DebtAndEquitySecuritiesGainLoss") not in p["labels"], "labels only for relayed concepts"


# ---------- exact decimals ----------

def test_decimals_are_exact_canonical_text():
    p = parsed_one([("us-gaap", "EarningsPerShareDiluted", "USD/shares", "EPS", [
        rec("2023-12-31", "1.10", "0000001234-24-000001", 2023, start="2023-01-01"),
        rec("2023-12-31", "-0.05", "0000001234-24-000002", 2023, start="2023-01-01"),
        rec("2023-12-31", "2.5E-5", "0000001234-24-000003", 2023, start="2023-01-01"),
        rec("2023-12-31", "3.0", "0000001234-24-000004", 2023, start="2023-01-01"),
        rec("2023-12-31", "12345678901234567890", "0000001234-24-000005", 2023, start="2023-01-01"),
        rec("2023-12-31", "0", "0000001234-24-000006", 2023, start="2023-01-01"),
        rec("2023-12-31", "0.1000000000000000055511151231257827", "0000001234-24-000007", 2023, start="2023-01-01"),
    ])])
    got = {r["accession"][-1]: r["value"] for r in p["rows"]}
    assert got == {"1": "1.1", "2": "-0.05", "3": "0.000025", "4": "3", "5": "12345678901234567890", "6": "0",
                   "7": "0.1000000000000000055511151231257827"}
    for v in got.values():
        assert common.is_number(v)


def test_values_are_not_scaled_or_sign_changed():
    r = row(CIK_JPM, "NoninterestExpense", "USD", "0000019617-20-000257", "2019-01-01/2019-12-31")
    assert r["value"] == "65497000000"
    loss = parsed_one([("us-gaap", "NetIncomeLoss", "USD", "Net Income (Loss)", [
        rec("2023-12-31", "-1500000", "0000001234-24-000001", 2023, start="2023-01-01")])])
    assert loss["rows"][0]["value"] == "-1500000"


# ---------- scope filters on built files ----------

def test_forms_fiscal_years_and_concepts_filter():
    A = "0000001234-24-00000"
    p = parsed_one([
        ("us-gaap", "Assets", "USD", "Assets", [
            rec("2023-12-31", "1", A + "1", 2023, form="10-K"),
            rec("2023-12-31", "2", A + "2", 2023, form="10-K/A", filed="2024-06-01"),
            rec("2023-12-31", "3", A + "3", 2023, form="10-Q", fp="Q1"),
            rec("2023-12-31", "4", A + "4", 2023, form="8-K"),
            rec("2023-12-31", "5", A + "5", 2023, form="20-F"),
            rec("2023-12-31", "6", A + "6", 2023, form="40-F"),
            rec("2018-12-31", "7", A + "7", 2018),
            rec("2019-12-31", "8", A + "8", 2019),
            rec("2024-12-31", "9", A + "9", 2024),
            rec("2025-12-31", "10", "0000001234-25-000010", 2025)]),
        ("us-gaap", "Goodwill", "USD", "Goodwill", [rec("2023-12-31", "11", A + "1", 2023)]),
        ("us-gaap", "SomeOtherConcept", "USD", "Other", [rec("2023-12-31", "12", A + "1", 2023)]),
        ("custom", "Assets", "USD", "Custom assets", [rec("2023-12-31", "13", A + "1", 2023)]),
    ])
    got = sorted((r["taxonomy"], r["concept"], r["value"]) for r in p["rows"])
    assert got == [("us-gaap", "Assets", "1"), ("us-gaap", "Assets", "2"), ("us-gaap", "Assets", "8"),
                   ("us-gaap", "Assets", "9"), ("us-gaap", "Goodwill", "11")]


def test_in_filing_duplicates_equal_values_are_one_cell():
    A = "0000001234-24-000001"
    p = parsed_one([("us-gaap", "Assets", "USD", "Assets", [
        rec("2023-12-31", "100", A, 2023), rec("2023-12-31", "100.0", A, 2023, fp="FY"),
        rec("2022-12-31", "90", A, 2023)])])
    assert sorted((r["period"], r["value"]) for r in p["rows"]) == [("2022-12-31", "90"), ("2023-12-31", "100")]
    assert p["conflicts"] == []


def test_in_filing_duplicates_with_conflicting_values_are_reported_and_skipped():
    A, B = "0000001234-24-000001", "0000001234-24-000002"
    p = parsed_one([("us-gaap", "Assets", "USD", "Assets", [
        rec("2023-12-31", "100", A, 2023), rec("2023-12-31", "101", A, 2023),
        rec("2023-12-31", "100", B, 2023), rec("2022-12-31", "90", A, 2023)])])
    assert sorted((r["accession"], r["period"], r["value"]) for r in p["rows"]) == [
        (A, "2022-12-31", "90"), (B, "2023-12-31", "100")]
    assert len(p["conflicts"]) == 1
    c = p["conflicts"][0]
    assert (c["cik"], c["taxonomy"], c["concept"], c["unit"], c["accession"], c["period"]) == (
        "0000001234", "us-gaap", "Assets", "USD", A, "2023-12-31")
    assert sorted(c["values"]) == ["100", "101"]


def test_same_period_in_two_units_or_two_concepts_is_two_cells():
    A = "0000001234-24-000001"
    p = parsed_one([
        ("us-gaap", "CommonStockSharesOutstanding", "shares", "Shares", [rec("2023-12-31", "5", A, 2023)]),
        ("us-gaap", "CommonStockSharesOutstanding", "pure", "Shares", [rec("2023-12-31", "5", A, 2023)]),
        ("us-gaap", "Assets", "USD", "Assets", [rec("2023-12-31", "5", A, 2023)])])
    assert len(p["rows"]) == 3


# ---------- filing metadata and filing_for ----------

def test_filings_of_the_fixtures():
    p = sec.parse_companyfacts(facts_file(CIK_GE))
    f = p["filings"]["0000040545-24-000027"]
    assert f == {"cik": CIK_GE, "form": "10-K", "filed": "2024-02-02", "fy": "2023", "fp": "FY",
                 "period_end": "2023-12-31"}
    assert set(p["filings"]) == {r["accession"] for r in p["rows"]}, "one filing per accession with cells"
    pj = sec.parse_companyfacts(facts_file(CIK_JPM))
    assert pj["filings"]["0000019617-24-000225"]["period_end"] == "2023-12-31", (
        "the cover-page share count is dated Jan 2024; period_end is the latest period of the statements, not the cover")


def test_filing_for_is_the_latest_filed_annual_report_of_the_fiscal_year():
    p = sec.parse_companyfacts(facts_file(CIK_GE))
    assert p["filing_for"] == {"2019": "0000040545-20-000009", "2020": "0000040545-21-000011",
                               "2021": "0000040545-22-000008", "2022": "0000040545-23-000023",
                               "2023": "0000040545-24-000027", "2024": "0000040545-25-000015"}


def test_filing_for_prefers_the_amendment_filed_later_and_uses_fy_not_the_calendar_year():
    # a January year end: the report filed in March 2024 for the year ended 2024-01-31 says fy 2023
    p = parsed_one([("us-gaap", "Assets", "USD", "Assets", [
        rec("2024-01-31", "10", "0000001234-24-000010", 2023, filed="2024-03-20"),
        rec("2024-01-31", "11", "0000001234-24-000050", 2023, form="10-K/A", filed="2024-07-01"),
        rec("2025-01-31", "12", "0000001234-25-000010", 2024, filed="2025-03-20")])])
    assert p["filing_for"] == {"2023": "0000001234-24-000050", "2024": "0000001234-25-000010"}
    assert p["filings"]["0000001234-24-000050"]["form"] == "10-K/A"
    assert p["filings"]["0000001234-24-000050"]["period_end"] == "2024-01-31"


def test_filing_for_same_day_ties_go_to_the_higher_accession():
    p = parsed_one([("us-gaap", "Assets", "USD", "Assets", [
        rec("2023-12-31", "10", "0000001234-24-000010", 2023, filed="2024-03-20"),
        rec("2023-12-31", "11", "0000001234-24-000011", 2023, form="10-K/A", filed="2024-03-20")])])
    assert p["filing_for"]["2023"] == "0000001234-24-000011"


# ---------- shape ----------

@pytest.mark.parametrize("data", [b"", b"[]", b"not json", b'{"cik":1}', b'{"cik":1,"entityName":"X"}',
                                  b'{"entityName":"X","facts":{}}'])
def test_changed_shape_is_refused(data):
    with pytest.raises(common.ShapeError):
        sec.parse_companyfacts(data)


def test_a_fact_without_its_required_members_or_a_non_numeric_value_is_refused():
    base = facts_json(1234, "X", [("us-gaap", "Assets", "USD", "Assets", [rec("2023-12-31", "1", "0000001234-24-000001", 2023)])])
    for old, new in ((b'"accn"', b'"acc"'), (b'"val":1', b'"val":"abc"'), (b'"end"', b'"stop"'), (b'"units"', b'"unit"')):
        assert old in base
        with pytest.raises(common.ShapeError):
            sec.parse_companyfacts(base.replace(old, new))


def test_a_company_with_no_annual_facts_in_scope_has_no_rows_and_no_error():
    p = parsed_one([("us-gaap", "Assets", "USD", "Assets", [rec("2023-12-31", "1", "0000001234-24-000001", 2023, form="10-Q", fp="Q1")])])
    assert p["rows"] == [] and p["filings"] == {} and p["filing_for"] == {}
    assert sec.count_cells(facts_json(1234, "X", [])) == 0
