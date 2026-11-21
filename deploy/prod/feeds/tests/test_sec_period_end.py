"""CONTRACT-sec-classes C2: the filing class's period_end is the submissions API's `reportDate`, relayed as filed, with
the submissions URL as its source. Older filings come from the pages of `filings.files`, fetched one request each, in
order, only while an in-scope accession is missing from `filings.recent`. A filing the submissions have no value for has no
period_end cell and is counted in the run report as `left_out_period_end`. cik, form and filed are still compared with
the companyfacts file (a disagreement is a ConformanceError); period_end has no second copy.

Run over the synthetic world of sec_class_helpers (Athene's two FY2020 filings are in it: the regression)."""
import json

import pytest

import sec
from common import ConformanceError
from sec_helpers import FILING_METHODS, FakeClock, args_of, cf_url, filing_cls, kinds, max_in_any_second, read_lines, sub_url
from sec_helpers import names  # noqa: F401
from sec_class_helpers import (ATHENE, ATHENE_10K, ATHENE_10KA, D1, E1, NOW, P1, P2, P3, P4, PAGE_URL, WORLD, World, page_name,
                               run_world, sha, subs_json)

import relay

STAMP = relay._stamp(NOW)


@pytest.fixture(scope="module")
def ran(names, tmp_path_factory):
    fc = FakeClock()
    path, f, s, o = run_world(names, tmp_path_factory.mktemp("pe"), fc=fc)
    return {"path": path, "f": f, "o": o, "fc": fc, "lines": read_lines(path), "names": names}


def pe_cells(ran):
    cls = filing_cls(ran["names"])
    return {args_of(c)["accession"]: c for c in ran["lines"] if "cell" in c and c["cell"]["fn"] == f"{cls}/period_end"}


def other_cells(ran, accn, method):
    cls = filing_cls(ran["names"])
    return [c for c in ran["lines"] if "cell" in c and c["cell"]["fn"] == f"{cls}/{method}" and args_of(c)["accession"] == accn]


def test_the_run_has_no_error_on_athene_and_both_filings_get_the_report_date(ran):
    pe = pe_cells(ran)
    for accn in (ATHENE_10K, ATHENE_10KA):
        assert pe[accn]["value"] == {"type": "date", "value": "2020-12-31"}, (
            "the 10-K/A's latest one-year period ends 2021-03-31 in companyfacts; its report date is 2020-12-31")
    # the other methods of the amendment still come from companyfacts
    (form,) = other_cells(ran, ATHENE_10KA, "form")
    assert form["value"] == {"type": "string", "value": "10-K/A"}
    (filed,) = other_cells(ran, ATHENE_10KA, "filed")
    assert filed["value"] == {"type": "date", "value": "2021-04-20"}


def test_period_end_is_the_submissions_record_with_the_submissions_url_as_its_source(names, ran):
    pe = pe_cells(ran)
    f = ran["f"]
    plain = [c for c in WORLD if c not in (P1, P2, P3, P4, E1, D1)]
    for co in plain:
        url = sub_url(names, co.cik10)
        for accn in ([ATHENE_10K, ATHENE_10KA] if co is ATHENE else [co.accn, co.old]):
            c = pe[accn]
            assert c["source"]["url"] == url and c["source"]["sha256"] == sha(f.served[url]), accn
            assert c["source"]["name"] == "sec.gov" and c["licence"] == names["sec"]["licence"]
            assert c["cell"]["args"] == {"accession": {"type": "string", "value": accn}}
            assert c["value"] == {"type": "date", "value": "2020-12-31" if co is ATHENE else "2023-12-31"}
        # while the filing's other cells keep the companyfacts file as their source
        assert other_cells(ran, co.accn if co is not ATHENE else ATHENE_10K, "form")[0]["source"]["url"] == cf_url(names, co.cik10)


def test_a_report_date_that_differs_from_the_companyfacts_periods_is_relayed_as_filed(names, ran):
    pe = pe_cells(ran)
    assert pe[D1.accn]["value"] == {"type": "date", "value": "2023-12-30"}, "companyfacts says 2023-12-31: no error, no derivation"
    assert pe[D1.old]["value"]["value"] == "2023-12-31"
    assert pe[D1.accn]["source"]["url"] == sub_url(names, D1.cik10)


def test_a_filing_not_in_recent_is_found_on_the_first_page_that_has_it(names, ran):
    f = ran["f"]
    p1 = PAGE_URL.format(name=page_name(P1.cik10, 1))
    assert f.pages_fetched(P1) == [p1], "found on page 1: page 2 is not asked for"
    c = pe_cells(ran)[P1.old]
    assert c["value"] == {"type": "date", "value": "2022-12-31"}
    assert c["source"]["url"] == p1 and c["source"]["sha256"] == sha(f.served[p1]), (
        "relayed from the page it was read from, with that page's hash")
    # the filing in recent is still from the main file
    assert pe_cells(ran)[P1.accn]["source"]["url"] == sub_url(names, P1.cik10)


def test_pages_are_fetched_in_order_one_request_each_until_the_accession_is_found(names, ran):
    f = ran["f"]
    urls = [PAGE_URL.format(name=page_name(P2.cik10, n)) for n in (1, 2)]
    assert f.pages_fetched(P2) == urls
    i = f.urls.index(sub_url(names, P2.cik10))
    assert f.urls[i + 1:i + 3] == urls, "the pages follow the main file, in the order `files` lists them"
    assert pe_cells(ran)[P2.old]["value"] == {"type": "date", "value": "2022-12-31"}
    assert pe_cells(ran)[P2.old]["source"]["url"] == urls[1]


def test_a_filing_found_nowhere_has_no_period_end_cell_and_all_pages_were_tried(names, ran):
    f = ran["f"]
    assert f.pages_fetched(P3) == [PAGE_URL.format(name=page_name(P3.cik10, n)) for n in (1, 2)]
    pe = pe_cells(ran)
    assert P3.old not in pe and P3.accn in pe
    assert other_cells(ran, P3.old, "cik"), "the filing's other cells are still relayed"


def test_pages_are_not_fetched_when_every_accession_is_in_recent(ran):
    f = ran["f"]
    assert f.pages_fetched(P4) == []
    allowed = {co.cik10 for co in (P1, P2, P3)}
    assert {u.split("/CIK")[1][:10] for u in f.urls if "-submissions-" in u} == allowed
    assert not [u for u in f.urls if "-submissions-" in u and "CIK" + P4.cik10 in u]


def test_an_empty_report_date_has_no_cell(ran):
    pe = pe_cells(ran)
    assert E1.accn not in pe and E1.old in pe
    assert other_cells(ran, E1.accn, "form"), "the form, filed and so on are still relayed"


def test_the_missing_values_are_counted_in_the_run_report(ran):
    rep = json.loads((ran["o"] / f"sec-scope-{STAMP}.json").read_text(encoding="utf-8"))
    assert rep["left_out"]["left_out_period_end"] == {"count": 2, "accessions": sorted([P3.old, E1.accn])}, (
        "the accessions with no period_end cell: not found in recent or any page, and an empty reportDate")


def test_period_end_tests_pin_the_relayed_value_and_none_is_written_for_a_missing_one(names, ran):
    cls = filing_cls(names)
    tests = {t["call"]: t["expect"] for t in ran["lines"] if "test" in t and t["test"] == f"{cls}/period_end"}
    call = lambda a: f'{cls}/period_end(accession: "{a}")'
    assert tests[call(ATHENE_10KA)] == {"type": "date", "value": "2020-12-31"}
    assert tests[call(ATHENE_10K)] == {"type": "date", "value": "2020-12-31"}
    assert tests[call(D1.accn)] == {"type": "date", "value": "2023-12-30"}
    assert call(P3.old) not in tests and call(E1.accn) not in tests
    assert call(P2.old) in tests and tests[call(P2.old)]["value"] == "2022-12-31"


def test_the_period_end_catalogue_examples_are_cells(names, ran):
    cls = filing_cls(names)
    (e,) = [l for l in ran["lines"] if l.get("catalog") == f"{cls}/period_end"]
    assert len(e["examples"]) >= 2
    pe = pe_cells(ran)
    for x in e["examples"]:
        assert x["expected"] == pe[x["call"].split(chr(34))[1]]["value"]


def test_the_rate_holds_with_the_pages(ran):
    assert max_in_any_second(ran["f"].times) <= 5


# ---------- cik, form and filed are still compared; period_end is not ----------

def _changed(co, **kw):
    """Submissions of a plain synthetic company with one thing changed."""
    rows = [(co.accn, "10-K", "2024-02-15", "2023-12-31"), (co.old, "10-K", "2023-02-15", "2023-12-31")]
    cik10 = kw.pop("cik10", co.cik10)
    if "form" in kw:
        rows[0] = (rows[0][0], kw["form"]) + rows[0][2:]
    if "filed" in kw:
        rows[0] = rows[0][:2] + (kw["filed"],) + rows[0][3:]
    if "report" in kw:
        rows[0] = rows[0][:3] + (kw["report"],)
    return subs_json(cik10, rows)


@pytest.mark.parametrize("what,kw", [("form", {"form": "10-K/A"}), ("filed", {"filed": "2024-02-16"}),
                                     ("cik", {"cik10": "0000999999"})])
def test_a_disagreement_on_cik_form_or_filed_still_stops_the_run(names, tmp_path, what, kw):
    co = WORLD[5]
    w = World(sub_overrides={co.cik10: _changed(co, **kw)})
    published = []
    with pytest.raises(ConformanceError):
        run_world(names, tmp_path, world=w, publish=published.append)
    assert published == [] and not list((tmp_path / "out").iterdir())


def test_a_different_report_date_stops_nothing(names, tmp_path):
    co = WORLD[5]
    w = World(sub_overrides={co.cik10: _changed(co, report="2023-12-29")})
    path, f, s, o = run_world(names, tmp_path, world=w)
    cls = filing_cls(names)
    got = {args_of(c)["accession"]: c["value"]["value"] for c in read_lines(path)
           if "cell" in c and c["cell"]["fn"] == f"{cls}/period_end"}
    assert got[co.accn] == "2023-12-29" and got[co.old] == "2023-12-31"


def test_parse_submissions_still_reads_recent_and_an_empty_report_date_is_none(names):
    p = sec.parse_submissions(E1.subs)
    assert p["filings"][E1.accn]["period_end"] is None and p["filings"][E1.old]["period_end"] == "2023-12-31"
