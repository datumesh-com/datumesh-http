"""SEC company facts (Build 2 Part 7, mesh-test only): fetch with fair access, scope by rule, relay as filed,
source-conformance against the frames API, and the run.

The relay (K2): one cell per reported figure per filing, keyed by (cik, taxonomy, concept, unit, accession, period),
so a figure in one annual report and the same figure restated in a later one are two cells. Nothing is normalised
under the relay name. Filing metadata (filing) and the accession of the latest annual report per fiscal year
(filing_for, D36) are stored helpers derived from the same records.

The scope (K3, D35): 10-K and 10-K/A, fiscal years 2019 to 2024, the listed concepts of sec_concepts.toml, and the
NYSE or Nasdaq companies ranked by us-gaap:Assets (USD) in the frames API for CY2023Q4I, the top N, where N is the
largest multiple of 25 whose estimated cells stay within 300,000 (or every company, when they all fit).

Fair access (K1, D34): one connection, sequential, at most 5 requests a second (a token bucket), the declared
User-Agent and nothing else about us, and a 403 stops the run. Standard library only.
"""
import decimal
import http.client
import json
import time
import tomllib
import urllib.error
import urllib.request
from collections.abc import Mapping
from datetime import date
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
FILING_FIELDS = ("cik", "form", "filed", "fy", "fp", "period_end")
ONE_YEAR_DAYS = (350, 380)
CONFORMANCE_MINIMUM = 20

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


def _period(start, end) -> str:
    return f"{start}/{end}" if start is not None else end


def _days(start, end) -> int:
    return (date.fromisoformat(end) - date.fromisoformat(start)).days


# ---------- companyfacts ----------

def parse_companyfacts(data: bytes) -> dict:
    """One companyfacts file -> the in-scope cells as filed, conflicts, labels, filing metadata and filing_for."""
    raw = _load(data)
    common.shape(isinstance(raw, dict), "companyfacts is not an object")
    cik = raw.get("cik")
    common.shape(_int(cik) and 0 < cik < 10 ** 10, "companyfacts cik is not a CIK")
    entity = raw.get("entityName")
    common.shape(isinstance(entity, str) and entity.strip() != "", "companyfacts entityName is missing")
    facts = raw.get("facts")
    common.shape(isinstance(facts, dict), "companyfacts facts is missing")
    cik10 = f"{cik:010d}"
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
                    _note_filing(meta, cik10, tax, r)
                    if not is_listed:
                        continue
                    key = (tax, concept, unit, r["accn"], _period(r.get("start"), r["end"]))
                    g = groups.setdefault(key, {"values": {}, "rec": r})
                    g["values"][canon(r["val"])] = True

    rows, conflicts = [], []
    for (tax, concept, unit, accn, period), g in groups.items():
        values = sorted(g["values"])
        if len(values) > 1:
            conflicts.append({"cik": cik10, "taxonomy": tax, "concept": concept, "unit": unit, "accession": accn,
                              "period": period, "values": values})
            continue
        r = g["rec"]
        rows.append({"cik": cik10, "taxonomy": tax, "concept": concept, "unit": unit, "accession": accn,
                     "period": period, "value": values[0], "form": r["form"], "fy": str(r["fy"]), "fp": r["fp"],
                     "filed": r["filed"]})

    filings = {}
    for accn in sorted({r["accession"] for r in rows}):
        m = meta[accn]
        ends = m["one_year"] or m["ends"] or m["other_ends"]
        filings[accn] = {"cik": cik10, "form": m["form"], "filed": m["filed"], "fy": m["fy"], "fp": m["fp"],
                         "period_end": max(ends)}

    by_fy = {}
    for accn, f in filings.items():
        by_fy.setdefault(f["fy"], []).append((f["filed"], accn))
    filing_for = {fy: max(c)[1] for fy, c in sorted(by_fy.items())}

    return {"cik": cik10, "entity": entity, "rows": rows, "conflicts": conflicts, "labels": labels,
            "filings": filings, "filing_for": filing_for}


def _note_filing(meta, cik10, tax, r):
    m = meta.setdefault(r["accn"], {"cik": cik10, "form": r["form"], "filed": r["filed"], "fy": str(r["fy"]),
                                    "fp": r["fp"], "one_year": set(), "ends": set(), "other_ends": set()})
    if tax != "us-gaap":
        m["other_ends"].add(r["end"])  # used only for a filing with no us-gaap facts (cover-page dates)
    elif "start" in r and ONE_YEAR_DAYS[0] <= _days(r["start"], r["end"]) <= ONE_YEAR_DAYS[1]:
        m["one_year"].add(r["end"])  # the statements' annual periods end on the period end
    else:
        m["ends"].add(r["end"])


def count_cells(data: bytes) -> int:
    return len(parse_companyfacts(data)["rows"])


# ---------- frames ----------

def parse_frames(data: bytes) -> dict:
    """A frames file -> its rows (cik, accession, period, value). The unit is spelled as companyfacts spells it."""
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
        rows.append({"cik": f"{d['cik']:010d}", "accession": d["accn"],
                     "period": _period(d.get("start"), d["end"]), "value": canon(d["val"])})
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


def scope_report(ranking, choice, *, no_facts=(), budget=BUDGET, step=STEP) -> dict:
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


def sec_table(host) -> dict:
    """The SEC section of sec_names.toml for one host (K7). Production is refused by name; any host the file does not
    list is common.UnknownHost. Names come with {host} spelled for the host."""
    if host == PRODUCTION:
        raise NotForProduction(f"the SEC names are rehearsal-only (Part 7, K7): {PRODUCTION} is refused")
    with open(SEC_NAMES, "rb") as f:
        raw = tomllib.load(f)
    if not isinstance(host, str) or host not in raw["hosts"]:
        raise common.UnknownHost(f"host {host!r} is not in sec_names.toml, which lists {sorted(raw['hosts'])}")
    body = {k: v for k, v in raw.items() if k != "hosts"}
    return common._with_host(body, host)


def names(host) -> dict:
    """common.names(host) plus the SEC tables: `sec` (the relay), `sec_register` (the register table of names.toml with
    the one SEC dataset), `sec_register_entry` and `sec_register_licence` (the licence facts)."""
    raw = sec_table(host)
    table = dict(common.names(host))
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


# ---------- lines ----------

def _fq(s):
    return f"{s['namespace']}/{s['function']}", f"{s['namespace']}/{s['filing_function']}", \
        f"{s['namespace']}/{s['filing_for_function']}"


def _call(fq, argnames, values) -> str:
    return f"{fq}({', '.join(f'{a}: {chr(34)}{values[a]}{chr(34)}' for a in argnames)})"


def _source(s, src) -> dict:
    return {"name": s["source_name"], "url": src["url"], "retrieved": src["retrieved"], "sha256": src["sha256"],
            "takeover": s["takeover"]}


def _fact_key(cik, r) -> str:
    return "|".join((cik, r["taxonomy"], r["concept"], r["unit"], r["accession"], r["period"]))


FACT_ARGS = ("cik", "taxonomy", "concept", "unit", "accession", "period")
FILING_ARGS = ("accession", "field")
FILING_FOR_ARGS = ("cik", "fiscal_year")

FIELD_LABELS = {
    "cik": "the company's CIK (10 digits)",
    "form": "the form, 10-K or 10-K/A",
    "filed": "the filing date",
    "fy": "the fiscal year of the report, as filed (not the period's year)",
    "fp": "the fiscal period of the report (FY)",
    "period_end": "the period end of the report's annual statements (not a cover-page date)",
}
UNIT_LABELS = {"USD": "US dollars", "shares": "shares", "USD/shares": "US dollars per share", "pure": "a pure number"}
TAXONOMY_LABELS = {"us-gaap": "US GAAP taxonomy", "dei": "document and entity information (dei)"}


def emit(items, state, names):
    """The lines of the new cells (and, on the first run, the functions and catalogue) and the new state. Nothing is
    emitted for a fact, filing or filing_for already in the state; with nothing new the state is returned as it was."""
    s = names["sec"]
    fq, ffq, gfq = _fq(s)
    seen_facts = set(state.get("facts", []))
    seen_filings = set(state.get("filings", []))
    filing_for_state = dict(state.get("filing_for", {}))
    cells = {"fact": [], "filing": [], "filing_for": []}
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
            cells["fact"].append(_fact_cell(s, fq, p["entity"], cik, r, src))
        for accn, f in p["filings"].items():
            if accn in seen_filings:
                continue
            seen_filings.add(accn)
            for field in FILING_FIELDS:
                cells["filing"].append(_filing_cell(s, ffq, p["entity"], accn, f, field, src))
        for fy, accn in p["filing_for"].items():
            key = f"{cik}|{fy}"
            if filing_for_state.get(key) == accn:
                continue
            filing_for_state[key] = accn
            cells["filing_for"].append(_filing_for_cell(s, gfq, p["entity"], cik, fy, accn, src))

    lines = []
    if not any(cells.values()):
        return [], state
    if not state.get("announced"):
        lines += _functions(s, fq, ffq, gfq)
        lines += _catalogs(s, fq, ffq, gfq, cells, all_rows, labels, entities)
    for kind in ("fact", "filing", "filing_for"):
        lines += cells[kind]
    new = dict(state)
    new.update({"announced": True, "facts": sorted(seen_facts), "filings": sorted(seen_filings),
                "filing_for": filing_for_state})
    return lines, new


def _fact_cell(s, fq, entity, cik, r, src):
    args = dict(zip(FACT_ARGS, (cik, r["taxonomy"], r["concept"], r["unit"], r["accession"], r["period"])))
    summary = (f"{entity} {r['form']} filed {r['filed']}, fiscal year {r['fy']}: {r['concept']} ({r['taxonomy']}) "
               f"for period {r['period']} is {r['value']} {r['unit']} as filed in accession {r['accession']}"
               f"{relay.tail(s)}")
    return {"cell": {"fn": fq, "args": {k: {"type": "string", "value": v} for k, v in args.items()}},
            "value": {"type": "number", "value": r["value"]}, "licence": s["licence"],
            "summary": common.nfc(summary), "source": _source(s, src)}


def _filing_cell(s, ffq, entity, accn, f, field, src):
    value = f[field]
    summary = (f"{entity} filing {accn}: {field} is {value} ({f['form']} filed {f['filed']}, fiscal year {f['fy']}, "
               f"{f['fp']})" + relay.tail(s))
    return {"cell": {"fn": ffq, "args": {"accession": {"type": "string", "value": accn},
                                         "field": {"type": "string", "value": field}}},
            "value": {"type": "string", "value": value}, "licence": s["licence"],
            "summary": common.nfc(summary), "source": _source(s, src)}


def _filing_for_cell(s, gfq, entity, cik, fy, accn, src):
    summary = (f"the latest-filed annual report of fiscal year {fy} of {entity} (cik {cik}) is accession {accn}; a "
               f"fact about the filings, not a figure" + relay.tail(s))
    return {"cell": {"fn": gfq, "args": {"cik": {"type": "string", "value": cik},
                                         "fiscal_year": {"type": "string", "value": fy}}},
            "value": {"type": "string", "value": accn}, "licence": s["licence"],
            "summary": common.nfc(summary), "source": _source(s, src)}


def _functions(s, fq, ffq, gfq):
    def sig(args, order, returns):
        return {"args": {k: {"type": "string", "valid": False} for k in args}, "order": order, "returns": returns}
    return [
        {"function": fq, "signature": sig(FACT_ARGS, list(FACT_ARGS), {"type": "number"})},
        {"function": ffq, "signature": sig(FILING_ARGS, list(FILING_ARGS), {"type": "string"})},
        {"function": gfq, "signature": sig(FILING_FOR_ARGS, list(FILING_FOR_ARGS), {"type": "string"})},
    ]


def _catalogs(s, fq, ffq, gfq, cells, rows, labels, entities):
    src_name, takeover = s["source_name"], s["takeover"]
    fact_cells = cells["fact"]
    used_concepts = sorted({(r["taxonomy"], r["concept"]) for r in rows})
    concept_dict = []
    for tax, c in used_concepts:
        lab = labels.get((tax, c), {"label": c, "description": None})
        e = {"value": c, "label": lab["label"]}
        if lab["description"] is not None:
            e["description"] = lab["description"]
        concept_dict.append(e)
    units = sorted({r["unit"] for r in rows})
    cik_dict = [{"value": c, "label": entities[c]} for c in sorted(entities)]
    fact_arg_text = {
        "cik": "the company's CIK, 10 digits with leading zeros",
        "taxonomy": "us-gaap or dei",
        "concept": "the XBRL element name exactly as filed (concept labels are in the dictionary)",
        "unit": "as filed: USD, shares, USD/shares or pure",
        "accession": "the accession number of the filing, for example 0000320193-23-000106",
        "period": "YYYY-MM-DD for an instant, or YYYY-MM-DD/YYYY-MM-DD for a duration",
    }
    fact_examples = [{"call": _call(fq, FACT_ARGS, dict(zip(FACT_ARGS, _cell_args(c)))),
                      "expected": c["value"]} for c in fact_cells[:2]]
    fact_cat = {"catalog": fq, "kind": "function",
                "label": "Company figure as filed (XBRL fact, one filing)",
                "description": (f"Relayed by datumesh.com from {src_name}: one figure as the company reported it in one "
                                "annual report (accession), exactly as filed: no scaling, no sign change, not "
                                "normalised. A figure restated in a later annual report is a separate cell with that "
                                "report's accession. Only 10-K and 10-K/A reports for fiscal years 2019 to 2024, and "
                                "the listed concepts; each figure is one cell per filing and period."),
                "args": fact_arg_text, "examples": fact_examples,
                "dictionary": {"concept": concept_dict,
                               "unit": [{"value": u, "label": UNIT_LABELS.get(u, u)} for u in units],
                               "taxonomy": [{"value": t, "label": TAXONOMY_LABELS[t]} for t in
                                            sorted({t for t, _ in used_concepts})],
                               "cik": cik_dict}}
    filing_examples = [{"call": _call(ffq, FILING_ARGS, dict(zip(FILING_ARGS, _cell_args(c)))),
                        "expected": c["value"]} for c in cells["filing"][:2]]
    filing_cat = {"catalog": ffq, "kind": "function", "label": "Filing field (as filed)",
                  "description": (f"Relayed by datumesh.com from {src_name}: one field of one annual report "
                                  "(accession), from the same companyfacts records, as filed; the period_end is the "
                                  "period end of the report's annual statements, not a cover-page date."),
                  "args": {"accession": "the accession number of the filing", "field": "one of " + ", ".join(FILING_FIELDS)},
                  "examples": filing_examples,
                  "dictionary": {"field": [{"value": f, "label": FIELD_LABELS[f]} for f in FILING_FIELDS]}}
    filing_for_examples = [{"call": _call(gfq, FILING_FOR_ARGS, dict(zip(FILING_FOR_ARGS, _cell_args(c)))),
                            "expected": c["value"]} for c in cells["filing_for"][:2]]
    filing_for_cat = {"catalog": gfq, "kind": "function", "label": "Latest annual report of a fiscal year",
                      "description": (f"Relayed by datumesh.com from {src_name}: the accession of the latest-filed annual "
                                      "report (10-K or 10-K/A) of a company's fiscal year (fy as filed, not the period's "
                                      "year); a later amendment wins; a tie on the filing date goes to the higher "
                                      "accession. A fact about the filings, not a figure."),
                      "args": {"cik": "the company's CIK, 10 digits with leading zeros",
                               "fiscal_year": "the fiscal year as filed, for example 2023"},
                      "examples": filing_for_examples,
                      "dictionary": {"cik": cik_dict}}
    return [fact_cat, filing_cat, filing_for_cat]


def _cell_args(cell_line):
    """The argument values of an emitted cell line, in the order of its call."""
    return [cell_line["cell"]["args"][k]["value"] for k in cell_line["cell"]["args"]]


# ---------- conformance ----------

def conformance(rows, frames_list, names, minimum=CONFORMANCE_MINIMUM):
    """Tests from the frames API (a second channel for the same concept, period and CIK), by the frame's own
    accession: a frame fact of an in-scope report must be in the rows with the same value; a frame entry whose report
    is out of scope is skipped. At least `minimum` tests, else ConformanceError."""
    s = names["sec"]
    fq = f"{s['namespace']}/{s['function']}"
    by_key = {(r["cik"], r["taxonomy"], r["concept"], r["unit"], r["accession"], r["period"]): r for r in rows}
    accessions = {}
    for r in rows:
        accessions.setdefault(r["cik"], set()).add(r["accession"])
    tests = []
    for f in frames_list:
        for fr in f["rows"]:
            key = (fr["cik"], f["taxonomy"], f["concept"], f["unit"], fr["accession"], fr["period"])
            row = by_key.get(key)
            if row is None:
                if fr["accession"] in accessions.get(fr["cik"], ()):
                    raise ConformanceError(
                        f"{fq} {fr['cik']}: the frame has {f['concept']} for {fr['period']} in {fr['accession']}, "
                        f"and the scope rows of that report do not")
                continue
            if Decimal(row["value"]) != Decimal(fr["value"]):
                raise ConformanceError(
                    f"{fq} {fr['cik']} {fr['accession']} {f['concept']} {fr['period']}: {row['value']} against "
                    f"{fr['value']} in the frame (the frame is the second copy); nothing is published")
            vals = dict(cik=fr["cik"], taxonomy=f["taxonomy"], concept=f["concept"], unit=f["unit"],
                        accession=fr["accession"], period=fr["period"])
            tests.append({"test": fq, "call": _call(fq, FACT_ARGS, vals),
                          "expect": {"type": "number", "value": fr["value"]}})
    if len(tests) < minimum:
        raise ConformanceError(f"{fq}: {len(tests)} source-conformance tests, at least {minimum} are needed; "
                               "nothing is published")
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
    against the frames, and write one publish-stored file and the left-out report. Nothing is written when a request
    is refused, a shape changes or a test disagrees. The state is saved after publish returns."""
    meta = names.get("meta", {})
    if meta.get("host") == PRODUCTION or meta.get("status") == "production" or "sec" not in names:
        raise NotForProduction(f"the SEC relay is rehearsal-only (Part 7, K7); host {meta.get('host')!r} is refused")
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

    lines, st = emit(items, state, names)
    # the register's six cells of sec.gov / company_facts, on the first run (register.py's emit: a later run emits
    # nothing unless the licence text changed); the class lines and descriptors are register.py's
    reg_blobs = {}
    reg_lines, reg_state = licence_register.emit(
        [names["sec_register_entry"]], {"class": True, **state.get("register", {})},
        {"register": names["sec_register"]}, licences=[names["sec_register_licence"]], blobs=reg_blobs)
    if reg_lines:
        st = dict(st)
        st["register"] = reg_state
    lines = lines + reg_lines
    rows = [r for it in items for r in it["parsed"]["rows"]]
    tests = conformance(rows, frames, names)
    tested = set(st.get("tested", []))
    new_tests = [t for t in tests if t["call"] not in tested]
    if new_tests:
        st = dict(st)
        st["tested"] = sorted(tested | {t["call"] for t in new_tests})
        lines += new_tests
    cells = sum(1 for l in lines if "cell" in l)
    common.warn(f"sec: {cells} cells, {len(new_tests)} tests written; {len(tests)} tests from the frames")
    if not lines:
        return None

    stamp = relay._stamp(now)
    path = out_dir / f"sec-{stamp}.jsonl"
    report_path = out_dir / f"sec-scope-{stamp}.json"
    report = scope_report(ranking, choice, no_facts=no_facts, budget=budget, step=step)
    common.write_jsonl(path, lines)
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
        path.unlink(missing_ok=True)
        raise
    common.warn(f"sec: left-out report written to {report_path}")
    if publish is not None:
        try:
            publish(path)
        except BaseException:
            path.unlink(missing_ok=True)
            report_path.unlink(missing_ok=True)
            raise
    common.save_state(state_path, st)
    return path


# ---------- the CLI ----------

def main(argv=None):
    p = common.base_parser()
    a = p.parse_args(argv)
    if a.host == PRODUCTION:
        common.warn(f"sec.py: {PRODUCTION} is refused: the SEC relay is rehearsal-only (Part 7, K7); use mesh-test.{PRODUCTION}")
        raise SystemExit(2)
    table = names(a.host)
    try:
        path = run(table, fetch=http_get, now=common.now_instant(), state_dir=a.state_dir, out_dir=a.out_dir)
    except SecRefused as e:
        common.warn(f"sec: stopped: {e}")
        return 1
    return common.finish(path)


if __name__ == "__main__":
    raise SystemExit(main())
