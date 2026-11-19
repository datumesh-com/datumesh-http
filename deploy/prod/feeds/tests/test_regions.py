"""The CHIRPS regions: the 22 wheat regions, their Natural Earth ids and exact polygons (10m admin-1, v5.1.2)."""
import json

import pytest

import chirps
from conftest import FEEDS
from helpers import NAME

# region id -> (lon min, lon max, lat min, lat max): a generous box of known geography, to catch a wrong feature
BOXES = {
    "us_kansas": (-102.2, -94.5, 36.9, 40.1),
    "ua_kharkiv": (34.5, 38.5, 48.5, 50.8),
    "ua_poltava": (31.5, 36.5, 48.3, 51.0),
    "ua_dnipropetrovsk": (32.5, 37.5, 47.0, 49.5),
    "ru_krasnodar": (36.0, 42.5, 43.0, 47.0),
    "ru_rostov": (37.5, 44.8, 45.5, 50.5),
    "ru_stavropol": (40.5, 46.8, 43.5, 46.8),
    "in_punjab": (73.5, 77.2, 29.3, 32.7),
    "in_uttar_pradesh": (77.0, 84.8, 23.7, 30.5),
    "in_madhya_pradesh": (74.0, 83.0, 20.9, 26.9),
    "cn_henan": (110.2, 116.8, 31.2, 36.5),
    "cn_shandong": (114.6, 122.9, 34.2, 38.6),
    "cn_hebei": (113.3, 120.0, 36.0, 42.8),
    "fr_centre_val_de_loire": (-0.1, 3.3, 46.3, 49.0),
    "fr_grand_est": (3.3, 8.3, 47.4, 50.3),
    "ar_buenos_aires": (-63.5, -56.5, -41.1, -33.1),
    "au_new_south_wales": (140.5, 159.5, -38.0, -28.0),
    "au_western_australia": (112.0, 129.5, -35.5, -13.5),
    "ca_saskatchewan": (-110.2, -101.2, 48.9, 60.1),
    "ca_alberta": (-120.2, -109.8, 48.9, 60.1),
    "kz_kostanay": (59.0, 72.0, 48.5, 55.5),
    "kz_north_kazakhstan": (66.0, 75.0, 52.5, 56.0),
}
DISSOLVED = {"fr_centre_val_de_loire": 6, "fr_grand_est": 10}


def test_the_22_regions_are_listed(names):
    assert list(names["chirps"]["regions"]) == list(BOXES)
    assert len(BOXES) == 22


def test_ids_labels_and_ne_ids(names):
    regions = names["chirps"]["regions"]
    labels = [r["label"] for r in regions.values()]
    assert len(set(labels)) == 22 and all(0 < len(l.encode()) <= 120 for l in labels)
    assert len({r["ne_id"] for r in regions.values()}) == 22
    for rid, r in regions.items():
        assert NAME.match(rid) and rid.split("_")[0] in {"us", "ua", "ru", "in", "cn", "fr", "ar", "au", "ca", "kz"}
        assert r["ne_id"] and r["file"] == f"fixtures/regions/{rid}.wkt"
    assert regions["us_kansas"]["ne_id"] == "USA-3530"
    assert regions["ua_dnipropetrovsk"]["ne_id"] == "UKR-326"


def test_a_region_of_several_features_says_so(names):
    regions = names["chirps"]["regions"]
    for rid, n in DISSOLVED.items():
        r = regions[rid]
        assert len(r["members"]) == n and r["ne_id"] == "+".join(r["members"])
        assert "dissolved" in r["note"] and str(n) in r["note"] and "ne_vertices" not in r
    for rid, r in regions.items():
        if rid not in DISSOLVED:
            assert "members" not in r and "note" not in r and r["ne_vertices"] > 3


@pytest.mark.parametrize("rid", list(BOXES))
def test_the_polygon_is_where_the_region_is_and_is_not_simplified(names, rid):
    r = names["chirps"]["regions"][rid]
    polys = chirps.parse_wkt_multi((FEEDS / r["file"]).read_text(encoding="utf-8"))
    assert polys
    lon0, lon1, lat0, lat1 = BOXES[rid]
    n = 0
    for rings in polys:
        for ring in rings:
            assert ring[0] == ring[-1] and len(ring) >= 4
            n += len(ring)
            for x, y in ring:
                assert lon0 <= x <= lon1 and lat0 <= y <= lat1, (rid, x, y)
    xs = [x for rings in polys for x, _ in rings[0]]
    ys = [y for rings in polys for _, y in rings[0]]
    # the polygon fills most of the box of the region: the right feature, not a fragment
    assert (max(xs) - min(xs)) > 0.3 * (lon1 - lon0) and (max(ys) - min(ys)) > 0.3 * (lat1 - lat0)
    if "ne_vertices" in r:
        assert n == r["ne_vertices"], "the polygon is the Natural Earth feature, vertex for vertex"


def test_the_source_is_the_pinned_10m_file(names):
    c = names["chirps"]
    assert "10m" in c["region_source"] and "admin-1" in c["region_source"]
    assert "/v5.1.2/geojson/ne_10m_admin_1_states_provinces.geojson" in c["region_source_url"]
    assert c["region_source_sha256"] == "22d0e3ad85eb3e27f17cabf8ba2d50e554fbc27a87796ff891d958185da62fb5"


def test_a_dissolved_region_names_its_members_in_the_summary(names):
    rows = [{"region": "fr_grand_est", "month": "2024-06-01", "value": "55.2"},
            {"region": "us_kansas", "month": "2024-06-01", "value": "80.1"}]
    src = {"url": names["chirps"]["raster_url"].format(year="2024", month="06"), "retrieved": "x", "sha256": "ab" * 32}
    lines, _ = chirps.emit(rows, {}, names, src)
    cat = [l for l in lines if "catalog" in l][0]
    assert len(cat["dictionary"]["region"]) == 22
    assert "dissolved" in cat["description"]
    cells = {l["cell"]["args"]["region"]["value"]: l for l in lines if "cell" in l}
    assert "dissolved" in cells["fr_grand_est"]["summary"] and "FRA-5268" in cells["fr_grand_est"]["summary"]
    assert "dissolved" not in cells["us_kansas"]["summary"]
    assert len(cells["fr_grand_est"]["summary"].encode()) <= 1024


def test_multipolygon_wkt_parses():
    t = "MULTIPOLYGON(((0 0, 4 0, 4 4, 0 0), (1 1, 2 1, 2 2, 1 1)), ((10 10, 11 10, 11 11, 10 10)))"
    polys = chirps.parse_wkt_multi(t)
    assert len(polys) == 2 and len(polys[0]) == 2 and len(polys[1]) == 1
    assert polys[1][0] == [(10.0, 10.0), (11.0, 10.0), (11.0, 11.0), (10.0, 10.0)]
    assert chirps.parse_wkt_multi("POLYGON((0 0, 4 0, 4 4, 0 0))") == [[[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 0.0)]]]
    from common import ShapeError
    for bad in ("", "POINT(1 2)", "MULTIPOLYGON()", "MULTIPOLYGON(((0 0, 1 1)))", "MULTIPOLYGON((0 0, 1 0, 1 1, 0 0))",
                "MULTIPOLYGON(((a b, 1 1, 2 2, a b)))"):
        with pytest.raises(ShapeError):
            chirps.parse_wkt_multi(bad)


def test_mean_over_a_multipolygon_counts_a_pixel_once():
    from test_chirps import expected, grid
    two = [[[(1, 58), (3, 58), (3, 60), (1, 60), (1, 58)]], [[(2, 57), (4, 57), (4, 59), (2, 59), (2, 57)]]]
    inside = lambda r, c: (2 <= r <= 3 and 1 <= c <= 2) or (3 <= r <= 4 and 2 <= c <= 3)
    assert float(chirps.region_mean(grid(), two)) == pytest.approx(expected(inside), abs=1e-9)
    # a polygon given as rings still works
    from test_chirps import RECT
    assert float(chirps.region_mean(grid(), RECT)) == pytest.approx(float(chirps.region_mean(grid(), [RECT])), abs=1e-12)
