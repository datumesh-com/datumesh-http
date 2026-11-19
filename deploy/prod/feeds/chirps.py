"""CHIRPS v3 monthly rainfall, regional means (F3). Computed by datumesh.com, never a relay name.

The aggregation runs here because the engine has no raster reader. A pixel counts when its centre is inside the
region's outer ring and outside its holes; its weight is cos(latitude of its centre); nodata is skipped.
Stored cells: the mean per region and month, with no `source` member (D17); the raster's URL and SHA-256, the method
and the Natural Earth region are in the summary and the function description. The raster is decoded with tifffile and numpy, imported only when a raster is decoded (D15)."""
import datetime as dt
import io
import math
import re
import time
from decimal import ROUND_HALF_EVEN, Decimal
from pathlib import Path

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

COMPUTED = "computed by datumesh.com from CHIRPS v3; not CHIRPS's own value"


def _label(sec, region):
    return sec["regions"][region]["label"]


def _mean_summary(sec, r, value, source):
    """The lineage is here and in the function description, not in a `source` member (D17): the raster's URL and
    SHA-256, the method, the Natural Earth region id and file."""
    region = r["region"]
    return (f"{_label(sec, region)} ({region}), {r['month'][:7]}: {value} mm, the area-weighted mean (weight cos "
            f"latitude) of the CHIRPS v3 pixels whose centres fall inside the region; {COMPUTED}. "
            f"Raster {source['url']} sha256 {source['sha256']}. "
            f"Region: {sec['region_source']}, id {sec['regions'][region]['ne_id']}, "
            f"file sha256 {sec['region_source_sha256']}.")


def _catalogue(sec, fq, rows):
    ordered = sorted(rows, key=lambda r: (r["month"], r["region"]))
    examples = []
    for r in ordered[-2:][::-1]:
        examples.append({"call": f'{fq}(dataset: "{sec["dataset"]}", region: "{r["region"]}", month: {r["month"]})',
                         "expected": {"type": "number", "value": decimal_text(r["value"])}})
    description = (f"Mean monthly rainfall in mm over a region, {COMPUTED}. Method: the area-weighted mean "
                   f"(weight cos latitude) of the pixels whose centres lie inside the region in the CHIRPS v3 monthly "
                   f"raster, nodata skipped, rounded to two places. Region: the polygon from {sec['region_source']}, "
                   f"by its region id. Each cell's summary gives its raster URL and SHA-256 and the region's Natural "
                   f"Earth id and file hash. Rasters: {sec['raster_url']}.")
    return {"catalog": fq, "kind": "function", "label": "Monthly rainfall by region (computed)",
            "description": description,
            "args": {"dataset": "the rainfall dataset: chirps",
                     "region": "the region id, snake_case, for example us_kansas",
                     "month": "the first day of the month"},
            "examples": examples,
            "dictionary": {"dataset": [{"value": sec["dataset"], "label": sec["dataset_label"]}],
                           "region": [{"value": k, "label": v["label"]} for k, v in sec["regions"].items()]}}


def emit(rows, state, names, source):
    """The mean cells for one month's rows after the state, and the new state. The first call also emits the
    function and catalogue lines. `source` = {url, retrieved, sha256} of the raster file. A cell has no `source`
    member: its receipt is primary, not relayed (D17)."""
    sec = names[KEY]
    fq = relay.fq_of(sec)
    new = [r for r in rows if r["month"] > state.get("last_date", "")]
    if not new:
        return [], state
    lines = []
    st = dict(state)
    if not state.get("fn"):
        lines.append({"function": fq, "signature": {
            "args": {"dataset": {"type": "string", "valid": False}, "region": {"type": "string", "valid": False},
                     "month": {"type": "date", "valid": True}},
            "order": ["dataset", "region", "month"], "returns": {"type": "number", "unit": sec["unit"]}}})
        lines.append(_catalogue(sec, fq, rows))
        st["fn"] = True
    for r in new:
        value = decimal_text(r["value"])
        lines.append({
            "cell": {"fn": fq, "args": {"dataset": {"type": "string", "value": sec["dataset"]},
                                        "region": {"type": "string", "value": r["region"]},
                                        "month": {"type": "date", "value": r["month"]}}},
            "value": {"type": "number", "value": value},
            "licence": sec["licence"],
            "summary": common.nfc(_mean_summary(sec, r, value, source)),
        })
    st["last_date"] = max([state.get("last_date", "")] + [r["month"] for r in new])
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
    for region, spec in sec["regions"].items():
        regions[region] = parse_wkt((common.FEEDS / spec["file"]).read_text(encoding="utf-8"))
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
    return common.finish(run(common.names(a.host), fetch=common.http_get, now=common.now_instant(),
                             state_dir=a.state_dir, out_dir=a.out_dir, max_months=a.max_months))


if __name__ == "__main__":
    raise SystemExit(main())
