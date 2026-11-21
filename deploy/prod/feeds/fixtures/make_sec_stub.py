"""Makes the SEC relay fixtures the http crate's tests publish (Build 2 Part 7, D38) from sec.py itself, so the stub
relay and the fetcher's output cannot diverge again:

  sec_views_stub_relay.jsonl   sec.emit and sec.company_emit over the saved companyfacts and submissions of Caterpillar and
                               JPMorgan Chase (fixtures/sec), and over the synthetic company of sec_helpers.restated_company
                               (cik 0000000777, fiscal 2020 figures restated by three later filings: the case sec/latest
                               must get exactly), with the source-conformance tests of the saved Assets frame: the relay the
                               views read (the relay company class and its fact, the filing class, sec/company's functions)
  sec_relay_forms.jsonl        sec.emit over one synthetic company that holds every kind of unit (money in three
                               currencies, shares, per share in two currencies, a pure number) and three units that are
                               not relayed; the company class with its method fact, and the filing class

usage: python -B make_sec_stub.py            (rewrites both files next to this script)
tests/test_sec_views.py checks that the files on disk are exactly what build_*() gives now."""
import hashlib
import json
import sys
from pathlib import Path

FEEDS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(FEEDS))
sys.path.insert(0, str(FEEDS / "tests"))
sys.dont_write_bytecode = True

import common  # noqa: E402
import sec  # noqa: E402
import sec_helpers  # noqa: E402

HOST = "mesh-test.datumesh.com"
# retrieved before the http tests' fixed clock (2026-11-15), as the store requires retrieved <= issued
NOW = "2026-11-10T12:00:00.000000Z"
FIXTURES = Path(__file__).resolve().parent


def _item(names, cik, data):
    url = names["sec"]["companyfacts_url"].format(cik=cik)
    return {"parsed": sec.parse_companyfacts(data),
            "source": {"url": url, "retrieved": NOW, "sha256": hashlib.sha256(data).hexdigest()}}


# The staged fixture (sec/latest's empty case): the filing class's filed() cells are published at a later tx than the
# other cells, so an asof between the two has annual_report and period_end resolved while filed(accession: *) is empty.
FILED_TX = "2026-11-16T12:00:00.000000Z"


def _stage_filed(lines):
    out = []
    for l in lines:
        if "cell" in l and l["cell"]["fn"].endswith("/filing/filed"):
            l = dict(l, tx=FILED_TX, asof=FILED_TX)
        out.append(l)
    return out


def _with_submissions(names, it, data):
    """Attach the item's submissions the way sec.run does (C2): the in-scope filings of its submissions file, each with the
    source it was read from. The fixtures' files list every filing in recent, so no page is read here."""
    cik = it["parsed"]["cik"]
    url = names["sec"]["submissions_url"].format(cik=cik)
    parsed = sec.parse_submissions(data)
    found = {a: parsed["filings"][a] for a in it["parsed"]["filings"] if a in parsed["filings"]}
    src = {"url": url, "retrieved": NOW, "sha256": hashlib.sha256(data).hexdigest()}
    it["submissions"] = {"cik": parsed["cik"], "filings": found, "sources": {a: src for a in found}}
    return it


def restated_submissions() -> bytes:
    """The submissions file of the restated company (sec_helpers.restated_company): its five filings, report dates as the
    annual statements end (fiscal 2020 is 2020-12-31 in A, E and B's own year; B's fiscal 2021 is 2021-12-31)."""
    a = sec_helpers.RESTATE_ACCN
    rows = [(a["A"], "10-K", "2021-02-26", "2020-12-31"), (a["E"], "10-K/A", "2021-06-01", "2020-12-31"),
            (a["B"], "10-K", "2022-02-25", "2021-12-31"), (a["C"], "10-K", "2023-02-24", "2022-12-31"),
            (a["D"], "10-K", "2024-02-23", "2023-12-31")]
    recent = {"accessionNumber": [r[0] for r in rows], "filingDate": [r[2] for r in rows],
              "reportDate": [r[3] for r in rows], "form": [r[1] for r in rows]}
    return json.dumps({"cik": sec_helpers.RESTATE_CIK, "name": sec_helpers.RESTATE_NAME,
                       "filings": {"recent": recent, "files": []}}).encode("utf-8")


def build_views_stub():
    """The relay the views read: the relay's lines (classes, descriptors, catalogue, cells with period_end from the
    submissions), sec/company's functions, catalogue and cells, and the relay's source-conformance tests of the saved
    Assets frame. Functions and descriptors come first, then catalogue, cells and tests (check_order_all)."""
    names = sec.names(HOST)
    ciks = (sec_helpers.CIK_CAT, sec_helpers.CIK_JPM)
    datas = {c: (FIXTURES / "sec" / f"companyfacts_CIK{c}.json").read_bytes() for c in ciks}
    items = [_with_submissions(names, _item(names, c, datas[c]),
                               (FIXTURES / "sec" / f"submissions_CIK{c}.json").read_bytes()) for c in ciks]
    items.append(_with_submissions(names, _item(names, sec_helpers.RESTATE_CIK, sec_helpers.restated_company()),
                                   restated_submissions()))
    lines, _ = sec.emit(items, {}, names)
    tickers = (FIXTURES / "sec" / "company_tickers_exchange.json").read_bytes()
    pairs, _ = sec.company_labels(tickers, [it["parsed"]["cik"] for it in items])
    clines, _ = sec.company_emit(items, pairs, {}, names)
    frame = sec.parse_frames((FIXTURES / "sec" / "frames_us-gaap_Assets_USD_CY2023Q4I.json").read_bytes())
    rows = [r for it in items for r in it["parsed"]["rows"]]
    tests = sec.conformance(rows, [frame], names, minimum=1)
    everything = _stage_filed(lines) + clines + tests
    rank = lambda l: 0 if ("function" in l or "descriptor" in l or "class" in l) else 1 if "catalog" in l else \
        2 if "cell" in l else 3
    everything.sort(key=rank)  # stable: the order within each kind is the order of emission
    return [common.dumps_line(l) for l in everything]


def build_forms():
    """The relay's lines for one company holding every kind of unit (no submissions: no period_end cell)."""
    names = sec.names(HOST)
    lines, _ = sec.emit([_item(names, "0000001234", sec_helpers.mixed_company())], {}, names)
    return [common.dumps_line(l) for l in lines]


def main():
    for name, build in (("sec_views_stub_relay.jsonl", build_views_stub), ("sec_relay_forms.jsonl", build_forms)):
        (FIXTURES / name).write_text("".join(l + "\n" for l in build()), encoding="utf-8", newline="\n")
        print("wrote", name)


if __name__ == "__main__":
    main()
