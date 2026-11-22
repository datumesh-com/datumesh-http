"""SEC company facts (SEC rebuild, CONTRACT-sec-rebuild.md; Build 2 Part 7 before it): fetch with fair access, scope by
rule, relay as filed, source-conformance against second copies, and the run.

The relay (N1 to N3), rehearsal hosts only (scratch.invalid, mesh-test.datumesh.com):
  * `<host>/relay/sec.gov/company`, a class keyed by cik, with one stored method `fact(cik, concept, unit, accession,
    period_start, period_end) -> number`: one cell per reported figure per filing and period. The concept keeps its
    prefix (us-gaap:Assets); the unit is the unit as filed (a currency code, shares, <CCY>/shares or pure), and the
    value's unit is {arg: unit}. Units that units.md cannot say are not relayed and are counted in the left-out report.
    Nothing is normalised under the relay name.
  * `<host>/relay/sec.gov/filing`, a class keyed by accession, with six stored methods (cik, form, filed, fiscal_year,
    fiscal_period, period_end). period_end is the submissions record's reportDate; the others are the companyfacts file's.
  * `<host>/sec/company` (sec_company-<stamp>.jsonl, CONTRACT-sec-classes C3): our subclass of the relay's company class,
    with annual_report (our selection, stored and signed by the operator with no source), loaded and from_label.

The scope (K3, D35): 10-K and 10-K/A, fiscal years 2019 to 2024, the listed concepts of sec_concepts.toml, and the
NYSE or Nasdaq companies ranked by us-gaap:Assets (USD) in the frames API for CY2023Q4I, the top N, where N is the
largest multiple of 25 whose estimated cells stay within 300,000 (or every company, when they all fit).

Fair access (K1, D34): one connection, sequential, at most 5 requests a second (a token bucket), the declared
User-Agent and nothing else about us, and a 403 stops the run. Standard library only.
"""
import argparse
import decimal
import re
import hashlib
import http.client
import json
import time
import tomllib
import unicodedata
import urllib.error
import urllib.request
from collections.abc import Mapping
from decimal import Decimal
from pathlib import Path
from urllib.parse import urlparse

import common
import register as licence_register
import relay
from common import ConformanceError, NotPublished, RateLimited, ShapeError

USER_AGENT = "Datumesh mesh-test relay (https://datumesh.com)"
MAX_REQUESTS_PER_SECOND = 5
ALLOWED_HOSTS = ("data.sec.gov", "www.sec.gov")
REQUEST_TIMEOUT = 180

FORMS = ("10-K", "10-K/A")
FIRST_FY, LAST_FY = 2019, 2024
BUDGET = 300_000
STEP = 25
PRODUCTION = "datumesh.com"
SEC_NAMES = common.FEEDS / "sec_names.toml"

RANKING_FRAME = ("us-gaap", "Assets", "USD", "CY2023Q4I")
CONFORMANCE_FRAMES = (
    RANKING_FRAME,
    ("us-gaap", "NetIncomeLoss", "USD", "CY2023"),
    ("us-gaap", "Liabilities", "USD", "CY2023Q4I"),
)
FILING_METHODS = ("cik", "form", "filed", "fiscal_year", "fiscal_period", "period_end")
FILING_RETURNS = {"cik": "string", "form": "string", "filed": "date", "fiscal_year": {"type": "number", "unit": "1"},
                  "fiscal_period": "string", "period_end": "date"}
FILING_VALUE_TYPES = {"cik": "string", "form": "string", "filed": "date", "fiscal_year": "number",
                      "fiscal_period": "string", "period_end": "date"}
FACT_ORDER = ("cik", "concept", "unit", "accession", "period_start", "period_end")
CONFORMANCE_MINIMUM = 20
FILING_CONFORMANCE_MINIMUM = 20
CF_FIELDS = ("cik", "form", "fiscal_year", "fiscal_period")  # the filing methods whose cells come from companyfacts
SUBMITTED_FIELDS = ("filed", "period_end")  # the filing methods relayed from the submissions record (C2, C2b)
MAX_SHOWN = 50  # disagreements listed in one ConformanceError; the rest are counted
SUBMISSIONS_PAGES = "https://data.sec.gov/submissions/"  # the older pages of a submissions file, by their file names (C2)

# The currency codes of units.md 2.1 (ISO 4217 List One of 2026-09-17 without XXX, and the withdrawn codes kept for
# history): a filed unit is money only if it is one of these, byte for byte.
CURRENCIES = frozenset("""
    AED AFN ALL AMD AOA ARS AUD AWG AZN BAM BBD BDT BHD BIF BMD BND BOB BOV BRL BSD BTN BWP BYN BZD
    CAD CDF CHE CHF CHW CLF CLP CNY COP COU CRC CUP CVE CZK DJF DKK DOP DZD EGP ERN ETB EUR FJD FKP
    GBP GEL GHS GIP GMD GNF GTQ GYD HKD HNL HTG HUF IDR ILS INR IQD IRR ISK JMD JOD JPY KES KGS KHR
    KMF KPW KRW KWD KYD KZT LAK LBP LKR LRD LSL LYD MAD MDL MGA MKD MMK MNT MOP MRU MUR MVR MWK MXN
    MXV MYR MZN NAD NGN NIO NOK NPR NZD OMR PAB PEN PGK PHP PKR PLN PYG QAR RON RSD RUB RWF SAR SBD
    SCR SDG SEK SGD SHP SLE SOS SRD SSP STN SVC SYP SZL THB TJS TMT TND TOP TRY TTD TWD TZS UAH UGX
    USD USN UYI UYU UYW UZS VED VES VND VUV WST XAD XAF XAG XAU XBA XBB XBC XBD XCD XCG XDR XOF XPD
    XPF XPT XSU XTS XUA YER ZAR ZMW ZWG BGN CYP EEK GRD HRK LTL LVL MTL ROL SIT SKK TRL
""".split())


def relayed_unit(unit) -> bool:
    """True for a filed unit that units.md can say: a currency code of CURRENCIES (the fact method's unit argument),
    `shares`, `<CCY>/shares` for a currency in CURRENCIES, and `pure`. Anything else is left out and counted."""
    if not isinstance(unit, str):
        return False
    if unit in CURRENCIES or unit in ("shares", "pure"):
        return True
    ccy, slash, rest = unit.partition("/")
    return bool(slash) and rest == "shares" and ccy in CURRENCIES


_CTX = decimal.Context(prec=1000)  # exact: a 34-digit value must not be rounded by the default 28 digits
_CONCEPTS = None


class SecRefused(Exception):
    """The SEC answered 403 (an undeclared automated tool, or the rate threshold). The job stops; it never retries."""

    def __init__(self, url, body, status=403):
        super().__init__(f"HTTP {status} from {url}: the SEC refused the request: {body[:500]}")
        self.url, self.body, self.status = url, body, status


class NotForProduction(ValueError):
    """The SEC relay is rehearsal-only (K7): the production host is refused before any fetch."""


# ---------- concepts and numbers ----------

def load_concepts() -> dict:
    """{(taxonomy, concept): reason} from sec_concepts.toml. An entry without a reason is refused."""
    global _CONCEPTS
    if _CONCEPTS is None:
        with open(common.FEEDS / "sec_concepts.toml", "rb") as f:
            entries = tomllib.load(f).get("concept", [])
        out = {}
        for e in entries:
            key = (e.get("taxonomy"), e.get("name"))
            reason = e.get("reason")
            common.shape(isinstance(reason, str) and len(reason.strip()) >= 10, f"concept {key} has no reason")
            common.shape(key not in out, f"concept {key} is listed twice")
            out[key] = reason
        _CONCEPTS = out
    return dict(_CONCEPTS)


def canon(value) -> str:
    """The figure as filed, in canonical decimal text: no exponent, no insignificant zeros, never rounded."""
    d = Decimal(value)
    common.shape(d.is_finite(), f"not a finite number: {value!r}")
    if d == 0:
        return "0"
    return format(d.normalize(_CTX), "f")


def _num(x) -> bool:
    return isinstance(x, (int, Decimal)) and not isinstance(x, bool)


def _int(x) -> bool:
    return isinstance(x, int) and not isinstance(x, bool)


def _iso(x) -> bool:
    return isinstance(x, str) and common.ISO_DATE.match(x) is not None


def _load(data: bytes):
    """JSON with numbers kept as Decimal (exact text), so 0.1000000000000000055511151231257827 survives."""
    text = common.decode(data)
    try:
        return json.loads(text, parse_float=Decimal)
    except ValueError as e:
        raise ShapeError(f"not JSON: {e}") from e


def _record(r):
    common.shape(isinstance(r, dict), "a fact record is not an object")
    for k in ("accn", "form"):
        common.shape(isinstance(r.get(k), str), f"fact record {k} is not text")
    common.shape(_iso(r.get("filed")) and _iso(r.get("end")), "fact record filed or end is not a date")
    common.shape(_num(r.get("val")), "fact record val is not a number")
    if r["form"] in FORMS:  # 8-K and proxy records carry null fy and fp; the annual reports must not
        common.shape(_int(r.get("fy")) and isinstance(r.get("fp"), str), "annual fact record fy or fp is missing")
    if "start" in r:
        common.shape(_iso(r["start"]), "fact record start is not a date")


def _cik10(cik) -> str:
    """A CIK as the SEC writes it (a number, or a zero-padded string for some companies) -> 10 digits."""
    if isinstance(cik, str) and 1 <= len(cik) <= 10 and cik.isascii() and cik.isdigit():
        cik = int(cik)
    common.shape(_int(cik) and 0 < cik < 10 ** 10, "a CIK is not a CIK")
    return f"{cik:010d}"


# ---------- companyfacts ----------

def parse_companyfacts(data: bytes) -> dict:
    """One companyfacts file -> the in-scope rows as filed, conflicts, labels, filing metadata and filing_for. A row is
    one (cik, concept, unit, accession, period_start, period_end) with a relayed unit (relayed_unit); any other unit is
    counted in `left_out_units` and is not a row. An instant has the same date as period_start and period_end."""
    raw = _load(data)
    common.shape(isinstance(raw, dict), "companyfacts is not an object")
    # The SEC writes the CIK as a number for most companies and as a zero-padded string for some
    # (seen: "0001889539", Corebridge Financial); both are read, nothing else is.
    cik10 = _cik10(raw.get("cik"))
    entity = raw.get("entityName")
    common.shape(isinstance(entity, str) and entity.strip() != "", "companyfacts entityName is missing")
    facts = raw.get("facts")
    common.shape(isinstance(facts, dict), "companyfacts facts is missing")
    listed = load_concepts()

    labels, groups, meta = {}, {}, {}
    for tax, concepts in facts.items():
        common.shape(isinstance(concepts, dict), f"taxonomy {tax} is not an object")
        for concept, c in concepts.items():
            common.shape(isinstance(c, dict), f"concept {tax}:{concept} is not an object")
            is_listed = (tax, concept) in listed
            if is_listed:
                common.shape(isinstance(c.get("label"), str), f"concept {tax}:{concept} has no label")
                desc = c.get("description")
                common.shape(desc is None or isinstance(desc, str), f"concept {tax}:{concept} description")
                labels[(tax, concept)] = {"label": c["label"], "description": desc}
            units = c.get("units")
            common.shape(isinstance(units, dict), f"concept {tax}:{concept} has no units")
            for unit, recs in units.items():
                common.shape(isinstance(recs, list), f"{tax}:{concept} {unit} is not a list")
                for r in recs:
                    _record(r)
                    if r["form"] not in FORMS or not FIRST_FY <= r["fy"] <= LAST_FY:
                        continue
                    _note_filing(meta, cik10, r)
                    if not is_listed:
                        continue
                    start = r.get("start", r["end"])
                    key = (f"{tax}:{concept}", unit, r["accn"], start, r["end"])
                    g = groups.setdefault(key, {"values": {}, "rec": r})
                    g["values"][canon(r["val"])] = True

    rows, conflicts, left_out_units = [], [], {}
    for (concept, unit, accn, start, end), g in groups.items():
        if not relayed_unit(unit):  # a unit units.md cannot say is not relayed, and is counted
            left_out_units[unit] = left_out_units.get(unit, 0) + 1
            continue
        values = sorted(g["values"])
        if len(values) > 1:
            conflicts.append({"cik": cik10, "concept": concept, "unit": unit, "accession": accn,
                              "period_start": start, "period_end": end, "values": values})
            continue
        r = g["rec"]
        rows.append({"cik": cik10, "concept": concept, "unit": unit, "accession": accn, "period_start": start,
                     "period_end": end, "value": values[0], "form": r["form"], "fy": str(r["fy"]), "fp": r["fp"],
                     "filed": r["filed"]})

    filings = {}
    for accn in sorted({r["accession"] for r in rows}):
        m = meta[accn]
        filings[accn] = {"cik": cik10, "form": m["form"], "filed": m["filed"], "fy": m["fy"], "fp": m["fp"]}

    by_fy = {}
    for accn, f in filings.items():
        by_fy.setdefault(f["fy"], []).append((f["filed"], accn))
    filing_for = {fy: max(c)[1] for fy, c in sorted(by_fy.items())}

    return {"cik": cik10, "entity": entity, "rows": rows, "conflicts": conflicts, "labels": labels,
            "filings": filings, "filing_for": filing_for, "left_out_units": left_out_units}


def _note_filing(meta, cik10, r):
    """The filing's metadata, from its first in-scope record. period_end is not derived here: it is the submissions'
    reportDate (CONTRACT-sec-classes C2); the periods of the facts were wrong on Athene's 10-K/A."""
    meta.setdefault(r["accn"], {"cik": cik10, "form": r["form"], "filed": r["filed"], "fy": str(r["fy"]), "fp": r["fp"]})


def count_cells(data: bytes) -> int:
    return len(parse_companyfacts(data)["rows"])


# ---------- submissions (the second copy of the filing class) ----------

def _rows(recent) -> dict:
    """The parallel arrays of a `filings.recent` block, or of a page of `filings.files` (the same arrays, at the top of the
    file) -> {accession: {"form", "filed", "period_end"}}. An empty reportDate is period_end None (an 8-K has none)."""
    common.shape(isinstance(recent, dict), "submissions filings block is not an object")
    cols = ("accessionNumber", "filingDate", "reportDate", "form")
    for k in cols:
        common.shape(isinstance(recent.get(k), list), f"submissions recent.{k} is missing")
    n = len(recent["accessionNumber"])
    common.shape(all(len(recent[k]) == n for k in cols), "submissions recent arrays differ in length")
    out = {}
    for accn, filed, rep, form in zip(*(recent[k] for k in cols)):
        common.shape(isinstance(accn, str) and isinstance(form, str), "submissions accession or form is not text")
        common.shape(_iso(filed), f"submissions filing date of {accn} is not a date")
        common.shape(rep == "" or _iso(rep), f"submissions report date of {accn} is not a date")
        common.shape(accn not in out, f"submissions lists {accn} twice")
        out[accn] = {"form": form, "filed": filed, "period_end": rep or None}
    return out


def parse_submissions(data: bytes) -> dict:
    """One submissions file -> {"cik", "filings": {accession: {"form", "filed", "period_end"}}, "pages": [file names]}: the
    `filings.recent` block, and the pages of `filings.files` in the order the file lists them (fetched only when an
    in-scope accession is not in recent: CONTRACT-sec-classes C2)."""
    raw = _load(data)
    common.shape(isinstance(raw, dict), "submissions is not an object")
    cik10 = _cik10(raw.get("cik"))
    filings = raw.get("filings")
    common.shape(isinstance(filings, dict) and isinstance(filings.get("recent"), dict),
                 "submissions filings.recent is missing")
    files = filings.get("files", [])
    common.shape(isinstance(files, list) and all(isinstance(f, dict) and isinstance(f.get("name"), str) for f in files),
                 "submissions filings.files is not a list of pages")
    return {"cik": cik10, "filings": _rows(filings["recent"]), "pages": [f["name"] for f in files]}


def parse_submissions_page(data: bytes) -> dict:
    """One page of `filings.files` -> {accession: {"form", "filed", "period_end"}}, as parse_submissions reads recent."""
    raw = _load(data)
    common.shape(isinstance(raw, dict), "submissions page is not an object")
    return _rows(raw)


# ---------- frames ----------

def parse_frames(data: bytes) -> dict:
    """A frames file -> its rows (cik, accession, period_start, period_end, value). The unit is spelled as companyfacts
    spells it, and an instant has the same date as period_start and period_end."""
    raw = _load(data)
    common.shape(isinstance(raw, dict), "frame is not an object")
    for k in ("taxonomy", "tag", "ccp", "uom"):
        common.shape(isinstance(raw.get(k), str), f"frame {k} is missing")
    entries = raw.get("data")
    common.shape(isinstance(entries, list), "frame data is missing")
    rows = []
    for d in entries:
        common.shape(isinstance(d, dict), "frame entry is not an object")
        common.shape(_int(d.get("cik")) and isinstance(d.get("accn"), str), "frame entry cik or accn")
        common.shape(_iso(d.get("end")) and _num(d.get("val")), "frame entry end or val")
        if "start" in d:
            common.shape(_iso(d["start"]), "frame entry start")
        rows.append({"cik": f"{d['cik']:010d}", "accession": d["accn"], "period_start": d.get("start", d["end"]),
                     "period_end": d["end"], "value": canon(d["val"])})
    return {"taxonomy": raw["taxonomy"], "concept": raw["tag"], "unit": raw["uom"].replace("-per-", "/"),
            "ccp": raw["ccp"], "rows": rows}


def rank_companies(tickers: bytes, frames: bytes) -> dict:
    """The NYSE and Nasdaq companies in the Assets frame, largest first (ties: the lower CIK). A company listed on
    both an exchange and OTC counts once, by its exchange row."""
    t = _load(tickers)
    common.shape(isinstance(t, dict) and isinstance(t.get("fields"), list) and isinstance(t.get("data"), list),
                 "tickers file: fields and data are missing")
    fields = t["fields"]
    for k in ("cik", "ticker", "exchange"):
        common.shape(k in fields, f"tickers file has no {k}")
    ci, ti, ei = fields.index("cik"), fields.index("ticker"), fields.index("exchange")
    listed = {}
    for row in t["data"]:
        common.shape(isinstance(row, list) and len(row) == len(fields), "tickers row does not match its fields")
        if row[ei] not in ("NYSE", "Nasdaq") or not _int(row[ci]):
            continue
        listed.setdefault(f"{row[ci]:010d}", {"ticker": row[ti], "exchange": row[ei]})

    fr = parse_frames(frames)
    assets = {}
    for r in fr["rows"]:
        assets.setdefault(r["cik"], r["value"])
    ranked = []
    for cik, value in assets.items():
        if cik not in listed:
            continue
        d = Decimal(value)
        common.shape(d == d.to_integral_value(), f"Assets of {cik} is not whole: {value}")
        ranked.append({"cik": cik, "ticker": listed[cik]["ticker"], "exchange": listed[cik]["exchange"],
                       "assets": int(d)})
    ranked.sort(key=lambda x: (-x["assets"], x["cik"]))
    return {"ranked": ranked, "frame_entities": len(assets),
            "frame_not_listed": sum(1 for c in assets if c not in listed),
            "listed_without_assets": sum(1 for c in listed if c not in assets)}


def choose_n(ciks, counts, *, budget=BUDGET, step=STEP) -> dict:
    """The largest multiple of `step` among the first N ciks whose cells stay within budget; every cik when they
    all fit. Counts are read only as far as the budget goes."""
    total = 0
    n, est = 0, 0
    for i, c in enumerate(ciks):
        total += counts[c]
        if total > budget:
            return {"n": n, "estimated": est}
        if (i + 1) % step == 0:
            n, est = i + 1, total
    return {"n": len(ciks), "estimated": total}


def scope_report(ranking, choice, *, no_facts=(), unsupported_units=None, budget=BUDGET, step=STEP) -> dict:
    """The left-out report (K3): what the rule chose, and what it leaves out, with counts."""
    ranked, n = ranking["ranked"], choice["n"]
    return {
        "rule": ("10-K and 10-K/A annual reports, fiscal years 2019 to 2024 (fy as filed); the listed us-gaap concepts "
                 "and dei:EntityCommonStockSharesOutstanding; NYSE or Nasdaq companies ranked by us-gaap:Assets (USD) "
                 "in the frames API for CY2023Q4I, the top N; N the largest multiple of %d whose estimated cells stay "
                 "within %d, or every company when they all fit" % (step, budget)),
        "n": n, "estimated_cells": choice["estimated"], "budget": budget, "step": step,
        "forms": list(FORMS), "fiscal_years": [FIRST_FY, LAST_FY], "concepts": len(load_concepts()),
        "ranked": len(ranked),
        "left_out": {
            "below_n": len(ranked) - n,
            "listed_without_assets": ranking["listed_without_assets"],
            "not_listed_on_nyse_or_nasdaq": ranking["frame_not_listed"],
            "no_companyfacts": sorted(no_facts),
            "foreign_private_issuers": "filers of 20-F and 40-F (foreign private issuers) are not in the forms",
            "other_concepts": "every us-gaap concept and dei fact not in sec_concepts.toml",
            "dimensional_facts": "facts with dimensions (segments, classes) are not in companyfacts, so not relayed",
            "quarterly_facts": "10-Q and other quarterly facts, and every form other than 10-K and 10-K/A",
            "years_outside_2019_2024": "facts of reports whose fy is before 2019 or after 2024",
            "units_not_in_units_md": {
                "rule": ("D38: only currency codes (the fact method's unit), shares, <currency>/shares and pure are relayed; "
                         "a cell in any other unit is left out. Cells by unit, for the chosen companies"),
                "cells": dict(sorted((unsupported_units or {}).items())),
            },
        },
        "lowest_included": dict(ranked[n - 1]) if n else None,
        "highest_excluded": dict(ranked[n]) if n < len(ranked) else None,
    }


# ---------- fetching ----------

class TokenBucket:
    """At most `rate` requests a second, one at a time: each request waits for the previous one's interval. Idle
    time is not banked into a burst."""

    def __init__(self, rate, *, clock=time.monotonic, sleep=time.sleep):
        if not (isinstance(rate, (int, float)) and 0 < rate <= 10):
            raise ValueError(f"rate {rate!r} is not between 0 and the SEC's own maximum of 10 a second")
        self.interval = 1.0 / rate + 1e-6  # the margin keeps float error from putting a sixth request in a second
        self.clock, self.sleep = clock, sleep
        self.next = None

    def wait(self):
        now = self.clock()
        if self.next is not None and now < self.next:
            self.sleep(self.next - now)
            now = self.clock()
        self.next = now + self.interval


def http_get(url: str, *, sleep=time.sleep) -> bytes:
    """The body of one SEC URL: one request with the declared User-Agent. 403 is SecRefused, never retried; 404 is
    NotPublished; 429 and 503 are RateLimited; a transient network failure is retried as common.http_get does."""
    parsed = urlparse(url)
    if parsed.scheme != "https" or parsed.hostname not in ALLOWED_HOSTS:
        raise ValueError(f"not an SEC URL to fetch: {url}")
    waits = common.RETRY_WAITS
    for attempt in range(len(waits) + 1):
        req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        try:
            with urllib.request.urlopen(req, timeout=REQUEST_TIMEOUT) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code == 403:
                raise SecRefused(url, e.read().decode("utf-8", "replace"), 403) from e
            if e.code == 404:
                raise NotPublished(url) from e
            if e.code in (429, 503):
                raise RateLimited(url, common._retry_after(e.headers), e.code) from e
            if not common._transient(e) or attempt == len(waits):
                raise
            err = e
        except (OSError, http.client.HTTPException) as e:
            if attempt == len(waits):
                raise
            err = e
        common.warn(f"sec http_get {url}: {type(err).__name__}: {err}; retry {attempt + 1} of {len(waits)}")
        sleep(waits[attempt])


def _sec_hosts() -> dict:
    with open(SEC_NAMES, "rb") as f:
        return tomllib.load(f)["hosts"]


def sec_table(host) -> dict:
    """The SEC section of sec_names.toml for one host (K7). Production is refused by name; any host the file does not
    list is common.UnknownHost. Names come with {host} spelled for the host."""
    if host == PRODUCTION:
        raise NotForProduction(f"the SEC names are rehearsal-only (K7): {PRODUCTION} is refused; use mesh-test.{PRODUCTION}")
    with open(SEC_NAMES, "rb") as f:
        raw = tomllib.load(f)
    if not isinstance(host, str) or host not in raw["hosts"]:
        raise common.UnknownHost(f"host {host!r} is not in sec_names.toml, which lists {sorted(raw['hosts'])}")
    body = {k: v for k, v in raw.items() if k != "hosts"}
    return common._with_host(body, host)


def _base_table(host) -> dict:
    """common's table for the host. The hosts of names.toml come from common.names; a rehearsal host that only the SEC
    file lists (scratch.invalid) gets the same table, spelled for it, from names.toml's own text (which names no host)."""
    if host in common._table()["hosts"]:
        return dict(common.names(host))
    status = _sec_hosts()[host]["status"]
    body = {k: v for k, v in common._table().items() if k != "hosts"}
    out = common._with_host(body, host)
    out["meta"] = {"host": host, "status": status, "takeover_base": f"https://{host}/takeover"}
    return out


def names(host) -> dict:
    """common's table plus the SEC tables: `sec` (the relay), `sec_register` (the register table of names.toml with
    the one SEC dataset), `sec_register_entry` and `sec_register_licence` (the licence facts)."""
    raw = sec_table(host)
    table = _base_table(host)
    reg = dict(table["register"])
    reg["datasets"] = raw["sec_register"]["datasets"]
    reg["source_labels"] = {**reg["source_labels"], **raw["sec_register"]["source_labels"]}
    table.update({"sec": raw["sec"], "sec_register": reg, "sec_register_entry": raw["sec_register_entry"],
                  "sec_register_licence": raw["sec_register_licence"]})
    return table


def _frame_url(s, frame) -> str:
    taxonomy, concept, unit, ccp = frame
    return s["frames_url"].format(taxonomy=taxonomy, concept=concept, unit=unit, ccp=ccp)


def _cf_url(s, cik) -> str:
    return s["companyfacts_url"].format(cik=cik)


def _sub_url(s, cik) -> str:
    return s["submissions_url"].format(cik=cik)


# ---------- lines ----------

S_ARG = {"type": "string", "valid": False}
D_ARG = {"type": "date", "valid": False}
N_ARG = {"type": "number", "unit": "1", "valid": False}
FACT_RETURNS = {"type": "number", "unit": {"arg": "unit"}}
# The steward's change process of a class (CONTRACT-classes-round K12): the page the site serves for production, which
# check_stewards.py fetches before any production publish. The rehearsal host's address is NOT a real page (rehearsal
# only; check_stewards.py exempts it and lists it).
PRODUCTION_PROCESS = "https://datumesh.com/classes/change"


def _steward_process(host: str) -> str:
    """The process address of a class of `host`: production's real page, else the rehearsal host's address (not a real page)."""
    return PRODUCTION_PROCESS if host == PRODUCTION else f"https://{host}/classes/change"


def _call(fq, order, vals, bare=()) -> str:
    """`fq(a: "x", b: 2023-01-01)`: strings are quoted, the arguments named in `bare` (dates, numbers) are not."""
    return f"{fq}({', '.join(f'{a}: {vals[a]}' if a in bare else f'{a}: {chr(34)}{vals[a]}{chr(34)}' for a in order)})"


def _names(s):
    """The four names the relay and its subclass are spelled with: the relay's company class, its stored method fact, the
    filing class, and our subclass of the company class (CONTRACT-sec-classes C1, C3)."""
    company = f"{s['namespace']}/{s['company_class']}"
    return company, f"{company}/fact", f"{s['namespace']}/{s['filing_class']}", f"{_host_of(s)}/sec/company"


def _fact_descriptor():
    """The stored method fact of the relay class company (C1): the declarations of the old company_fact, in its order, and its returns.
    The cik argument has no key_normalisation: it matches the class key."""
    return {"code": {"body": "", "lang": "stored"}, "inputs": [],
            "signature": {"args": {"cik": dict(S_ARG), "concept": dict(S_ARG), "unit": dict(S_ARG),
                                   "accession": dict(S_ARG), "period_start": dict(D_ARG), "period_end": dict(D_ARG)},
                          "order": list(FACT_ORDER), "returns": dict(FACT_RETURNS)}}


def _source(s, src) -> dict:
    return {"name": s["source_name"], "url": src["url"], "retrieved": src["retrieved"], "sha256": src["sha256"],
            "takeover": s["takeover"]}


def _fact_key(cik, r) -> str:
    return "|".join((cik, r["concept"], r["unit"], r["accession"], r["period_start"], r["period_end"]))


def _fact_cell(s, entity, cik, r, src):
    value = canon(r["value"])
    if r["period_start"] == r["period_end"]:
        when = f"as at {r['period_end']}"
    else:
        when = f"for {r['period_start']} to {r['period_end']}"
    summary = (f"{entity} {r['form']} filed {r['filed']}, fiscal year {r['fy']}: {r['concept']} {when} is {value} "
               f"{r['unit']} as filed in accession {r['accession']}{relay.tail(s)}")
    args = {"cik": cik, "concept": r["concept"], "unit": r["unit"], "accession": r["accession"],
            "period_start": r["period_start"], "period_end": r["period_end"]}
    return {"cell": {"fn": _names(s)[1],
                     "args": {k: {"type": "date" if k.startswith("period") else "string", "value": args[k]}
                              for k in FACT_ORDER}},
            "value": {"type": "number", "value": value}, "licence": s["licence"],
            "summary": common.nfc(summary), "source": _source(s, src)}


def _filing_value(f, field):
    if field == "fiscal_year":
        return f["fy"]
    if field == "fiscal_period":
        return f["fp"]
    return f[field]


def _filing_cell(s, cls, entity, accn, f, field, src):
    value = _filing_value(f, field)
    summary = (f"{entity} filing {accn}: {field} is {value} ({f['form']} filed {f['filed']}, fiscal year {f['fy']}, "
               f"{f['fp']})" + relay.tail(s))
    return {"cell": {"fn": f"{cls}/{field}", "args": {"accession": {"type": "string", "value": accn}}},
            "value": {"type": FILING_VALUE_TYPES[field], "value": value}, "licence": s["licence"],
            "summary": common.nfc(summary), "source": _source(s, src)}


def _annual_cell(s, entity, cik, fy, accn):
    summary = (f"the latest-filed annual report (10-K or 10-K/A) of fiscal year {fy} of {entity} (cik {cik}) is accession "
               f"{accn}: Datumesh's selection over the SEC's filings, not a figure the company reported")
    return {"cell": {"fn": f"{_names(s)[3]}/annual_report",
                     "args": {"cik": {"type": "string", "value": cik},
                              "fiscal_year": {"type": "number", "value": fy}}},
            "value": {"type": "string", "value": accn}, "licence": s["licence"],
            "summary": common.nfc(summary)}


def _method_descriptor(m):
    """The stored method's descriptor, as register.py writes its methods: keyed by accession, no inputs."""
    return {"code": {"body": "", "lang": "stored"}, "inputs": [],
            "signature": {"args": {"accession": dict(S_ARG)}, "order": ["accession"], "returns": FILING_RETURNS[m]}}


def _host_of(s):
    """The host a name of the table is spelled for: the first part of the relay's namespace."""
    return s["namespace"].split("/", 1)[0]


def _class_lines(s):
    """The stored descriptors, then the class lines that name their hashes (a method's descriptor comes before the class
    line that names its hash): the company class with its one method fact (C1), then the filing class with its six."""
    company, _, cls, _ = _names(s)
    steward = {"namespace": s["namespace"], "process": _steward_process(_host_of(s))}
    fact = _fact_descriptor()
    lines = [{"descriptor": fact}]
    hashes = {}
    for m in FILING_METHODS:
        d = _method_descriptor(m)
        lines.append({"descriptor": d})
        hashes[m] = hashlib.sha256(licence_register.jcs(d)).hexdigest()
    lines.append({"class": company, "descriptor": {
        "keys": {"cik": {"type": "string", "key_normalisation": "none"}}, "order": ["cik"],
        "methods": {"fact": hashlib.sha256(licence_register.jcs(fact)).hexdigest()}, "steward": dict(steward)}})
    lines.append({"class": cls, "descriptor": {
        "keys": {"accession": {"type": "string", "key_normalisation": "none"}}, "order": ["accession"],
        "methods": hashes, "steward": dict(steward)}})
    return lines


def _unit_label(u):
    if u == "shares":
        return "a count of shares"
    if u == "pure":
        return "a pure number (no unit)"
    if u.endswith("/shares"):
        return f"{_currency_label(u[:-7])} per share"
    return _currency_label(u)


CURRENCY_LABELS = {"USD": "US dollars", "EUR": "euros", "USN": "US dollars (next day)"}
FIELD_LABELS = {
    "cik": "the company's CIK (10 digits)",
    "form": "the form, 10-K or 10-K/A",
    "filed": "the filing date",
    "fiscal_year": "the fiscal year of the report, as filed (not the period's year)",
    "fiscal_period": "the fiscal period of the report (FY)",
    "period_end": "the period end of the report's annual statements (not a cover-page date)",
}
FILING_TEXT = {
    "cik": ("The company's CIK", "the company's CIK, 10 digits with leading zeros"),
    "form": ("The form of the report", "the form of the report, 10-K or 10-K/A"),
    "filed": ("The filing date", "the date the report was filed"),
    "fiscal_year": ("The fiscal year of the report", "the fiscal year of the report (fy), a number in unit 1; it is not "
                                                     "the year of the period end for a year that ends in another month"),
    "fiscal_period": ("The fiscal period of the report", "the fiscal period of the report (FY for an annual report)"),
    "period_end": ("The period end of the report", "the period end of the report's annual statements; not a cover-page "
                                                   "date, which is later"),
}


def _currency_label(c):
    return CURRENCY_LABELS.get(c, f"{c} (ISO 4217 currency code)")


def _relay_catalogs(s, cells, rows, labels, entities):
    """The relay's catalogue (C1, C2): the company class, its method fact, the filing class and its six methods."""
    company, fq, cls, _ = _names(s)
    src = s["source_name"]
    out = [{"catalog": company, "kind": "class", "label": "SEC company (one figure per cik, as filed)",
            "description": (f"Relayed by datumesh.com from {src}: one company, keyed by its cik (10 digits). Its stored method "
                            "fact relays one figure as filed, of one annual report (accession) and period; the cells of the "
                            "class are the figures of the companies in scope."),
            "args": {"cik": "the company's CIK, 10 digits with leading zeros"},
            "open_keys": ["cik"], "examples": [],
            "dictionary": {"cik": [{"value": c, "label": entities[c]} for c in sorted(entities)]}}]
    fact_cells = cells["fact"]
    mine = rows
    used = sorted({r["concept"] for r in mine})
    concept_dict = []
    for c in used:
        tax, _, name = c.partition(":")
        lab = labels.get((tax, name), {"label": name, "description": None})
        e = {"value": c, "label": lab["label"]}
        if lab["description"] is not None:
            e["description"] = lab["description"]
        concept_dict.append(e)
    examples = [{"call": _call(fq, FACT_ORDER, {k: v["value"] for k, v in c["cell"]["args"].items()},
                               bare=("period_start", "period_end")), "expected": c["value"]} for c in fact_cells[:2]]
    out.append({"catalog": fq, "kind": "function", "label": "Company figure as filed (XBRL fact, one filing)",
                "description": (
                    f"Relayed by datumesh.com from {src}: one figure as the company filed it in one annual report "
                    "(accession), exactly as filed: no scaling, no sign change, not normalised. A figure restated in a "
                    "later annual report is a separate cell with that report's accession. The unit is the unit as "
                    "filed (a currency code such as USD, shares, <currency>/shares or pure) and is returned as the "
                    "unit of the value. Only 10-K and 10-K/A reports for fiscal years 2019 to 2024, and the listed "
                    "concepts; each figure is one cell per filing and period. An instant (a balance) has the same date "
                    "as period_start and period_end."),
                "args": {
                    "cik": "the company's CIK, 10 digits with leading zeros",
                    "concept": "the element as filed with its prefix, such as us-gaap:Assets or dei:EntityCommonStockSharesOutstanding (see the dictionary)",
                    "unit": "the unit as filed: a currency code such as USD or EUR, shares, <currency>/shares such as USD/shares, or pure",
                    "accession": "the accession number of the filing, for example 0000320193-23-000106",
                    "period_start": "YYYY-MM-DD, the start of the period; an instant has the same date as period_end",
                    "period_end": "YYYY-MM-DD, the end of the period (for an instant, the same date as period_start)",
                },
                "open_keys": ["accession"],
                "examples": examples,
                "dictionary": {
                    "concept": concept_dict,
                    "unit": [{"value": u, "label": _unit_label(u)} for u in sorted({r["unit"] for r in mine})],
                    "cik": [{"value": c, "label": entities[c]} for c in sorted({r["cik"] for r in mine})],
                }})

    accn_cells = cells["filing"]
    filing_cat = {"catalog": cls, "kind": "class", "label": "SEC annual report (filing, as filed)",
                  "description": (f"Relayed by datumesh.com from {src}: one annual report (10-K or 10-K/A), keyed by its "
                                  "accession; its six methods are the report's cik, form, filed date, fiscal year, fiscal "
                                  "period and period end, each as filed in the company's submissions record."),
                  "args": {"accession": "the accession number of the filing, for example 0000320193-23-000106"},
                  "open_keys": ["accession"], "examples": [], "dictionary": {}}
    out.append(filing_cat)
    for m in FILING_METHODS:
        mc = [c for c in accn_cells if c["cell"]["fn"] == f"{cls}/{m}"]
        what, detail = FILING_TEXT[m]
        out.append({"catalog": f"{cls}/{m}", "kind": "function", "label": f"{what} (filing, as filed)",
                    "description": (f"Relayed by datumesh.com from {src}: {detail}, of one annual report (accession), "
                                    "as filed in the company's submissions record; one cell per annual report of the "
                                    "companies in scope."),
                    "args": {"accession": "the accession number of the filing, for example 0000320193-23-000106"},
                    "open_keys": ["accession"],
                    "examples": [{"call": _call(f"{cls}/{m}", ["accession"], {"accession": c["cell"]["args"]["accession"]["value"]}),
                                  "expected": c["value"]} for c in mc[:2]],
                    "dictionary": {}})

    return out


def _key(s, cik) -> str:
    """The key encoding of our subclass's object for one company: <host>/sec/company(cik:"0000320193")."""
    return f'{_names(s)[3]}(cik:"{cik}")'


def _loaded_cell(s, entity, cik):
    """loaded(cik) is true for each company of this run's scope: primary, so no source, under the SEC's licence."""
    summary = (f"{entity} (cik {cik}) is loaded: its figures are in the scope of this run, the top N of the ranking by "
               "us-gaap:Assets in the frames API for CY2023Q4I (see the left-out report)")
    return {"cell": {"fn": f"{_names(s)[3]}/loaded", "args": {"cik": {"type": "string", "value": cik}}},
            "value": {"type": "boolean", "value": True}, "licence": s["licence"], "summary": common.nfc(summary)}


LABEL_KINDS = {"title": "the title as listed", "ticker": "a ticker as listed",
               "short name": "the short name (the title without its legal-form words)"}


def _label_cell(s, entity, cik, label, kind):
    """from_label(label) is the key of the company, for one label as the tickers file lists it (C3)."""
    summary = (f'{LABEL_KINDS[kind]} of {entity} (cik {cik}): "{label}", from company_tickers_exchange.json; a label is '
               "exact and case-sensitive, and names this company only")
    return {"cell": {"fn": f"{_names(s)[3]}/from_label", "args": {"label": {"type": "string", "value": label}}},
            "value": {"type": "string", "value": _key(s, cik)}, "licence": s["licence"], "summary": common.nfc(summary)}


def _company_functions(s):
    """The stored functions of our subclass that are not formulas (C3): annual_report (a plain accession string, not a
    class key: the fact's accession is a string, and chaining on a returned key fails at define), loaded (the extent's
    keys) and from_label (the lookup, returning the key of the company)."""
    sub = _names(s)[3]
    return [
        {"function": f"{sub}/annual_report", "signature": {
            "args": {"cik": dict(S_ARG), "fiscal_year": dict(N_ARG)}, "order": ["cik", "fiscal_year"], "returns": "string"}},
        {"function": f"{sub}/loaded", "signature": {"args": {"cik": dict(S_ARG)}, "order": ["cik"], "returns": "boolean"}},
        {"function": f"{sub}/from_label", "signature": {
            "args": {"label": {"type": "string", "valid": False}}, "order": ["label"],
            "returns": {"type": "string", "classes": [sub]}}},
    ]


def _company_catalogs(s, cells, entities):
    """The catalogue of sec/company's functions (C3): annual_report (our selection), loaded and from_label, with their
    examples taken from this run's cells."""
    sub = _names(s)[3]
    ann_cells, loaded_cells, label_cells = cells["annual"], cells["loaded"], cells["label"]
    out = []
    out.append({"catalog": f"{sub}/annual_report", "kind": "function", "label": "Latest annual report of a fiscal year (our selection)",
                "description": ("Datumesh's selection over the SEC's filings, not relayed: the accession of the latest-filed "
                                "annual report (10-K or 10-K/A) of a company's fiscal year (fy as filed, not the period's "
                                "year). A later amendment wins; a tie on the filing date goes to the higher accession. A "
                                "fact about the filings, not a figure; stored by the operator with no source."),
                "args": {"cik": "the company's CIK, 10 digits with leading zeros",
                         "fiscal_year": "the fiscal year as filed, a number such as 2023 (unit 1)"},
                "examples": [{"call": _call(f"{sub}/annual_report", ["cik", "fiscal_year"],
                                            {k: v["value"] for k, v in c["cell"]["args"].items()}, bare=("fiscal_year",)),
                              "expected": c["value"]} for c in ann_cells[:2]],
                "dictionary": {"cik": [{"value": c, "label": entities[c]} for c in sorted(entities)]}})
    out.append({"catalog": f"{sub}/loaded", "kind": "function", "label": "Company loaded in this run (its figures are in scope)",
                "description": ("Datumesh's list of the companies whose figures are in the scope of this run: true for each of "
                                "them, and for no other company. It is the extent of the class sec/company (the companies it "
                                "lists). A fact about the scope, primary, with no source."),
                "args": {"cik": "the company's CIK, 10 digits with leading zeros"},
                "examples": [{"call": f'{sub}/loaded(cik: "{c["cell"]["args"]["cik"]["value"]}")',
                              "expected": c["value"]} for c in loaded_cells[:2]],
                "dictionary": {"cik": [{"value": c, "label": entities[c]} for c in sorted(entities)]}})
    out.append({"catalog": f"{sub}/from_label", "kind": "function", "label": "Company by its name or ticker (exact)",
                "description": ("Datumesh's lookup of a company by a label as the SEC lists it in company_tickers_exchange.json: "
                                "the company's name, each of its tickers, and a short name (the name without its legal-form "
                                "words). Labels are exact and case-sensitive: no folding and no other spelling. A label that "
                                "names more than one company is not in the lookup. The value is the key of that company in "
                                "sec/company."),
                "args": {"label": "a name or a ticker exactly as the SEC lists it, case-sensitive"},
                "examples": [{"call": f'{sub}/from_label(label: "{c["cell"]["args"]["label"]["value"]}")',
                              "expected": c["value"]} for c in label_cells[:2]]})
    return out


KIND_OF_LABEL = {"title": "title", "ticker": "ticker", "short name": "short name"}
LEGAL_WORDS = ("Inc", "Inc.", "Incorporated", "Corp", "Corp.", "Corporation", "Co", "Co.", "Company", "Ltd", "Ltd.",
               "Limited", "plc", "PLC", "N.V.", "NV", "S.A.", "SA", "AG", "SE", "LP", "L.P.", "LLC", "& Co", "& Co.")


def short_name(title: str) -> str:
    """The short name of a title (C3 (c)): trailing legal-form words removed, repeatedly, with a trailing /XX/ state suffix;
    then trailing spaces, commas and periods trimmed. The words are whole words, case-insensitive, the longest first (so
    `& Co` goes before `Co`). Empty when the title is only legal-form words."""
    t = title
    words = sorted(LEGAL_WORDS, key=len, reverse=True)
    while True:
        before = t
        t = t.rstrip()
        m = re.search(r"\s*/[A-Z]{2}/$", t)
        if m:
            t = t[:m.start()]
        for w in words:
            m = re.search(r"(?i)(?:^|\s)" + re.escape(w) + r"$", t)
            if m:
                t = t[:m.start()]
                break
        if t == before:
            break
    return t.rstrip(" ,.")


def company_labels(tickers: bytes, ciks):
    """The labels that find a company, from the tickers file the scope rule already fetched (no request): the title as
    listed, each ticker as listed, and the short name (C3). NFC; case-sensitive; no folding. -> (pairs, ambiguous): pairs is
    the sorted [(label, cik, kind)] of the labels that name one company of `ciks`, and ambiguous is {label: [ciks]}, the
    labels that name more than one (none of those is emitted)."""
    t = _load(tickers)
    common.shape(isinstance(t, dict) and isinstance(t.get("fields"), list) and isinstance(t.get("data"), list),
                 "tickers file: fields and data are missing")
    fields = t["fields"]
    for k in ("cik", "ticker", "name"):
        common.shape(k in fields, f"tickers file has no {k}")
    ci, ti, ni = fields.index("cik"), fields.index("ticker"), fields.index("name")
    want = set(ciks)
    titles, tickers_of = {}, {}
    for row in t["data"]:
        common.shape(isinstance(row, list) and len(row) == len(fields), "tickers row does not match its fields")
        if not _int(row[ci]):
            continue
        cik = f"{row[ci]:010d}"
        if cik not in want:
            continue
        common.shape(isinstance(row[ni], str) and isinstance(row[ti], str), f"tickers row of {cik} has no name or ticker text")
        titles.setdefault(cik, unicodedata.normalize("NFC", row[ni]))
        lst = tickers_of.setdefault(cik, [])
        tk = unicodedata.normalize("NFC", row[ti])
        if tk not in lst:
            lst.append(tk)
    by_label = {}
    for cik in sorted(titles):
        title = titles[cik]
        named = [(title, "title")] + [(x, "ticker") for x in tickers_of[cik]]
        short = short_name(title)
        if short and short != title:
            named.append((short, "short name"))
        for label, kind in named:
            by_label.setdefault(label, {}).setdefault(cik, kind)
    ambiguous = {label: sorted(c) for label, c in by_label.items() if len(c) > 1}
    pairs = sorted((label, cik, kind) for label, c in by_label.items() if len(c) == 1 for cik, kind in c.items())
    return pairs, ambiguous


def _submitted(item, accn, field) -> bool:
    """True when the item's submissions list the filing with a value for the field (filed always; period_end when the
    reportDate is not empty)."""
    sub = item.get("submissions")
    return sub is not None and accn in sub["filings"] and sub["filings"][accn][field] is not None


def filed_differences(items) -> dict:
    """{accession: {"companyfacts": d1, "submissions": d2}} where the companyfacts filed date differs from the submissions'
    filingDate (C2b). Listed in the run report, never raised."""
    out = {}
    for it in items:
        sub = it.get("submissions")
        if sub is None:
            continue
        for accn, f in it["parsed"]["filings"].items():
            if accn in sub["filings"] and sub["filings"][accn]["filed"] != f["filed"]:
                out[accn] = {"companyfacts": f["filed"], "submissions": sub["filings"][accn]["filed"]}
    return out


def latest_filings(items):
    """Sets each company's filing_for (C2b): the latest-filed annual report of each fiscal year, ordered by the relayed
    filed date where the submissions list the filing, and by the companyfacts file's otherwise; a tie goes to the higher
    accession."""
    for it in items:
        sub = it.get("submissions")
        relayed = {a: v["filed"] for a, v in sub["filings"].items()} if sub is not None else {}
        by_fy = {}
        for accn, f in it["parsed"]["filings"].items():
            by_fy.setdefault(f["fy"], []).append((relayed.get(accn, f["filed"]), accn))
        it["parsed"]["filing_for"] = {fy: max(c)[1] for fy, c in sorted(by_fy.items())}


def emit(items, state, names):
    """The relay's lines for what is new in `items` (CONTRACT-sec-classes C1, C2): on the first run, the stored descriptors
    of the company class (its method fact) and of the filing class (six methods), their two class lines and the relay's
    catalogue; then the cells: each figure (company/fact), each filing's five companyfacts methods, and its period_end from
    the submissions when an item carries them (`item["submissions"]`, attached by run). Nothing is emitted for a fact or a
    filing already in the state; with nothing new the state is returned as it was."""
    s = names["sec"]
    cls = _names(s)[2]
    seen_facts = set(state.get("facts", []))
    seen_filings = set(state.get("filings", []))
    cells = {"fact": [], "filing": []}
    all_rows, labels, entities = [], {}, {}
    for it in items:
        p = it["parsed"]
        src = {"url": it["source"]["url"], "retrieved": it["source"]["retrieved"], "sha256": it["source"]["sha256"]}
        cik = p["cik"]
        entities[cik] = p["entity"]
        labels.update(p["labels"])
        all_rows += p["rows"]
        for r in p["rows"]:
            k = _fact_key(cik, r)
            if k in seen_facts:
                continue
            seen_facts.add(k)
            cells["fact"].append(_fact_cell(s, p["entity"], cik, r, src))
        sub = it.get("submissions")
        for accn, f in p["filings"].items():
            if accn in seen_filings:
                continue
            seen_filings.add(accn)
            for field in CF_FIELDS:
                cells["filing"].append(_filing_cell(s, cls, p["entity"], accn, f, field, src))
            for field in SUBMITTED_FIELDS:  # C2, C2b: from the submissions record, or no cell
                if _submitted(it, accn, field):
                    value = sub["filings"][accn][field]
                    cells["filing"].append(_filing_cell(s, cls, p["entity"], accn, dict(f, **{field: value}), field,
                                                        sub["sources"][accn]))
    if not any(cells.values()):
        return [], state
    lines = []
    if not state.get("announced"):
        lines += _class_lines(s) + _relay_catalogs(s, cells, all_rows, labels, entities)
    for kind in ("fact", "filing"):
        lines += cells[kind]
    new = dict(state)
    new.update({"announced": True, "facts": sorted(seen_facts), "filings": sorted(seen_filings)})
    return lines, new


def company_emit(items, labels, state, names):
    """The lines of sec_company-<stamp>.jsonl (C3) for this run's companies. On the first run: the three stored functions
    and their catalogue. Then the cells: loaded (true for each company of the scope), annual_report (our selection of each
    fiscal year) and from_label (each label of `labels`, the (label, cik, kind) triples of company_labels). Nothing is
    emitted for a company, year or label already in the state; with nothing new the state is returned as it was."""
    s = names["sec"]
    seen_loaded = set(state.get("loaded", []))
    seen_labels = {tuple(x) for x in state.get("labels", [])}
    filing_for_state = dict(state.get("filing_for", {}))
    cells = {"loaded": [], "annual": [], "label": []}
    entities = {}
    for it in items:
        p = it["parsed"]
        cik = p["cik"]
        entities[cik] = p["entity"]
        if cik not in seen_loaded:
            seen_loaded.add(cik)
            cells["loaded"].append(_loaded_cell(s, p["entity"], cik))
        for fy, accn in p["filing_for"].items():
            key = f"{cik}|{fy}"
            if filing_for_state.get(key) == accn:
                continue
            filing_for_state[key] = accn
            cells["annual"].append(_annual_cell(s, p["entity"], cik, fy, accn))
    for label, cik, kind in labels:
        if cik in entities and (label, cik) not in seen_labels:
            seen_labels.add((label, cik))
            cells["label"].append(_label_cell(s, entities[cik], cik, label, kind))
    if not any(cells.values()):
        return [], state
    lines = []
    if not state.get("announced"):
        lines += _company_functions(s) + _company_catalogs(s, cells, entities)
    for kind in ("loaded", "annual", "label"):
        lines += cells[kind]
    new = dict(state)
    new.update({"announced": True, "loaded": sorted(seen_loaded), "labels": sorted([list(x) for x in seen_labels]),
                "filing_for": filing_for_state})
    return lines, new


def _submissions(get, s, cik, accns, now):
    """One company's submissions (the second copy of the filing class, C2): filings.recent first, then the pages of
    filings.files in order, one request each, only while an in-scope accession (`accns`) is still missing. -> {"cik",
    "filings", "sources"} for the in-scope filings found, each with the source it was read from; None when the company has
    no submissions file (its filings then have no period_end and no second copy)."""
    url = _sub_url(s, cik)
    try:
        data = get(url)
    except NotPublished:
        common.warn(f"sec: no submissions for {cik}; its period ends and filing tests are not written this run")
        return None
    main = parse_submissions(data)
    found, sources = {}, {}

    def take(filings, source):
        for accn in accns:
            if accn in filings and accn not in found:
                found[accn] = filings[accn]
                sources[accn] = source

    take(main["filings"], {"url": url, "retrieved": now, "sha256": common.sha256_hex(data)})
    for name in main["pages"]:
        if all(accn in found for accn in accns):
            break
        purl = SUBMISSIONS_PAGES + name
        try:
            pdata = get(purl)
        except NotPublished:
            common.warn(f"sec: submissions page {purl} is not published; it is not read")
            continue
        take(parse_submissions_page(pdata), {"url": purl, "retrieved": now, "sha256": common.sha256_hex(pdata)})
    return {"cik": main["cik"], "filings": found, "sources": sources}


def company_tests(items, labels, names):
    """The pins of sec_company (C3): annual_report of each company's earliest and latest fiscal year; loaded and from_label
    of the first three companies of the scope (rank order): loaded is true, and each kind of label of each resolves to it."""
    s = names["sec"]
    sub = _names(s)[3]
    first = [it["parsed"]["cik"] for it in items[:3]]
    tests = annual_report_tests(items, names)
    for cik in first:
        tests.append({"test": f"{sub}/loaded", "call": f'{sub}/loaded(cik: "{cik}")',
                      "expect": {"type": "boolean", "value": True}})
    for label, cik, kind in labels:
        if cik in first:
            tests.append({"test": f"{sub}/from_label", "call": f'{sub}/from_label(label: "{label}")',
                          "expect": {"type": "string", "value": _key(s, cik)}})
    return tests


# ---------- conformance ----------

def _fact_test_call(fq, cik, concept, unit, accession, start, end):
    return _call(fq, FACT_ORDER, {"cik": cik, "concept": concept, "unit": unit, "accession": accession,
                                  "period_start": start, "period_end": end}, bare=("period_start", "period_end"))


def conformance(rows, frames_list, names, minimum=CONFORMANCE_MINIMUM):
    """Tests of the relay method company/fact from the frames API (a second channel for the same concept, period and CIK), by the frame's
    own accession: a frame fact of an in-scope report must be in the rows with the same value; a frame entry whose
    report is out of scope is skipped, and so is a frame in a unit that is not relayed. Every test calls company/fact
    with the frame's unit. At least `minimum` tests, else ConformanceError."""
    s = names["sec"]
    fq = _names(s)[1]
    by_key = {(r["cik"], r["concept"], r["unit"], r["accession"], r["period_start"], r["period_end"]): r for r in rows}
    accessions = {}
    for r in rows:
        accessions.setdefault(r["cik"], set()).add(r["accession"])
    tests = []
    for f in frames_list:
        if not relayed_unit(f["unit"]):
            continue
        concept = f"{f['taxonomy']}:{f['concept']}"
        for fr in f["rows"]:
            key = (fr["cik"], concept, f["unit"], fr["accession"], fr["period_start"], fr["period_end"])
            row = by_key.get(key)
            if row is None:
                if fr["accession"] in accessions.get(fr["cik"], ()):
                    raise ConformanceError(
                        f"{fq} {fr['cik']}: the frame has {concept} for {fr['period_end']} in {fr['accession']}, "
                        f"and the scope rows of that report do not")
                continue
            if Decimal(row["value"]) != Decimal(fr["value"]):
                raise ConformanceError(
                    f"{fq} {fr['cik']} {fr['accession']} {concept} {fr['period_end']}: {row['value']} against "
                    f"{fr['value']} in the frame (the frame is the second copy); nothing is published")
            tests.append({"test": fq, "call": _fact_test_call(fq, fr["cik"], concept, f["unit"], fr["accession"],
                                                              fr["period_start"], fr["period_end"]),
                          "expect": {"type": "number", "value": fr["value"]}})
    if len(tests) < minimum:
        raise ConformanceError(f"{fq}: {len(tests)} source-conformance tests, at least {minimum} are needed; "
                               "nothing is published")
    return tests


def filing_conformance(items, submissions_list, names, minimum=FILING_CONFORMANCE_MINIMUM):
    """Tests of the six filing methods against the submissions API (the second copy of the filing class), one per
    method per annual report that both files have. A report the submissions do not list is skipped, not failed. The
    expected values are the submissions' own. cik and form are compared with the companyfacts file: every disagreement is
    collected, and the run stops with one ConformanceError that lists them (at most MAX_SHOWN, then the count). filed and
    period_end are the submissions' own (C2, C2b): no second copy is compared, and a filed date that differs from the
    companyfacts file is listed in the run report (filed_differences). The submissions API has no fiscal year or fiscal
    period, so those two have no second copy either: their tests pin the companyfacts file's own value, and no derived
    guess is compared (a year derived from the filing date was wrong on real filings)."""
    s = names["sec"]
    cls = _names(s)[2]
    mine = {}
    for it in items:
        for accn, f in it["parsed"]["filings"].items():
            mine[accn] = f
    tests, bad = [], []
    for sub in submissions_list:
        for accn, sf in sub["filings"].items():
            f = mine.get(accn)
            if f is None:
                continue
            if sub["cik"] != f["cik"]:
                bad.append(f"{cls} {accn}: the submissions say cik {sub['cik']} and the companyfacts file says cik {f['cik']}")
                continue
            theirs = {"cik": sub["cik"], "form": sf["form"]}
            for m in FILING_METHODS:
                if m == "period_end":
                    # C2: the submissions' reportDate; none is no cell and no test
                    if sf["period_end"] is None:
                        continue
                    value = sf["period_end"]
                elif m == "filed":
                    # C2b: the submissions' filingDate; a difference is listed in the report, not raised
                    value = sf["filed"]
                elif m in theirs:
                    mine_v = _filing_value(f, m)
                    if str(mine_v) != str(theirs[m]):
                        bad.append(f"{cls}/{m} {accn}: the companyfacts file says {mine_v} and the submissions API says "
                                   f"{theirs[m]}")
                    value = theirs[m]
                else:
                    # No second copy of fiscal_year and fiscal_period: the test pins the relayed value.
                    value = _filing_value(f, m)
                tests.append({"test": f"{cls}/{m}", "call": f'{cls}/{m}(accession: "{accn}")',
                              "expect": {"type": FILING_VALUE_TYPES[m], "value": str(value)}})
    if bad:
        more = f" (and {len(bad) - MAX_SHOWN} more)" if len(bad) > MAX_SHOWN else ""
        raise ConformanceError(f"{len(bad)} disagreements with the submissions API: " + "; ".join(bad[:MAX_SHOWN])
                               + f"{more}; nothing is published")
    if len(tests) < minimum:
        raise ConformanceError(f"{cls}: {len(tests)} filing tests against the submissions, at least {minimum} are "
                               "needed; nothing is published")
    return tests


def annual_report_tests(items, names):
    """Pins of sec/annual_report: the earliest and the latest fiscal year of each company, the accession our selection
    gives (the filing_for of its companyfacts file)."""
    fn = f"{_names(names['sec'])[3]}/annual_report"
    tests = []
    for it in items:
        fy = it["parsed"]["filing_for"]
        if not fy:
            continue
        cik = it["parsed"]["cik"]
        for y in sorted({min(fy, key=int), max(fy, key=int)}, key=int):
            tests.append({"test": fn, "call": _call(fn, ["cik", "fiscal_year"], {"cik": cik, "fiscal_year": y},
                                                    bare=("fiscal_year",)),
                          "expect": {"type": "string", "value": fy[y]}})
    return tests


# ---------- the run ----------

class _Counts(Mapping):
    """The cells of each company in ranking order, read from its companyfacts file when first asked for. A company
    with no companyfacts file counts 0 and is listed."""

    def __init__(self, ciks, load):
        self.ciks, self.load = list(ciks), load
        self.cache, self.no_facts = {}, []

    def __getitem__(self, cik):
        if cik not in self.cache:
            try:
                self.cache[cik] = count_cells(self.load(cik))
            except NotPublished:
                self.cache[cik] = 0
                self.no_facts.append(cik)
        return self.cache[cik]

    def __iter__(self):
        return iter(self.ciks)

    def __len__(self):
        return len(self.ciks)


def run(names, *, fetch, now, state_dir, out_dir, publish=None, sleep=time.sleep, clock=time.monotonic,
        budget=BUDGET, step=STEP):
    """Fetch the ranking, count the cells in rank order up to the budget, emit the chosen companies' cells, test them
    against the frames and the submissions, and write the files: the relay (sec-<stamp>.jsonl: the relay classes, their
    cells and tests, the register's cells), the subclass's functions (sec_company-<stamp>.jsonl: annual_report, loaded and
    from_label, written only when it has lines) and the left-out report. Nothing is written when a request is refused, a
    shape changes or a test disagrees. The files are published relay first, then the company file; on a failure both are
    removed and no state is kept. The state is saved after publish returns. Returns the relay path (the company file's
    when the relay has no lines), or None when nothing is new."""
    meta = names.get("meta", {})
    if meta.get("host") == PRODUCTION or meta.get("status") == "production" or "sec" not in names:
        raise NotForProduction(f"the SEC relay is rehearsal-only (K7); host {meta.get('host')!r} is refused")
    s = names["sec"]
    bucket = TokenBucket(MAX_REQUESTS_PER_SECOND, clock=clock, sleep=sleep)

    def get(url):
        bucket.wait()
        return fetch(url)

    state_path = Path(state_dir) / "sec.json"
    out_dir = Path(out_dir)
    state = common.load_state(state_path)

    tickers = get(s["tickers_url"])
    ranking_bytes = get(_frame_url(s, RANKING_FRAME))
    ranking = rank_companies(tickers, ranking_bytes)
    frames = [parse_frames(ranking_bytes)]
    for frame in CONFORMANCE_FRAMES[1:]:
        try:
            frames.append(parse_frames(get(_frame_url(s, frame))))
        except NotPublished:
            common.warn(f"sec: frame {frame} is not published; no tests from it this run")

    ranked = ranking["ranked"]
    counts = _Counts([r["cik"] for r in ranked], lambda cik: get(_cf_url(s, cik)))
    choice = choose_n(counts.ciks, counts, budget=budget, step=step)
    n = choice["n"]
    chosen = [r["cik"] for r in ranked[:n]]
    no_facts = [c for c in counts.no_facts if c in set(chosen)]
    common.warn(f"sec scope N={n} estimated_cells={choice['estimated']} budget={budget} step={step} ranked={len(ranked)}")

    items = []
    for cik in chosen:
        if cik in no_facts:
            continue
        url = _cf_url(s, cik)
        try:
            data = get(url)
        except NotPublished:
            no_facts.append(cik)
            continue
        items.append({"parsed": parse_companyfacts(data),
                      "source": {"url": url, "retrieved": now, "sha256": common.sha256_hex(data)}})

    for it in items:
        it["submissions"] = _submissions(get, s, it["parsed"]["cik"], sorted(it["parsed"]["filings"]), now)
    submissions = [it["submissions"] for it in items if it["submissions"] is not None]
    latest_filings(items)

    lines, st = emit(items, state, names)
    pairs, ambiguous = company_labels(tickers, [it["parsed"]["cik"] for it in items])
    clines, cst = company_emit(items, pairs, state.get("company", {}), names)
    if clines:
        st = dict(st)
        st["company"] = cst
    # the register's cells of sec.gov / company_facts, on the first run (register.py's emit: a later run emits
    # nothing unless the licence text changed); the register's class lines are register.py's, not this file's
    reg_blobs = {}
    reg_lines, reg_state = licence_register.emit(
        [names["sec_register_entry"]], {"class": True, **state.get("register", {})},
        {"register": names["sec_register"]}, licences=[names["sec_register_licence"]], blobs=reg_blobs)
    if reg_lines:
        st = dict(st)
        st["register"] = reg_state
    lines = lines + reg_lines
    rows = [r for it in items for r in it["parsed"]["rows"]]
    tests = conformance(rows, frames, names) + filing_conformance(items, submissions, names)
    ctests = company_tests(items, pairs, names)
    tested = set(st.get("tested", []))
    new_tests = [t for t in tests if t["call"] not in tested]
    new_ctests = [t for t in ctests if t["call"] not in tested]
    if new_tests or new_ctests:
        st = dict(st)
        st["tested"] = sorted(tested | {t["call"] for t in new_tests + new_ctests})
        lines += new_tests
        clines += new_ctests
    cells = sum(1 for l in lines + clines if "cell" in l)
    common.warn(f"sec: {cells} cells, {len(new_tests) + len(new_ctests)} tests written; {len(tests)} tests from the second copies")
    if not lines and not clines:
        return None

    stamp = relay._stamp(now)
    path = out_dir / f"sec-{stamp}.jsonl"
    company_path = out_dir / f"sec_company-{stamp}.jsonl"
    report_path = out_dir / f"sec-scope-{stamp}.json"
    left_out_units = {}
    for it in items:
        for u, k in it["parsed"]["left_out_units"].items():
            left_out_units[u] = left_out_units.get(u, 0) + k
    if left_out_units:
        common.warn(f"sec: units not relayed (D38), cells left out: {dict(sorted(left_out_units.items()))}")
    report = scope_report(ranking, choice, no_facts=no_facts, unsupported_units=left_out_units, budget=budget,
                          step=step)
    for field, key in (("period_end", "left_out_period_end"), ("filed", "left_out_filed")):
        missing = sorted(accn for it in items for accn in it["parsed"]["filings"] if not _submitted(it, accn, field))
        report["left_out"][key] = {"count": len(missing), "accessions": missing}
    report["filed_differs"] = filed_differences(items)
    report["left_out"]["ambiguous_labels"] = ambiguous
    written = []
    if lines:
        common.write_jsonl(path, lines)
        written.append(path)
    if clines:
        common.write_jsonl(company_path, clines)
        written.append(company_path)
    for rel, data in reg_blobs.items():  # content-addressed, as register.py writes them
        target = out_dir / rel
        target.parent.mkdir(exist_ok=True)
        if not target.exists():
            target.write_bytes(data)
    try:
        with open(report_path, "x", encoding="utf-8", newline="\n") as f:
            json.dump(report, f, ensure_ascii=False, indent=2, sort_keys=True)
            f.write("\n")
    except BaseException:
        for p in written:
            p.unlink(missing_ok=True)
        raise
    common.warn(f"sec: left-out report written to {report_path}")
    if publish is not None:
        try:
            for p in written:
                publish(p)
        except BaseException:
            for p in written + [report_path]:
                p.unlink(missing_ok=True)
            raise
    common.save_state(state_path, st)
    return written[0]


# ---------- the CLI ----------

def _host_arg(value: str) -> str:
    """The SEC relay's hosts are the hosts of sec_names.toml (rehearsal only); names() refuses anything else."""
    try:
        names(value)
    except (common.UnknownHost, NotForProduction) as e:
        raise argparse.ArgumentTypeError(str(e)) from e
    return value


def main(argv=None):
    p = argparse.ArgumentParser(description="The SEC company facts relay (rehearsal hosts only).")
    p.add_argument("--host", required=True, type=_host_arg,
                   help="a rehearsal host of sec_names.toml, spelled exactly (no default; datumesh.com is refused)")
    p.add_argument("--state-dir", required=True)
    p.add_argument("--out-dir", required=True)
    a = p.parse_args(argv)
    table = names(a.host)
    try:
        path = run(table, fetch=http_get, now=common.now_instant(), state_dir=a.state_dir, out_dir=a.out_dir)
    except SecRefused as e:
        common.warn(f"sec: stopped: {e}")
        return 1
    return common.finish(path)


if __name__ == "__main__":
    raise SystemExit(main())
