# Feed jobs (Build 2 Part 5)

One fetch job per source, one `publish-stored` file per run. Names come only from `names.toml`, the **rehearsal** spelling (option B, `mesh-test.datumesh.com/relay/ecb_europa_eu/...`). A script never contains a namespace, function name, mesh host or takeover URL: it reads them from the table.

Contract: `/root/datumesh-libs/CONTRACT-build2-part5-feeds.md` (F1 to F6). Fixtures and where they came from: `fixtures/SOURCES.md`.

Status: all scripts are written and the tests pass. Nothing has been fetched from the live sources or published from this VM: `/srv/datumesh` does not exist here.

## Run the tests

```
docker run --rm -v "$PWD/deploy/prod/feeds:/feeds" -w /feeds python:3.13-slim \
  sh -c "pip install -q pytest && python -B -m pytest -q -p no:cacheprovider tests"
```

Python 3.13, standard library only in the scripts (`tomllib`, `xml.etree`, `csv`, `zipfile`, `decimal`, `urllib`, `hashlib`). The one exception is the CHIRPS raster decode, which imports `tifffile` and `numpy` only when a raster is read; `test_chirps_store.py` skips its decode test where they are missing. To run that test too: add `numpy==2.1.3 tifffile==2024.8.30` to the `pip install`.

## Layout

```
names.toml                    every name, namespace, unit, URL, licence, takeover, per source
common.py                     ShapeError, ConformanceError, EmptyRegion (all ValueError); decimals, JSON lines, state, HTTP
relay.py                      the shared emit, conformance and run for the five relay sources
ecb_fx.py estr.py policy_rates.py treasury.py boe.py   relay sources: parsers, TEXT (wording), CLI
chirps.py                     CHIRPS regional means and their stored cells (computed, not relayed)
register.py                   the licence register (run by hand from register.json)
run_feed.sh                   one source's job: fetch, check, write, publish, keep state (docker, cron)
install_cron.sh               prints the crontab lines; installs nothing
fixtures/                     real samples (SOURCES.md), register.json, a synthetic raster, a region polygon
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
- `region_mean(grid: dict, rings) -> float`. `grid` is `{"west", "north", "cell_size", "nodata", "values": [[..]]}`, rows north to south (the shape of `fixtures/synthetic_raster.json`; `read_geotiff(bytes)` produces it from a CHIRPS GeoTIFF with `tifffile` (values as a numpy array; the pixel scale and tiepoint give the origin; decided in D15, the libraries are pinned in `run_feed.sh`)). A pixel counts when its centre is inside the outer ring and not inside a hole; its weight is cos(latitude of its centre); nodata pixels are skipped; pixels outside the grid do not exist. No pixel left: `common.EmptyRegion`.
- `region_row(region, month, grid, rings) -> {"region", "month": "YYYY-MM-01", "value"}`, value rounded half-even to two places, trailing zeros stripped (this is a computed value, so rounding is allowed here and only here).
- `emit(rows, state, names, source)` as above, with `source` = `{"url": raster url, "retrieved", "sha256": raster hash}`. Function `chirps_region_rain(region: string, month: date [valid]) -> number` (unit mm) under the **computed** namespace of the table (never a relay name); `source.name` is `chc.ucsb.edu`, no `takeover`; the summary says "computed by datumesh.com from CHIRPS v3; not the source's own value"; the catalogue dictionary is `[chirps.regions]`. State key `last_date` is the last month.
- `raster_lines(...)`: the raster file's SHA-256 and byte size per month, as stored cells (`chirps_raster_sha256(month) -> string`, `chirps_raster_bytes(month) -> number`, names in `[chirps]`); `region_wkt_lines(...)`: each region's polygon once, as the stored cell `chirps_region_wkt(region) -> string`, with the Natural Earth file as its source.
- `run(...)`: the months after the state's last month, up to the month before now; one raster file at a time, at least 60 s apart (`seconds_to_wait`); a month not yet published (404) stops the run and keeps the months before it. Pixels outside the regions are never stored.
- Tests: `tests/test_chirps.py` (the means, on the synthetic grid) and `tests/test_chirps_store.py` (the stored cells, the throttle, the run, the decode path).

## Fetch jobs, cron and publish (F5)

- `python -B <module>.py --state-dir D --out-dir D [--mode history]` (`chirps.py` also `--max-months N`): fetch, check, write one `<module>-<stamp>.jsonl` to `D`, print its path. Prints nothing when nothing is new. It never publishes, and it writes the state file in `D` (the shell moves it after the publish succeeds).
- `run_feed.sh <module> [history]`: the one entry point. It takes `PYTHON_DIGEST` and `DATUMESH_PUBLISH_LANES` from `/srv/datumesh/.env`; fetches in `python:3.13-slim@sha256:$PYTHON_DIGEST` on the default bridge (outbound internet, not `datumesh_backend`); writes `/srv/datumesh/feeds/out/`; runs `docker compose -f /srv/datumesh/compose.yml run --rm -e DATUMESH_PUBLISH_LANES datumesh-mesh publish-stored <file>`; then moves the new state into `/srv/datumesh/feeds/state/`. A failed step leaves the state as it was, so the next run emits the same values again. Logs: `/srv/datumesh/logs/feeds-<module>.log`. A lock per module stops overlapping runs. CHIRPS installs `numpy` and `tifffile` in the container before its run.
- The compose service must see `/srv/datumesh/feeds/out/`. This is not checked here: `/srv/datumesh/compose.yml` is not on this VM.
- `install_cron.sh` prints the lines: ECB FX, €STR, policy rates, Treasury, BoE daily at off-peak minutes; CHIRPS on the 3rd of each month. It installs nothing. Paste them into the `datumesh` user's crontab only after the names are answered.
- ECB FX backfill: `run_feed.sh ecb_fx history` once, from `eurofxref-hist.zip`, before the daily cron. The daily feed carries only the latest day, so a missed day is a gap until a history run.
- Treasury: a run fetches the month of now, and every earlier month back to the last date kept, so a gap of days or months is refilled.

## `register.py` (F4)

- `parse(data: bytes) -> list[dict]` reads `register.json`, keeps `verdict == "RELAY"`, raises `ShapeError` on a missing key or a bad sha256.
- `run(names, path=..., now=..., state_dir=..., out_dir=...)` and the CLI `python -B register.py --register FILE --state-dir D --out-dir D`, run by hand (not on cron).
- `emit(entries, state, names) -> (lines, new_state)`: two function lines (`licence_text_sha256(source: string, retrieved: date) -> string`, `licence_page(...)`, names from `[register]`), their two catalogue lines (dictionary: the `source` domains), and per entry two cells: the licence text's SHA-256, and the licence page URL; `source` is the licence page itself (name its host, `url`, `retrieved` and `sha256` from the entry, no takeover). State key `done` is the list of `source|retrieved` pairs already emitted.
