# SPDX-License-Identifier: Apache-2.0
"""The restore drill's comparison (restore_drill.sh): the live mesh and the mesh on the restored
dump must give the same receipts, blobs, bindings and key document.

    python -I drill_check.py <live base URL> <drill base URL>

Compared: the key document; the bindings of the scripture names (without the answer's own
`asof`, which is each mesh's now); for a sample of cells (the first and last verse of each corpus
in each edition, every edition cell, and 40 verses spread across the Tanakh), the receipt in the
envelope byte for byte after canonical JSON, and the blob it names. Prints one JSON object.
"""
import json
import sys
import urllib.error
import urllib.parse
import urllib.request

live, drill = sys.argv[1].rstrip("/"), sys.argv[2].rstrip("/")
H = "datumesh.com/scripture"


def get(base, path):
    try:
        with urllib.request.urlopen(base + path, timeout=60) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def canon(b):
    try:
        return json.dumps(json.loads(b), sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    except ValueError:
        return None


cells = []
for corpus, book, ch, v, eds in (("tanakh", "Gen", 1, 1, ["uxlc"]), ("tanakh", "2Chr", 36, 23, ["uxlc"]), ("tanakh", "Neh", 13, 31, ["uxlc"]),
                                 ("nt", "Matt", 1, 1, ["sblgnt"]), ("nt", "Rev", 22, 21, ["sblgnt"]),
                                 ("quran", "Quran", 1, 1, ["uthmani", "simple"]), ("quran", "Quran", 114, 6, ["uthmani", "simple"])):
    for e in eds:
        cells.append(f'{H}/verse/text(corpus:"{corpus}",book:"{book}",chapter:{ch},verse:{v},edition:"{e}")')
for ch in range(1, 41):
    cells.append(f'{H}/verse/text(corpus:"tanakh",book:"Ps",chapter:{ch * 3},verse:1,edition:"uxlc")')
for corpus, code in (("tanakh", "uxlc"), ("nt", "sblgnt"), ("quran", "uthmani"), ("quran", "simple")):
    for m in ("version", "source_url", "file_hash", "notice") + (() if code == "uxlc" else ("file",)):
        cells.append(f'{H}/edition/{m}(corpus:"{corpus}",code:"{code}")')

diffs, compared = [], 0
s1, k1 = get(live, "/.well-known/datumesh.json")
s2, k2 = get(drill, "/.well-known/datumesh.json")
compared += 1
if (s1, canon(k1)) != (s2, canon(k2)):
    diffs.append("key document")
for name in ("verse", "verse/text", "edition", "edition/notice", "chapter_count", "verse_count"):
    q = "/bindings?name=" + urllib.parse.quote(f"{H}/{name}", safe="")
    a, b = [json.loads(get(x, q)[1]) for x in (live, drill)]
    a.pop("asof", None)
    b.pop("asof", None)
    compared += 1
    if a != b:
        diffs.append(f"bindings {name}")
for c in cells:
    p = "/cell/" + urllib.parse.quote(c, safe="/")
    (sa, ba), (sb, bb) = get(live, p), get(drill, p)
    ra, rb = json.loads(ba).get("receipt"), json.loads(bb).get("receipt")
    compared += 1
    if sa != 200 or sb != 200 or ra is None or ra != rb:
        diffs.append(f"cell {c}: {sa} {sb}")
        continue
    if ra.get("type") == "blob":
        sha = ra["value"].split(":")[0]
        compared += 1
        if get(live, f"/blob/{sha}") != get(drill, f"/blob/{sha}"):
            diffs.append(f"blob {sha}")
print(json.dumps({"compared": compared, "differ": diffs}))
sys.exit(1 if diffs else 0)
