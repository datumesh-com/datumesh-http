"""The `run` entry point of each relay script: fetch, check, write one publish-stored file, keep state."""
import importlib
import json
import pathlib

import pytest

from conftest import fixture
from specs import SPECS

NOW = "2026-11-19T12:00:00.000000Z"
MARKERS = [  # url fragment -> fixture file (what the real server would send)
    ("eurofxref-daily.xml", "ecb_daily.xml"), ("EXR/D..EUR.SP00.A", "ecb_sdmx_20261118.csv"),
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


@pytest.mark.parametrize("key", ["ecb_fx", "treasury"])
def test_run_includes_the_conformance_tests(key, names, tmp_path):
    mod = importlib.import_module(key)
    state_dir, out_dir = dirs(tmp_path)
    path = mod.run(names, fetch=Fetcher(), now=NOW, state_dir=state_dir, out_dir=out_dir)
    tests = [l for l in read(path) if "test" in l]
    assert len(tests) == (29 if key == "ecb_fx" else 168)


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
        assert url == names["ecb_fx"]["history_url"]
        return buf.getvalue()

    state_dir, out_dir = dirs(tmp_path)
    path = ecb_fx.run(names, fetch=fetch, now=NOW, state_dir=state_dir, out_dir=out_dir, mode="history")
    cells = [l for l in read(path) if "cell" in l]
    assert seen == [names["ecb_fx"]["history_url"]] and len(cells) == 284
