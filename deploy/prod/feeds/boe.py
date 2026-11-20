"""Bank of England Bank Rate (F1), series IUDBEDR: daily from 1975. Licence OGL-UK-3.0 (from names.toml)."""
import datetime as dt
import re

import common
import relay
from common import DECIMAL, shape

KEY = "boe"
KEYCOLS = []
MONTHS = {m: i for i, m in enumerate(["Jan", "Feb", "Mar", "Apr", "May", "Jun",
                                      "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"], start=1)}
BOE_DATE = re.compile(r"^(\d\d) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d{4})$")
FIRST_DAY = dt.date(1975, 1, 1)


def parse(data: bytes) -> list:
    """The CSV from the database: header DATE,IUDBEDR; dates as DD Mon YYYY (CRLF line ends)."""
    header, raw = common.csv_table(data)
    shape(header == ["DATE", "IUDBEDR"], f"Bank of England: header is {header}")
    rows = []
    for cells in raw:
        m = BOE_DATE.match(cells[0].strip())
        shape(m, f"Bank of England: bad date {cells[0]!r}")
        date = dt.date(int(m.group(3)), MONTHS[m.group(2)], int(m.group(1))).isoformat()
        v = cells[1].strip()
        if v == "":
            continue
        shape(DECIMAL.match(v), f"Bank of England: bad value {v!r} on {date}")
        rows.append({"date": date, "value": v})
    shape(rows, "Bank of England: no rows")
    return rows


PAGE_DAY = re.compile(r"^(\d\d) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d\d)$")


def parse_page(data: bytes) -> list:
    """The second copy: the Bank's page of the dates Bank Rate changed (two-digit years: the page starts in 1975, so
    75 to 99 are 19xx and the rest 20xx)."""
    text = common.decode(data)
    table = re.search(r'<table[^>]*id="stats-table"[^>]*>(.*?)</table>', text, re.S)
    shape(table, "Bank Rate page: no table")
    shape("Date Changed" in table.group(1) and "Rate" in table.group(1), "Bank Rate page: the columns are not those known")
    out = []
    for row in re.findall(r"<tr[^>]*>(.*?)</tr>", table.group(1).split("<tbody", 1)[-1], re.S):
        cells = [c.strip() for c in re.findall(r"<td[^>]*>(.*?)</td>", row, re.S)]
        shape(len(cells) == 2, f"Bank Rate page: unexpected row {cells!r}")
        m = PAGE_DAY.match(cells[0])
        shape(m, f"Bank Rate page: bad date {cells[0]!r}")
        yy = int(m.group(3))
        date = dt.date(1900 + yy if yy >= 75 else 2000 + yy, MONTHS[m.group(2)], int(m.group(1))).isoformat()
        shape(DECIMAL.match(cells[1]), f"Bank Rate page: bad value {cells[1]!r} on {date}")
        out.append({"date": date, "value": cells[1]})
    shape(out, "Bank Rate page: no rows")
    return out


def _dd_mon_yyyy(d: dt.date) -> str:
    return f"{d.day:02d}/{['Jan','Feb','Mar','Apr','May','Jun','Jul','Aug','Sep','Oct','Nov','Dec'][d.month - 1]}/{d.year}"


def _summary(sec, r, value):
    return f"Bank Rate on {r['date']}: {value} percent{relay.tail(sec)}."


TEXT = {
    "label": "Bank of England Bank Rate",
    "description": ("the Bank Rate in percent (series IUDBEDR), a value for each day it is published; "
                    "days with no value are absent"),
    "args": {"date": "the day"},
    "dictionary": lambda rows: {},
    "summary": _summary,
}


def emit(rows, state, names, source, second_rows=None):
    return relay.emit(KEY, rows, state, names, source, keycols=KEYCOLS, per_key=False, text=TEXT,
                      second_rows=second_rows)


def run(names, *, fetch, now, state_dir, out_dir, publish=None, mode="daily"):
    sec = names[KEY]
    today = dt.date.fromisoformat(now[:10])

    def inputs(state):
        last = state.get("last_date", "")
        start = dt.date.fromisoformat(last) if last else FIRST_DAY
        return [(sec["url"].format(**{"from": _dd_mon_yyyy(start), "to": _dd_mon_yyyy(today)}), parse)]

    return relay.run_series(
        KEY, names, fetch=fetch, now=now, state_dir=state_dir, out_dir=out_dir, publish=publish,
        inputs=inputs, second=(sec["second_url"], parse_page), keycols=KEYCOLS,
        emit_one=lambda rows, st, src: emit(rows, st, names, src))


def main(argv=None):
    a = common.base_parser().parse_args(argv)
    return common.finish(run(common.names(a.host), fetch=common.http_get, now=common.now_instant(),
                             state_dir=a.state_dir, out_dir=a.out_dir))


if __name__ == "__main__":
    raise SystemExit(main())
