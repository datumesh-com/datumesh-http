"""Makes the SEC relay fixtures the http crate's tests publish (Build 2 Part 7, D38) from sec.py itself, so the stub
relay and the fetcher's output cannot diverge again:

  sec_views_stub_relay.jsonl   sec.emit over the saved companyfacts of Caterpillar and JPMorgan Chase (fixtures/sec) and
                               over the synthetic company of sec_helpers.restated_company (cik 0000000777, fiscal 2020
                               figures restated by three later filings: the case sec/latest must get exactly), with the
                               source-conformance tests of the saved Assets frame: the relay the views read
  sec_relay_forms.jsonl        sec.emit over one synthetic company that holds every kind of unit (money in three
                               currencies, shares, per share in two currencies, a pure number) and three units that are
                               not relayed; the one company_fact function, the filing class and sec/annual_report

usage: python -B make_sec_stub.py            (rewrites both files next to this script)
tests/test_sec_views.py checks that the files on disk are exactly what build_*() gives now."""
import hashlib
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


def build_views_stub():
    names = sec.names(HOST)
    ciks = (sec_helpers.CIK_CAT, sec_helpers.CIK_JPM)
    datas = {c: (FIXTURES / "sec" / f"companyfacts_CIK{c}.json").read_bytes() for c in ciks}
    items = [_item(names, c, datas[c]) for c in ciks]
    items.append(_item(names, sec_helpers.RESTATE_CIK, sec_helpers.restated_company()))
    lines, _ = sec.emit(items, {}, names)
    frame = sec.parse_frames((FIXTURES / "sec" / "frames_us-gaap_Assets_USD_CY2023Q4I.json").read_bytes())
    rows = [r for it in items for r in it["parsed"]["rows"]]
    return [common.dumps_line(l) for l in _stage_filed(lines) + sec.conformance(rows, [frame], names, minimum=1)]


def build_forms():
    names = sec.names(HOST)
    lines, _ = sec.emit([_item(names, "0000001234", sec_helpers.mixed_company())], {}, names)
    return [common.dumps_line(l) for l in lines]


def main():
    for name, build in (("sec_views_stub_relay.jsonl", build_views_stub), ("sec_relay_forms.jsonl", build_forms)):
        (FIXTURES / name).write_text("".join(l + "\n" for l in build()), encoding="utf-8", newline="\n")
        print("wrote", name)


if __name__ == "__main__":
    main()
