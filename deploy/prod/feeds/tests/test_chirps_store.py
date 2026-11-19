"""F3 additions: the raster's SHA-256 and size as stored cells, the region WKT as a stored cell, the one-file-a-minute
throttle, the monthly run, and the real-raster decode path (tifffile and numpy, skipped where they are not installed)."""
import copy
import json

import pytest

import chirps
from conftest import fixture
from helpers import RETRIEVED, check_order, check_source_object, check_summary, host_of, sha256, split_lines
from test_chirps import RECT, TRIANGLE, WITH_HOLE, expected, grid

RASTER = b"not really a GeoTIFF, but bytes with a known hash and size"


def test_raster_cells_are_the_hash_and_size_under_computed_names(names):
    sec = names["chirps"]
    url = sec["raster_url"].format(year="2024", month="06")
    lines, state = chirps.raster_lines(names, "2024-06-01", sha256(RASTER), len(RASTER), url, RETRIEVED, {})
    check_order(lines)
    fns, cats, cells, tests = split_lines(lines)
    fq_sha = f"{sec['namespace']}/{sec['raster_sha_function']}"
    fq_size = f"{sec['namespace']}/{sec['raster_bytes_function']}"
    assert [f["function"] for f in fns] == [fq_sha, fq_size] and tests == []
    assert fns[0]["signature"] == {"args": {"month": {"type": "date", "valid": True}}, "order": ["month"],
                                   "returns": "string"}
    assert fns[1]["signature"]["returns"] == {"type": "number", "unit": "bytes"}
    assert sorted(c["catalog"] for c in cats) == sorted([fq_sha, fq_size])
    assert "/relay/" not in fq_sha and "relayed" not in json.dumps(cells)
    by = {c["cell"]["fn"]: c for c in cells}
    assert by[fq_sha]["value"] == {"type": "string", "value": sha256(RASTER)}
    assert by[fq_size]["value"] == {"type": "number", "value": str(len(RASTER))}
    for c in cells:
        assert c["cell"]["args"] == {"month": {"type": "date", "value": "2024-06-01"}}
        assert c["licence"] == sec["licence"]
        check_summary(c["summary"])
        check_source_object(c["source"], host_of(sec["namespace"]), name=sec["source_name"], url=url,
                            sha=sha256(RASTER), retrieved=RETRIEVED, takeover=None)
    # the next month: cells only, no function lines again
    more, _ = chirps.raster_lines(names, "2024-07-01", sha256(b"x"), 1, url, RETRIEVED, state)
    assert [l for l in more if "cell" in l] and not [l for l in more if "function" in l or "catalog" in l]


def test_region_wkt_is_one_stored_cell_once(names):
    sec = names["chirps"]
    wkt = chirps.parse_wkt(fixture("ne_admin1_kansas.wkt").decode().strip())
    text = fixture("ne_admin1_kansas.wkt").decode().strip()
    lines, state = chirps.region_wkt_lines(names, "US-KS", text, RETRIEVED, {})
    fns, cats, cells, _ = split_lines(lines)
    fq = f"{sec['namespace']}/{sec['region_wkt_function']}"
    assert [f["function"] for f in fns] == [fq] and len(cells) == 1 and len(cats) == 1
    cell = cells[0]
    assert cell["cell"] == {"fn": fq, "args": {"region": {"type": "string", "value": "US-KS"}}}
    assert cell["value"] == {"type": "string", "value": text}
    assert cell["licence"] == sec["region_licence"] and wkt
    check_summary(cell["summary"])
    check_source_object(cell["source"], host_of(sec["namespace"]), name="raw.githubusercontent.com",
                        url=sec["region_source_url"], sha=sec["region_source_sha256"], retrieved=RETRIEVED,
                        takeover=None)
    again, _ = chirps.region_wkt_lines(names, "US-KS", text, RETRIEVED, state)
    assert again == []


def test_throttle_waits_out_the_minute():
    assert chirps.seconds_to_wait(None, 1000.0) == 0.0
    assert chirps.seconds_to_wait(1000.0, 1000.0) == 60.0
    assert chirps.seconds_to_wait(1000.0, 1025.0) == 35.0
    assert chirps.seconds_to_wait(1000.0, 1061.0) == 0.0


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
    n["chirps"]["region_files"] = {"US-KS": str(wkt)}
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
    assert sleeps == [60.0]
    lines = [json.loads(l) for l in path.read_text().splitlines()]
    cells = [l for l in lines if "cell" in l]
    # per month: 2 raster cells and one mean per region (the region has data in the synthetic grid)
    assert sum(1 for c in cells if c["cell"]["fn"].endswith("chirps_raster_sha256")) == 2
    assert sum(1 for c in cells if c["cell"]["fn"].endswith("chirps_region_wkt")) == 1
    means = [c for c in cells if c["cell"]["fn"].endswith("chirps_region_rain")]
    assert len(means) == 2
    st = json.loads((state_dir / "chirps.json").read_text())
    assert st["last_date"] == "2024-07-01" and st["wkt_fn"] and st["fn"] and st["raster_fns"]


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
