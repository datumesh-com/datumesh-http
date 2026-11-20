"""Quirks of the real sources met on the rehearsal: a Treasury day with no values (Columbus Day), and CHIRPS answering
429 and 503 (rate limits), which the CHIRPS job backs off from and does not lose its months to."""
import email.utils
import json
import urllib.error

import pytest

import chirps
import common
import treasury
from common import RateLimited, ShapeError
from conftest import fixture
from helpers import canon
from test_chirps_store import RASTER, _fake_clock, _names_with_region_in_grid
from test_chirps import grid
from test_treasury_history import NOW as HNOW, World, dirs as hdirs, all_years, month_xml, read as hread

NOW = "2026-11-19T12:00:00.000000Z"


# ---- Treasury: a day with no values ---------------------------------------------------------------------------

EMPTY_DAY = ('<entry><content type="application/xml"><m:properties><d:Id m:type="Edm.Int32">9</d:Id>'
             '<d:NEW_DATE m:type="Edm.DateTime">2010-10-11T00:00:00</d:NEW_DATE></m:properties></content></entry>')


def with_day(xml: bytes, entry: str) -> bytes:
    assert b"</feed>" in xml
    return xml.replace(b"</feed>", entry.encode() + b"</feed>")


def test_a_day_with_no_tenor_values_is_a_day_without_data():
    plain = fixture("treasury_202611.xml")
    base = treasury.parse(plain)
    got = treasury.parse(with_day(plain, EMPTY_DAY))
    assert got == base, "no rows for the day, and no error"
    assert "2010-10-11" not in {r["date"] for r in got}
    # tenor elements that are all null are the same
    nulls = EMPTY_DAY.replace("</m:properties>", '<d:BC_1MONTH m:null="true" /><d:BC_10YEAR m:null="true" /></m:properties>')
    assert treasury.parse(with_day(plain, nulls)) == base


def test_the_empty_day_fixture_entry_parses_in_a_real_month_file():
    data = fixture("treasury_201010_columbus_day.xml")
    rows = treasury.parse(data)
    dates = {r["date"] for r in rows}
    assert "2010-10-11" not in dates and "2010-10-08" in dates and "2010-10-12" in dates
    assert b"2010-10-11T00:00:00" in data


def test_what_is_still_refused_in_a_day_without_values():
    plain = fixture("treasury_202611.xml")
    for entry in (EMPTY_DAY.replace("</m:properties>", "<d:BC_10YR>1</d:BC_10YR></m:properties>"),   # unknown element
                  EMPTY_DAY.replace("</m:properties>", "<d:BC_10YEAR>abc</d:BC_10YEAR></m:properties>"),  # not a number
                  EMPTY_DAY.replace("<d:NEW_DATE", "<d:NEWDATE").replace("</d:NEW_DATE>", "</d:NEWDATE>"),  # no date
                  EMPTY_DAY.replace("2010-10-11T00:00:00", "10/11/2010")):  # bad date
        with pytest.raises(ShapeError):
            treasury.parse(with_day(plain, entry))
    with pytest.raises(ShapeError):  # a month of nothing is a changed shape
        treasury.parse(b'<?xml version="1.0"?><feed xmlns:d="http://schemas.microsoft.com/ado/2007/08/dataservices" '
                       b'xmlns:m="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata" '
                       b'xmlns="http://www.w3.org/2005/Atom">' + EMPTY_DAY.encode() + b"</feed>")


def test_history_goes_through_a_month_with_an_empty_day(names, tmp_path):
    state_dir, out_dir = hdirs(tmp_path)
    w = World(names)
    plain = w.fetch

    def fetch(url):
        data = plain(url)
        if url.endswith("199005"):
            data = with_day(data, EMPTY_DAY.replace("2010-10-11", "1990-05-07"))
        return data

    w.fetch = fetch
    paths = all_years(names, w, state_dir, out_dir)
    assert len(paths) == 3
    cells = [l for l in hread(paths[0]) if "cell" in l]
    assert not any(c["cell"]["args"]["date"]["value"] == "1990-05-07" for c in cells)
    assert len(cells) == 12 * 4 * 3


def test_a_history_state_through_2009_reads_and_resumes_at_2010(names, tmp_path):
    """The shape of the rehearsal's state file: history_year 2009, history_last and last_date at 2009-12-31."""
    state_dir, out_dir = hdirs(tmp_path)
    marks = {t: "2009-12-31" for t in ("10Y", "1M", "1Y", "20Y", "2Y", "30Y", "3M", "3Y", "5Y", "6M", "7Y")}
    (state_dir / "treasury.json").write_text(json.dumps({
        "announced": True, "history_last": "2009-12-31", "history_year": 2009, "last_date": "2009-12-31",
        "tested": marks, "tested_history": marks}))
    w = World(names, years=range(2009, 2012))
    p = treasury.run(names, fetch=w.fetch, now="2010-02-15T12:00:00.000000Z", state_dir=state_dir, out_dir=out_dir,
                     mode="history", sleep=w.sleep, clock=w.clock)
    months = [u[-6:] for _, u in w.log if "pages/xml" in u]
    assert p is not None and months == ["201001", "201002"], "2009 is done: the next call is 2010, to the month of now"
    st = json.loads((state_dir / "treasury.json").read_text())
    assert st["history_year"] == 2010 and st["last_date"] == "2010-02-05"


# ---- CHIRPS: 429 and 503 ----------------------------------------------------------------------------------------

def test_http_get_turns_429_and_503_into_rate_limited_with_the_retry_after(monkeypatch):
    def raiser(code, headers):
        def urlopen(req, timeout=None):
            raise urllib.error.HTTPError(req.full_url, code, "x", headers, None)
        return urlopen

    for code in (429, 503):
        monkeypatch.setattr(urllib.request, "urlopen", raiser(code, {"Retry-After": "90"}))
        with pytest.raises(RateLimited) as e:
            common.http_get("https://example.org/x")
        assert e.value.retry_after == 90.0
        monkeypatch.setattr(urllib.request, "urlopen", raiser(code, {}))
        with pytest.raises(RateLimited) as e:
            common.http_get("https://example.org/x")
        assert e.value.retry_after is None
    when = email.utils.formatdate(timeval=__import__("time").time() + 300, usegmt=True)
    monkeypatch.setattr(urllib.request, "urlopen", raiser(429, {"Retry-After": when}))
    with pytest.raises(RateLimited) as e:
        common.http_get("https://example.org/x")
    assert 200 <= e.value.retry_after <= 301  # an HTTP date: the seconds until then
    monkeypatch.setattr(urllib.request, "urlopen", raiser(429, {"Retry-After": "soon"}))
    with pytest.raises(RateLimited) as e:
        common.http_get("https://example.org/x")
    assert e.value.retry_after is None
    monkeypatch.setattr(urllib.request, "urlopen", raiser(404, {}))
    with pytest.raises(common.NotPublished):
        common.http_get("https://example.org/x")
    monkeypatch.setattr(urllib.request, "urlopen", raiser(500, {}))
    with pytest.raises(urllib.error.HTTPError):
        common.http_get("https://example.org/x")


import urllib.request  # noqa: E402


def test_backoff_doubles_from_two_minutes_to_thirty():
    assert [chirps.backoff_seconds(i) for i in range(7)] == [120.0, 240.0, 480.0, 960.0, 1800.0, 1800.0, 1800.0]
    assert chirps.retry_delay(RateLimited("u", 90.0), 3) == 90.0  # Retry-After is honoured
    assert chirps.retry_delay(RateLimited("u", None), 3) == 960.0
    assert chirps.retry_delay(RateLimited("u", 86400.0), 0) == 3600.0  # but not a day
    assert chirps.retry_delay(RateLimited("u", 0.0), 0) == 0.0


class Flaky:
    """A fake CHIRPS: answers each URL with the next of its script: bytes, or an exception to raise."""

    def __init__(self, script):
        self.script, self.calls = script, []

    def __call__(self, url):
        self.calls.append(url)
        steps = self.script.get(url[-11:-4], [RASTER])  # "2024.06" of ...2024.06.tif
        step = steps[min(len([c for c in self.calls if c == url]) - 1, len(steps) - 1)]
        if isinstance(step, Exception):
            raise step
        return step


def setup(names, tmp_path, last="2024-05-01"):
    names = _names_with_region_in_grid(names, tmp_path)
    s, o = tmp_path / "s", tmp_path / "o"
    s.mkdir(), o.mkdir()
    (s / "chirps.json").write_text(json.dumps({"last_date": last}))
    return names, s, o


def run(names, s, o, fetch, max_months, clock_sleep, **kw):
    clock, sleep, _ = clock_sleep
    return chirps.run(names, fetch=fetch, now=NOW, state_dir=s, out_dir=o, sleep=sleep, clock=clock,
                      max_months=max_months, decode=lambda data: grid(), **kw)


def lines_of(path):
    return [json.loads(l) for l in path.read_text(encoding="utf-8").splitlines()]


def test_a_429_with_retry_after_waits_that_long_and_retries_the_same_month(names, tmp_path):
    names, s, o = setup(names, tmp_path)
    cs = _fake_clock()
    f = Flaky({"2024.06": [RateLimited("u", 90.0), RASTER]})
    p = run(names, s, o, f, 1, cs)
    assert [c for c in f.calls].count(f.calls[0]) == 2 and len(f.calls) == 2
    assert cs[2] == [90.0], "the Retry-After wait, no spacing wait before the first request"
    assert [l for l in lines_of(p) if "cell" in l][0]["cell"]["args"]["month"]["value"] == "2024-06-01"
    assert json.loads((s / "chirps.json").read_text())["last_date"] == "2024-06-01"


@pytest.mark.parametrize("code", [429, 503])
def test_without_retry_after_it_backs_off_exponentially(code, names, tmp_path):
    names, s, o = setup(names, tmp_path)
    cs = _fake_clock()
    f = Flaky({"2024.06": [RateLimited("u", None, code), RateLimited("u", None, code), RateLimited("u", None, code), RASTER]})
    p = run(names, s, o, f, 1, cs)
    assert cs[2] == [120.0, 240.0, 480.0] and len(f.calls) == 4 and p is not None


def test_after_the_retries_the_run_stops_and_keeps_the_months_read(names, tmp_path, capsys):
    names, s, o = setup(names, tmp_path)
    cs = _fake_clock()
    always = [RateLimited("u", None)]
    f = Flaky({"2024.07": always})
    p = run(names, s, o, f, 3, cs, retries=3)
    urls6 = [c for c in f.calls if c.endswith("2024.06.tif")]
    urls7 = [c for c in f.calls if c.endswith("2024.07.tif")]
    assert len(urls6) == 1 and len(urls7) == 4, "the first try and three retries"
    assert not [c for c in f.calls if c.endswith("2024.08.tif")], "the run stopped"
    assert [l for l in lines_of(p) if "cell" in l][0]["cell"]["args"]["month"]["value"] == "2024-06-01"
    st = json.loads((s / "chirps.json").read_text())
    assert st["last_date"] == "2024-06-01"
    assert 120.0 in cs[2] and 240.0 in cs[2] and 480.0 in cs[2]
    assert "rate limited" in capsys.readouterr().err
    # the next run starts at the month that was refused
    f2 = Flaky({})
    run(names, s, o, f2, 1, _fake_clock())
    assert f2.calls[0].endswith("2024.07.tif")


def test_a_first_month_that_is_never_answered_leaves_no_file_and_no_state_change(names, tmp_path, capsys):
    names, s, o = setup(names, tmp_path)
    before = (s / "chirps.json").read_text()
    f = Flaky({"2024.06": [RateLimited("u", None)]})
    assert run(names, s, o, f, 2, _fake_clock(), retries=2) is None
    assert list(o.iterdir()) == [] and (s / "chirps.json").read_text() == before
    assert len(f.calls) == 3 and "rate limited" in capsys.readouterr().err


def test_other_errors_still_lose_the_run(names, tmp_path):
    names, s, o = setup(names, tmp_path)
    f = Flaky({"2024.06": [urllib.error.HTTPError("u", 500, "boom", {}, None)]})
    with pytest.raises(urllib.error.HTTPError):
        run(names, s, o, f, 1, _fake_clock())
    assert list(o.iterdir()) == []
