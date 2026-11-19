# Feed jobs (Build 2 Part 5)

One script per source here, one `publish-stored` file per run. **Tests and fixtures come first; the scripts do not exist yet.** Read `names.toml` before anything: it is the only place a name lives, and it is the **rehearsal** spelling (option B, `mesh-test.datumesh.com/relay/ecb_europa_eu/...`). A script never contains a namespace, function name, mesh host or takeover URL: it reads them from the table.

Contract: `/root/datumesh-libs/CONTRACT-build2-part5-feeds.md` (F1 to F6). Fixtures and where they came from: `fixtures/SOURCES.md`.

## Run the tests

```
docker run --rm -v "$PWD/deploy/prod/feeds:/feeds" -w /feeds python:3.13-slim \
  sh -c "pip install -q pytest && python -B -m pytest -q -p no:cacheprovider tests"
```

Python 3.13, standard library only in the scripts (`tomllib`, `xml.etree`, `csv`, `zipfile`, `decimal`, `urllib`, `hashlib`); the raster reader is the one open question (see CHIRPS).

## Layout

```
names.toml                    every name, namespace, unit, URL, licence, takeover, per source
common.py                     ShapeError, ConformanceError, EmptyRegion (all ValueError); shared helpers
ecb_fx.py estr.py policy_rates.py treasury.py boe.py chirps.py register.py
fixtures/                     real samples (SOURCES.md), register.json, a synthetic raster
tests/                        pytest; read the names from names.toml, never hard-code them
```

A module's name is its section in `names.toml`.

## Interface of a relay script (`ecb_fx`, `estr`, `policy_rates`, `treasury`, `boe`)

- `parse(data: bytes) -> list[Row]`. A row is `{"date": "YYYY-MM-DD", "value": "<decimal exactly as published>", <key>: str}` where `<key>` is `currency` (ecb_fx), `rate` (policy_rates: `mro`, `dfr`, `mlfr`), `tenor` (treasury: `1M 1.5M 2M 3M 4M 6M 1Y 2Y 3Y 5Y 7Y 10Y 20Y 30Y`); estr and boe have no key. The value string keeps what the file prints (`0.83290`, `137.00`, `4.10`); a day or cell the file lacks (N/A, empty, weekend, holiday) is not a row. Unknown columns or elements (an unknown tenor, an unknown series key) and non-numeric values raise `common.ShapeError` with a message; nothing is emitted. `parse` reads the file named in `names.toml` `url` (ecb_fx: the daily XML; treasury: the month XML; policy_rates: one series file, the series taken from its `KEY` column and mapped back with `[policy_rates.series]`; boe: the `DD Mon YYYY` CSV).
- Second copies: `ecb_fx.parse_sdmx(bytes)` (EXR csvdata), `treasury.parse_csv(bytes)` (the year CSV, 14 tenors), same row shape. `ecb_fx.parse_hist(bytes)` reads the unzipped `eurofxref-hist.csv` and `ecb_fx.parse_hist_zip(bytes)` the zip (member `eurofxref-hist.csv`).
- `emit(rows, state: dict, names: dict, source: dict, second_rows=None) -> (lines: list[dict], new_state: dict)`. `names` is the whole parsed `names.toml`; `source` is `{"url", "retrieved", "sha256"}` of the fetch the rows came from (the script adds `name` and `takeover` from the table). Pure: no clock, no network.
  - Only rows with a date after `state["last_date"]` are emitted (a plain-JSON state; policy_rates keeps the date per rate; `{}` means first run). A second call with the returned state and the same rows returns `[]` and the same state.
  - Order of `lines`, first run: one `function` line, one `catalog` line, the `cell` lines, then the `test` lines (only when `second_rows` is given). Later runs: cell lines only. No `bound`, `tx` or `asof` anywhere (rule A: each run takes the clock's now).
  - `{"function": "<ns>/<fn>", "signature": {"args": {<key>: {"type": "string", "valid": false}, "date": {"type": "date", "valid": true}}, "order": [<key>, "date"], "returns": {"type": "number", "unit": "<unit>"}}}`.
  - `{"cell": {"fn": "<ns>/<fn>", "args": {<key>: {"type": "string", "value": ..}, "date": {"type": "date", "value": ..}}}, "value": {"type": "number", "value": <canon>}, "licence": <names licence>, "summary": ..., "source": {"name", "url", "retrieved", "sha256", "takeover"}}`. `<canon>` is the published decimal without insignificant trailing zeros (`0.83290` is `0.8329`, `137.00` is `137`): the same number, never rounded or rescaled. The summary (one line, NFC, at most 1024 bytes) names the source, says "relayed by datumesh.com", gives the takeover URL, the date and the value. `source` follows spec-0.6.0 C2: `url` is the exact file fetched, `sha256` the hash of its bytes.
  - `{"catalog": "<ns>/<fn>", "kind": "function", "label", "description", "args": {<arg>: text}, "examples": [{"call": "<ns>/<fn>(currency: \"USD\", date: 2026-11-18)", "expected": {"type": "number", "value": ..}}, ...], "dictionary": {<key>: [{"value", "label"}]}}`. At least two examples, each a row of the first run, so the value is the file's; the dictionary has every value of every key argument (policy rates: exactly mro, dfr, mlfr; treasury: all 14 tenors; ecb_fx: every currency in the rows, `USD` is "US dollar"; `1M` is "1-month"; `mro` is "main refinancing operations rate"), none for estr and boe (`{}`).
  - `{"test": "<ns>/<fn>", "call": "<same call syntax>", "expect": {"type": "number", "value": <canon of the second copy>}}`.
- `conformance(rows, second_rows, names) -> list[test line]`: one test per key and date present in both copies; raises `common.ConformanceError` when a pair disagrees as numbers (nothing is published that an independent copy contradicts); keys only one copy has are ignored.
- `run(names, *, fetch, now, state_dir, out_dir, publish=None, mode="daily") -> Path | None`. `fetch(url) -> bytes` (the real one sends `User-Agent: datumesh-feeds/0.1`, nothing else about us); `now` is an instant `YYYY-MM-DDTHH:MM:SS.ffffffZ` and becomes `source.retrieved`. It fetches, parses (a `ShapeError` leaves no file and no state), reads `<state_dir>/<module>.json`, emits, writes `<out_dir>/<module>-<stamp>.jsonl` (one JSON object a line, `ensure_ascii=False`), and returns its path; with nothing new it returns `None` and writes nothing. If `publish` is given it is called with the path (the real one runs `datumesh-mesh publish-stored`) and the state is saved **only after it returns**; if it raises, the state is untouched. The state file holds `last_date`. ecb_fx also fetches `second_url` for the conformance tests; `mode="history"` fetches `history_url` (the zip) instead and no second copy. treasury picks the month of `now` for `url` and the year for `second_url`. boe asks from the last date in the state (1975-01-01 first) to `now`.
- Each script is also a CLI: `python -I <module>.py --state-dir D --out-dir D [--publish] [--mode history]`, a thin wrapper over `run` with the real fetch (not tested).

## `chirps.py` (F3)

- `parse_wkt(text) -> list[ring]`, a ring a list of `(lon, lat)` floats, outer ring first, then holes; anything but a `POLYGON` raises `ShapeError`.
- `region_mean(grid: dict, rings) -> float`. `grid` is `{"west", "north", "cell_size", "nodata", "values": [[..]]}`, rows north to south (the shape of `fixtures/synthetic_raster.json`; the real GeoTIFF reader that produces it is not specified here and is the one place a library may be needed: CHIRPS v3 monthly files are 22 MB and the python image has no numpy, so decide before building: a small pure-Python TIFF reader, or the netcdf/cog form). A pixel counts when its centre is inside the outer ring and not inside a hole; its weight is cos(latitude of its centre); nodata pixels are skipped; pixels outside the grid do not exist. No pixel left: `common.EmptyRegion`.
- `region_row(region, month, grid, rings) -> {"region", "month": "YYYY-MM-01", "value"}`, value rounded half-even to two places, trailing zeros stripped (this is a computed value, so rounding is allowed here and only here).
- `emit(rows, state, names, source)` as above, with `source` = `{"url": raster url, "retrieved", "sha256": raster hash}`. Function `chirps_region_rain(region: string, month: date [valid]) -> number` (unit mm) under the **computed** namespace of the table (never a relay name); `source.name` is `chc.ucsb.edu`, no `takeover`; the summary says "computed by datumesh.com from CHIRPS v3; not the source's own value"; the catalogue dictionary is `[chirps.regions]`. State key `last_date` is the last month.
- Not covered by the tests yet: the stored cells for the raster file (hash and size) and the region polygon (WKT), and the 1-file-a-minute throttle; the implementer adds both with tests.

## `register.py` (F4)

- `parse(data: bytes) -> list[dict]` reads `register.json`, keeps `verdict == "RELAY"`, raises `ShapeError` on a missing key or a bad sha256.
- `emit(entries, state, names) -> (lines, new_state)`: two function lines (`licence_text_sha256(source: string, retrieved: date) -> string`, `licence_page(...)`, names from `[register]`), their two catalogue lines (dictionary: the `source` domains), and per entry two cells: the licence text's SHA-256, and the licence page URL; `source` is the licence page itself (name its host, `url`, `retrieved` and `sha256` from the entry, no takeover). State key `done` is the list of `source|retrieved` pairs already emitted.
