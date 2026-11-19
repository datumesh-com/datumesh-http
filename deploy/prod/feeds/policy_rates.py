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
    series = common.names()[KEY]["series"]
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


def _dictionary(rows):
    series = common.names()[KEY]["series"]
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
        inputs=lambda st: [(sec["urls"][r], parse) for r in rates], second=None, keycols=KEYCOLS, per_key=True,
        emit_one=lambda rows, st, src: emit(rows, st, names, src))


def main(argv=None):
    a = common.base_parser().parse_args(argv)
    return common.finish(run(common.names(), fetch=common.http_get, now=common.now_instant(),
                             state_dir=a.state_dir, out_dir=a.out_dir))


if __name__ == "__main__":
    raise SystemExit(main())
