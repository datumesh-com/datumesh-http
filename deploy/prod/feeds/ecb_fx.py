"""ECB euro foreign exchange reference rates (F1). Daily XML; history zip (backfill); SDMX copy (conformance)."""
import csv
import io
import re
import xml.etree.ElementTree as ET
import zipfile
import zlib

import common
import relay
from common import DECIMAL, ISO_DATE, ShapeError, decode, shape

KEY = "ecb_fx"
KEYCOLS = ["currency"]
NS = "{http://www.ecb.int/vocabulary/2002-08-01/eurofxref}"
CODE = re.compile(r"^[A-Z]{3}$")
HIST_MEMBER = "eurofxref-hist.csv"
CURRENCY_LABELS = {
    "USD": "US dollar", "JPY": "Japanese yen", "BGN": "Bulgarian lev", "CYP": "Cypriot pound",
    "CZK": "Czech koruna", "DKK": "Danish krone", "EEK": "Estonian kroon", "GBP": "Pound sterling",
    "HUF": "Hungarian forint", "LTL": "Lithuanian litas", "LVL": "Latvian lats", "MTL": "Maltese lira",
    "PLN": "Polish zloty", "ROL": "Romanian leu (old)", "RON": "Romanian leu", "SEK": "Swedish krona",
    "SIT": "Slovenian tolar", "SKK": "Slovak koruna", "CHF": "Swiss franc", "ISK": "Icelandic krona",
    "NOK": "Norwegian krone", "HRK": "Croatian kuna", "RUB": "Russian rouble", "TRL": "Turkish lira (old)",
    "TRY": "Turkish lira", "AUD": "Australian dollar", "BRL": "Brazilian real", "CAD": "Canadian dollar",
    "CNY": "Chinese yuan renminbi", "HKD": "Hong Kong dollar", "IDR": "Indonesian rupiah",
    "ILS": "Israeli new shekel", "INR": "Indian rupee", "KRW": "South Korean won", "MXN": "Mexican peso",
    "MYR": "Malaysian ringgit", "NZD": "New Zealand dollar", "PHP": "Philippine peso", "SGD": "Singapore dollar",
    "THB": "Thai baht", "ZAR": "South African rand",
}


def _local(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def parse(data: bytes) -> list:
    """The daily XML (one day, about 30 currencies)."""
    try:
        root = ET.fromstring(data)
    except ET.ParseError as e:
        raise ShapeError(f"ECB daily XML does not parse: {e}") from e
    outer = root.findall(f"{NS}Cube")
    shape(len(outer) == 1, "ECB daily XML: expected one outer Cube")
    days = outer[0].findall(f"{NS}Cube")
    shape(days, "ECB daily XML: no day")
    rows = []
    for day in days:
        shape(set(day.attrib) == {"time"} and len(day) > 0, f"ECB daily XML: day element {dict(day.attrib)}")
        date = day.attrib["time"]
        shape(ISO_DATE.match(date), f"ECB daily XML: bad date {date!r}")
        for c in day:
            shape(c.tag == f"{NS}Cube" and set(c.attrib) == {"currency", "rate"},
                  f"ECB daily XML: unexpected element {c.tag} {sorted(c.attrib)}")
            cur, rate = c.attrib["currency"], c.attrib["rate"]
            shape(CODE.match(cur), f"ECB daily XML: bad currency {cur!r}")
            shape(DECIMAL.match(rate), f"ECB daily XML: bad rate {rate!r} for {cur}")
            rows.append({"date": date, "value": rate, "currency": cur})
    return rows


def parse_hist(data: bytes) -> list:
    """eurofxref-hist.csv: one column per currency, a row per day; N/A cells and the trailing comma are not rows."""
    reader = csv.reader(io.StringIO(decode(data), newline=""))
    try:
        header = next(reader)
    except StopIteration:
        raise ShapeError("ECB history: empty file") from None
    shape(header and header[0] == "Date", "ECB history: first column is not Date")
    cols = [(i, h) for i, h in enumerate(header) if i > 0 and h]
    shape(cols and all(CODE.match(h) for _, h in cols), "ECB history: a column is not a currency code")
    rows = []
    for raw in reader:
        if not raw:
            continue
        shape(len(raw) == len(header), f"ECB history: row has {len(raw)} cells, header {len(header)}")
        date = raw[0]
        shape(ISO_DATE.match(date), f"ECB history: bad date {date!r}")
        for i, cur in cols:
            v = raw[i].strip()
            if v in ("", "N/A"):
                continue
            shape(DECIMAL.match(v), f"ECB history: bad rate {v!r} for {cur} on {date}")
            rows.append({"date": date, "value": v, "currency": cur})
    shape(rows, "ECB history: no rates")
    return rows


def parse_hist_zip(data: bytes) -> list:
    try:
        with zipfile.ZipFile(io.BytesIO(data)) as z:
            shape(HIST_MEMBER in z.namelist(), f"ECB history zip: no {HIST_MEMBER}")
            return parse_hist(z.read(HIST_MEMBER))
    except (zipfile.BadZipFile, zlib.error, KeyError) as e:
        raise ShapeError(f"ECB history zip does not open: {e}") from e


def parse_sdmx(data: bytes) -> list:
    """The SDMX csvdata (EXR), the second, independent copy."""
    rows = common.csv_dicts(data, {"KEY", "CURRENCY", "CURRENCY_DENOM", "TIME_PERIOD", "OBS_VALUE"})
    out = []
    for r in rows:
        cur = r["CURRENCY"]
        shape(CODE.match(cur) and r["KEY"] == f"EXR.D.{cur}.EUR.SP00.A", f"ECB SDMX: unexpected key {r['KEY']!r}")
        shape(r["CURRENCY_DENOM"] == "EUR", "ECB SDMX: denominator is not EUR")
        date, v = r["TIME_PERIOD"], r["OBS_VALUE"].strip()
        shape(ISO_DATE.match(date), f"ECB SDMX: bad date {date!r}")
        if v in ("", "N/A"):
            continue
        shape(DECIMAL.match(v), f"ECB SDMX: bad value {v!r}")
        out.append({"date": date, "value": v, "currency": cur})
    shape(out, "ECB SDMX: no rows")
    return out


def _dictionary(rows):
    return {"currency": [{"value": c, "label": CURRENCY_LABELS.get(c, c)}
                         for c in sorted({r["currency"] for r in rows})]}


def _summary(sec, r, value):
    return f"1 EUR = {value} {r['currency']} on {r['date']}, ECB euro reference rate{relay.tail(sec)}."


TEXT = {
    "label": "ECB euro foreign exchange reference rate",
    "description": ("units of the currency per 1 euro (unit 1), the ECB's daily reference rate "
                    "for each day it publishes one; days with no rate for a currency are absent"),
    "args": {"currency": "ISO 4217 code of the currency, for example USD",
             "date": "the reference day (a day the ECB published rates)"},
    "dictionary": _dictionary,
    "summary": _summary,
}


def emit(rows, state, names, source, second_rows=None):
    return relay.emit(KEY, rows, state, names, source, keycols=KEYCOLS, per_key=False, text=TEXT,
                      second_rows=second_rows)


def conformance(rows, second_rows, names):
    return relay.conformance(KEY, rows, second_rows, names, KEYCOLS)


def run(names, *, fetch, now, state_dir, out_dir, publish=None, mode="daily"):
    sec = names[KEY]
    if mode == "history":
        inputs = lambda st: [(sec["history_url"], parse_hist_zip)]  # noqa: E731
        second = (sec["second_url_history"], parse_sdmx)  # the last 20 observations: tests on the latest dates
    else:
        inputs = lambda st: [(sec["url"], parse)]  # noqa: E731
        second = (sec["second_url"], parse_sdmx)
    return relay.run_series(
        KEY, names, fetch=fetch, now=now, state_dir=state_dir, out_dir=out_dir, publish=publish,
        inputs=inputs, second=second, keycols=KEYCOLS,
        emit_one=lambda rows, st, src: emit(rows, st, names, src))


def main(argv=None):
    a = common.base_parser(("daily", "history")).parse_args(argv)
    return common.finish(run(common.names(a.host), fetch=common.http_get, now=common.now_instant(),
                             state_dir=a.state_dir, out_dir=a.out_dir, mode=a.mode))


if __name__ == "__main__":
    raise SystemExit(main())
