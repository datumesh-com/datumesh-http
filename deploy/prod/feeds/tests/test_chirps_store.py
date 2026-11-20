"""F3 additions (names answer 4): the one-file-a-minute throttle, the monthly run, and the real-raster decode path (tifffile and numpy, skipped where they are not installed)."""
import copy
import json

import pytest

import chirps
from conftest import fixture
from helpers import sha256
from test_chirps import RECT, TRIANGLE, WITH_HOLE, expected, grid

RASTER = b"not really a GeoTIFF, but bytes with a known hash and size"


def test_throttle_waits_out_the_minute():
    assert chirps.seconds_to_wait(None, 1000.0) == 0.0
    assert chirps.THROTTLE_SECONDS == 120.0
    assert chirps.seconds_to_wait(1000.0, 1000.0) == 120.0
    assert chirps.seconds_to_wait(1000.0, 1025.0) == 95.0
    assert chirps.seconds_to_wait(1000.0, 1121.0) == 0.0


def test_pending_months_start_and_stop():
    assert chirps.pending_months(None, "2026-11-19T12:00:00.000000Z")[:2] == ["1981-01-01", "1981-02-01"]
    assert chirps.pending_months("2026-09-01", "2026-11-19T12:00:00.000000Z") == ["2026-10-01"]
    assert chirps.pending_months("2026-10-01", "2026-11-19T12:00:00.000000Z") == []
    assert chirps.pending_months("2024-12-01", "2025-02-03T00:00:00.000000Z") == ["2025-01-01"]


def _fake_clock():
    t = [1000.0]
    sleeps = []

    def clock():
        return t[0]

    def sleep(s):
        sleeps.append(s)
        t[0] += s
    return clock, sleep, sleeps


def _names_with_region_in_grid(names, tmp_path):
    """The synthetic grid covers lon 0..6, lat 56..62: the region is the rectangle RECT, written as WKT."""
    wkt = tmp_path / "rect.wkt"
    wkt.write_text("POLYGON((1 57, 4 57, 4 60, 1 60, 1 57))")
    n = copy.deepcopy(names)
    n["chirps"]["regions"] = {"us_kansas": {**n["chirps"]["regions"]["us_kansas"], "file": str(wkt)}}
    return n


def test_run_fetches_each_month_a_minute_apart_and_writes_one_file(names, tmp_path):
    names = _names_with_region_in_grid(names, tmp_path)
    state_dir, out_dir = tmp_path / "s", tmp_path / "o"
    state_dir.mkdir(), out_dir.mkdir()
    (state_dir / "chirps.json").write_text(json.dumps({"last_date": "2024-05-01"}))
    clock, sleep, sleeps = _fake_clock()
    fetched = []

    def fetch(url):
        fetched.append(url)
        return RASTER

    path = chirps.run(names, fetch=fetch, now="2026-11-19T12:00:00.000000Z", state_dir=state_dir, out_dir=out_dir,
                      sleep=sleep, clock=clock, max_months=2, decode=lambda data: grid())
    assert fetched == [names["chirps"]["raster_url"].format(year="2024", month="06"),
                       names["chirps"]["raster_url"].format(year="2024", month="07")]
    assert sleeps == [120.0]
    lines = [json.loads(l) for l in path.read_text().splitlines()]
    cells = [l for l in lines if "cell" in l]
    # per month: one mean per region (the region has data in the synthetic grid), and nothing else: no raster
    # or polygon cells, no source member anywhere
    fq = f"{names['chirps']['namespace']}/{names['chirps']['function']}"
    assert len(cells) == 2 and {c["cell"]["fn"] for c in cells} == {fq}
    assert [c["cell"]["args"]["month"]["value"] for c in cells] == ["2024-06-01", "2024-07-01"]
    assert all("source" not in l for l in lines)
    for c, month in zip(cells, ("2024-06", "2024-07")):
        url = names["chirps"]["raster_url"].format(year=month[:4], month=month[5:])
        assert url in c["summary"] and sha256(RASTER) in c["summary"]
    st = json.loads((state_dir / "chirps.json").read_text())
    assert st["last_date"] == "2024-07-01" and st["fn"]


def test_a_month_not_yet_published_stops_the_run_and_keeps_the_rest(names, tmp_path):
    from common import NotPublished
    names = _names_with_region_in_grid(names, tmp_path)
    state_dir, out_dir = tmp_path / "s", tmp_path / "o"
    state_dir.mkdir(), out_dir.mkdir()
    (state_dir / "chirps.json").write_text(json.dumps({"last_date": "2024-05-01"}))
    clock, sleep, _ = _fake_clock()
    calls = []

    def fetch(url):
        calls.append(url)
        if len(calls) == 2:
            raise NotPublished(url)
        return RASTER

    path = chirps.run(names, fetch=fetch, now="2026-11-19T12:00:00.000000Z", state_dir=state_dir, out_dir=out_dir,
                      sleep=sleep, clock=clock, max_months=5, decode=lambda data: grid())
    assert len(calls) == 2 and path is not None
    assert json.loads((state_dir / "chirps.json").read_text())["last_date"] == "2024-06-01"


def test_run_nothing_pending_returns_none(names, tmp_path):
    state_dir, out_dir = tmp_path / "s", tmp_path / "o"
    state_dir.mkdir(), out_dir.mkdir()
    (state_dir / "chirps.json").write_text(json.dumps({"last_date": "2026-10-01"}))
    clock, sleep, _ = _fake_clock()
    assert chirps.run(names, fetch=lambda u: RASTER, now="2026-11-19T12:00:00.000000Z", state_dir=state_dir,
                      out_dir=out_dir, sleep=sleep, clock=clock) is None
    assert list(out_dir.iterdir()) == []


# ---- the real decode path: needs tifffile and numpy (not in the python image; CI skips) -------------------

def _write_geotiff(path, values, west, north, cs, nodata=-9999.0):
    tifffile = pytest.importorskip("tifffile")
    import numpy as np
    arr = np.array(values, dtype="float32")
    tifffile.imwrite(path, arr, extratags=[
        (33550, "d", 3, (cs, cs, 0.0), True),
        (33922, "d", 6, (0.0, 0.0, 0.0, west, north, 0.0), True),
        (42113, "s", 0, str(nodata), True),
    ])


def test_real_raster_decodes_to_the_same_means(tmp_path):
    pytest.importorskip("tifffile")
    pytest.importorskip("numpy")
    path = tmp_path / "synthetic.tif"
    g = grid()
    _write_geotiff(path, g["values"], g["west"], g["north"], g["cell_size"])
    decoded = chirps.read_geotiff(path.read_bytes())
    assert (decoded["west"], decoded["north"], decoded["cell_size"], decoded["nodata"]) == (0.0, 62.0, 1.0, -9999.0)
    for rings, inside in ((RECT, lambda r, c: 2 <= r <= 4 and 1 <= c <= 3),
                          (TRIANGLE, lambda r, c: c <= r),
                          (WITH_HOLE, lambda r, c: not (2 <= r <= 3 and 2 <= c <= 3))):
        assert float(chirps.region_mean(decoded, rings)) == pytest.approx(expected(inside), abs=1e-6)


def test_decode_without_the_libraries_says_so(monkeypatch):
    import builtins
    real = builtins.__import__

    def no_tifffile(name, *a, **k):
        if name == "tifffile":
            raise ImportError("no tifffile")
        return real(name, *a, **k)
    monkeypatch.setattr(builtins, "__import__", no_tifffile)
    with pytest.raises(RuntimeError, match="tifffile"):
        chirps.read_geotiff(b"x")
