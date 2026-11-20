"""Source-conformance tests on every run that has a second copy: the second copies of estr, policy_rates and boe
(the ECB's and the Bank's own web pages), the capped, high-water-marked tests, and a run with only new tests."""
import json

import pytest

import boe
import estr
import policy_rates
import relay
from common import ConformanceError, ShapeError
from conftest import fixture
from helpers import RETRIEVED, call_text, check_order, sha256, split_lines


def mutate(name, old, new):
    data = fixture(name)
    assert old in data
    return data.replace(old, new, 1)


# ---- the second copies --------------------------------------------------------------------------------------

def test_estr_page_is_the_latest_rate_and_its_reference_date():
    assert estr.parse_page(fixture("ecb_estr_page.html")) == [{"date": "2026-11-18", "value": "2.442"}]
    for old, new in [(b"Reference date", b"Ref date"), (b"<strong>2.442</strong>", b"<strong>abc</strong>"),
                     (b"18-11-2026", b"2026-11-18"), (b"<th><strong>Rate</strong></th>", b"")]:
        with pytest.raises(ShapeError):
            estr.parse_page(mutate("ecb_estr_page.html", old, new))
    with pytest.raises(ShapeError):
        estr.parse_page(b"")


def test_key_rates_page_has_the_three_rates_by_date_of_change():
    rows = policy_rates.parse_page(fixture("ecb_key_rates.html"))
    by = {(r["rate"], r["date"]): r["value"] for r in rows}
    assert by[("dfr", "2026-09-16")] == "2.50" and by[("mro", "2026-09-16")] == "2.65"
    assert by[("mlfr", "2026-09-16")] == "2.90"
    assert by[("dfr", "2024-09-18")] == "3.50"  # a date with a footnote mark
    assert by[("mro", "2023-05-10")] == "3.75" and by[("dfr", "2025-06-11")] == "2.00"
    assert all(r["rate"] in ("mro", "dfr", "mlfr") for r in rows)
    assert len({(r["rate"], r["date"]) for r in rows}) == len(rows)
    # the variable rate column is not the fixed rate: a '-' in the fixed column is no row
    assert all(r["value"] not in ("-", "") for r in rows)
    assert len(rows) > 60
    for old, new in [(b"Deposit facility", b"Deposit"), (b'<td class="number">2.65</td>', b'<td class="number">x</td>'),
                     (b"16 Sep.", b"16 Xyz.")]:
        with pytest.raises(ShapeError):
            policy_rates.parse_page(mutate("ecb_key_rates.html", old, new))
    with pytest.raises(ShapeError):
        policy_rates.parse_page(b"<table></table>")


def test_bank_rate_page_is_the_dates_the_rate_changed():
    rows = boe.parse_page(fixture("boe_bank_rate.html"))
    assert len(rows) == 259
    assert rows[0] == {"date": "2026-11-05", "value": "4.00"} and rows[-1] == {"date": "1975-01-20", "value": "11.25"}
    assert {"date": "1998-12-10", "value": "6.00"} in rows or any(r["date"].startswith("1998") for r in rows)
    assert all(r["date"] >= "1975-01-01" for r in rows)  # two-digit years: 75 to 99 are 19xx, the rest 20xx
    for old, new in [(b"05 Nov 26", b"2026-11-05"), (b'id="stats-table"', b'id="other"'),
                     (b'<td align="right">  4.00</td>', b'<td align="right">  abc</td>')]:
        with pytest.raises(ShapeError):
            boe.parse_page(mutate("boe_bank_rate.html", old, new))


@pytest.mark.parametrize("mod,first,second,n", [
    (estr, "estr.csv", "ecb_estr_page.html", 1),
    (boe, "boe_iudbedr.csv", "boe_bank_rate.html", 1),
])
def test_the_two_copies_agree_on_the_fixtures(mod, first, second, n):
    a, b = mod.parse(fixture(first)), mod.parse_page(fixture(second))
    pairs = [(r["date"], r["value"]) for r in b if r["date"] in {x["date"] for x in a}]
    assert len(pairs) == n
    from decimal import Decimal
    have = {r["date"]: Decimal(r["value"]) for r in a}
    assert all(have[d] == Decimal(v) for d, v in pairs)


def test_policy_pages_agree_with_the_three_series():
    from decimal import Decimal
    first = []
    for f in ("policy_MRR_FR.csv", "policy_DFR.csv", "policy_MLFR.csv"):
        first += policy_rates.parse(fixture(f))
    page = policy_rates.parse_page(fixture("ecb_key_rates.html"))
    have = {(r["rate"], r["date"]): Decimal(r["value"]) for r in first}
    pairs = [(r["rate"], r["date"]) for r in page if (r["rate"], r["date"]) in have]
    assert len(pairs) == 18
    byp = {(r["rate"], r["date"]): Decimal(r["value"]) for r in page}
    assert all(byp[k] == have[k] for k in pairs)


# ---- the tests of a run: new pairs only, the latest 20 dates a key, a high-water mark ------------------------------

NAMES_KEY = "ecb_fx"


def series(n, key="USD", start=1):
    return [{"currency": key, "date": f"2026-10-{d:02d}" if d < 32 else f"2026-11-{d - 31:02d}", "value": f"1.{d:04d}"}
            for d in range(start, start + n)]


def test_conformance_is_capped_to_the_latest_dates_a_key(names):
    rows = series(25) + series(25, "JPY")
    tests = relay.conformance("ecb_fx", rows, rows, names, ["currency"], cap=20)
    assert len(tests) == 40
    dates = {t["call"].split("date: ")[1].rstrip(")") for t in tests}
    assert min(dates) == "2026-10-06" and max(dates) == "2026-10-25"  # dates 6 to 25 of 1 to 25
    assert len(relay.conformance("ecb_fx", rows, rows, names, ["currency"])) == 50  # uncapped: all


def test_emit_keeps_a_high_water_mark_and_tests_only_new_pairs(names):
    import ecb_fx
    src = {"url": names["ecb_fx"]["url"], "retrieved": RETRIEVED, "sha256": "ab" * 32}
    rows = series(25)
    lines, state = ecb_fx.emit(rows, {}, names, src, second_rows=rows)
    check_order(lines)
    fns, cats, cells, tests = split_lines(lines)
    assert len(cells) == 25 and len(tests) == 20
    state = json.loads(json.dumps(state))
    assert state["tested"], state
    # the same rows again: nothing, not even the older five dates
    assert ecb_fx.emit(rows, state, names, src, second_rows=rows)[0] == []
    # three new days on both copies: three cells and three tests
    more = rows + series(3, start=26)
    lines, state2 = ecb_fx.emit(more, state, names, src, second_rows=more)
    fns, cats, cells, tests = split_lines(lines)
    assert not fns and not cats and len(cells) == 3 and len(tests) == 3
    # a day the second copy gets later, for a cell stored before: a run with tests and no cells
    lines, state3 = ecb_fx.emit(more + series(1, start=29), state2, names, src, second_rows=more + series(1, start=29))
    assert len(split_lines(lines)[2]) == 1
    only = more + series(1, start=29)
    lines, _ = ecb_fx.emit(only, json.loads(json.dumps(state3)), names, src, second_rows=only)
    assert lines == []
    # first copy has the date, the second copy gets it later: tests only, no function or catalogue lines
    s0 = dict(state3)
    two = only + series(1, start=30)
    _, s1 = ecb_fx.emit(two, s0, names, src)  # cell without a second copy: nothing tested yet
    lines, s2 = ecb_fx.emit(two, s1, names, src, second_rows=two)
    assert [list(l) for l in lines] == [["test", "call", "expect"]]
    assert ecb_fx.emit(two, s2, names, src, second_rows=two)[0] == []


def test_a_disagreement_among_the_selected_pairs_is_refused_and_nothing_is_kept(names):
    import ecb_fx
    src = {"url": names["ecb_fx"]["url"], "retrieved": RETRIEVED, "sha256": "ab" * 32}
    rows = series(5)
    bad = [dict(r) for r in rows]
    bad[4]["value"] = "9.9999"
    with pytest.raises(ConformanceError):
        ecb_fx.emit(rows, {}, names, src, second_rows=bad)


def test_keys_have_their_own_high_water_marks(names):
    import treasury
    src = {"url": names["treasury"]["url"], "retrieved": RETRIEVED, "sha256": "cd" * 32}
    rows = [{"tenor": t, "date": d, "value": "4.10"} for t in ("1M", "10Y") for d in ("2026-11-02", "2026-11-03")]
    _, state = treasury.emit(rows[:2] + rows[2:3], {}, names, src, second_rows=rows[:2] + rows[2:3])
    # 10Y has a second date the other key has not: only its new pair is tested
    lines, _ = treasury.emit(rows, state, names, src, second_rows=rows)
    tests = split_lines(lines)[3]
    assert [t["call"] for t in tests] == [call_text(f"{names['treasury']['namespace']}/{names['treasury']['function']}",
                                                    ["tenor", "date"], {"tenor": "10Y", "date": "2026-11-03"})]
