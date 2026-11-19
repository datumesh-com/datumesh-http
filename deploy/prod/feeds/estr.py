"""ECB euro short-term rate, €STR (F1): one row per TARGET day, from the SDMX csvdata."""
import common
import relay
from common import DECIMAL, ISO_DATE, shape

KEY = "estr"
KEYCOLS = []
SERIES = "EST.B.EU000A2X2A25.WT"


def parse(data: bytes) -> list:
    rows = common.csv_dicts(data, {"KEY", "TIME_PERIOD", "OBS_VALUE"})
    out = []
    for r in rows:
        shape(r["KEY"] == SERIES, f"€STR: unexpected series {r['KEY']!r}")
        date, v = r["TIME_PERIOD"], r["OBS_VALUE"].strip()
        shape(ISO_DATE.match(date), f"€STR: bad date {date!r}")
        if v in ("", "N/A"):
            continue
        shape(DECIMAL.match(v), f"€STR: bad value {v!r} on {date}")
        out.append({"date": date, "value": v})
    shape(out, "€STR: no rows")
    return out


def _summary(sec, r, value):
    return f"Euro short-term rate (EUR STR) on {r['date']}: {value} percent{relay.tail(sec)}."


TEXT = {
    "label": "Euro short-term rate (€STR)",
    "description": ("the euro short-term rate, the overnight unsecured borrowing rate, in percent, "
                    "for each TARGET day it is published; days with no rate are absent"),
    "args": {"date": "the TARGET day"},
    "dictionary": lambda rows: {},
    "summary": _summary,
}


def emit(rows, state, names, source, second_rows=None):
    return relay.emit(KEY, rows, state, names, source, keycols=KEYCOLS, per_key=False, text=TEXT,
                      second_rows=second_rows)


def conformance(rows, second_rows, names):
    return relay.conformance(KEY, rows, second_rows, names, KEYCOLS)


def run(names, *, fetch, now, state_dir, out_dir, publish=None, mode="daily"):
    sec = names[KEY]
    return relay.run_series(
        KEY, names, fetch=fetch, now=now, state_dir=state_dir, out_dir=out_dir, publish=publish,
        inputs=lambda st: [(sec["url"], parse)], second=None, keycols=KEYCOLS,
        emit_one=lambda rows, st, src: emit(rows, st, names, src))


def main(argv=None):
    a = common.base_parser().parse_args(argv)
    return common.finish(run(common.names(), fetch=common.http_get, now=common.now_instant(),
                             state_dir=a.state_dir, out_dir=a.out_dir))


if __name__ == "__main__":
    raise SystemExit(main())
