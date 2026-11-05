# SPDX-License-Identifier: Apache-2.0
"""The synthetic ECB-shaped seed (the user's answer 2a of 2026-10-26, with the change of the
seed data): JSON lines for `datumesh-mesh publish-stored`, every value synthetic and every
summary saying test data.

    python -I ecb.py <out dir> <host>

Writes <out dir>/ecb.jsonl under <host>/ecb:
* fx_rate(base, quote, date): EUR in USD, GBP, JPY and CHF for the 30 weekdays 2026-09-21 to
  2026-10-30 (a deterministic walk, not ECB data), published at 14:15 UTC on the day, licence
  CC-BY-4.0; and one cell, EUR in NOK on 2026-10-30, under the restricted test licence
  https://<host>/licences/restricted-test (not open: the licence gate's test);
* holiday(calendar, date): TARGET for every day of 2026 (true on the six TARGET closing
  days), licence CC0-1.0;
* fx_cross(base, quote, date) = fx_rate(base: "EUR", quote: quote, date: date) /
  fx_rate(base: "EUR", quote: base, date: date), a formula over the FX cells: over open cells
  it can be evaluated anywhere, over the NOK cell it reads a restricted input (the gate's case);
* fx_cross_undeclared(base, quote, date), defined exactly like fx_cross (since http 0.2.1,
  spec-0.4.3): the test mesh's key document declares the restricted test licence for fx_cross
  only (seed/declarations.py), so over the NOK cell this one answers `unlicensed` (SPEC 8.5).
"""
import datetime
import json
import math
import os
import sys
from decimal import ROUND_HALF_EVEN, Decimal

out, host = sys.argv[1], sys.argv[2]
NS = f"{host}/ecb"
BOUND = "2026-09-01T00:00:00.000000Z"
RESTRICTED = f"https://{host}/licences/restricted-test"
START = {"USD": ("1.1650", 4), "GBP": ("0.8710", 4), "JPY": ("172.40", 2), "CHF": ("0.9340", 4)}
TARGET = {"2026-01-01", "2026-04-03", "2026-04-06", "2026-05-01", "2026-12-25", "2026-12-26"}
os.makedirs(out, exist_ok=True)
lines = []


def S(x):
    return {"type": "string", "value": x}


def num(d, places):
    q = d.quantize(Decimal(1).scaleb(-places), rounding=ROUND_HALF_EVEN)
    t = format(q, "f")
    return t.rstrip("0").rstrip(".") if "." in t else t


fx_sig = {"args": {"base": {"type": "string", "valid": False}, "date": {"type": "date", "valid": True},
                   "quote": {"type": "string", "valid": False}},
          "order": ["base", "quote", "date"],
          "returns": {"type": "number", "unit": {"of": {"arg": "quote"}, "per": {"arg": "base"}}}}
lines.append({"function": f"{NS}/fx_rate", "bound": BOUND, "signature": fx_sig})
lines.append({"function": f"{NS}/holiday", "bound": BOUND, "signature": {
    "args": {"calendar": {"type": "string", "valid": False}, "date": {"type": "date", "valid": True}},
    "order": ["calendar", "date"], "returns": "boolean"}})

days = []
d = datetime.date(2026, 9, 21)
while d <= datetime.date(2026, 10, 30):
    if d.weekday() < 5:
        days.append(d.isoformat())
    d += datetime.timedelta(days=1)
assert len(days) == 30
for quote, (start, places) in START.items():
    for i, day in enumerate(days):
        # A deterministic walk: within about 1% of the start, nothing like the real series.
        level = Decimal(start) * (1 + Decimal(str(round(0.006 * math.sin(i * 0.7 + len(quote)) + 0.003 * math.cos(i * 1.3), 6))))
        t = f"{day}T14:15:00.000000Z"
        lines.append({"cell": {"fn": f"{NS}/fx_rate", "args": {"base": S("EUR"), "date": {"type": "date", "value": day}, "quote": S(quote)}},
                      "value": {"type": "number", "value": num(level, places)}, "tx": t, "asof": t, "licence": "CC-BY-4.0",
                      "summary": f"Synthetic test data, not ECB data: 1 EUR in {quote} on {day} on the test mesh. Licence CC-BY-4.0."})
day = days[-1]
t = f"{day}T14:15:00.000000Z"
lines.append({"cell": {"fn": f"{NS}/fx_rate", "args": {"base": S("EUR"), "date": {"type": "date", "value": day}, "quote": S("NOK")}},
              "value": {"type": "number", "value": "11.6825"}, "tx": t, "asof": t, "licence": RESTRICTED,
              "summary": f"Synthetic test data, not ECB data: 1 EUR in NOK on {day} on the test mesh, under a restricted test licence "
                         f"(not open; for the licence-gate test). Licence {RESTRICTED}."})
d = datetime.date(2026, 1, 1)
while d.year == 2026:
    day = d.isoformat()
    lines.append({"cell": {"fn": f"{NS}/holiday", "args": {"calendar": S("TARGET"), "date": {"type": "date", "value": day}}},
                  "value": {"type": "boolean", "value": day in TARGET}, "tx": BOUND, "asof": BOUND, "licence": "CC0-1.0",
                  "summary": f"Synthetic test data: whether {day} is a TARGET closing day in the test mesh's calendar. Licence CC0-1.0."})
    d += datetime.timedelta(days=1)
for name in ("fx_cross", "fx_cross_undeclared"):
    lines.append({"formula": f"{NS}/{name}", "bound": BOUND, "signature": fx_sig,
                  "source": f'{NS}/fx_rate(base: "EUR", quote: quote, date: date) / {NS}/fx_rate(base: "EUR", quote: base, date: date)'})
with open(os.path.join(out, "ecb.jsonl"), "w", encoding="utf-8") as f:
    for l in lines:
        f.write(json.dumps(l, sort_keys=True) + "\n")
print(json.dumps({"cells": sum("cell" in l for l in lines), "fx_rate": 4 * 30 + 1, "holiday": 365,
                  "licences": sorted({l["licence"] for l in lines if "licence" in l})}))
