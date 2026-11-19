"""US Treasury par yield curve (F1): 14 tenors per business day. The month XML is the relayed copy, the year CSV the
second, independent copy for conformance. BC_30YEARDISPLAY is a duplicate of BC_30YEAR and is ignored."""
import datetime as dt
import re
import xml.etree.ElementTree as ET

import common
import relay
from common import DECIMAL, ShapeError, shape

KEY = "treasury"
KEYCOLS = ["tenor"]
DS = "{http://schemas.microsoft.com/ado/2007/08/dataservices}"
MD = "{http://schemas.microsoft.com/ado/2007/08/dataservices/metadata}"
XML_TENORS = {"BC_1MONTH": "1M", "BC_1_5MONTH": "1.5M", "BC_2MONTH": "2M", "BC_3MONTH": "3M", "BC_4MONTH": "4M",
              "BC_6MONTH": "6M", "BC_1YEAR": "1Y", "BC_2YEAR": "2Y", "BC_3YEAR": "3Y", "BC_5YEAR": "5Y",
              "BC_7YEAR": "7Y", "BC_10YEAR": "10Y", "BC_20YEAR": "20Y", "BC_30YEAR": "30Y"}
XML_IGNORED = {"Id", "BC_30YEARDISPLAY"}
CSV_TENORS = {"1 Mo": "1M", "1.5 Month": "1.5M", "2 Mo": "2M", "3 Mo": "3M", "4 Mo": "4M", "6 Mo": "6M",
              "1 Yr": "1Y", "2 Yr": "2Y", "3 Yr": "3Y", "5 Yr": "5Y", "7 Yr": "7Y", "10 Yr": "10Y",
              "20 Yr": "20Y", "30 Yr": "30Y"}
TENORS = ["1M", "1.5M", "2M", "3M", "4M", "6M", "1Y", "2Y", "3Y", "5Y", "7Y", "10Y", "20Y", "30Y"]
TENOR_LABELS = {"1M": "1-month", "1.5M": "1.5-month", "2M": "2-month", "3M": "3-month", "4M": "4-month",
                "6M": "6-month", "1Y": "1-year", "2Y": "2-year", "3Y": "3-year", "5Y": "5-year",
                "7Y": "7-year", "10Y": "10-year", "20Y": "20-year", "30Y": "30-year"}
DATE_T = re.compile(r"^(\d{4}-\d\d-\d\d)T00:00:00$")
US_DATE = re.compile(r"^(\d\d)/(\d\d)/(\d{4})$")


def _local(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def parse(data: bytes) -> list:
    """The month XML (OData atom feed: one entry per day, one property per tenor)."""
    try:
        root = ET.fromstring(data)
    except ET.ParseError as e:
        raise ShapeError(f"Treasury XML does not parse: {e}") from e
    rows = []
    for el in root.iter():
        if _local(el.tag) != "properties":
            continue
        date, seen, vals = None, set(), {}
        for c in el:
            name = _local(c.tag)
            if name in XML_IGNORED:
                continue
            if name == "NEW_DATE":
                m = DATE_T.match((c.text or "").strip())
                shape(m, f"Treasury XML: bad date {c.text!r}")
                date = m.group(1)
                continue
            shape(name in XML_TENORS, f"Treasury XML: unknown element {name}")
            seen.add(name)
            if c.get(f"{MD}null") == "true":
                continue
            v = (c.text or "").strip()
            if v == "":
                continue
            shape(DECIMAL.match(v), f"Treasury XML: bad value {v!r} for {name}")
            vals[XML_TENORS[name]] = v
        shape(date and seen == set(XML_TENORS), f"Treasury XML: entry for {date} has {len(seen)} tenors")
        for t in TENORS:
            if t in vals:
                rows.append({"date": date, "value": vals[t], "tenor": t})
    shape(rows, "Treasury XML: no entries")
    return rows


def parse_csv(data: bytes) -> list:
    """The year CSV (daily yield curve), the second copy. Dates are MM/DD/YYYY."""
    header, raw = common.csv_table(data)
    shape(header and header[0] == "Date" and set(header[1:]) == set(CSV_TENORS) and len(header) == 15,
          f"Treasury CSV: header is not Date and the 14 tenors: {header}")
    cols = {i: CSV_TENORS[h] for i, h in enumerate(header) if i > 0}
    rows = []
    for cells in raw:
        m = US_DATE.match(cells[0])
        shape(m, f"Treasury CSV: bad date {cells[0]!r}")
        date = dt.date(int(m.group(3)), int(m.group(1)), int(m.group(2))).isoformat()
        for i, t in cols.items():
            v = cells[i].strip()
            if v == "":
                continue
            shape(DECIMAL.match(v), f"Treasury CSV: bad value {v!r} for {t} on {date}")
            rows.append({"date": date, "value": v, "tenor": t})
    shape(rows, "Treasury CSV: no rows")
    return rows


def _dictionary(rows):
    return {"tenor": [{"value": t, "label": TENOR_LABELS[t]} for t in TENORS]}


def _summary(sec, r, value):
    return f"US Treasury par yield, {r['tenor']}, on {r['date']}: {value} percent{relay.tail(sec)}."


TEXT = {
    "label": "US Treasury par yield",
    "description": ("the par yield curve rate in percent for each of 14 tenors (1M to 30Y), "
                    "per business day; days a tenor is not quoted are absent"),
    "args": {"tenor": "the tenor, one of 1M, 1.5M, 2M, 3M, 4M, 6M, 1Y, 2Y, 3Y, 5Y, 7Y, 10Y, 20Y, 30Y",
             "date": "the business day"},
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
    today = dt.date.fromisoformat(now[:10])

    def months(state):
        # the month of now, and every earlier month back to the last date kept (a gap of days or months)
        start = state.get("last_date", "")[:7] or today.strftime("%Y-%m")
        y, m = int(start[:4]), int(start[5:7])
        out = []
        while (y, m) <= (today.year, today.month):
            out.append(f"{y:04d}{m:02d}")
            y, m = (y + 1, 1) if m == 12 else (y, m + 1)
        return out

    def inputs(state):
        return [(sec["url"].format(month=mm), parse) for mm in months(state)]

    return relay.run_series(
        KEY, names, fetch=fetch, now=now, state_dir=state_dir, out_dir=out_dir, publish=publish,
        inputs=inputs, second=(sec["second_url"].format(year=today.year), parse_csv), keycols=KEYCOLS,
        emit_one=lambda rows, st, src: emit(rows, st, names, src))


def main(argv=None):
    a = common.base_parser().parse_args(argv)
    return common.finish(run(common.names(), fetch=common.http_get, now=common.now_instant(),
                             state_dir=a.state_dir, out_dir=a.out_dir))


if __name__ == "__main__":
    raise SystemExit(main())
