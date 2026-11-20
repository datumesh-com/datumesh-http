"""The real CHIRPS v3 monthly GeoTIFF is LZW compressed: decoding it needs imagecodecs (pinned, with numpy and
tifffile, in run_feed.sh and the README). The fixture is a crop of a real raster, written LZW compressed."""
import re

import pytest

import chirps
from conftest import FEEDS, fixture

KANSAS_MEAN = 125.29368375747895  # the full file's mean and the crop's, asserted when the fixture was made


def test_the_real_crop_is_lzw_and_decodes_to_the_full_files_kansas_mean():
    tifffile = pytest.importorskip("tifffile")
    pytest.importorskip("numpy")
    data = fixture("chirps_v3_2024-06_kansas_lzw.tif")
    with tifffile.TiffFile(__import__("io").BytesIO(data)) as tf:
        assert tf.pages[0].compression.name == "LZW"
    g = chirps.read_geotiff(data)  # a ValueError about imagecodecs here means the pin is missing
    assert g["values"].shape == (102, 190)
    assert (g["west"], g["north"]) == (-103.0999988541007, 41.049999717622995) and abs(g["cell_size"] - 0.05) < 1e-8
    rings = chirps.parse_wkt_multi((FEEDS / "fixtures/regions/us_kansas.wkt").read_text(encoding="utf-8"))
    assert chirps.region_mean(g, rings) == pytest.approx(KANSAS_MEAN, abs=1e-9)
    assert chirps.region_row("us_kansas", "2024-06-01", g, rings)["value"] == "125.29"


def _write_tif(path, values, west, north, cs, nodata_tag):
    tifffile = pytest.importorskip("tifffile")
    import numpy as np
    extra = [(33550, "d", 3, (cs, cs, 0.0), True), (33922, "d", 6, (0.0, 0.0, 0.0, west, north, 0.0), True)]
    if nodata_tag:
        extra.append((42113, "s", 0, "-9999", True))
    tifffile.imwrite(path, np.array(values, dtype="float32"), extratags=extra)


def test_minus_9999_is_nodata_even_when_the_file_has_no_nodata_tag(tmp_path):
    """The real raster has no nodata tag, and -9999 where it has no data."""
    pytest.importorskip("tifffile")
    rows = [[10, 20, 30], [40, -9999, 60], [70, 80, 90]]
    for tag in (True, False):
        p = tmp_path / f"t{tag}.tif"
        _write_tif(p, rows, 0.0, 3.0, 1.0, tag)
        g = chirps.read_geotiff(p.read_bytes())
        assert g["nodata"] == -9999.0
        square = [[(0, 0), (3, 0), (3, 3), (0, 3), (0, 0)]]
        mean = chirps.region_mean(g, square)
        import math
        w = lambda r: math.cos(math.radians(3.0 - (r + 0.5)))  # noqa: E731
        vals = [(w(r), v) for r, row in enumerate(rows) for v in row if v != -9999]
        assert mean == pytest.approx(sum(a * b for a, b in vals) / sum(a for a, _ in vals), abs=1e-9)


def test_a_missing_imagecodecs_is_said_in_words(monkeypatch):
    tifffile = pytest.importorskip("tifffile")

    class NoCodecs:
        def __init__(self, *a, **k):
            pass

        def __enter__(self):
            raise ValueError("<COMPRESSION.LZW: 5> requires the 'imagecodecs' package")

        def __exit__(self, *a):
            return False

    monkeypatch.setattr(tifffile, "TiffFile", NoCodecs)
    with pytest.raises(RuntimeError, match="imagecodecs"):
        chirps.read_geotiff(b"x")


def test_the_dependencies_are_pinned_where_the_job_installs_them():
    pin = re.compile(r"numpy==(\d[\w.]*) tifffile==(\d[\w.]*) imagecodecs==(\d{4}\.\d+\.\d+)")
    sh = pin.search((FEEDS / "run_feed.sh").read_text())
    readme = pin.search((FEEDS / "README.md").read_text())
    assert sh and readme and sh.groups() == readme.groups() == ("2.1.3", "2024.8.30", "2024.12.30")
    # the README's test command installs the same three
    assert "imagecodecs" in re.search(r"pip install[^\n]*", (FEEDS / "README.md").read_text()).group(0)
