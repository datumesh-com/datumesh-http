"""Builders and fakes for the SEC company-facts tests (Build 2 Part 7). No network: every file is a fixture under
fixtures/sec (SOURCES.md) or is built here. Names come from names.toml, never from this file.

The interface these tests pin for sec.py (CONTRACT-build2-part7-sec.md K1 to K3, as changed by CONTRACT-sec-rebuild.md
N1 to N5; decisions D34 to D36, D38 replaced):

  sec.USER_AGENT                    "Datumesh mesh-test relay (https://datumesh.com)", no email address anywhere
  sec.MAX_REQUESTS_PER_SECOND       5
  sec.FORMS, sec.FIRST_FY, sec.LAST_FY, sec.BUDGET, sec.STEP   ("10-K", "10-K/A"), 2019, 2024, 300000, 25
  sec.CONFORMANCE_FRAMES            [(taxonomy, concept, unit, ccp), ...]; the first is the ranking frame
  sec.FILING_CONFORMANCE_MINIMUM    20 (tests, for filing_conformance)
  sec.SecRefused(url, body, status=403)   the SEC answered 403; .status, .url, .body; str() carries the SEC's words
  sec.NotForProduction(ValueError)  the host is datumesh.com
  sec.load_concepts()               {(taxonomy, concept): reason} from sec_concepts.toml (unchanged)
  sec.relayed_unit(unit)            True for a currency of units.md, `shares`, `<CCY>/shares`, `pure`; else False
  sec.parse_companyfacts(bytes)     {"cik", "entity", "rows", "conflicts", "labels", "filings", "filing_for", "left_out_units"}
                                    a row: {"cik", "concept" (with its prefix: "us-gaap:Assets"), "unit", "accession",
                                    "period_start", "period_end" (ISO dates; an instant has start == end), "value", "form",
                                    "fy", "fp", "filed"}; no "taxonomy" and no "period"; labels stay keyed (taxonomy, name)
  sec.count_cells(bytes)            len(rows)
  sec.parse_frames(bytes)           {"taxonomy", "concept", "unit", "ccp", "rows": [{"cik", "accession", "period_start",
                                    "period_end", "value"}]}
  sec.parse_submissions(bytes)      {"cik", "filings": {accession: {"form", "filed", "period_end" (None when reportDate is "")}}}
  sec.rank_companies(tickers, frames)   {"ranked": [...], "frame_entities", "frame_not_listed", "listed_without_assets"}
  sec.choose_n(ciks, counts, budget=, step=)   {"n", "estimated"}
  sec.scope_report(ranking, choice, *, no_facts=())   a JSON-able dict (the left-out report)
  sec.TokenBucket(rate, clock=, sleep=).wait()
  sec.http_get(url)                 bytes; one request, the declared User-Agent, no retry on 403
  sec.emit(items, state, names)     items: [{"parsed": parse_companyfacts(..), "source": {"url", "retrieved", "sha256"}}]
                                    -> (lines, new_state); first run: the descriptor lines of the relay classes (company: fact; filing: six
                                    methods), their two class lines, catalogue lines, cell lines; no tests. It may also
                                    carry lines named <host>/sec/... (the implementer decides whether run splits them
                                    into the second file); the emit tests ignore those and period_end cells
  sec.conformance(rows, frames_list, names, minimum=20)   -> test lines of the relay method company/fact (from the frames API)
  sec.filing_conformance(items, submissions_list, names, minimum=sec.FILING_CONFORMANCE_MINIMUM)
                                    -> test lines of the six filing methods (from the submissions API)
  sec.annual_report_tests(items, names)   -> test lines of sec/annual_report (the earliest and the latest fiscal year of
                                    each company)
  sec.run(names, *, fetch, now, state_dir, out_dir, publish=None, sleep=, clock=, budget=, step=) -> Path | None
  sec.run(...) writes TWO files (CONTRACT-sec-classes C3): sec-<stamp>.jsonl (the relay: the two relay classes, their cells
                                    and tests; nothing named <host>/sec/...) and sec_company-<stamp>.jsonl (functions
                                    annual_report, loaded, from_label under <host>/sec/company, their cells, tests and
                                    catalogue); it returns the first path. The run report (sec-scope-<stamp>.json) has,
                                    under "left_out": "left_out_period_end" {"count", "accessions"} and
                                    "ambiguous_labels" {label: [ciks]}.
  sec.names(host), sec.sec_table(host)   mesh-test.datumesh.com and scratch.invalid; datumesh.com refused
  sec.main(argv)                    CLI; --host datumesh.com is refused

The names section `names["sec"]` (sec_names.toml) has: namespace ({host}/relay/sec.gov), company_class (company),
company_methods (["fact"]; CONTRACT-sec-classes C1: no `function` key, no company_fact),
filing_class (filing), filing_methods (the six method names, in order), licence ("public:https://www.sec.gov/about/privacy-information"), source_name,
takeover, companyfacts_url, submissions_url, tickers_url, frames_url. The keys shares_function, per_share_function,
ratio_function, filing_function and filing_for_function no longer exist.
"""
import json

import pytest

from conftest import REHEARSAL, fixture

CIK_CAT, CIK_JPM, CIK_GE = "0000018230", "0000019617", "0000040545"
CIKS = (CIK_CAT, CIK_JPM, CIK_GE)
IN_SCOPE = {CIK_CAT: 493, CIK_JPM: 395, CIK_GE: 600}  # cells of the fixtures by the K3 rule (counted from the files)
NOW = "2026-11-20T12:00:00.000000Z"
SYNTH_CELLS = 6  # in-scope cells of one synthetic company (synth_company)
RANKING_FRAME = ("us-gaap", "Assets", "USD", "CY2023Q4I")


def sec_fixture(name: str) -> bytes:
    return fixture("sec/" + name)


def facts_file(cik: str) -> bytes:
    return sec_fixture(f"companyfacts_CIK{cik}.json")


def submissions_file(cik: str) -> bytes:
    return sec_fixture(f"submissions_CIK{cik}.json")


TICKERS = "company_tickers_exchange.json"
FRAMES_ASSETS = "frames_us-gaap_Assets_USD_CY2023Q4I.json"


def rec(end, val, accn, fy, form="10-K", filed="2024-02-15", fp="FY", start=None):
    """One fact record as companyfacts has it. `val` is the raw JSON number text (kept exactly)."""
    r = {"accn": accn, "fy": fy, "fp": fp, "form": form, "filed": filed, "end": end, "val": val}
    if start is not None:
        r["start"] = start
    return r


def _rec_json(r):
    body = ",".join(f'"{k}":{json.dumps(v)}' for k, v in r.items() if k != "val")
    return "{" + body + f',"val":{r["val"]}' + "}"


def facts_json(cik: int, name: str, entries) -> bytes:
    """entries: [(taxonomy, concept, unit, label, [rec, ...])] -> a companyfacts file."""
    facts = {}
    for tax, concept, unit, label, recs in entries:
        t = facts.setdefault(tax, {})
        c = t.setdefault(concept, {"label": label, "description": f"{label} (description)", "units": {}})
        c["units"].setdefault(unit, []).extend(recs)
    parts = []
    for tax, cs in facts.items():
        cparts = []
        for concept, c in cs.items():
            units = ",".join(f"{json.dumps(u)}:[{','.join(_rec_json(r) for r in rs)}]" for u, rs in c["units"].items())
            cparts.append(f'{json.dumps(concept)}:{{"label":{json.dumps(c["label"])},'
                          f'"description":{json.dumps(c["description"])},"units":{{{units}}}}}')
        parts.append(f'{json.dumps(tax)}:{{{",".join(cparts)}}}')
    return f'{{"cik":{cik},"entityName":{json.dumps(name)},"facts":{{{",".join(parts)}}}}}'.encode("utf-8")


def mixed_company():
    """One company holding every kind of unit the one relay method company/fact relays (N1): money in three currencies, shares, per
    share in two currencies, a pure number, and three units that are not relayed (CAD/EUR, acre, USD-per-gal)."""
    a = "0000001234-24-000001"
    r = lambda v, s="2023-01-01": rec("2023-12-31", v, a, 2023, start=s)
    return facts_json(1234, "MIXED CO", [
        ("us-gaap", "Assets", "USD", "Assets", [rec("2023-12-31", "10", a, 2023)]),
        ("us-gaap", "Assets", "EUR", "Assets", [rec("2023-12-31", "9", a, 2023)]),
        ("us-gaap", "Assets", "USN", "Assets", [rec("2023-12-31", "8", a, 2023)]),
        ("us-gaap", "EarningsPerShareBasic", "EUR/shares", "EPS", [r("1.5")]),
        ("us-gaap", "EarningsPerShareDiluted", "USD/shares", "EPS d", [r("1.25")]),
        ("us-gaap", "CommonStockSharesOutstanding", "shares", "Shares", [rec("2023-12-31", "100", a, 2023)]),
        ("us-gaap", "GrossProfit", "pure", "Ratio", [r("0.5")]),
        ("us-gaap", "Liabilities", "CAD/EUR", "Odd", [rec("2023-12-31", "7", a, 2023)]),
        ("us-gaap", "Revenues", "acre", "Acres", [r("3")]),
        ("us-gaap", "NetIncomeLoss", "USD-per-gal", "Odd2", [r("4")]),
    ])


def synth_company(cik: int, name: str, accn: str, end: str, val: int) -> bytes:
    """A small companyfacts file whose latest annual `Assets` is the frame's (accn, end, val): 6 cells in scope
    (Assets, Revenues and NetIncomeLoss, each in the FY2023 report and the FY2022 report), and 3 facts out of scope
    (a 10-Q, a fiscal year 2018 report, a concept not on the list)."""
    old = f"{cik:010d}-23-000001"  # accessions are unique to a company, as the SEC numbers them
    return facts_json(cik, name, [
        ("us-gaap", "Assets", "USD", "Assets", [
            rec(end, str(val), accn, 2023, filed="2024-02-15"),
            rec(end, str(val + 1), old, 2022, filed="2023-02-15"),
            rec(end, str(val), f"{cik:010d}-24-000009", 2024, form="10-Q", fp="Q1", filed="2024-05-01"),
            rec(end, str(val), f"{cik:010d}-19-000001", 2018, filed="2019-02-15")]),
        ("us-gaap", "Revenues", "USD", "Revenues", [
            rec("2023-12-31", "1000", accn, 2023, start="2023-01-01"),
            rec("2023-12-31", "1010", old, 2022, filed="2023-02-15", start="2023-01-01")]),
        ("us-gaap", "NetIncomeLoss", "USD", "Net Income (Loss)", [
            rec("2023-12-31", "100", accn, 2023, start="2023-01-01"),
            rec("2023-12-31", "101", old, 2022, filed="2023-02-15", start="2023-01-01")]),
        ("us-gaap", "DeferredTaxAssetsGross", "USD", "Deferred Tax Assets, Gross", [
            rec("2023-12-31", "5", accn, 2023)]),
    ])


RESTATE_CIK = "0000000777"
RESTATE_NAME = "RESTATE CO"
RESTATE_ACCN = {"A": "0000000777-21-000001", "E": "0000000777-21-000005", "B": "0000000777-22-000002",
                "C": "0000000777-23-000003", "D": "0000000777-24-000004"}


def restated_company() -> bytes:
    """A company whose fiscal 2020 figures are restated by later filings, one of them three fiscal years later, with a
    filing in between that does not report them (the case the old sec/latest, which looked two filings ahead, got wrong):

      A  10-K    fy 2020 filed 2021-02-26   Assets 2020-12-31 = 100   Revenues 2020 = 50
      E  10-K/A  fy 2020 filed 2021-06-01   Assets 2020-12-31 = 110   Revenues 2020 = 55   (accession lower than B's)
      B  10-K    fy 2021 filed 2022-02-25   Assets 2020-12-31 = 120   Assets 2021-12-31 = 150
      C  10-K    fy 2022 filed 2023-02-24   Assets 2022-12-31 = 200   (reports nothing of 2020)
      D  10-K    fy 2023 filed 2024-02-23   Assets 2020-12-31 = 130   Revenues 2020 = 70   Revenues 2023 = 80

    annual_report(fiscal 2020) is E (the latest filed of fiscal 2020); the latest figure of 2020 is D's."""
    a = RESTATE_ACCN
    d = lambda end, v, k, fy, form="10-K", filed="", start=None: rec(end, v, a[k], fy, form=form, filed=filed, start=start)
    return facts_json(777, RESTATE_NAME, [
        ("us-gaap", "Assets", "USD", "Assets", [
            d("2020-12-31", "100", "A", 2020, filed="2021-02-26"),
            d("2020-12-31", "110", "E", 2020, form="10-K/A", filed="2021-06-01"),
            d("2020-12-31", "120", "B", 2021, filed="2022-02-25"),
            d("2021-12-31", "150", "B", 2021, filed="2022-02-25"),
            d("2022-12-31", "200", "C", 2022, filed="2023-02-24"),
            d("2020-12-31", "130", "D", 2023, filed="2024-02-23")]),
        ("us-gaap", "Revenues", "USD", "Revenues", [
            d("2020-12-31", "50", "A", 2020, filed="2021-02-26", start="2020-01-01"),
            d("2020-12-31", "55", "E", 2020, form="10-K/A", filed="2021-06-01", start="2020-01-01"),
            d("2020-12-31", "70", "D", 2023, filed="2024-02-23", start="2020-01-01"),
            d("2023-12-31", "80", "D", 2023, filed="2024-02-23", start="2023-01-01")]),
    ])


def synth_submissions(cik: int, accn: str) -> bytes:
    """What the submissions API would say of a synth_company: its fiscal 2023 report (`accn`) and its fiscal 2022 report,
    both for the year ended 2023-12-31 as synth_company's statements say (the synthetic data is as the first copy has
    it), and an 8-K that is not an annual report."""
    old = f"{cik:010d}-23-000001"
    rows = [(accn, "10-K", "2024-02-15", "2023-12-31"), (old, "10-K", "2023-02-15", "2023-12-31"),
            (f"{cik:010d}-24-900001", "8-K", "2024-03-01", "")]
    recent = {"accessionNumber": [r[0] for r in rows], "filingDate": [r[2] for r in rows],
              "reportDate": [r[3] for r in rows], "form": [r[1] for r in rows]}
    return json.dumps({"cik": f"{cik:010d}", "name": f"CO {cik}", "filings": {"recent": recent, "files": []}}).encode("utf-8")


def frames_json(entries, concept="Assets", unit="USD", ccp="CY2023Q4I", taxonomy="us-gaap") -> bytes:
    """entries: [(cik:int, name, accn, end, val:int)] -> a frames file as the SEC serves it."""
    data = [{"accn": a, "cik": c, "entityName": n, "loc": "US-NY", "end": e, "val": v} for c, n, a, e, v in entries]
    return json.dumps({"taxonomy": taxonomy, "tag": concept, "ccp": ccp, "uom": unit, "label": concept,
                       "description": concept, "pts": len(data), "data": data}).encode("utf-8")


class FakeClock:
    """A clock that only moves when something sleeps."""

    def __init__(self):
        self.now = 1000.0

    def clock(self):
        return self.now

    def sleep(self, seconds):
        assert seconds >= 0
        self.now += seconds


def max_in_any_second(times):
    """The most requests that fall in any half-open window [t, t + 1) starting at a request."""
    times = sorted(times)
    return max((sum(1 for u in times if t <= u < t + 1.0) for t in times), default=0)


def read_lines(path):
    return [json.loads(l) for l in open(path, encoding="utf-8").read().splitlines()]


def dirs(tmp_path):
    s, o = tmp_path / "state", tmp_path / "out"
    s.mkdir(), o.mkdir()
    return s, o


def source_of(url, data, retrieved=NOW):
    import hashlib
    return {"url": url, "retrieved": retrieved, "sha256": hashlib.sha256(data).hexdigest()}


def cf_url(names, cik):
    return names["sec"]["companyfacts_url"].format(cik=cik)


def sub_url(names, cik):
    return names["sec"]["submissions_url"].format(cik=cik)


FILING_METHODS = ["cik", "form", "filed", "fiscal_year", "fiscal_period", "period_end"]
FILING_RETURNS = {"cik": "string", "form": "string", "filed": "date", "fiscal_year": {"type": "number", "unit": "1"},
                  "fiscal_period": "string", "period_end": "date"}
FACT_ORDER = ["cik", "concept", "unit", "accession", "period_start", "period_end"]


def fact_fn(names):
    """The relay class's stored method `fact` (CONTRACT-sec-classes C1): <host>/relay/sec.gov/company/fact."""
    s = names["sec"]
    return f"{s['namespace']}/{s['company_class']}/fact"


def company_cls(names):
    """The relay class keyed by cik: <host>/relay/sec.gov/company."""
    s = names["sec"]
    return f"{s['namespace']}/{s['company_class']}"


def host_name(names):
    return names["meta"]["host"]


def sec_company(names):
    """Our subclass of the relay's company class (CONTRACT-sec-classes C3): <host>/sec/company."""
    return f"{host_name(names)}/sec/company"


def filing_cls(names):
    s = names["sec"]
    return f"{s['namespace']}/{s['filing_class']}"


def annual_fn(names):
    """sec/annual_report moved under the subclass (C3): <host>/sec/company/annual_report."""
    return f"{sec_company(names)}/annual_report"


def kinds(lines):
    """{"function", "descriptor", "class", "catalog", "cell", "test": [lines]}: every line is exactly one of these (a class
    line has a `descriptor` member too: it is a class line)."""
    out = {k: [] for k in ("function", "descriptor", "class", "catalog", "cell", "test")}
    for l in lines:
        assert isinstance(l, dict) and "\n" not in json.dumps(l, ensure_ascii=False)
        k = "class" if "class" in l else next((k for k in out if k in l), None)
        assert k is not None, list(l)
        out[k].append(l)
    return out


def check_order_all(lines):
    """Function, descriptor and class lines first (a method's descriptor before the class line that names its hash), then
    catalogue lines, then cells, then tests."""
    rank = {"function": 0, "class": 0, "descriptor": 0, "catalog": 1, "cell": 2, "test": 3}
    ranks = [rank["class" if "class" in l else next(k for k in rank if k in l)] for l in lines]
    assert ranks == sorted(ranks), ranks
    cls = [i for i, l in enumerate(lines) if "class" in l]
    desc = [i for i, l in enumerate(lines) if "descriptor" in l and "class" not in l]
    assert not cls or not desc or max(desc) < min(cls)


def args_of(cell):
    return {k: v["value"] for k, v in cell["cell"]["args"].items()}


def item(names, cik, data=None, parse=None):
    """An emit() item for a fixture company. A fixture company's submissions are attached as run attaches them (C2, C2b):
    filed and period_end come from them, so a filing they do not list has no filed cell."""
    import sec
    data = data if data is not None else facts_file(cik)
    it = {"parsed": (parse or sec.parse_companyfacts)(data), "source": source_of(cf_url(names, cik), data)}
    if cik in CIKS:
        sdata = submissions_file(cik)
        sub = sec.parse_submissions(sdata)
        src = source_of(sub_url(names, cik), sdata)
        it["submissions"] = {"cik": sub["cik"], "filings": sub["filings"], "sources": {a: src for a in sub["filings"]}}
    return it


@pytest.fixture(scope="session")
def names():
    """The SEC names: the rehearsal table with the SEC section from sec_names.toml (sec.names). The SEC modules of the
    tests import this fixture, so no test reads the sec section from common.names (which has none, K7)."""
    import sec
    return sec.names(REHEARSAL)


def line_name(l):
    """The name a line is about (None for a descriptor line, which has none)."""
    if "cell" in l:
        return l["cell"]["fn"]
    return l.get("function") or l.get("class") or l.get("catalog") or l.get("test")


def relay_core(lines, names):
    """What the emit tests pin of emit's lines: those of the relay namespace (nothing named <host>/sec/...), without the
    period_end cells of the filing class (their value comes from the submissions: pinned by the run tests)."""
    own = host_name(names) + "/sec/"
    return [l for l in lines if not (line_name(l) or "").startswith(own)
            and not ("cell" in l and l["cell"]["fn"].endswith("/period_end"))]
