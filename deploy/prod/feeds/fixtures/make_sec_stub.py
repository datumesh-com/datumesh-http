"""Makes the SEC relay fixtures the http crate's tests publish (Build 2 Part 7, D38) from sec.py itself, so the stub
relay and the fetcher's output cannot diverge again:

  sec_views_stub_relay.jsonl   sec.emit over the saved companyfacts of Caterpillar and JPMorgan Chase (fixtures/sec),
                               with the source-conformance tests of the saved Assets frame: the relay the views read
  sec_relay_forms.jsonl        sec.emit over one synthetic company that holds every unit form (money in three
                               currencies, shares, per share in two currencies, a pure number) and three units that are
                               not relayed; the descriptors of all four fact functions

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
NOW = sec_helpers.NOW
FIXTURES = Path(__file__).resolve().parent


def _item(names, cik, data):
    url = names["sec"]["companyfacts_url"].format(cik=cik)
    return {"parsed": sec.parse_companyfacts(data),
            "source": {"url": url, "retrieved": NOW, "sha256": hashlib.sha256(data).hexdigest()}}


def build_views_stub():
    names = sec.names(HOST)
    ciks = (sec_helpers.CIK_CAT, sec_helpers.CIK_JPM)
    datas = {c: (FIXTURES / "sec" / f"companyfacts_CIK{c}.json").read_bytes() for c in ciks}
    items = [_item(names, c, datas[c]) for c in ciks]
    lines, _ = sec.emit(items, {}, names)
    frame = sec.parse_frames((FIXTURES / "sec" / "frames_us-gaap_Assets_USD_CY2023Q4I.json").read_bytes())
    rows = [r for it in items for r in it["parsed"]["rows"]]
    return [common.dumps_line(l) for l in lines + sec.conformance(rows, [frame], names, minimum=1)]


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
