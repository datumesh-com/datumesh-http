"""Makes fixtures/chirps_v3_2024-06_kansas_lzw.tif from one real CHIRPS v3 monthly GeoTIFF (untrusted data: it is only
read as a raster, run with python -I).

usage: python -I make_chirps_kansas_lzw.py FEEDS_DIR chirps-v3.0.2024.06.tif OUT.tif
Needs numpy, tifffile and imagecodecs (the raster is LZW compressed). It crops the raster in memory to a window around
Kansas (the polygon's box plus one degree), writes it LZW compressed with the georeferencing tags of the original
(pixel scale, tiepoint moved to the window's corner, nodata, the GeoKey tags), and checks that read_geotiff gives the
same Kansas mean on the crop as on the full file."""
import math
import sys
from pathlib import Path

import numpy as np
import tifffile

feeds, src, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
sys.path.insert(0, str(feeds))
import chirps  # noqa: E402

data = src.read_bytes()
full = chirps.read_geotiff(data)
rings = chirps.parse_wkt_multi((feeds / "fixtures/regions/us_kansas.wkt").read_text(encoding="utf-8"))
mean_full = chirps.region_mean(full, rings)

west, north, cs = full["west"], full["north"], full["cell_size"]
xs = [x for poly in rings for x, _ in poly[0]]
ys = [y for poly in rings for _, y in poly[0]]
c0 = math.floor((min(xs) - 1 - west) / cs)
c1 = math.ceil((max(xs) + 1 - west) / cs)
r0 = math.floor((north - (max(ys) + 1)) / cs)
r1 = math.ceil((north - (min(ys) - 1)) / cs)
crop = np.ascontiguousarray(full["values"][r0:r1, c0:c1])

with tifffile.TiffFile(src) as tf:
    page = tf.pages[0]
    scale, tie, nodata = page.tags.get(33550), page.tags.get(33922), page.tags.get(42113)
    keys = [page.tags.get(t) for t in (34735, 34736, 34737)]
    new_tie = (0.0, 0.0, 0.0, west + c0 * cs, north - r0 * cs, 0.0)
    extra = [(33550, "d", 3, tuple(scale.value), True), (33922, "d", 6, new_tie, True)]
    if nodata is not None:
        extra.append((42113, "s", 0, str(nodata.value).strip(), True))
    for t in keys:
        if t is not None:
            code = {34735: "H", 34736: "d", 34737: "s"}[t.code]
            val = t.value if t.code != 34737 else str(t.value)
            extra.append((t.code, code, 0 if t.code == 34737 else len(val), val, True))
tifffile.imwrite(out, crop, compression="lzw", extratags=extra, photometric="minisblack")

with tifffile.TiffFile(out) as tf:
    assert tf.pages[0].compression.name == "LZW", tf.pages[0].compression
small = chirps.read_geotiff(out.read_bytes())
mean_small = chirps.region_mean(small, rings)
assert (small["west"], small["north"], small["cell_size"], small["nodata"]) == (
    west + c0 * cs, north - r0 * cs, cs, full["nodata"]), "georeferencing changed"
assert mean_small == mean_full, (mean_small, mean_full)
print("crop rows", r0, r1, "cols", c0, c1, "shape", crop.shape, "bytes", out.stat().st_size)
print("origin", small["west"], small["north"], "cell", cs, "nodata", full["nodata"])
print("kansas mean (full == crop):", repr(mean_full))
print("rounded", chirps.region_row("us_kansas", "2024-06-01", small, rings))
