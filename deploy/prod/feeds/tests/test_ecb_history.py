"""ECB euro reference rates, history (eurofxref-hist.zip) and its overlap with the daily file."""
import io
import json
import zipfile

import pytest

import ecb_fx
from conftest import fixture
from helpers import RETRIEVED, sha256, split_lines


def hist():
    return fixture("ecb_hist_slice.csv")


def test_parse_hist_exact_decimals_and_gaps():
    rows = ecb_fx.parse_hist(hist())
    assert len(rows) == 284  # N/A cells and the trailing comma produce nothing
    by = {(r["currency"], r["date"]): r["value"] for r in rows}
    assert by[("USD", "2026-11-18")] == "1.0968"
    assert by[("ISK", "2026-11-18")] == "137"  # as this file prints it
    assert by[("CYP", "1999-01-04")] == "0.58231"
    assert ("CYP", "2026-11-18") not in by and ("BGN", "1999-01-04") not in by
    assert ("USD", "2026-11-14") not in by  # a Saturday: absent, not zero
    assert all(r["value"] not in ("N/A", "") for r in rows)
    assert {r["date"] for r in rows} == {"2026-11-18", "2026-11-17", "2026-11-16", "2026-11-13", "2026-11-12",
                                        "2026-11-11", "2026-11-10", "1999-01-06", "1999-01-05", "1999-01-04"}


def test_hist_changed_shape_refused():
    from common import ShapeError
    for old, new in [(b"Date,", b"Day,"), (b"1.0968", b"abc"), (b"2026-11-18", b"18/11/2026")]:
        assert old in hist()
        with pytest.raises(ShapeError):
            ecb_fx.parse_hist(hist().replace(old, new, 1))
    with pytest.raises(ShapeError):
        ecb_fx.parse_hist(b"")


def zip_of(members):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        for n, b in members.items():
            z.writestr(n, b)
    return buf.getvalue()


def test_parse_hist_zip():
    from common import ShapeError
    z = zip_of({"eurofxref-hist.csv": hist()})
    assert ecb_fx.parse_hist_zip(z) == ecb_fx.parse_hist(hist())
    with pytest.raises(ShapeError):
        ecb_fx.parse_hist_zip(zip_of({"other.csv": hist()}))
    with pytest.raises(ShapeError):
        ecb_fx.parse_hist_zip(b"not a zip")


def test_daily_after_history_adds_nothing_for_a_published_date(names):
    rows = ecb_fx.parse_hist(hist())
    src = {"url": names["ecb_fx"]["history_url"], "retrieved": RETRIEVED, "sha256": sha256(hist())}
    lines, state = ecb_fx.emit(rows, {}, names, src)
    state = json.loads(json.dumps(state))
    assert state["last_date"] == "2026-11-18"
    daily = ecb_fx.parse(fixture("ecb_daily.xml"))
    src2 = {"url": names["ecb_fx"]["url"], "retrieved": RETRIEVED, "sha256": sha256(fixture("ecb_daily.xml"))}
    assert ecb_fx.emit(daily, state, names, src2)[0] == []
    # the history file's rows keep their own source: the history url and hash
    cells = split_lines(lines)[2]
    assert {c["source"]["url"] for c in cells} == {names["ecb_fx"]["history_url"]}
