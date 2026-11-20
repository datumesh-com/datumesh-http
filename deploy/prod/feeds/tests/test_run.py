"""The `run` entry point of each relay script: fetch, check, write one publish-stored file, keep state."""
import importlib
import json
import pathlib

import pytest

from conftest import fixture
from specs import SPECS

NOW = "2026-11-19T12:00:00.000000Z"
MARKERS = [  # url fragment -> fixture file (what the real server would send)
    ("eurofxref-daily.xml", "ecb_daily.xml"), ("EXR/D..EUR.SP00.A?format=csvdata&lastNObservations=20", "ecb_sdmx_last3.csv"),
    ("EXR/D..EUR.SP00.A", "ecb_sdmx_20261118.csv"),
    ("euro_short-term_rate/html", "ecb_estr_page.html"), ("key_ecb_interest_rates", "ecb_key_rates.html"),
    ("database/Bank-Rate.asp", "boe_bank_rate.html"),
    ("EST/B.EU000A2X2A25", "estr.csv"), ("KR.MRR_FR.LEV", "policy_MRR_FR.csv"), ("KR.DFR.LEV", "policy_DFR.csv"),
    ("KR.MLFR.LEV", "policy_MLFR.csv"), ("pages/xml?data=daily_treasury_yield_curve", "treasury_202611.xml"),
    ("daily-treasury-rates.csv", "treasury_202611.csv"), ("SeriesCodes=IUDBEDR", "boe_iudbedr.csv"),
]


class Fetcher:
    def __init__(self, mutate=None):
        self.urls, self.mutate = [], mutate

    def __call__(self, url):
        self.urls.append(url)
        for frag, name in MARKERS:
            if frag in url:
                data = fixture(name)
                return self.mutate(data) if self.mutate else data
        raise AssertionError(f"unexpected url {url}")


def dirs(tmp_path):
    s, o = tmp_path / "state", tmp_path / "out"
    s.mkdir(), o.mkdir()
    return s, o


def read(path):
    return [json.loads(l) for l in pathlib.Path(path).read_text(encoding="utf-8").splitlines()]


@pytest.mark.parametrize("key", SPECS)
def test_run_writes_one_file_then_nothing(key, names, tmp_path):
    mod, spec = importlib.import_module(key), SPECS[key]
    state_dir, out_dir = dirs(tmp_path)
    f = Fetcher()
    path = mod.run(names, fetch=f, now=NOW, state_dir=state_dir, out_dir=out_dir)
    assert path is not None and pathlib.Path(path).parent == out_dir and pathlib.Path(path).exists()
    lines = read(path)
    cells = [l for l in lines if "cell" in l]
    assert len(cells) == spec["nrows"]
    assert all(l["source"]["retrieved"] == NOW and l["source"]["url"] in f.urls for l in cells)
    assert all(u.startswith("https://") for u in f.urls)
    st = json.loads((pathlib.Path(state_dir) / f"{key}.json").read_text())
    assert "last_date" in st
    n_files = len(list(out_dir.iterdir()))
    again = mod.run(names, fetch=f, now=NOW, state_dir=state_dir, out_dir=out_dir)
    assert again is None and len(list(out_dir.iterdir())) == n_files  # a second run emits nothing new
    assert json.loads((pathlib.Path(state_dir) / f"{key}.json").read_text()) == st


@pytest.mark.parametrize("key", SPECS)
def test_run_refuses_a_changed_shape_and_leaves_nothing(key, names, tmp_path):
    from common import ShapeError
    mod, spec = importlib.import_module(key), SPECS[key]
    state_dir, out_dir = dirs(tmp_path)
    with pytest.raises(ShapeError):
        mod.run(names, fetch=Fetcher(mutate=spec["mutations"][0]), now=NOW, state_dir=state_dir, out_dir=out_dir)
    assert list(out_dir.iterdir()) == [] and list(state_dir.iterdir()) == []


# the tests of a first run on the fixtures: the pairs present in both copies (at most 20 dates a key)
FIRST_RUN_TESTS = {"ecb_fx": 29, "treasury": 168, "estr": 1, "policy_rates": 18, "boe": 1}


@pytest.mark.parametrize("key", SPECS)
def test_run_includes_the_conformance_tests(key, names, tmp_path):
    mod = importlib.import_module(key)
    state_dir, out_dir = dirs(tmp_path)
    path = mod.run(names, fetch=Fetcher(), now=NOW, state_dir=state_dir, out_dir=out_dir)
    lines = read(path)
    tests = [l for l in lines if "test" in l]
    assert len(tests) == FIRST_RUN_TESTS[key]
    fq = f"{names[key]['namespace']}/{names[key]['function']}"
    assert {t["test"] for t in tests} == {fq}
    st = json.loads((state_dir / f"{key}.json").read_text())
    assert st["tested"], "the high-water mark of the tested dates is kept"


@pytest.mark.parametrize("key", SPECS)
def test_every_second_copy_is_fetched_from_a_url_of_the_table(key, names, tmp_path):
    mod = importlib.import_module(key)
    state_dir, out_dir = dirs(tmp_path)
    f = Fetcher()
    mod.run(names, fetch=f, now=NOW, state_dir=state_dir, out_dir=out_dir)
    sec = names[key]
    second = sec["second_url"].format(year=2026) if key == "treasury" else sec["second_url"]
    assert second in f.urls and f.urls.index(second) > 0 and all(u.startswith("https://") for u in f.urls)


def test_a_second_copy_that_cannot_be_fetched_does_not_stop_the_relay_and_tests_come_later(names, tmp_path, capsys):
    import urllib.error
    import estr
    state_dir, out_dir = dirs(tmp_path)
    page = names["estr"]["second_url"]
    down = lambda u: (_ for _ in ()).throw(urllib.error.URLError("down")) if u == page else Fetcher()(u)  # noqa: E731
    p1 = estr.run(names, fetch=down, now=NOW, state_dir=state_dir, out_dir=out_dir)
    l1 = read(p1)
    assert len([l for l in l1 if "cell" in l]) == 8 and not [l for l in l1 if "test" in l]
    assert "second copy" in capsys.readouterr().err
    # the page is back: no new cells, but the test for the date it shows is new, so a file is written
    p2 = estr.run(names, fetch=Fetcher(), now="2026-11-19T13:00:00.000000Z", state_dir=state_dir, out_dir=out_dir)
    l2 = read(p2)
    assert [list(l) for l in l2] == [["test", "call", "expect"]]
    assert l2[0]["call"].endswith("(date: 2026-11-18)") and l2[0]["expect"] == {"type": "number", "value": "2.442"}
    assert estr.run(names, fetch=Fetcher(), now="2026-11-19T14:00:00.000000Z", state_dir=state_dir, out_dir=out_dir) is None


@pytest.mark.parametrize("key,old,new", [
    ("estr", b"<strong>2.442</strong>", b"<strong>2.443</strong>"),
    ("policy_rates", b'<td class="number">2.65</td>', b'<td class="number">2.66</td>'),
    ("boe", b'<td align="right"> 4.00</td>', b'<td align="right"> 4.25</td>'),
])
def test_a_second_copy_that_disagrees_refuses_the_run(key, old, new, names, tmp_path):
    from common import ConformanceError
    mod = importlib.import_module(key)
    state_dir, out_dir = dirs(tmp_path)
    second = names[key]["second_url"]

    def fetch(url):
        data = Fetcher()(url)
        return data.replace(old, new, 1) if url == second else data

    with pytest.raises(ConformanceError):
        mod.run(names, fetch=fetch, now=NOW, state_dir=state_dir, out_dir=out_dir)
    assert list(out_dir.iterdir()) == [] and list(state_dir.iterdir()) == []


def test_state_is_saved_only_after_the_publish_succeeds(names, tmp_path):
    mod = importlib.import_module("boe")
    state_dir, out_dir = dirs(tmp_path)
    calls = []

    def bad(path):
        calls.append(path)
        raise RuntimeError("publish-stored failed")

    with pytest.raises(RuntimeError):
        mod.run(names, fetch=Fetcher(), now=NOW, state_dir=state_dir, out_dir=out_dir, publish=bad)
    assert len(calls) == 1 and list(state_dir.iterdir()) == []  # the next run emits the same values again
    ok = []
    path = mod.run(names, fetch=Fetcher(), now=NOW, state_dir=state_dir, out_dir=out_dir, publish=ok.append)
    assert ok == [path] and (state_dir / "boe.json").exists()
    assert mod.run(names, fetch=Fetcher(), now=NOW, state_dir=state_dir, out_dir=out_dir, publish=ok.append) is None
    assert len(ok) == 1  # nothing to publish, publish not called


def test_ecb_history_mode_fetches_the_history_file(names, tmp_path):
    import io, zipfile
    import ecb_fx
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr("eurofxref-hist.csv", fixture("ecb_hist_slice.csv"))
    seen = []

    def fetch(url):
        seen.append(url)
        if url == names["ecb_fx"]["history_url"]:
            return buf.getvalue()
        assert url == names["ecb_fx"]["second_url_history"]  # the SDMX copy of the latest observations
        return fixture("ecb_sdmx_last3.csv")

    state_dir, out_dir = dirs(tmp_path)
    path = ecb_fx.run(names, fetch=fetch, now=NOW, state_dir=state_dir, out_dir=out_dir, mode="history")
    lines = read(path)
    cells = [l for l in lines if "cell" in l]
    tests = [l for l in lines if "test" in l]
    assert seen == [names["ecb_fx"]["history_url"], names["ecb_fx"]["second_url_history"]] and len(cells) == 284
    # a history load carries tests too: the dates both copies have (2026-11-16 to 18, the currencies the ECB still quotes)
    assert 60 <= len(tests) <= 90 and all(t["call"].split("date: ")[1][:7] == "2026-11" for t in tests)
    assert "lastNObservations=20" in names["ecb_fx"]["second_url_history"]
