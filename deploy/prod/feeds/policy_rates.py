"""ECB key interest rates (F1): MRO, deposit facility and marginal lending facility.

These are date-of-change series: a row exists only on a decision date, so only those observations are cells (D14).
No value is carried forward to other days. The state keeps the last date per rate."""
import re

import common
import relay
from common import DECIMAL, ISO_DATE, shape

KEY = "policy_rates"
KEYCOLS = ["rate"]
KEY_RE = re.compile(r"^FM\.B\.U2\.EUR\.4F\.KR\.(\w+)\.LEV$")
LABELS = {"mro": "main refinancing operations rate", "dfr": "deposit facility rate",
          "mlfr": "marginal lending facility rate"}


def _rate_of_series() -> dict:
    series = common.series(KEY)
    return {s: rate for rate, s in series.items()}


def parse(data: bytes) -> list:
    """One series file (its KEY column names the series, mapped to a rate through names.toml)."""
    back = _rate_of_series()
    rows = common.csv_dicts(data, {"KEY", "TIME_PERIOD", "OBS_VALUE"})
    out = []
    for r in rows:
        m = KEY_RE.match(r["KEY"])
        shape(m and m.group(1) in back, f"policy rates: unknown series {r['KEY']!r}")
        date, v = r["TIME_PERIOD"], r["OBS_VALUE"].strip()
        shape(ISO_DATE.match(date), f"policy rates: bad date {date!r}")
        if v in ("", "N/A"):
            continue
        shape(DECIMAL.match(v), f"policy rates: bad value {v!r} on {date}")
        out.append({"date": date, "value": v, "rate": back[m.group(1)]})
    shape(out, "policy rates: no rows")
    return out


MONTHS = {m: i for i, m in enumerate(["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
                                     start=1)}
PAGE_DAY = re.compile(r"^(\d{1,2})\s+([A-Za-z]{3})[A-Za-z]*\.?$")
# the page's columns: year, date, deposit facility, main refinancing (fixed rate), main refinancing (minimum bid),
# marginal lending
PAGE_RATES = {2: "dfr", 3: "mro", 5: "mlfr"}


def parse_page(data: bytes) -> list:
    """The second copy: the ECB's page of the key interest rates, one row per change with effect from a date. The
    fixed rate tenders column is the main refinancing rate; '-' is no value."""
    text = common.decode(data)
    table = re.search(r"<table[^>]*>(.*?)</table>", text, re.S)
    shape(table, "key rates page: no table")
    head = re.sub(r"<[^>]+>", " ", table.group(1).split("<tbody", 1)[0])
    shape(all(h in head for h in ("Deposit facility", "Main refinancing operations", "Marginal lending facility")),
          "key rates page: the columns are not those known")
    body = re.search(r"<tbody[^>]*>(.*?)</tbody>", table.group(1), re.S)
    shape(body, "key rates page: no table body")
    out, year = [], None
    for row in re.findall(r"<tr[^>]*>(.*?)</tr>", body.group(1), re.S):
        cells = [re.sub(r"<[^>]+>", "", re.sub(r"<sup>.*?</sup>", "", c, flags=re.S)).replace("&nbsp;", " ").strip()
                 for c in re.findall(r"<td[^>]*>(.*?)</td>", row, re.S)]
        shape(len(cells) == 6 and (re.match(r"^\d{4}$", cells[0]) or (cells[0] == "" and year)),
              f"key rates page: unexpected row {cells!r}")
        year = cells[0] or year  # a second change in a year may leave the year cell empty
        m = PAGE_DAY.match(cells[1])
        shape(m and m.group(2) in MONTHS, f"key rates page: bad date {cells[1]!r}")
        date = f"{year}-{MONTHS[m.group(2)]:02d}-{int(m.group(1)):02d}"
        for i, rate in PAGE_RATES.items():
            v = cells[i].replace("\u2212", "-")  # the page writes a negative rate with a real minus sign
            if v in ("-", ""):
                continue
            shape(DECIMAL.match(v), f"key rates page: bad value {v!r} on {date}")
            out.append({"date": date, "value": v, "rate": rate})
    shape(out, "key rates page: no rows")
    return out


def _dictionary(rows):
    series = common.series(KEY)
    return {"rate": [{"value": k, "label": LABELS[k]} for k in sorted(series)]}


def _summary(sec, r, value):
    return f"ECB {LABELS[r['rate']]} ({r['rate']}) set on {r['date']}: {value} percent{relay.tail(sec)}."


TEXT = {
    "label": "ECB key interest rate",
    "description": ("the ECB key interest rates in percent (mro, dfr, mlfr), one observation per "
                    "decision date on which the rate changed; no value is carried to other days"),
    "args": {"rate": "mro (main refinancing operations), dfr (deposit facility) or mlfr (marginal lending facility)",
             "date": "the decision date on which the rate was set"},
    "dictionary": _dictionary,
    "summary": _summary,
}


def emit(rows, state, names, source, second_rows=None):
    return relay.emit(KEY, rows, state, names, source, keycols=KEYCOLS, per_key=True, text=TEXT,
                      second_rows=second_rows)


def run(names, *, fetch, now, state_dir, out_dir, publish=None, mode="daily"):
    sec = names[KEY]
    rates = sorted(sec["urls"])
    return relay.run_series(
        KEY, names, fetch=fetch, now=now, state_dir=state_dir, out_dir=out_dir, publish=publish,
        inputs=lambda st: [(sec["urls"][r], parse) for r in rates], second=(sec["second_url"], parse_page), keycols=KEYCOLS, per_key=True,
        emit_one=lambda rows, st, src: emit(rows, st, names, src))


def main(argv=None):
    a = common.base_parser().parse_args(argv)
    return common.finish(run(common.names(a.host), fetch=common.http_get, now=common.now_instant(),
                             state_dir=a.state_dir, out_dir=a.out_dir))


if __name__ == "__main__":
    raise SystemExit(main())
