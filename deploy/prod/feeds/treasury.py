"""US Treasury par yield curve (F1): 14 tenors per business day. The month XML is the relayed copy, the year CSV the
second, independent copy for conformance. BC_30YEARDISPLAY is a duplicate of BC_30YEAR and is ignored."""
import datetime as dt
import re
import time
import xml.etree.ElementTree as ET
from pathlib import Path

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
        # a day has the tenors Treasury quoted then: 9 in 1990, 14 since 2022; an unknown element is refused above
        shape(date and seen, f"Treasury XML: entry for {date} has no tenors")
        for t in TENORS:
            if t in vals:
                rows.append({"date": date, "value": vals[t], "tenor": t})
    shape(rows, "Treasury XML: no entries")
    return rows


def parse_csv(data: bytes) -> list:
    """The year CSV (daily yield curve), the second copy. Dates are MM/DD/YYYY."""
    header, raw = common.csv_table(data)
    shape(header and header[0] == "Date" and len(header) > 1 and set(header[1:]) <= set(CSV_TENORS)
          and len(set(header)) == len(header), f"Treasury CSV: header is not Date and known tenors: {header}")
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


FIRST_YEAR = 1990
THROTTLE_SECONDS = 2.0  # at most one request every two seconds to the Treasury in a history run


def seconds_to_wait(last_fetch, now_epoch: float) -> float:
    if last_fetch is None:
        return 0.0
    return max(0.0, THROTTLE_SECONDS - (now_epoch - last_fetch))


def _throttled(fetch, sleep, clock):
    last = [None]

    def get(url):
        wait = seconds_to_wait(last[0], clock())
        if wait > 0:
            sleep(wait)
        data = fetch(url)
        last[0] = clock()
        return data
    return get


def run_history(names, *, fetch, now, state_dir, out_dir, publish=None, sleep=time.sleep, clock=time.time):
    """One year of the history from 1990 per call: the year after state["history_year"], its months one file at a
    time (at least 2 s apart), the year's CSV for the tests of its latest dates. It writes one file for that year and
    returns it; the next call does the next year, so a failure loses at most the year it was in. The history has its
    own cursor (`history_last`), so a mesh that already has later daily cells still gets the earlier years."""
    sec = names[KEY]
    today = dt.date.fromisoformat(now[:10])
    state_path = Path(state_dir) / f"{KEY}.json"
    state = common.load_state(state_path)
    get = _throttled(fetch, sleep, clock)
    year = state.get("history_year", FIRST_YEAR - 1) + 1
    while year <= today.year:
        last_month = today.month if year == today.year else 12
        st, lines, rows_all = dict(state), [], []
        for month in range(1, last_month + 1):
            url = sec["url"].format(month=f"{year:04d}{month:02d}")
            data = get(url)
            rows = parse(data)
            rows_all += rows
            src = {"url": url, "retrieved": now, "sha256": common.sha256_hex(data)}
            more, st = relay.emit(KEY, rows, st, names, src, keycols=KEYCOLS, per_key=False, text=TEXT,
                                  cursor="history_last")
            lines += more
        second_rows = relay.second_copy((sec["second_url"].format(year=year), parse_csv), get)
        if second_rows is not None:
            tests, st = relay.new_tests(KEY, rows_all, second_rows, names, KEYCOLS, st, tested_key="tested_history")
            lines += tests
        st["history_year"] = year
        if "history_last" in st:  # the daily run goes on from the latest date there is
            st["last_date"] = max(st.get("last_date", ""), st["history_last"])
        marks = dict(st.get("tested", {}))
        for k, d in st.get("tested_history", {}).items():
            marks[k] = max(marks.get(k, ""), d)
        if marks:
            st["tested"] = marks
        state = st
        if lines:
            path = Path(out_dir) / f"{KEY}-{relay._stamp(now)}-{year}.jsonl"
            common.write_jsonl(path, lines)
            if publish is not None:
                try:
                    publish(path)
                except BaseException:
                    path.unlink(missing_ok=True)
                    raise
            common.save_state(state_path, state)
            return path
        year += 1
    common.save_state(state_path, state)
    return None


def run(names, *, fetch, now, state_dir, out_dir, publish=None, mode="daily", sleep=time.sleep, clock=time.time):
    if mode == "history":
        return run_history(names, fetch=fetch, now=now, state_dir=state_dir, out_dir=out_dir, publish=publish,
                           sleep=sleep, clock=clock)
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
    a = common.base_parser(("daily", "history")).parse_args(argv)
    return common.finish(run(common.names(a.host), fetch=common.http_get, now=common.now_instant(),
                             state_dir=a.state_dir, out_dir=a.out_dir, mode=a.mode))


if __name__ == "__main__":
    raise SystemExit(main())
