"""CHIRPS v3 monthly rainfall, regional means (F3). Computed by datumesh.com, never a relay name.

The aggregation runs here because the engine has no raster reader. A pixel counts when its centre is inside the
region's outer ring and outside its holes; its weight is cos(latitude of its centre); nodata is skipped.
Stored cells: the mean per region and month, the raster file's SHA-256 and size per month, and each region's WKT
once. The raster is decoded with tifffile and numpy, imported only when a raster is decoded (D15)."""
import datetime as dt
import io
import math
import re
import time
from decimal import ROUND_HALF_EVEN, Decimal
from pathlib import Path
from urllib.parse import urlparse

import common
import relay
from common import EmptyRegion, NotPublished, ShapeError, decimal_text, shape

KEY = "chirps"
FIRST_MONTH = "1981-01-01"
THROTTLE_SECONDS = 60.0  # CHC answers 429 to quick repeats: at most one raster file a minute
RING_RE = re.compile(r"\(([^()]*)\)")


# ---- region polygons -------------------------------------------------------------------------------------

def parse_wkt(text: str) -> list:
    """POLYGON((outer), (hole), ...) -> [ring, ...], a ring a list of (lon, lat); the outer ring first."""
    s = text.strip()
    shape(s.startswith("POLYGON") and s.endswith(")") and "(" in s, "WKT: not a POLYGON")
    inner = s[s.index("(") + 1:-1].strip()
    shape(inner.startswith("(") and inner.endswith(")"), "WKT: POLYGON has no rings")
    rest = RING_RE.sub("", inner)
    shape(rest.strip(" ,") == "", "WKT: text outside the rings")
    rings = []
    for body in RING_RE.findall(inner):
        pts = []
        for pair in body.split(","):
            parts = pair.split()
            shape(len(parts) == 2, f"WKT: bad coordinate {pair!r}")
            try:
                x, y = float(parts[0]), float(parts[1])
            except ValueError as e:
                raise ShapeError(f"WKT: bad number in {pair!r}") from e
            shape(math.isfinite(x) and math.isfinite(y), "WKT: coordinate is not finite")
            pts.append((x, y))
        shape(len(pts) >= 4 and pts[0] == pts[-1], "WKT: a ring must be closed, with at least 3 vertices")
        rings.append(pts)
    return rings


def _in_ring(x, y, ring) -> bool:
    inside = False
    for i in range(len(ring) - 1):
        (x1, y1), (x2, y2) = ring[i], ring[i + 1]
        if (y1 > y) != (y2 > y) and x < (x2 - x1) * (y - y1) / (y2 - y1) + x1:
            inside = not inside
    return inside


def _inside(x, y, rings) -> bool:
    return _in_ring(x, y, rings[0]) and not any(_in_ring(x, y, h) for h in rings[1:])


# ---- the grid and the mean -------------------------------------------------------------------------------

def _shape_of(values):
    try:
        return values.shape
    except AttributeError:
        return len(values), len(values[0])


def region_mean(grid: dict, rings) -> float:
    """Area-weighted mean (weight cos latitude) of the pixels whose centres are in the region, nodata skipped."""
    west, north, cs = grid["west"], grid["north"], grid["cell_size"]
    nodata, values = grid.get("nodata"), grid["values"]
    nrows, ncols = _shape_of(values)
    xs = [p[0] for p in rings[0]]
    ys = [p[1] for p in rings[0]]
    c0 = max(0, math.floor((min(xs) - west) / cs) - 1)
    c1 = min(ncols - 1, math.floor((max(xs) - west) / cs) + 1)
    r0 = max(0, math.floor((north - max(ys)) / cs) - 1)
    r1 = min(nrows - 1, math.floor((north - min(ys)) / cs) + 1)
    num = den = 0.0
    for r in range(r0, r1 + 1):
        lat = north - (r + 0.5) * cs
        w = math.cos(math.radians(lat))
        for c in range(c0, c1 + 1):
            lon = west + (c + 0.5) * cs
            if not _inside(lon, lat, rings):
                continue
            v = float(values[r][c])
            if math.isnan(v) or (nodata is not None and v == nodata):
                continue
            num += w * v
            den += w
    if den == 0.0:
        raise EmptyRegion("no pixel with data has its centre in the region")
    return num / den


def region_row(region: str, month: str, grid: dict, rings) -> dict:
    """The regional mean, rounded half-even to two places (a computed value: rounding is allowed here only)."""
    mean = region_mean(grid, rings)
    q = Decimal(repr(mean)).quantize(Decimal("0.01"), rounding=ROUND_HALF_EVEN)
    return {"region": region, "month": month, "value": decimal_text(format(q, "f"))}


def read_geotiff(data: bytes) -> dict:
    """Decode a CHIRPS GeoTIFF (tifffile, numpy: imported here only). Returns a grid like synthetic_raster.json,
    its values a numpy array."""
    try:
        import tifffile
    except ImportError as e:
        raise RuntimeError("decoding a CHIRPS raster needs tifffile and numpy (see run_feed.sh)") from e
    with tifffile.TiffFile(io.BytesIO(data)) as tf:
        page = tf.pages[0]
        scale = page.tags.get(33550)
        tie = page.tags.get(33922)
        nodata_tag = page.tags.get(42113)
        shape(scale is not None and tie is not None, "GeoTIFF: no pixel scale or tiepoint")
        sx, sy = scale.value[0], scale.value[1]
        shape(abs(sx - sy) < 1e-12, "GeoTIFF: pixels are not square")
        i, j, _, x, y, _ = tie.value
        west = x - i * sx
        north = y + j * sy
        nodata = float(str(nodata_tag.value).strip()) if nodata_tag is not None else None
        values = page.asarray()
    shape(values.ndim == 2, "GeoTIFF: not a single band")
    return {"west": west, "north": north, "cell_size": sx, "nodata": nodata, "values": values}


# ---- lines -----------------------------------------------------------------------------------------------

def _label(sec, region):
    return sec["regions"][region]


def _mean_summary(sec, r, value):
    region = r["region"]
    return (f"{_label(sec, region)} ({region}), {r['month'][:7]}: {value} mm, the area-weighted mean of CHIRPS v3 "
            f"pixels inside the region; computed by datumesh.com from CHIRPS v3; not the source's own value.")


def _catalogue(sec, fq, rows):
    ordered = sorted(rows, key=lambda r: (r["month"], r["region"]))
    examples = []
    for r in ordered[-2:][::-1]:
        examples.append({"call": f'{fq}(region: "{r["region"]}", month: {r["month"]})',
                         "expected": {"type": "number", "value": decimal_text(r["value"])}})
    return {"catalog": fq, "kind": "function", "label": "CHIRPS regional rainfall (computed)",
            "description": ("the mean monthly rainfall in mm over a region, computed by datumesh.com from CHIRPS v3 "
                            "pixels whose centres lie in the region (area-weighted); not the source's own value"),
            "args": {"region": "the region code, for example US-KS", "month": "the first day of the month"},
            "examples": examples,
            "dictionary": {"region": [{"value": k, "label": v} for k, v in sec["regions"].items()]}}


def emit(rows, state, names, source):
    """The mean cells for one month's rows after the state, and the new state. The first call also emits the
    function and catalogue lines. `source` = {url, retrieved, sha256} of the raster file."""
    sec = names[KEY]
    fq = relay.fq_of(sec)
    new = [r for r in rows if r["month"] > state.get("last_date", "")]
    if not new:
        return [], state
    lines = []
    st = dict(state)
    if not state.get("fn"):
        lines.append({"function": fq, "signature": {
            "args": {"region": {"type": "string", "valid": False}, "month": {"type": "date", "valid": True}},
            "order": ["region", "month"], "returns": {"type": "number", "unit": sec["unit"]}}})
        lines.append(_catalogue(sec, fq, rows))
        st["fn"] = True
    src = {"url": source["url"], "retrieved": source["retrieved"], "sha256": source["sha256"]}
    for r in new:
        value = decimal_text(r["value"])
        lines.append({
            "cell": {"fn": fq, "args": {"region": {"type": "string", "value": r["region"]},
                                        "month": {"type": "date", "value": r["month"]}}},
            "value": {"type": "number", "value": value},
            "licence": sec["licence"],
            "summary": common.nfc(_mean_summary(sec, r, value)),
            "source": {"name": sec["source_name"], "url": src["url"], "retrieved": src["retrieved"],
                       "sha256": src["sha256"]},
        })
    st["last_date"] = max([state.get("last_date", "")] + [r["month"] for r in new])
    return lines, st


def raster_lines(names, month: str, sha: str, size: int, url: str, retrieved: str, state: dict):
    """The raster file's SHA-256 and size for the month, as stored cells under datumesh.com."""
    sec = names[KEY]
    fq_sha = f"{sec['namespace']}/{sec['raster_sha_function']}"
    fq_size = f"{sec['namespace']}/{sec['raster_bytes_function']}"
    st = dict(state)
    lines = []
    if not state.get("raster_fns"):
        margs = {"month": {"type": "date", "valid": True}}
        lines.append({"function": fq_sha, "signature": {"args": margs, "order": ["month"], "returns": "string"}})
        lines.append({"function": fq_size, "signature": {"args": margs, "order": ["month"],
                                                         "returns": {"type": "number", "unit": "bytes"}}})
        for fq, what in ((fq_sha, "SHA-256 of the CHIRPS v3 monthly GeoTIFF"), (fq_size, "size in bytes of the CHIRPS v3 monthly GeoTIFF")):
            lines.append({"catalog": fq, "kind": "function", "label": what[:60],
                          "description": (f"The {what} for the month, as fetched by datumesh.com; the raster is "
                                          f"the source's, the file's hash and size are datumesh.com's record."),
                          "args": {"month": "the first day of the month"}, "examples": [], "dictionary": {}})
        st["raster_fns"] = True
    src_host = sec["source_name"]
    margs = {"month": {"type": "date", "value": month}}
    summary = f"CHIRPS v3 monthly GeoTIFF for {month[:7]} as fetched by datumesh.com from {src_host}"
    for fq, value, vobj, what in ((fq_sha, sha, {"type": "string", "value": sha}, "SHA-256"),
                                  (fq_size, str(size), {"type": "number", "value": str(size)}, "size in bytes")):
        lines.append({"cell": {"fn": fq, "args": margs}, "value": vobj, "licence": sec["licence"],
                      "summary": f"{what} of the {summary}: {value}. Recorded by datumesh.com; not the source's own value.",
                      "source": {"name": src_host, "url": url, "retrieved": retrieved, "sha256": sha}})
    return lines, st


def region_wkt_lines(names, region: str, wkt: str, now: str, state: dict):
    """A region's polygon, once, as a stored cell (WKT) under datumesh.com; its source is the Natural Earth file."""
    if region in state.get("wkt_regions", []):
        return [], state
    sec = names[KEY]
    fq = f"{sec['namespace']}/{sec['region_wkt_function']}"
    st = dict(state)
    lines = []
    if not state.get("wkt_fn"):
        lines.append({"function": fq, "signature": {"args": {"region": {"type": "string", "valid": False}},
                                                    "order": ["region"], "returns": "string"}})
        lines.append({"catalog": fq, "kind": "function", "label": "CHIRPS region polygon (WKT)",
                      "description": ("the polygon of a region used for CHIRPS regional means, as WKT, from "
                                      "Natural Earth admin-1 (public domain); computed by datumesh.com"),
                      "args": {"region": "the region code, for example US-KS"}, "examples": [],
                      "dictionary": {"region": [{"value": k, "label": v} for k, v in sec["regions"].items()]}})
        st["wkt_fn"] = True
    url = sec["region_source_url"]
    lines.append({
        "cell": {"fn": fq, "args": {"region": {"type": "string", "value": region}}},
        "value": {"type": "string", "value": wkt},
        "licence": sec["region_licence"],
        "summary": common.nfc(f"Polygon of {sec['regions'][region]} ({region}) as WKT, converted by datumesh.com "
                              f"from the Natural Earth admin-1 file named in the source (public domain)."),
        "source": {"name": urlparse(url).hostname, "url": url, "retrieved": now,
                   "sha256": sec["region_source_sha256"]},
    })
    st["wkt_regions"] = sorted(set(state.get("wkt_regions", [])) | {region})
    return lines, st


# ---- run -------------------------------------------------------------------------------------------------

def _next_month(m: str) -> str:
    y, mo = int(m[:4]), int(m[5:7])
    y, mo = (y + 1, 1) if mo == 12 else (y, mo + 1)
    return f"{y:04d}-{mo:02d}-01"


def pending_months(last, now: str) -> list:
    """From the month after the last one kept (1981-01 first) to the month before now's (published a little later)."""
    today = dt.date.fromisoformat(now[:10])
    target = (today.replace(day=1) - dt.timedelta(days=1)).replace(day=1).isoformat()
    m = _next_month(last[:7] + "-01") if last else FIRST_MONTH
    out = []
    while m <= target:
        out.append(m)
        m = _next_month(m)
    return out


def seconds_to_wait(last_fetch, now_epoch: float) -> float:
    if last_fetch is None:
        return 0.0
    return max(0.0, THROTTLE_SECONDS - (now_epoch - last_fetch))


def run(names, *, fetch, now, state_dir, out_dir, publish=None, sleep=time.sleep, clock=time.time,
        max_months=None, decode=None):
    sec = names[KEY]
    state_path = Path(state_dir) / f"{KEY}.json"
    state = common.load_state(state_path)
    months = pending_months(state.get("last_date"), now)
    if max_months is not None:
        months = months[:max_months]
    if not months:
        return None
    decode = decode or read_geotiff
    regions = {}
    for region, rel in sec["region_files"].items():
        regions[region] = parse_wkt((common.FEEDS / rel).read_text(encoding="utf-8"))
    wkt_text = {region: (common.FEEDS / rel).read_text(encoding="utf-8").strip() for region, rel in sec["region_files"].items()}
    lines = []
    for month in months:
        url = sec["raster_url"].format(year=month[:4], month=month[5:7])
        wait = seconds_to_wait(state.get("last_fetch"), clock())
        if wait > 0:
            sleep(wait)
        try:
            data = fetch(url)
        except NotPublished:
            break  # not published yet: keep what was read, the next run starts here
        state["last_fetch"] = clock()
        sha = common.sha256_hex(data)
        src = {"url": url, "retrieved": now, "sha256": sha}
        grid = decode(data)
        rows = []
        for region, rings in regions.items():
            try:
                rows.append(region_row(region, month, grid, rings))
            except EmptyRegion:
                continue
        more, state = raster_lines(names, month, sha, len(data), url, now, state)
        lines += more
        for region in regions:
            more, state = region_wkt_lines(names, region, wkt_text[region], now, state)
            lines += more
        more, state = emit(rows, state, names, src)
        lines += more
        state["last_date"] = month
    if not lines:
        return None
    out = Path(out_dir) / f"{KEY}-{now.replace(':', '').replace('-', '')}.jsonl"
    common.write_jsonl(out, lines)
    if publish is not None:
        try:
            publish(out)
        except BaseException:
            out.unlink(missing_ok=True)
            raise
    common.save_state(state_path, state)
    return out


def main(argv=None):
    p = common.base_parser()
    p.add_argument("--max-months", type=int, default=None)
    a = p.parse_args(argv)
    return common.finish(run(common.names(), fetch=common.http_get, now=common.now_instant(),
                             state_dir=a.state_dir, out_dir=a.out_dir, max_months=a.max_months))


if __name__ == "__main__":
    raise SystemExit(main())
