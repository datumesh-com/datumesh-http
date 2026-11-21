"""F1 (CONTRACT-0.4.9): chirps.py emits `test` lines for earth/monthly_rainfall, as the relays do.

Specified here: a run writes, after the cells, `test` lines `{"test": fq, "call": ..., "expect": ...}`:
  * `test` is the full function name (`<namespace>/<function>`) and `call` names it in full too, with
    all three arguments (dataset, region, month);
  * each test pins the value of a cell of the same file (the call is that cell's call, `expect` is that
    cell's `value`): the value is derived a second time from the raster (here: the test of the value
    against an independent re-computation of the region's mean from the decoded grid);
  * every region-month of the file has at least one test, and a file holds at most 20 tests;
  * a run that emits no cells emits no tests, and a rerun emits nothing."""
import copy
import json

import pytest

import chirps
from conftest import fixture  # noqa: F401  (the fixtures dir is shared with the other chirps tests)
from helpers import call_text, check_order, split_lines
from test_chirps import RECT, expected, grid

RASTER = b"not really a GeoTIFF, but bytes with a known hash and size"
NOW = "2026-11-19T12:00:00.000000Z"
ARGS = ["dataset", "region", "month"]


def _fake_clock():
    t = [1000.0]

    def clock():
        return t[0]

    def sleep(s):
        t[0] += s
    return clock, sleep


def _names(names, tmp_path, regions):
    """`regions` copies of the rectangle region, all inside the synthetic grid (lon 0..6, lat 56..62)."""
    wkt = tmp_path / "rect.wkt"
    wkt.write_text("POLYGON((1 57, 4 57, 4 60, 1 60, 1 57))")
    n = copy.deepcopy(names)
    base = n["chirps"]["regions"]["us_kansas"]
    n["chirps"]["regions"] = {f"r{i}": {**base, "file": str(wkt)} for i in range(regions)}
    return n


def _run(names, tmp_path, months, last="2024-05-01"):
    state_dir, out_dir = tmp_path / "s", tmp_path / "o"
    state_dir.mkdir(), out_dir.mkdir()
    (state_dir / "chirps.json").write_text(json.dumps({"last_date": last}))
    clock, sleep = _fake_clock()
    path = chirps.run(names, fetch=lambda u: RASTER, now=NOW, state_dir=state_dir, out_dir=out_dir, sleep=sleep,
                      clock=clock, max_months=months, decode=lambda data: grid())
    lines = [json.loads(l) for l in path.read_text().splitlines()]
    return path, lines, state_dir, out_dir, (clock, sleep)


def _cell_call(fq, cell):
    a = cell["cell"]["args"]
    return call_text(fq, ARGS, {k: a[k]["value"] for k in ARGS})


def test_a_run_writes_test_lines_after_the_cells(names, tmp_path):
    names = _names(names, tmp_path, 1)
    _, lines, *_ = _run(names, tmp_path, 2)
    check_order(lines)
    _, _, cells, tests = split_lines(lines)
    assert len(cells) == 2
    assert tests, "chirps emits test lines"
    fq = f"{names['chirps']['namespace']}/{names['chirps']['function']}"
    assert fq == "mesh-test.datumesh.com/earth/monthly_rainfall"
    for t in tests:
        assert set(t) == {"test", "call", "expect"}, t
        assert t["test"] == fq
        assert t["call"].startswith(fq + "("), "the call names the function by its full name"


def test_every_test_pins_the_value_of_a_cell_of_the_same_file(names, tmp_path):
    names = _names(names, tmp_path, 1)
    _, lines, *_ = _run(names, tmp_path, 2)
    _, _, cells, tests = split_lines(lines)
    fq = f"{names['chirps']['namespace']}/{names['chirps']['function']}"
    by_call = {_cell_call(fq, c): c for c in cells}
    assert len(by_call) == len(cells)
    assert tests
    for t in tests:
        assert t["call"] in by_call, t["call"]
        assert t["expect"] == by_call[t["call"]]["value"], "the test pins the emitted cell's value"
        assert t["expect"]["type"] == "number"


def test_the_pinned_value_is_the_second_computation_of_the_regional_mean(names, tmp_path):
    names = _names(names, tmp_path, 1)
    _, lines, *_ = _run(names, tmp_path, 1)
    _, _, cells, tests = split_lines(lines)
    want = expected(lambda r, c: 2 <= r <= 4 and 1 <= c <= 3)  # computed here, apart from chirps.py
    assert tests
    for t in tests:
        assert float(t["expect"]["value"]) == pytest.approx(want, abs=0.005)
        assert t["expect"] == cells[0]["value"]


def test_at_least_one_test_per_region_month(names, tmp_path):
    names = _names(names, tmp_path, 3)  # 3 regions x 2 months = 6 region-months, under the cap
    _, lines, *_ = _run(names, tmp_path, 2)
    _, _, cells, tests = split_lines(lines)
    fq = f"{names['chirps']['namespace']}/{names['chirps']['function']}"
    assert len(cells) == 6
    tested = {t["call"] for t in tests}
    for c in cells:
        assert _cell_call(fq, c) in tested, f"no test for {_cell_call(fq, c)}"
    assert len(tests) <= 20


def test_at_most_20_tests_a_file(names, tmp_path):
    names = _names(names, tmp_path, 8)  # 8 regions x 4 months = 32 region-months
    _, lines, *_ = _run(names, tmp_path, 4)
    _, _, cells, tests = split_lines(lines)
    fq = f"{names['chirps']['namespace']}/{names['chirps']['function']}"
    assert len(cells) == 32
    assert 1 <= len(tests) <= 20
    by_call = {_cell_call(fq, c): c for c in cells}
    for t in tests:
        assert t["expect"] == by_call[t["call"]]["value"]


def test_a_rerun_emits_nothing_and_a_nothing_pending_run_no_tests(names, tmp_path):
    names = _names(names, tmp_path, 1)
    path, lines, state_dir, out_dir, (clock, sleep) = _run(names, tmp_path, 1)
    assert [l for l in lines if "test" in l]
    again = chirps.run(names, fetch=lambda u: RASTER, now=NOW, state_dir=state_dir, out_dir=out_dir, sleep=sleep,
                       clock=clock, max_months=1, decode=lambda data: grid(), retries=0)
    # the next month is pending and has its own cells and tests; none of the first month's again
    assert again is not None and again != path
    more = [json.loads(l) for l in again.read_text().splitlines()]
    _, _, cells2, tests2 = split_lines(more)
    assert {c["cell"]["args"]["month"]["value"] for c in cells2} == {"2024-07-01"}
    assert tests2 and all("month: 2024-07-01" in t["call"] for t in tests2)
