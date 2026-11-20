"""Builders and fakes for the SEC company-facts tests (Build 2 Part 7). No network: every file is a fixture under
fixtures/sec (SOURCES.md) or is built here. Names come from names.toml, never from this file.

The interface these tests pin for sec.py (CONTRACT-build2-part7-sec.md K1 to K3; decisions D34 to D36):

  sec.USER_AGENT                    "Datumesh mesh-test relay (https://datumesh.com)", no email address anywhere
  sec.MAX_REQUESTS_PER_SECOND       5
  sec.FORMS, sec.FIRST_FY, sec.LAST_FY, sec.BUDGET, sec.STEP   ("10-K", "10-K/A"), 2019, 2024, 300000, 25
  sec.CONFORMANCE_FRAMES            [(taxonomy, concept, unit, ccp), ...]; the first is the ranking frame
  sec.SecRefused(url, body, status=403)   the SEC answered 403; .status, .url, .body; str() carries the SEC's words
  sec.NotForProduction(ValueError)  the host is datumesh.com
  sec.load_concepts()               {(taxonomy, concept): reason} from sec_concepts.toml
  sec.parse_companyfacts(bytes)     {"cik", "entity", "rows", "conflicts", "labels", "filings", "filing_for"}
  sec.count_cells(bytes)            len(rows)
  sec.parse_frames(bytes)           {"taxonomy", "concept", "unit", "ccp", "rows": [{"cik", "accession", "period", "value"}]}
  sec.rank_companies(tickers, frames)   {"ranked": [...], "frame_entities", "frame_not_listed", "listed_without_assets"}
  sec.choose_n(ciks, counts, budget=, step=)   {"n", "estimated"}
  sec.scope_report(ranking, choice, *, no_facts=())   a JSON-able dict (the left-out report)
  sec.TokenBucket(rate, clock=, sleep=).wait()
  sec.http_get(url)                 bytes; one request, the declared User-Agent, no retry on 403
  sec.emit(items, state, names)     items: [{"parsed": parse_companyfacts(..), "source": {"url", "retrieved", "sha256"}}]
                                    -> (lines, new_state); function, catalog, cell lines; no tests
  sec.conformance(rows, frames_list, names, minimum=20)   -> test lines
  sec.run(names, *, fetch, now, state_dir, out_dir, publish=None, sleep=, clock=, budget=, step=) -> Path | None
  sec.main(argv)                    CLI; --host datumesh.com is refused
"""
import json

from conftest import fixture

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


def item(names, cik, data=None, parse=None):
    """An emit() item for a fixture company."""
    import sec
    data = data if data is not None else facts_file(cik)
    return {"parsed": (parse or sec.parse_companyfacts)(data), "source": source_of(cf_url(names, cik), data)}
