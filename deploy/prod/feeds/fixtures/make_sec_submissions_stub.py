"""Makes the submissions stubs of the SEC filing class tests (SEC rebuild, N2): sec/submissions_CIK<cik>.json.

THESE ARE STUBS, NOT FETCHES. The tests of the filing class are pinned to a second copy, the SEC submissions API
(https://data.sec.gov/submissions/CIK<cik>.json). The rebuild's test step may not fetch, so each stub is written here in
the shape of that API's `filings.recent` block (parallel arrays accessionNumber, filingDate, reportDate, form,
primaryDocument), from the saved companyfacts of the same company: one row for each in-scope annual report, plus an
8-K and a 10-Q that are not in scope, and one 10-K of fiscal 2018 (out of scope). The values of the in-scope rows
(form, filing date, period end) are the companyfacts values; the implementation step replaces these files by real
fetches (SOURCES.md says which are stubs) and the tests keep passing only if the feed's two copies agree.

The submissions API has no fiscal-year or fiscal-period member: the stub has none either.

usage: python -B make_sec_submissions_stub.py   (rewrites the three files under sec/)"""
import json
import sys
from pathlib import Path

FEEDS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(FEEDS))
sys.dont_write_bytecode = True

import sec  # noqa: E402

SEC_DIR = Path(__file__).resolve().parent / "sec"
CIKS = ("0000018230", "0000019617", "0000040545")
NAMES = {"0000018230": "CATERPILLAR INC", "0000019617": "JPMORGAN CHASE & CO", "0000040545": "GENERAL ELECTRIC COMPANY"}


def build(cik):
    p = sec.parse_companyfacts((SEC_DIR / f"companyfacts_CIK{cik}.json").read_bytes())
    rows = [(a, f["form"], f["filed"], f["period_end"]) for a, f in p["filings"].items()]
    n = int(cik)
    rows += [(f"{n:010d}-24-900001", "8-K", "2024-03-01", ""),
             (f"{n:010d}-24-900002", "10-Q", "2024-05-01", "2024-03-31"),
             (f"{n:010d}-19-900003", "10-K", "2019-02-20", "2018-12-31")]
    rows.sort(key=lambda r: (r[2], r[0]), reverse=True)
    recent = {"accessionNumber": [r[0] for r in rows], "filingDate": [r[2] for r in rows],
              "reportDate": [r[3] for r in rows], "form": [r[1] for r in rows],
              "primaryDocument": [f"{r[0].replace('-', '')}.htm" for r in rows]}
    doc = {"cik": cik, "entityType": "operating", "name": NAMES[cik], "fiscalYearEnd": "1231",
           "filings": {"recent": recent, "files": []}}
    return (json.dumps(doc, separators=(",", ":")) + "\n").encode("utf-8")


def main():
    for cik in CIKS:
        (SEC_DIR / f"submissions_CIK{cik}.json").write_bytes(build(cik))
        print("wrote", cik)


if __name__ == "__main__":
    main()
