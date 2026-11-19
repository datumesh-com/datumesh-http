"""Extract the CHIRPS regions' polygons from Natural Earth 10m admin-1 (v5.1.2) as WKT, exactly: nothing is simplified.

usage (the GeoJSON is untrusted data: this only parses it, and runs with -I):
  python -I extract_regions.py ne_10m_admin_1_states_provinces.geojson OUT_DIR

A region is one feature by its adm1_code, or, for the French regions (the 10m file has departments), the union of
its departments, selected by the `region` property: shapely's unary_union, no simplification. Writes
OUT_DIR/<region id>.wkt and OUT_DIR/regions.json (ne_id, vertex count or members) for names.toml.
Needs shapely only for the dissolved regions."""
import json
import sys
from pathlib import Path

SINGLE = {
    "us_kansas": "USA-3530", "ua_kharkiv": "UKR-328", "ua_poltava": "UKR-330", "ua_dnipropetrovsk": "UKR-326",
    "ru_krasnodar": "RUS-2371", "ru_rostov": "RUS-2367", "ru_stavropol": "RUS-2306",
    "in_punjab": "IND-3249", "in_uttar_pradesh": "IND-3253", "in_madhya_pradesh": "IND-3261",
    "cn_henan": "CHN-1812", "cn_shandong": "CHN-1814", "cn_hebei": "CHN-1811",
    "ar_buenos_aires": "ARG-1295", "au_new_south_wales": "AUS-2654", "au_western_australia": "AUS-2651",
    "ca_saskatchewan": "CAN-631", "ca_alberta": "CAN-632", "kz_kostanay": "KAZ-3196", "kz_north_kazakhstan": "KAZ-3204",
}
DISSOLVED = {"fr_centre_val_de_loire": ("FRA", "Centre-Val de Loire"), "fr_grand_est": ("FRA", "Grand Est")}


def ring_text(ring):
    return "(" + ", ".join(f"{repr(float(x))} {repr(float(y))}" for x, y in ring) + ")"


def poly_text(rings):
    return "(" + ", ".join(ring_text(r) for r in rings) + ")"


def wkt_of(geom):
    if geom["type"] == "Polygon":
        return "POLYGON" + poly_text(geom["coordinates"]), sum(len(r) for r in geom["coordinates"])
    assert geom["type"] == "MultiPolygon", geom["type"]
    n = sum(len(r) for p in geom["coordinates"] for r in p)
    return "MULTIPOLYGON(" + ", ".join(poly_text(p) for p in geom["coordinates"]) + ")", n


def main(src, out):
    data = json.loads(Path(src).read_text(encoding="utf-8"))
    feats = data["features"]
    by_code = {f["properties"]["adm1_code"]: f for f in feats}
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    info = {}
    for rid, code in SINGLE.items():
        f = by_code[code]
        text, n = wkt_of(f["geometry"])
        (out / f"{rid}.wkt").write_text(text + "\n", encoding="utf-8")
        info[rid] = {"ne_id": code, "name": f["properties"]["name"], "type": f["geometry"]["type"], "ne_vertices": n}
    for rid, (adm0, region) in DISSOLVED.items():
        from shapely.geometry import shape
        from shapely.ops import unary_union
        from shapely import to_wkt
        members = sorted((f for f in feats if f["properties"]["adm0_a3"] == adm0 and f["properties"].get("region") == region),
                         key=lambda f: f["properties"]["adm1_code"])
        union = unary_union([shape(f["geometry"]) for f in members])
        text = to_wkt(union, rounding_precision=-1, trim=True)
        assert union.geom_type in ("Polygon", "MultiPolygon") and union.is_valid
        (out / f"{rid}.wkt").write_text(text + "\n", encoding="utf-8")
        info[rid] = {"members": [f["properties"]["adm1_code"] for f in members],
                     "names": [f["properties"]["name"] for f in members], "type": union.geom_type,
                     "member_vertices": sum(wkt_of(f["geometry"])[1] for f in members),
                     "parts": len(union.geoms) if union.geom_type == "MultiPolygon" else 1,
                     "holes": sum(len(p.interiors) for p in (union.geoms if union.geom_type == "MultiPolygon" else [union]))}
    (out / "regions.json").write_text(json.dumps(info, indent=1, ensure_ascii=False), encoding="utf-8")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
