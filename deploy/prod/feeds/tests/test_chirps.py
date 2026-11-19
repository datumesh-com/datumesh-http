"""F3: the CHIRPS regional mean runs in the job, and its result is a stored cell under the computed name."""
import json
import math
from decimal import Decimal

import pytest

import chirps
from conftest import fixture
from helpers import RETRIEVED, check_order, check_summary, sha256, split_lines

RECT = [[(1, 57), (4, 57), (4, 60), (1, 60), (1, 57)]]
# the slanted edge x + (y - 56) = 6.2 keeps every pixel centre off the boundary: inside iff c <= r
TRIANGLE = [[(0, 56), (6.2, 56), (0, 62.2), (0, 56)]]
WITH_HOLE = [[(0, 56), (6, 56), (6, 62), (0, 62), (0, 56)], [(2, 58), (4, 58), (4, 60), (2, 60), (2, 58)]]


def grid():
    g = json.loads(fixture("synthetic_raster.json"))
    g.pop("_note")
    return g


def expected(inside):
    """Area-weighted mean of the pixels whose centre is inside: weight cos(latitude of the centre); nodata is skipped.
    Pixel (r, c) holds 10*r + c; its centre is (c + 0.5 E, 61.5 - r N)."""
    num = den = 0.0
    for r in range(6):
        for c in range(6):
            if (r, c) == (2, 2) or not inside(r, c):
                continue
            w = math.cos(math.radians(61.5 - r))
            num += w * (10 * r + c)
            den += w
    return num / den


def test_parse_wkt_kansas():
    rings = chirps.parse_wkt(fixture("ne_admin1_kansas.wkt").decode())
    assert len(rings) == 1
    r = rings[0]
    assert r[0] == r[-1] and len(r) > 50
    xs, ys = [p[0] for p in r], [p[1] for p in r]
    assert (-102.1, 36.9) < (min(xs), min(ys)) and (max(xs), max(ys)) < (-94.5, 40.1)
    assert abs(min(xs) - (-102.051744)) < 0.1 and abs(min(ys) - 36.99) < 0.1


def test_parse_wkt_with_hole_and_rejects_garbage():
    from common import ShapeError
    rings = chirps.parse_wkt("POLYGON((0 0, 4 0, 4 4, 0 0), (1 1, 2 1, 2 2, 1 1))")
    assert rings == [[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 0.0)], [(1.0, 1.0), (2.0, 1.0), (2.0, 2.0), (1.0, 1.0)]]
    for bad in ("", "POINT(1 2)", "POLYGON((0 0, 1 1))", "POLYGON((a b, 1 1, 2 2, a b))"):
        with pytest.raises(ShapeError):
            chirps.parse_wkt(bad)


def test_rectangle_area_weighted_and_nodata_skipped():
    got = chirps.region_mean(grid(), RECT)
    want = expected(lambda r, c: 2 <= r <= 4 and 1 <= c <= 3)
    assert float(got) == pytest.approx(want, abs=1e-9)
    # not the plain mean: the weights differ, and (2,2) is nodata
    plain = sum(10 * r + c for r in (2, 3, 4) for c in (1, 2, 3) if (r, c) != (2, 2)) / 8
    assert abs(want - plain) > 1e-3


def test_triangle_and_hole():
    assert float(chirps.region_mean(grid(), TRIANGLE)) == pytest.approx(expected(lambda r, c: c <= r), abs=1e-9)
    in_hole = lambda r, c: 2 <= r <= 3 and 2 <= c <= 3
    assert float(chirps.region_mean(grid(), WITH_HOLE)) == pytest.approx(expected(lambda r, c: not in_hole(r, c)), abs=1e-9)


def test_pixels_outside_never_count_and_empty_region_refused():
    from common import EmptyRegion
    # a polygon half outside the grid uses only the pixels in the grid
    half = [[(-3, 57), (2, 57), (2, 60), (-3, 60), (-3, 57)]]
    assert float(chirps.region_mean(grid(), half)) == pytest.approx(expected(lambda r, c: 2 <= r <= 4 and c <= 1), abs=1e-9)
    for far in ([[(20, 10), (21, 10), (21, 11), (20, 10)]], [[(2.6, 59.6), (2.9, 59.6), (2.9, 59.9), (2.6, 59.6)]]):
        with pytest.raises(EmptyRegion):  # no pixel centre inside, or all of them nodata
            chirps.region_mean(grid(), far)
    only_nodata = [[(2.1, 59.1), (2.9, 59.1), (2.9, 59.9), (2.1, 59.9), (2.1, 59.1)]]
    with pytest.raises(EmptyRegion):
        chirps.region_mean(grid(), only_nodata)


def test_region_row_rounds_to_two_places_only_here():
    row = chirps.region_row("us_kansas", "2024-06-01", grid(), RECT)
    assert set(row) == {"region", "month", "value"} and row["region"] == "us_kansas" and row["month"] == "2024-06-01"
    want = Decimal(repr(expected(lambda r, c: 2 <= r <= 4 and 1 <= c <= 3)))
    v = Decimal(row["value"])
    assert abs(v - want) <= Decimal("0.005") and -v.as_tuple().exponent <= 2


def src(names, month="2024-06"):
    y, m = month.split("-")
    url = names["chirps"]["raster_url"].format(year=y, month=m)
    return {"url": url, "retrieved": RETRIEVED, "sha256": sha256(b"a raster")}


def test_emitted_lines_are_a_stored_cell_with_no_source_member(names):
    sec = names["chirps"]
    rows = [chirps.region_row("us_kansas", "2024-06-01", grid(), RECT)]
    lines, state = chirps.emit(rows, {}, names, src(names))
    check_order(lines)
    fns, cats, cells, tests = split_lines(lines)
    fq = f"{sec['namespace']}/{sec['function']}"
    assert fq == "mesh-test.datumesh.com/earth/monthly_rainfall"
    assert len(fns) == 1 and len(cats) == 1 and len(cells) == 1 and tests == []
    assert fns[0] == {"function": fq, "signature": {
        "args": {"dataset": {"type": "string", "valid": False}, "region": {"type": "string", "valid": False},
                 "month": {"type": "date", "valid": True}},
        "order": ["dataset", "region", "month"], "returns": {"type": "number", "unit": "mm"}}}
    c = cats[0]
    assert c["catalog"] == fq and c["kind"] == "function" and set(c["args"]) == {"dataset", "region", "month"}
    assert {e["value"]: e["label"] for e in c["dictionary"]["region"]} == {k: v["label"] for k, v in sec["regions"].items()}
    assert [e["value"] for e in c["dictionary"]["dataset"]] == ["chirps"] and c["dictionary"]["dataset"][0]["label"]
    d = c["description"]
    assert len(d.encode()) <= 1024 and len(c["label"].encode()) <= 120
    for must in ("computed by datumesh.com from CHIRPS v3; not CHIRPS's own value", "area-weighted", "Natural Earth",
                 "admin-1", "pixels whose centres"):
        assert must in d, must
    assert "chirps" not in fq.lower()
    cell = cells[0]
    # D17: no source member, so the receipt is primary and never relayed
    assert set(cell) == {"cell", "value", "licence", "summary"}
    assert cell["cell"] == {"fn": fq, "args": {"dataset": {"type": "string", "value": "chirps"},
                                               "region": {"type": "string", "value": "us_kansas"},
                                               "month": {"type": "date", "value": "2024-06-01"}}}
    assert cell["value"] == {"type": "number", "value": rows[0]["value"]}
    assert cell["licence"] == sec["licence"]
    check_summary(cell["summary"])
    s = cell["summary"]
    ne = sec["regions"]["us_kansas"]
    for must in ("computed by datumesh.com from CHIRPS v3; not CHIRPS's own value", "2024-06", ne["label"], "us_kansas",
                 src(names)["url"], src(names)["sha256"], "area-weighted", "Natural Earth", ne["ne_id"],
                 sec["region_source_sha256"]):
        assert must in s, must
    assert "relayed by" not in s and "takeover" not in s
    assert all("source" not in l for l in lines)
    # idempotent
    assert chirps.emit(rows, json.loads(json.dumps(state)), names, src(names))[0] == []


def test_a_later_month_is_new_and_has_no_catalogue(names):
    r1 = chirps.region_row("us_kansas", "2024-06-01", grid(), RECT)
    r2 = chirps.region_row("us_kansas", "2024-07-01", grid(), TRIANGLE)
    _, state = chirps.emit([r1], {}, names, src(names))
    lines, _ = chirps.emit([r1, r2], state, names, src(names, "2024-07"))
    assert len(lines) == 1 and set(lines[0]) == {"cell", "value", "licence", "summary"}
    assert lines[0]["cell"]["args"]["month"]["value"] == "2024-07-01"
