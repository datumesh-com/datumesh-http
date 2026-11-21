"""CONTRACT-sec-classes C2b: the filing class's filed is the submissions' filingDate, relayed as filed, with the
submissions URL (or its page) as its source; a filing the submissions do not list has no filed cell and is counted in
left_out_filed. Where the companyfacts filed date differs, nothing is raised: the difference is listed in filed_differs
(always present). The annual-report selection orders by the relayed filed date where present. cik and form disagreements
are collected and raised once, with at most 50 shown and the count.

The regression is KLA (cik 0000319201): its fiscal 2020 10-K was accepted after 5:30 pm Eastern on 2020-08-07; the
companyfacts file says 2020-08-10, the submissions say 2020-08-07. Both are real; they are not two copies of one value."""
import hashlib
import json

import pytest

import relay
import sec
from common import ConformanceError
from sec_class_helpers import ATHENE, NOW, P3, WORLD, Co, World, company_file, run_world, subs_json
from sec_helpers import args_of, facts_json, filing_cls, read_lines, rec, sub_url
from sec_helpers import names  # noqa: F401  (the SEC names: sec.names)

KLA_CIK = 319201
KLA_A, KLA_B, KLA_C = "0000319201-20-000047", "0000319201-20-000060", "0000319201-21-000045"
STAMP = relay._stamp(NOW)


def kla_facts() -> bytes:
    """A: the 10-K of fiscal 2020 (companyfacts filed 2020-08-10). B: its 10-K/A (companyfacts filed 2020-08-09), so the
    companyfacts order puts A last and the relayed order puts B last. C: the 10-K of fiscal 2021, which the submissions do
    not list."""
    return facts_json(KLA_CIK, "KLA CORP", [
        ("us-gaap", "Assets", "USD", "Assets", [
            rec("2020-06-30", "1000", KLA_A, 2020, filed="2020-08-10"),
            rec("2020-06-30", "1000", KLA_B, 2020, form="10-K/A", filed="2020-08-09"),
            rec("2021-06-30", "1100", KLA_C, 2021, filed="2021-08-06")])])


def kla_subs() -> bytes:
    return subs_json("0000319201", [(KLA_A, "10-K", "2020-08-07", "2020-06-30"),
                                    (KLA_B, "10-K/A", "2020-08-11", "2020-06-30")])


KLA = Co(KLA_CIK, "KLA CORP", ["KLAC"], facts=kla_facts(), subs=kla_subs())


def kla_item(names):
    return {"parsed": sec.parse_companyfacts(kla_facts()),
            "source": {"url": names["sec"]["companyfacts_url"].format(cik="0000319201"), "retrieved": NOW,
                       "sha256": hashlib.sha256(kla_facts()).hexdigest()}}


def fn(names, m):
    return f"{filing_cls(names)}/{m}"


@pytest.fixture(scope="module")
def kla(names, tmp_path_factory):
    path, f, s, o = run_world(names, tmp_path_factory.mktemp("kla"), world=World(cos=WORLD + [KLA]))
    return {"lines": read_lines(path), "clines": read_lines(company_file(o)),
            "rep": json.loads((o / f"sec-scope-{STAMP}.json").read_text(encoding="utf-8")), "names": names}


def test_the_filed_cell_is_the_submissions_filing_date_with_its_source(names, kla):
    cells = [l for l in kla["lines"] if "cell" in l and l["cell"]["fn"] == fn(names, "filed") and args_of(l)["accession"] == KLA_A]
    assert len(cells) == 1, "one filed cell, from the submissions: none from the companyfacts file"
    c = cells[0]
    assert c["value"] == {"type": "date", "value": "2020-08-07"}
    assert c["source"]["url"] == sub_url(names, "0000319201")
    assert c["source"]["sha256"] == hashlib.sha256(kla_subs()).hexdigest()


def test_the_filed_test_pins_the_submissions_value(names, kla):
    call = f'{fn(names, "filed")}(accession: "{KLA_A}")'
    tests = [t for t in kla["lines"] if "test" in t and t["call"] == call]
    assert tests == [{"test": fn(names, "filed"), "call": call, "expect": {"type": "date", "value": "2020-08-07"}}]


def test_filed_differs_lists_each_difference_and_is_always_present(names, kla):
    assert kla["rep"]["filed_differs"] == {
        KLA_A: {"companyfacts": "2020-08-10", "submissions": "2020-08-07"},
        KLA_B: {"companyfacts": "2020-08-09", "submissions": "2020-08-11"}}


def test_a_default_run_has_no_difference_and_an_empty_dict(names, tmp_path):
    path, f, s, o = run_world(names, tmp_path)
    rep = json.loads((o / f"sec-scope-{STAMP}.json").read_text(encoding="utf-8"))
    assert rep["filed_differs"] == {}
    assert rep["left_out"]["left_out_filed"] == {"count": 1, "accessions": [P3.old]}, (
        "the one filing that no submissions file lists")


def test_a_filing_the_submissions_do_not_list_has_no_filed_cell_and_is_counted(names, kla):
    assert kla["rep"]["left_out"]["left_out_filed"] == {"count": 2, "accessions": sorted([KLA_C, P3.old])}
    assert not [l for l in kla["lines"] if "cell" in l and l["cell"]["fn"] == fn(names, "filed") and args_of(l)["accession"] == KLA_C]


def test_the_annual_report_is_the_latest_by_the_relayed_filed_date(names, kla):
    ann = f"{names['meta']['host']}/sec/company/annual_report"
    got = {args_of(l)["fiscal_year"]: l["value"]["value"] for l in kla["clines"]
           if "cell" in l and l["cell"]["fn"] == ann and args_of(l)["cik"] == "0000319201"}
    assert got == {"2020": KLA_B, "2021": KLA_C}, "B (submissions 2020-08-11) is later than A (2020-08-07) by the relayed date"


def test_latest_filings_orders_by_the_relayed_date_where_present(names):
    items = [kla_item(names)]
    items[0]["submissions"] = {"cik": "0000319201", "sources": {}, "filings": sec.parse_submissions(kla_subs())["filings"]}
    sec.latest_filings(items)
    assert items[0]["parsed"]["filing_for"] == {"2020": KLA_B, "2021": KLA_C}
    no_subs = [kla_item(names)]
    sec.latest_filings(no_subs)
    assert no_subs[0]["parsed"]["filing_for"] == {"2020": KLA_A, "2021": KLA_C}, "companyfacts order when not listed"


def test_the_kla_filing_conformance_raises_nothing_and_pins_the_relayed_filed(names):
    it = kla_item(names)
    it["submissions"] = None
    sub = sec.parse_submissions(kla_subs())
    tests = sec.filing_conformance([it], [sub], names, minimum=1)
    by = {t["call"]: t["expect"] for t in tests}
    assert by[f'{fn(names, "filed")}(accession: "{KLA_A}")'] == {"type": "date", "value": "2020-08-07"}


def test_cik_and_form_disagreements_are_collected_and_raised_once(names, tmp_path):
    """Every filing of every company but Athene has a form the companyfacts file does not: one error for all of them, 50
    listed and the count."""
    cos = [c for c in WORLD if c is not ATHENE]
    n = 2 * len(cos)
    overrides = {c.cik10: subs_json(c.cik10, [(c.accn, "10-K/A", "2024-02-15", "2023-12-31"),
                                              (c.old, "10-K/A", "2023-02-15", "2023-12-31")]) for c in cos}
    published = []
    with pytest.raises(ConformanceError) as e:
        run_world(names, tmp_path, world=World(sub_overrides=overrides), publish=published.append)
    m = str(e.value)
    assert f"{n} disagreements" in m and f"(and {n - 50} more)" in m and "nothing is published" in m
    assert m.count("the companyfacts file says") == 50
    assert published == []
