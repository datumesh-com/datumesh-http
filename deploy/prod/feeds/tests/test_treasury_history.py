"""Treasury history from 1990: one month XML at a time, at least 2 s apart, one publish-stored file a year, resumable,
with the tests of each year taken from that year's CSV (fake fetch, fake clock)."""
import json
import pathlib
import re
import urllib.error

import pytest

import treasury
from conftest import FEEDS, fixture
from helpers import canon, check_order, split_lines

NOW = "1992-03-15T12:00:00.000000Z"
NS = ('xmlns:d="http://schemas.microsoft.com/ado/2007/08/dataservices" '
      'xmlns:m="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata" xmlns="http://www.w3.org/2005/Atom"')
DAYS = (2, 3, 4, 5)
XML_COLS = (("BC_3MONTH", "3 Mo", "3M"), ("BC_10YEAR", "10 Yr", "10Y"), ("BC_30YEAR", "30 Yr", "30Y"))


def value(col, y, m, d):
    return f"{5 + (y - 1989) + m / 10 + d / 100 + col:.2f}"


def month_xml(y, m):
    entries = []
    for d in DAYS:
        props = "".join(f'<d:{x} m:type="Edm.Double">{value(i, y, m, d)}</d:{x}>' for i, (x, _, _) in enumerate(XML_COLS))
        entries.append(f'<entry><content type="application/xml"><m:properties><d:Id m:type="Edm.Int32">1</d:Id>'
                       f'<d:NEW_DATE m:type="Edm.DateTime">{y}-{m:02d}-{d:02d}T00:00:00</d:NEW_DATE>{props}'
                       f'</m:properties></content></entry>')
    return f'<?xml version="1.0"?><feed {NS}>' + "".join(entries) + "</feed>"


def year_csv(y, lie=None):
    rows = ["Date," + ",".join(f'"{c}"' for _, c, _ in XML_COLS)]
    for m in range(12, 0, -1):
        for d in sorted(DAYS, reverse=True):
            vals = [value(i, y, m, d) for i in range(len(XML_COLS))]
            if lie and lie == (y, m, d):
                vals[1] = "99.99"
            rows.append(f"{m:02d}/{d:02d}/{y}," + ",".join(vals))
    return ("\n".join(rows) + "\n").encode()


class World:
    """A fake Treasury: month XML and year CSV by URL; a fake clock the job sleeps on."""

    def __init__(self, names, fail=None, lie=None, csv_down=()):
        sec = names["treasury"]
        self.xml = {sec["url"].format(month=f"{y}{m:02d}"): (y, m) for y in range(1989, 1995) for m in range(1, 13)}
        self.csv = {sec["second_url"].format(year=y): y for y in range(1989, 1995)}
        self.t, self.sleeps, self.log, self.fail, self.lie, self.csv_down = 1000.0, [], [], fail, lie, set(csv_down)

    def clock(self):
        return self.t

    def sleep(self, s):
        self.sleeps.append(s)
        self.t += s

    def fetch(self, url):
        self.log.append((self.t, url))
        if url in self.xml:
            if self.fail == self.xml[url]:
                raise urllib.error.URLError("connection reset")
            return month_xml(*self.xml[url]).encode()
        if url in self.csv:
            if self.csv[url] in self.csv_down:
                raise urllib.error.URLError("csv down")
            return year_csv(self.csv[url], self.lie)
        raise AssertionError(f"unexpected url {url}")


def dirs(tmp_path):
    s, o = tmp_path / "state", tmp_path / "out"
    s.mkdir(), o.mkdir()
    return s, o


def read(path):
    return [json.loads(l) for l in pathlib.Path(path).read_text(encoding="utf-8").splitlines()]


def history(names, w, state_dir, out_dir, now=NOW, publish=None):
    return treasury.run(names, fetch=w.fetch, now=now, state_dir=state_dir, out_dir=out_dir, mode="history",
                        sleep=w.sleep, clock=w.clock, publish=publish)


def all_years(names, w, state_dir, out_dir, **kw):
    paths = []
    while True:
        p = history(names, w, state_dir, out_dir, **kw)
        if p is None:
            return paths
        paths.append(p)


# ---- the parsers accept the tenors a year had ------------------------------------------------------------------

def test_1990_files_have_nine_tenors_and_agree():
    from common import ShapeError
    xml = treasury.parse(fixture("treasury_199001.xml"))
    csv = treasury.parse_csv(fixture("treasury_1990.csv"))
    assert {r["tenor"] for r in xml} == {"3M", "6M", "1Y", "2Y", "3Y", "5Y", "7Y", "10Y", "30Y"}
    assert {r["tenor"] for r in csv} == {r["tenor"] for r in xml}
    a = {(r["tenor"], r["date"]): canon(r["value"]) for r in xml}
    b = {(r["tenor"], r["date"]): canon(r["value"]) for r in csv}
    assert a and all(b[k] == v for k, v in a.items())
    assert all(r["date"].startswith("1990-01") for r in xml) and len({r["date"] for r in csv}) > 200
    # a tenor that does not exist is still refused, as are a missing date and an empty file
    with pytest.raises(ShapeError):
        treasury.parse(fixture("treasury_199001.xml").replace(b"d:BC_10YEAR", b"d:BC_10YR"))
    with pytest.raises(ShapeError):
        treasury.parse_csv(fixture("treasury_1990.csv").replace(b'"10 Yr"', b'"10 Year"'))
    with pytest.raises(ShapeError):
        treasury.parse_csv(fixture("treasury_1990.csv").replace(b'"10 Yr","30 Yr"', b'"10 Yr","10 Yr"'))
    with pytest.raises(ShapeError):
        treasury.parse(fixture("treasury_199001.xml").replace(b"NEW_DATE", b"NEWDATE"))


# ---- history ----------------------------------------------------------------------------------------------------

def test_one_file_a_year_the_first_with_the_function_and_catalogue(names, tmp_path):
    w = World(names)
    state_dir, out_dir = dirs(tmp_path)
    paths = all_years(names, w, state_dir, out_dir)
    assert len(paths) == 3 and len(set(paths)) == 3 and all(pathlib.Path(p).parent == out_dir for p in paths)
    for i, (p, year, months) in enumerate(zip(paths, (1990, 1991, 1992), (12, 12, 3))):
        lines = read(p)
        check_order(lines)
        fns, cats, cells, tests = split_lines(lines)
        assert len(fns) == len(cats) == (1 if i == 0 else 0), "the function and its catalogue come with the first file"
        assert len(cells) == months * len(DAYS) * 3
        assert {c["cell"]["args"]["date"]["value"][:4] for c in cells} == {str(year)}
        assert all(c["source"]["retrieved"] == NOW and c["source"]["url"].startswith("https://") for c in cells)
        # one source per month: the month's file
        assert len({c["source"]["url"] for c in cells}) == months
        # tests from that year's CSV: the latest 20 dates a tenor, or all the year has
        dates = months * len(DAYS)
        assert len(tests) == 3 * min(20, dates)
        want = {}
        csv = {(r["tenor"], r["date"]): r["value"] for r in treasury.parse_csv(year_csv(year))}
        for t in tests:
            m = re.search(r'tenor: "([^"]+)", date: (\d{4}-\d\d-\d\d)', t["call"])
            assert t["expect"] == {"type": "number", "value": canon(csv[(m.group(1), m.group(2))])}
            want.setdefault(m.group(1), []).append(m.group(2))
        assert all(len(v) == min(20, dates) for v in want.values())
        assert all(min(v) >= sorted({d for (_, d) in csv if d <= f"{year}-{months:02d}-31"})[-min(20, dates)] for v in want.values())
    st = json.loads((state_dir / "treasury.json").read_text())
    assert st["last_date"] == "1992-03-05" and st["history_year"] == 1992
    assert history(names, w, state_dir, out_dir) is None  # all done


def test_requests_are_at_least_two_seconds_apart(names, tmp_path):
    w = World(names)
    state_dir, out_dir = dirs(tmp_path)
    all_years(names, w, state_dir, out_dir)
    times = [t for t, _ in w.log]
    assert len(times) == 12 + 1 + 12 + 1 + 3 + 1  # the month files, then the year's CSV
    # every request after the first of a run waits out the gap (the fake clock does not advance on its own)
    runs = [w.log[:13], w.log[13:26], w.log[26:]]
    for run in runs:
        gaps = [b[0] - a[0] for a, b in zip(run, run[1:])]
        assert gaps and all(g >= 2.0 for g in gaps), gaps
    assert treasury.THROTTLE_SECONDS == 2.0
    assert treasury.seconds_to_wait(None, 5.0) == 0.0 and treasury.seconds_to_wait(10.0, 11.0) == 1.0
    assert treasury.seconds_to_wait(10.0, 12.5) == 0.0
    months = [u for _, u in w.log if "pages/xml" in u]
    assert months[0].endswith("199001") and months[11].endswith("199012") and months[12].endswith("199101")
    assert months[-1].endswith("199203") and len(months) == 27


def test_a_failed_month_keeps_the_years_done_and_loses_only_its_year(names, tmp_path):
    state_dir, out_dir = dirs(tmp_path)
    w = World(names, fail=(1991, 5))
    first = history(names, w, state_dir, out_dir)
    assert first is not None and len(read(first)) > 0
    before = (state_dir / "treasury.json").read_text()
    with pytest.raises(urllib.error.URLError):
        history(names, w, state_dir, out_dir)
    assert (state_dir / "treasury.json").read_text() == before and len(list(out_dir.iterdir())) == 1
    w2 = World(names)  # the network is back
    rest = all_years(names, w2, state_dir, out_dir)
    assert len(rest) == 2 and first not in rest
    fetched = [u for _, u in w2.log if "pages/xml" in u]
    assert fetched[0].endswith("199101"), "resumes at the failed year, not at 1990"
    assert not any(u.endswith(("1990" + f"{m:02d}")) for u in fetched for m in range(1, 13))
    cells = [l for p in [first] + rest for l in read(p) if "cell" in l]
    assert len(cells) == (12 + 12 + 3) * len(DAYS) * 3 and len({json.dumps(c["cell"], sort_keys=True) for c in cells}) == len(cells)


def test_a_failed_publish_keeps_the_state_and_the_next_run_redoes_the_year(names, tmp_path):
    state_dir, out_dir = dirs(tmp_path)
    w = World(names)
    calls = []

    def bad(path):
        calls.append(path)
        raise RuntimeError("publish-stored failed")

    with pytest.raises(RuntimeError):
        history(names, w, state_dir, out_dir, publish=bad)
    assert len(calls) == 1 and list(state_dir.iterdir()) == [] and list(out_dir.iterdir()) == []
    ok = []
    p = history(names, w, state_dir, out_dir, publish=ok.append)
    assert ok == [p] and "1990-12-05" in json.loads((state_dir / "treasury.json").read_text())["last_date"]


def test_a_disagreeing_year_csv_refuses_the_year(names, tmp_path):
    from common import ConformanceError
    state_dir, out_dir = dirs(tmp_path)
    w = World(names, lie=(1990, 12, 5))  # in the latest dates of the year
    with pytest.raises(ConformanceError):
        history(names, w, state_dir, out_dir)
    assert list(state_dir.iterdir()) == [] and list(out_dir.iterdir()) == []


def test_a_year_csv_that_is_down_gives_a_file_without_tests_and_a_warning(names, tmp_path, capsys):
    state_dir, out_dir = dirs(tmp_path)
    w = World(names, csv_down=(1990,))
    p = history(names, w, state_dir, out_dir)
    lines = read(p)
    assert [l for l in lines if "cell" in l] and not [l for l in lines if "test" in l]
    assert "second copy" in capsys.readouterr().err


def test_history_does_not_depend_on_the_daily_state(names, tmp_path):
    """A mesh that already has daily cells (a last_date) still gets the years before them."""
    state_dir, out_dir = dirs(tmp_path)
    (state_dir / "treasury.json").write_text(json.dumps({"last_date": "1992-03-05", "announced": True}))
    w = World(names)
    paths = all_years(names, w, state_dir, out_dir)
    assert len(paths) == 3
    first = read(paths[0])
    assert split_lines(first)[2], "1990 is emitted although the daily state is later"
    assert not split_lines(first)[0], "the function was announced already"
    assert json.loads((state_dir / "treasury.json").read_text())["last_date"] == "1992-03-05"


def test_daily_continues_after_history(names, tmp_path):
    state_dir, out_dir = dirs(tmp_path)
    w = World(names)
    all_years(names, w, state_dir, out_dir)
    w2 = World(names)
    csv_1992 = names["treasury"]["second_url"].format(year=1992)

    def upto(month):  # the real year file has no day that is not published yet
        def fetch(url):
            if url != csv_1992:
                return w2.fetch(url)
            keep = [l for l in year_csv(1992).decode().splitlines() if l.startswith("Date") or int(l[:2]) <= month]
            return ("\n".join(keep) + "\n").encode()
        return fetch

    assert treasury.run(names, fetch=upto(3), now="1992-03-20T12:00:00.000000Z", state_dir=state_dir, out_dir=out_dir,
                        sleep=w2.sleep, clock=w2.clock) is None  # only March and what it has already
    p = treasury.run(names, fetch=upto(4), now="1992-04-10T12:00:00.000000Z", state_dir=state_dir, out_dir=out_dir,
                     sleep=w2.sleep, clock=w2.clock)
    cells = [l for l in read(p) if "cell" in l]
    assert {c["cell"]["args"]["date"]["value"][:7] for c in cells} == {"1992-04"}


def test_the_job_script_takes_history_and_loops_until_nothing_is_new():
    sh = (FEEDS / "run_feed.sh").read_text()
    assert re.search(r'"\$module" = treasury', sh) and "--mode $mode" in sh
    assert "while :" in sh and 'if [ "$mode" != history ]' in sh
    assert "history: ecb_fx treasury" in sh
