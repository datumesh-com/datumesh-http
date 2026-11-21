"""A synthetic SEC for the class tests (CONTRACT-sec-classes C2 and C3): a world of companies (a tickers file, an Assets
frame, a companyfacts file and a submissions file for each, and submissions pages for the ones whose older filings are not
in `filings.recent`), and a fetcher that serves it. Every file is built here; nothing is fetched.

The world has more than 20 companies so that the frames' conformance minimum (20 tests) is met; the order of WORLD is the
order of the ranking (assets fall down the list), so the first three are the first three loaded.
"""
import hashlib
import json
import re
import unicodedata
from dataclasses import dataclass, field

import common
import sec
from sec_helpers import (FakeClock, dirs, facts_json, frames_json, rec, submissions_file, synth_company, synth_submissions)

NOW = "2026-11-20T12:00:00.000000Z"


@dataclass(eq=False)
class Co:
    cik: int
    title: str
    tickers: list
    exchange: str = "Nasdaq"
    facts: bytes | None = None        # replaces synth_company
    subs: bytes | None = None         # replaces synth_submissions
    note: str = ""

    @property
    def cik10(self):
        return f"{self.cik:010d}"

    @property
    def accn(self):                   # the fiscal 2023 report: the frame's accession
        return f"{self.cik10}-24-000001"

    @property
    def old(self):                    # the fiscal 2022 report
        return f"{self.cik10}-23-000001"


# ---------- Athene (the C2 regression) ----------

ATHENE_CIK = "0001527469"
ATHENE_10K, ATHENE_10KA = "0001527469-21-000025", "0001527469-21-000032"


def athene_facts() -> bytes:
    """Athene Holding's FY2020: the 10-K (filed 2021-02-19) and the 10-K/A (filed 2021-04-20), both for the year ended
    2020-12-31. The 10-K/A also carries a one-year period that ends 2021-03-31 (a transition-quarter style restatement), which
    is what the old derivation took as the amendment's period end."""
    return facts_json(1527469, "Athene Holding Ltd", [
        ("us-gaap", "Assets", "USD", "Assets", [
            rec("2020-12-31", "300000000000", ATHENE_10K, 2020, filed="2021-02-19"),
            rec("2020-12-31", "300000000000", ATHENE_10KA, 2020, form="10-K/A", filed="2021-04-20")]),
        ("us-gaap", "Revenues", "USD", "Revenues", [
            rec("2020-12-31", "9000000000", ATHENE_10K, 2020, filed="2021-02-19", start="2020-01-01"),
            rec("2020-12-31", "9000000000", ATHENE_10KA, 2020, form="10-K/A", filed="2021-04-20", start="2020-01-01"),
            rec("2021-03-31", "9500000000", ATHENE_10KA, 2020, form="10-K/A", filed="2021-04-20", start="2020-04-01")]),
    ])


def athene_submissions() -> bytes:
    return subs_json("0001527469", [(ATHENE_10KA, "10-K/A", "2021-04-20", "2020-12-31"),
                                    (ATHENE_10K, "10-K", "2021-02-19", "2020-12-31"),
                                    ("0001527469-21-000050", "8-K", "2021-05-05", "")])


def subs_json(cik10, rows, files=()) -> bytes:
    """A submissions file: rows (accession, form, filed, reportDate) as the parallel arrays of filings.recent, and `files`
    the names of the pages that hold older filings."""
    recent = {"accessionNumber": [r[0] for r in rows], "filingDate": [r[2] for r in rows],
              "reportDate": [r[3] for r in rows], "form": [r[1] for r in rows]}
    return json.dumps({"cik": cik10, "name": f"CO {cik10}",
                       "filings": {"recent": recent, "files": [{"name": n, "filingCount": 1, "filingFrom": "2000-01-01",
                                                                 "filingTo": "2020-01-01"} for n in files]}}).encode()


def page_json(rows) -> bytes:
    """A page of filings.files: the same parallel arrays as `recent`, at the top of the file."""
    return json.dumps({"accessionNumber": [r[0] for r in rows], "filingDate": [r[2] for r in rows],
                       "reportDate": [r[3] for r in rows], "form": [r[1] for r in rows]}).encode()


def page_name(cik10, n):
    return f"CIK{cik10}-submissions-{n:03d}.json"


PAGE_URL = "https://data.sec.gov/submissions/{name}"


def _two(co, new_report="2023-12-31", old_report="2023-12-31"):
    """The two in-scope filings of a synthetic company, as synth_submissions has them (dates of its facts)."""
    return [(co.accn, "10-K", "2024-02-15", new_report), (co.old, "10-K", "2023-02-15", old_report)]


def _page_world():
    """Companies for the C2 paging and missing-value rules. Each has two in-scope filings: co.accn and co.old."""
    out = []
    # P1: the older filing is not in recent; the first page has it: that page is fetched, the second is not
    c = Co(1000050, "Pageone Corp", ["PGONE"], note="old accession on page 1")
    c.subs = subs_json(c.cik10, _two(c)[:1], files=[page_name(c.cik10, 1), page_name(c.cik10, 2)])
    out.append(c)
    # P2: not in recent, not on page 1, on page 2: both pages are fetched, in order
    c = Co(1000051, "Pagetwo Corp", ["PGTWO"], note="old accession on page 2")
    c.subs = subs_json(c.cik10, _two(c)[:1], files=[page_name(c.cik10, 1), page_name(c.cik10, 2)])
    out.append(c)
    # P3: not anywhere: both pages are fetched, no period_end cell, counted as left out
    c = Co(1000052, "Pagethree Corp", ["PGTHR"], note="old accession nowhere")
    c.subs = subs_json(c.cik10, _two(c)[:1], files=[page_name(c.cik10, 1), page_name(c.cik10, 2)])
    out.append(c)
    # P4: everything is in recent: the page it lists is never fetched
    c = Co(1000053, "Pagefour Corp", ["PGFOR"], note="all in recent, one page listed")
    c.subs = subs_json(c.cik10, _two(c), files=[page_name(c.cik10, 1)])
    out.append(c)
    # E1: an empty reportDate for the new filing: no cell, counted
    c = Co(1000054, "Emptydate Corp", ["EMPTY"], note="empty reportDate")
    c.subs = subs_json(c.cik10, [(c.accn, "10-K", "2024-02-15", ""), (c.old, "10-K", "2023-02-15", "2023-12-31")])
    out.append(c)
    # D1: the submissions' report date differs from the period of the companyfacts: relayed as filed, no error
    c = Co(1000055, "Differing Corp", ["DIFFR"], note="reportDate differs from companyfacts")
    c.subs = subs_json(c.cik10, _two(c, new_report="2023-12-30"))
    out.append(c)
    return out


PAGE_WORLD = _page_world()
P1, P2, P3, P4, E1, D1 = PAGE_WORLD
PAGES = {
    P1: {1: [(P1.old, "10-K", "2023-02-15", "2022-12-31")], 2: [("0001000050-19-000001", "10-K", "2019-02-15", "2018-12-31")]},
    P2: {1: [("0001000051-19-000001", "10-K", "2019-02-15", "2018-12-31")], 2: [(P2.old, "10-K", "2023-02-15", "2022-12-31")]},
    P3: {1: [("0001000052-19-000001", "10-K", "2019-02-15", "2018-12-31")], 2: [("0001000052-18-000001", "10-K", "2018-02-15", "2017-12-31")]},
    P4: {1: [(P4.old, "10-K", "2023-02-15", "2022-12-31")]},
}

# name -> (title, tickers) worlds of the label rules. Short names expected per CONTRACT-sec-classes C3 (c).
# The order is the order of the ranking.
LABEL_CASES = [
    # (cik, title, tickers, expected short name or None)
    (320193, "Apple Inc.", ["AAPL"], "Apple"),
    (1045810, "NVIDIA CORP", ["NVDA"], "NVIDIA"),
    (19617, "JPMORGAN CHASE & CO", ["JPM", "JPM-PC"], "JPMORGAN CHASE"),
    (59478, "ELI LILLY & Co", ["LLY"], "ELI LILLY"),
    (1326801, "Meta Platforms, Inc.", ["META"], "Meta Platforms"),
    (1067983, "BERKSHIRE HATHAWAY INC", ["BRK-B", "BRK-A"], "BERKSHIRE HATHAWAY"),
    (1000001, "ACME CORP /DE/", ["ACME"], "ACME"),
    (1000002, "Foo Co Inc", ["FOO"], "Foo"),
    (1000003, "Bar Corp Ltd.", ["BAR"], "Bar"),
    (1000004, "Incredible Corp", ["INCR"], "Incredible"),
    (1000005, "Tesco", ["TSCO"], None),
    (1000006, "Corp.", ["CRPX"], None),
    (1000007, "Zeta, Inc.", ["ZETA"], "Zeta"),
    (1000008, "Shell plc", ["SHEL"], "Shell"),
    (1000009, "Unilever PLC", ["UL"], "Unilever"),
    (1000010, "Philips N.V.", ["PHG"], "Philips"),
    (1000011, "SAP SE", ["SAP"], "SAP"),
    (1000012, "Siemens AG", ["SIEGY"], "Siemens"),
    (1000013, "Enbridge Energy Partners LP", ["EEP"], "Enbridge Energy Partners"),
    (1000014, "Blue Chip L.P.", ["BLUE"], "Blue Chip"),
    (1000015, "Quark LLC", ["QRK"], "Quark"),
    (1000016, "Danone S.A.", ["DANOY"], "Danone"),
    (1000017, "ZENITH INCORPORATED", ["ZNTH"], "ZENITH"),
    (1000018, "Alcoa Corporation", ["AA"], "Alcoa"),
    (1000019, "Sears Holdings Co.", ["SHLD"], "Sears Holdings"),
    (1000020, "Solo SA", ["SOLO"], "Solo"),
    (1000021, "Whole NV", ["WHOL"], "Whole"),
    (1000022, "Mixed Case Limited", ["MIXD"], "Mixed Case"),
    (1000030, "Café Corp", ["CAFE"], None),  # NFD in the file: only NFC labels may come out
]
# ambiguity: (cik, title, tickers). "Delta" is the short name of two companies; "Orion" is one title and one short name;
# "SUN" is one ticker and one short name; "Sun" (another case) is only a short name: no folding
AMBIGUOUS_CASES = [
    (1000040, "Delta Corp", ["DLTA"]),
    (1000041, "Delta Inc", ["DLTB"]),
    (1000042, "Orion", ["ORN"]),
    (1000043, "Orion Inc", ["ORNI"]),
    (1000044, "Moon Holdings", ["SUN"]),
    (1000045, "SUN Inc", ["SUNI"]),
    (1000046, "Sun Corp", ["SNCP"]),
]
AMBIGUOUS = {"Delta": ["0001000040", "0001000041"], "Orion": ["0001000042", "0001000043"],
             "SUN": ["0001000044", "0001000045"]}


def build_world():
    cos = [Co(c, t, tk, "NYSE" if i % 2 else "Nasdaq") for i, (c, t, tk, _) in enumerate(LABEL_CASES)]
    cos += [Co(c, t, tk) for c, t, tk in AMBIGUOUS_CASES]
    cos += PAGE_WORLD
    cos.append(Co(1527469, "Athene Holding Ltd", ["ATH"], "NYSE", facts=athene_facts(), subs=athene_submissions()))
    return cos


WORLD = build_world()
ATHENE = WORLD[-1]
FIRST_THREE = [c.cik10 for c in WORLD[:3]]


def assets_of(i):
    return 900_000_000_000 - i * 1_000_000


class World:
    """The files of WORLD. Athene's frame accession is its 10-K (in scope); the others' is co.accn."""

    def __init__(self, cos=WORLD, pages=PAGES, sub_overrides=None):
        self.cos, self.pages, self.sub_overrides = cos, pages, sub_overrides or {}
        self.by_cik = {c.cik10: c for c in cos}
        self.tickers = json.dumps({"fields": ["cik", "name", "ticker", "exchange"],
                                   "data": [[c.cik, c.title, t, c.exchange] for c in cos for t in c.tickers]},
                                  ensure_ascii=False).encode("utf-8")
        entries = []
        for i, c in enumerate(cos):
            accn, end = (ATHENE_10K, "2020-12-31") if c is ATHENE else (c.accn, "2023-12-31")
            entries.append((c.cik, c.title, accn, end, 300_000_000_000 if c is ATHENE else assets_of(i)))
        self.frame = frames_json(entries)

    def facts(self, co):
        if co.facts is not None:
            return co.facts
        i = self.cos.index(co)
        return synth_company(co.cik, co.title, co.accn, "2023-12-31", assets_of(i))

    def submissions(self, co):
        if co.cik10 in self.sub_overrides:
            return self.sub_overrides[co.cik10]
        return co.subs if co.subs is not None else synth_submissions(co.cik, co.accn)

    def page(self, co, n):
        return page_json(self.pages[co][n])


class Fetcher:
    """What data.sec.gov and www.sec.gov would send for a World. Records urls and times."""

    def __init__(self, names, world=None, clock=None, refuse_at=None):
        self.names, self.w, self.clock, self.refuse_at = names, world or World(), clock, refuse_at
        self.urls, self.times = [], []
        s = names["sec"]
        self.cf = re.compile(re.escape(s["companyfacts_url"]).replace(r"\{cik\}", r"(\d{10})"))
        self.sub = re.compile(re.escape(s["submissions_url"]).replace(r"\{cik\}", r"(\d{10})"))
        self.page_re = re.compile(r"https://data\.sec\.gov/submissions/(CIK(\d{10})-submissions-(\d{3})\.json)")
        self.served = {}   # url -> bytes

    def __call__(self, url):
        self.urls.append(url)
        if self.clock:
            self.times.append(self.clock())
        if self.refuse_at is not None and len(self.urls) == self.refuse_at:
            raise sec.SecRefused(url, "automated tools outside of the acceptable policy", 403)
        s = self.names["sec"]
        if url == s["tickers_url"]:
            data = self.w.tickers
        elif url == s["frames_url"].format(taxonomy="us-gaap", concept="Assets", unit="USD", ccp="CY2023Q4I"):
            data = self.w.frame
        elif self.cf.fullmatch(url):
            data = self.w.facts(self.w.by_cik[self.cf.fullmatch(url).group(1)])
        elif self.sub.fullmatch(url):
            data = self.w.submissions(self.w.by_cik[self.sub.fullmatch(url).group(1)])
        elif self.page_re.fullmatch(url):
            m = self.page_re.fullmatch(url)
            data = self.w.page(self.w.by_cik[m.group(2)], int(m.group(3)))
        elif "/frames/" in url:
            raise common.NotPublished(url)
        else:
            raise AssertionError(f"unexpected url {url}")
        self.served[url] = data
        return data

    def pages_fetched(self, co):
        return [u for u in self.urls if self.page_re.fullmatch(u) and f"CIK{co.cik10}-" in u]


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run_world(names, tmp_path, world=None, fc=None, **kw):
    """sec.run over a World with a FakeClock; returns (relay path, fetcher, state dir, out dir)."""
    fc = fc or FakeClock()
    f = Fetcher(names, world, fc.clock)
    s, o = dirs(tmp_path)
    path = sec.run(names, fetch=f, now=NOW, state_dir=s, out_dir=o, sleep=fc.sleep, clock=fc.clock, **kw)
    return path, f, s, o


def company_file(o, now=NOW):
    import relay
    return o / f"sec_company-{relay._stamp(now)}.jsonl"


def key_encoding(names, cik10):
    return f'{names["meta"]["host"]}/sec/company(cik:"{cik10}")'


def is_nfc(s):
    return unicodedata.is_normalized("NFC", s)
