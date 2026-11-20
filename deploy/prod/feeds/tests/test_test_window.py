"""When the daily fetch window of the first copy does not reach the dates of the second copy, the run reads a test
window from the first copy (rows used only for tests, never emitted), and every run says on stderr what it wrote."""
import json
import pathlib
import re
import datetime as dt

import pytest

import boe
import ecb_fx
import estr
import treasury
from conftest import fixture
from helpers import canon
from test_run import Fetcher, dirs, read
from test_treasury_history import DAYS, World, year_csv

NOW = "2026-11-19T12:00:00.000000Z"
SUMMARY = re.compile(r"^(\w+): (\d+) cells, (\d+) tests written; second copy: (.*)$", re.M)


def month_abbr(d):
    return ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][d.month - 1]


class BoeServer:
    """IUDBEDR as the Bank would send it for the dates asked, consistent with the page's change dates."""

    def __init__(self, names):
        self.names, self.urls = names, []
        self.page = boe.parse_page(fixture("boe_bank_rate.html"))  # newest first

    def rate_on(self, d):
        return next(r["value"] for r in self.page if r["date"] <= d)

    def __call__(self, url):
        self.urls.append(url)
        if url == self.names["boe"]["second_url"]:
            return fixture("boe_bank_rate.html")
        m = re.search(r"Datefrom=(\d\d)/(\w{3})/(\d{4})&Dateto=(\w+)", url)
        assert m, url
        day = dt.date(int(m.group(3)), boe.MONTHS[m.group(2)], int(m.group(1)))
        end = dt.date.fromisoformat(NOW[:10])
        rows = ["DATE,IUDBEDR"]
        while day <= end:
            rows.append(f"{day.day:02d} {month_abbr(day)} {day.year},{canon(self.rate_on(day.isoformat()))}")
            day += dt.timedelta(days=1)
        return ("\r\n".join(rows) + "\r\n").encode()


def test_boe_rerun_whose_window_misses_the_change_dates_reads_a_test_window(names, tmp_path, capsys):
    state_dir, out_dir = dirs(tmp_path)
    (state_dir / "boe.json").write_text(json.dumps({"last_date": "2026-11-18", "announced": True}))
    srv = BoeServer(names)
    path = boe.run(names, fetch=srv, now=NOW, state_dir=state_dir, out_dir=out_dir)
    lines = read(path)
    cells = [l for l in lines if "cell" in l]
    tests = [l for l in lines if "test" in l]
    assert [c["cell"]["args"]["date"]["value"] for c in cells] == ["2026-11-19"], "the window rows are not cells"
    assert len(tests) == 20
    want = {r["date"] for r in srv.page[:20]}
    assert {t["call"].split("date: ")[1].rstrip(")") for t in tests} == want
    oldest = dt.date.fromisoformat(srv.page[19]["date"])
    frm = f"Datefrom={oldest.day:02d}/{month_abbr(oldest)}/{oldest.year}&"
    assert any(frm in u for u in srv.urls), "the test window starts at the page's 20th latest change date"
    st = json.loads((state_dir / "boe.json").read_text())
    assert st["last_date"] == "2026-11-19" and st["tested"] == {"": srv.page[0]["date"]}
    err = capsys.readouterr().err
    assert re.search(r"boe: 1 cells, 20 tests written; second copy: 20 pairs compared", err)
    # nothing is tested twice, and no window is fetched when nothing is untested
    n = len(srv.urls)
    assert boe.run(names, fetch=srv, now=NOW, state_dir=state_dir, out_dir=out_dir) is None
    assert len(srv.urls) == n + 2  # the daily window and the page; no test window
    assert "0 cells, 0 tests written" in capsys.readouterr().err


def test_boe_window_rows_that_disagree_refuse_the_run(names, tmp_path):
    from common import ConformanceError
    state_dir, out_dir = dirs(tmp_path)
    (state_dir / "boe.json").write_text(json.dumps({"last_date": "2026-11-18", "announced": True}))
    srv = BoeServer(names)

    def lying(url):
        data = srv(url)
        return data if "Datefrom=18/Nov" in url else data.replace(b"05 Nov 2026,4\r\n", b"05 Nov 2026,4.5\r\n")

    with pytest.raises(ConformanceError):
        boe.run(names, fetch=lying, now=NOW, state_dir=state_dir, out_dir=out_dir)
    assert list(out_dir.iterdir()) == []


def hist_90d_xml(dates):
    rows = [r for r in ecb_fx.parse_hist(fixture("ecb_hist_slice.csv")) if r["date"] in dates]
    out = ['<?xml version="1.0" encoding="UTF-8"?>',
           '<gesmes:Envelope xmlns:gesmes="http://www.gesmes.org/xml/2002-08-01" '
           'xmlns="http://www.ecb.int/vocabulary/2002-08-01/eurofxref"><Cube>']
    for d in sorted(dates, reverse=True):
        out.append(f"<Cube time='{d}'>")
        out += [f"<Cube currency='{r['currency']}' rate='{r['value']}'/>" for r in rows if r["date"] == d]
        out.append("</Cube>")
    out.append("</Cube></gesmes:Envelope>")
    return "\n".join(out).encode()


def test_ecb_daily_whose_second_copy_has_earlier_days_reads_the_90_day_file(names, tmp_path, capsys):
    state_dir, out_dir = dirs(tmp_path)
    sec = names["ecb_fx"]
    assert sec["window_url"].endswith("eurofxref-hist-90d.xml")
    seen = []

    def fetch(url):
        seen.append(url)
        if url == sec["second_url"]:
            return fixture("ecb_sdmx_last3.csv")
        if url == sec["window_url"]:
            return hist_90d_xml({"2026-11-17", "2026-11-18"})
        return fixture("ecb_daily.xml")

    path = ecb_fx.run(names, fetch=fetch, now=NOW, state_dir=state_dir, out_dir=out_dir)
    lines = read(path)
    cells = [l for l in lines if "cell" in l]
    tests = [l for l in lines if "test" in l]
    assert len(cells) == 29 and {c["cell"]["args"]["date"]["value"] for c in cells} == {"2026-11-18"}
    # the SDMX copy has 17, 18 and 19 November; the first copy has the first two (19 is not published as a file yet)
    assert len(tests) == 58 and {t["call"].split("date: ")[1][:10] for t in tests} == {"2026-11-17", "2026-11-18"}
    assert sec["window_url"] in seen
    assert re.search(rf"ecb_fx: 29 cells, {len(tests)} tests written; second copy: {len(tests)} pairs compared",
                     capsys.readouterr().err)


def test_ecb_daily_does_not_fetch_a_window_when_the_copies_overlap(names, tmp_path):
    state_dir, out_dir = dirs(tmp_path)
    f = Fetcher()
    ecb_fx.run(names, fetch=f, now=NOW, state_dir=state_dir, out_dir=out_dir)
    assert names["ecb_fx"]["window_url"] not in f.urls


def test_treasury_daily_reads_the_earlier_months_the_year_csv_has(names, tmp_path):
    state_dir, out_dir = dirs(tmp_path)
    (state_dir / "treasury.json").write_text(json.dumps({"last_date": "1992-03-05", "announced": True}))
    w = World(names)
    keep = [l for l in year_csv(1992).decode().splitlines() if l.startswith("Date") or int(l[:2]) <= 3]
    trimmed = ("\n".join(keep) + "\n").encode()
    csv_url = names["treasury"]["second_url"].format(year=1992)

    def fetch(url):
        return trimmed if url == csv_url else w.fetch(url)

    now = "1992-03-20T12:00:00.000000Z"
    path = treasury.run(names, fetch=fetch, now=now, state_dir=state_dir, out_dir=out_dir)
    lines = read(path)
    assert not [l for l in lines if "cell" in l] and len([l for l in lines if "test" in l]) == 3 * 3 * len(DAYS)
    months = [u[-6:] for _, u in w.log if "pages/xml" in u]
    assert sorted(months) == ["199201", "199202", "199203"] and months.count("199203") == 1
    assert json.loads((state_dir / "treasury.json").read_text())["last_date"] == "1992-03-05"
    assert treasury.run(names, fetch=fetch, now=now, state_dir=state_dir, out_dir=out_dir) is None


@pytest.mark.parametrize("key", ["ecb_fx", "estr", "policy_rates", "treasury", "boe"])
def test_every_run_says_what_it_wrote_on_stderr(key, names, tmp_path, capsys):
    import importlib
    mod = importlib.import_module(key)
    state_dir, out_dir = dirs(tmp_path)
    mod.run(names, fetch=Fetcher(), now=NOW, state_dir=state_dir, out_dir=out_dir)
    first = SUMMARY.findall(capsys.readouterr().err)
    assert len(first) == 1 and first[0][0] == key and int(first[0][1]) > 0 and int(first[0][2]) > 0
    assert re.match(r"\d+ pairs compared", first[0][3])
    mod.run(names, fetch=Fetcher(), now=NOW, state_dir=state_dir, out_dir=out_dir)
    again = SUMMARY.findall(capsys.readouterr().err)
    assert len(again) == 1 and again[0][1:3] == ("0", "0")  # a no-op is visible too


def test_a_missing_second_copy_is_said(names, tmp_path, capsys):
    import urllib.error
    state_dir, out_dir = dirs(tmp_path)
    page = names["estr"]["second_url"]
    f = Fetcher()

    def fetch(url):
        if url == page:
            raise urllib.error.URLError("down")
        return f(url)

    estr.run(names, fetch=fetch, now=NOW, state_dir=state_dir, out_dir=out_dir)
    assert "estr: 8 cells, 0 tests written; second copy: unavailable" in capsys.readouterr().err
