"""The scope rule (K3, D35): NYSE or Nasdaq companies ranked by us-gaap:Assets (USD) for CY2023Q4I, the top N, N the
largest multiple of 25 whose estimated cells stay within 300,000; and the report of what the rule leaves out."""
import json

import pytest

import common
import sec
from sec_helpers import RANKING_FRAME, TICKERS, FRAMES_ASSETS, frames_json, sec_fixture


def cik10(n):
    return f"{n:010d}"


# ---------- the frames file ----------

def test_parse_frames_of_the_real_file():
    f = sec.parse_frames(sec_fixture(FRAMES_ASSETS))
    assert (f["taxonomy"], f["concept"], f["unit"], f["ccp"]) == RANKING_FRAME
    assert len(f["rows"]) == 6430
    r = next(r for r in f["rows"] if r["cik"] == "0000018230")
    assert r == {"cik": "0000018230", "accession": "0000018230-25-000008", "period_start": "2023-12-31",
                 "period_end": "2023-12-31", "value": "87476000000"}, "an instant has the same date in both"


def test_parse_frames_duration_and_per_share_unit():
    data = json.dumps({"taxonomy": "us-gaap", "tag": "EarningsPerShareDiluted", "ccp": "CY2023", "uom": "USD-per-shares",
                       "label": "x", "description": "x", "pts": 1,
                       "data": [{"accn": "0000001234-24-000001", "cik": 1234, "entityName": "X", "loc": "US-NY",
                                 "start": "2023-01-01", "end": "2023-12-31", "val": 1.25}]}).encode()
    f = sec.parse_frames(data)
    assert f["unit"] == "USD/shares", "the frames API spells the unit USD-per-shares; companyfacts says USD/shares"
    assert f["rows"] == [{"cik": "0000001234", "accession": "0000001234-24-000001", "period_start": "2023-01-01",
                          "period_end": "2023-12-31", "value": "1.25"}]


@pytest.mark.parametrize("data", [b"", b"[]", b'{"data":[]}', b'{"taxonomy":"us-gaap","tag":"Assets","ccp":"CY2023Q4I","uom":"USD"}'])
def test_parse_frames_changed_shape_refused(data):
    with pytest.raises(common.ShapeError):
        sec.parse_frames(data)


def test_the_ranking_frame_is_first_among_the_conformance_frames():
    assert sec.CONFORMANCE_FRAMES[0] == RANKING_FRAME
    assert len(sec.CONFORMANCE_FRAMES) >= 3
    assert len(set(sec.CONFORMANCE_FRAMES)) == len(sec.CONFORMANCE_FRAMES)
    assert all(len(t) == 4 and t[3].startswith("CY") for t in sec.CONFORMANCE_FRAMES)


# ---------- ranking ----------

@pytest.fixture(scope="module")
def ranking():
    return sec.rank_companies(sec_fixture(TICKERS), sec_fixture(FRAMES_ASSETS))


def test_ranking_of_the_real_files(ranking):
    r = ranking["ranked"]
    assert len(r) == 4170
    assert [x["cik"] for x in r[:4]] == ["0000019617", "0000070858", "0000831001", "0000072971"]
    assert r[0]["assets"] == 3875393000000 and r[0]["ticker"] == "JPM" and r[0]["exchange"] == "NYSE"
    assert r[1]["ticker"] == "BAC", "a company listed on NYSE and traded OTC counts once, by its NYSE row"
    assert all(a["assets"] >= b["assets"] for a, b in zip(r, r[1:]))
    assert len({x["cik"] for x in r}) == len(r)
    assert {x["exchange"] for x in r} == {"NYSE", "Nasdaq"}
    assert [x["cik"] for x in r].index("0000040545") == 57 and [x["cik"] for x in r].index("0000018230") == 108


def test_ties_in_assets_go_to_the_lower_cik(ranking):
    r = ranking["ranked"]
    for a, b in zip(r, r[1:]):
        if a["assets"] == b["assets"]:
            assert a["cik"] < b["cik"]


def test_ranking_counts_what_it_leaves_out(ranking):
    assert ranking["frame_entities"] == 6430
    assert ranking["frame_not_listed"] == 2260
    assert ranking["listed_without_assets"] == 1854
    assert len(ranking["ranked"]) + ranking["frame_not_listed"] == ranking["frame_entities"]


def test_ranking_rule_on_a_small_example():
    tickers = json.dumps({"fields": ["cik", "name", "ticker", "exchange"], "data": [
        [1, "A", "A", "NYSE"], [2, "B", "B", "Nasdaq"], [3, "C", "C", "OTC"], [4, "D", "D", None], [5, "E", "E", "CBOE"],
        [6, "F", "F", "Nasdaq"], [7, "G", "G", "NYSE"], [7, "G", "GP", "OTC"], [8, "H", "H", "NYSE"]]}).encode()
    frames = frames_json([(1, "A", "0000000001-24-000001", "2023-12-31", 50), (2, "B", "0000000002-24-000001", "2023-12-31", 70),
                          (3, "C", "0000000003-24-000001", "2023-12-31", 900), (4, "D", "0000000004-24-000001", "2023-12-31", 800),
                          (5, "E", "0000000005-24-000001", "2023-12-31", 700), (7, "G", "0000000007-24-000001", "2023-12-31", 70),
                          (99, "Z", "0000000099-24-000001", "2023-12-31", 1000)])
    r = sec.rank_companies(tickers, frames)
    assert [(x["cik"], x["assets"]) for x in r["ranked"]] == [(cik10(2), 70), (cik10(7), 70), (cik10(1), 50)]
    assert r["frame_entities"] == 7 and r["frame_not_listed"] == 4
    assert r["listed_without_assets"] == 2, "6 (Nasdaq) and 8 (NYSE) are listed and have no Assets in the frame"


def test_ranking_refuses_changed_shapes():
    with pytest.raises(common.ShapeError):
        sec.rank_companies(b'{"fields":["cik","name"],"data":[]}', sec_fixture(FRAMES_ASSETS))
    with pytest.raises(common.ShapeError):
        sec.rank_companies(sec_fixture(TICKERS), b"{}")
    with pytest.raises(common.ShapeError):
        sec.rank_companies(b"", sec_fixture(FRAMES_ASSETS))


# ---------- the estimator: N ----------

def ciks(n):
    return [cik10(i + 1) for i in range(n)]


def test_n_is_the_largest_multiple_of_25_within_the_budget():
    cs = ciks(4000)
    assert sec.choose_n(cs, {c: 100 for c in cs}) == {"n": 3000, "estimated": 300000}, "at the budget is within it"
    assert sec.choose_n(cs, {c: 101 for c in cs}) == {"n": 2950, "estimated": 297950}
    assert sec.choose_n(cs, {c: 99 for c in cs}) == {"n": 3025, "estimated": 299475}


def test_n_with_unequal_sizes_and_a_small_budget():
    cs = ciks(500)
    counts = {c: (20 if i < 25 else 10) for i, c in enumerate(cs)}
    assert sec.choose_n(cs, counts, budget=1000, step=25) == {"n": 75, "estimated": 1000}
    assert sec.choose_n(cs, counts, budget=999, step=25) == {"n": 50, "estimated": 750}


def test_n_needs_counts_only_as_far_as_the_budget_goes():
    cs = ciks(4000)
    counts = {c: 100 for c in cs[:150]}
    assert sec.choose_n(cs, counts, budget=12345, step=25) == {"n": 100, "estimated": 10000}


def test_n_when_everything_fits_is_all_of_them():
    cs = ciks(4010)
    assert sec.choose_n(cs, {c: 1 for c in cs}) == {"n": 4010, "estimated": 4010}
    cs = ciks(30)
    assert sec.choose_n(cs, {c: 1 for c in cs}) == {"n": 30, "estimated": 30}


def test_n_zero_when_not_even_one_step_fits():
    cs = ciks(100)
    assert sec.choose_n(cs, {c: 1000 for c in cs}, budget=24_999, step=25) == {"n": 0, "estimated": 0}


def test_n_steps_and_default_budget():
    cs = ciks(300)
    assert sec.choose_n(cs, {c: 10 for c in cs}, budget=1000, step=10) == {"n": 100, "estimated": 1000}
    assert sec.choose_n(cs, {c: 1_000 for c in cs}) == {"n": 300, "estimated": 300_000}


# ---------- the left-out report ----------

def test_scope_report_names_what_the_rule_leaves_out(ranking):
    rep = sec.scope_report(ranking, {"n": 125, "estimated": 2220}, no_facts=["0000072971"])
    json.dumps(rep)
    assert rep["n"] == 125 and rep["estimated_cells"] == 2220 and rep["budget"] == 300000 and rep["step"] == 25
    assert rep["forms"] == ["10-K", "10-K/A"] and rep["fiscal_years"] == [2019, 2024]
    assert rep["concepts"] == 46 and rep["ranked"] == 4170
    lo = rep["left_out"]
    assert lo["below_n"] == 4170 - 125
    assert lo["listed_without_assets"] == 1854 and lo["not_listed_on_nyse_or_nasdaq"] == 2260
    assert lo["no_companyfacts"] == ["0000072971"]
    for k in ("foreign_private_issuers", "other_concepts", "dimensional_facts", "quarterly_facts", "years_outside_2019_2024"):
        assert isinstance(lo[k], str) and lo[k].strip(), k
    assert "20-F" in lo["foreign_private_issuers"] and "40-F" in lo["foreign_private_issuers"]
    assert rep["lowest_included"]["cik"] == ranking["ranked"][124]["cik"]
    assert rep["highest_excluded"]["cik"] == ranking["ranked"][125]["cik"]
    assert "rule" in rep and "Assets" in rep["rule"] and "CY2023Q4I" in rep["rule"]


def test_scope_report_when_nothing_is_left_below(ranking):
    n = len(ranking["ranked"])
    rep = sec.scope_report(ranking, {"n": n, "estimated": 1})
    assert rep["left_out"]["below_n"] == 0 and rep["highest_excluded"] is None and rep["left_out"]["no_companyfacts"] == []
