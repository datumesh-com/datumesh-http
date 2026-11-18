# SPDX-License-Identifier: Apache-2.0
"""The production smoke (step 1.7a item 7), with Python datumesh 0.5.0:

    python -I smoke_prod.py [<base URL> [<namespace>]]

Defaults: https://datumesh.com and datumesh.com/scripture. With another base (a rehearsal), the
key document is read from that base and checked for host datumesh.com as given; over HTTPS on
datumesh.com it is also fetched by the library itself (TLS verified, no redirect).

Checks: the first and last verse of each corpus in each edition, read as method calls, verified at
`signed` with the blob checked and the envelope bound to the request (trust primary); the method
call and the plain read of verse/text are one receipt; the verse counts, summed from the mesh's
verse_count cells, equal the source files' own counts (UXLC 23,213; SBLGNT 7,939; Tanzil 6,236),
and the chapter counts (929, 260, 114); each edition's file_hash and notice() answer; /find
returns the class for "quran", "torah", "bible verse" and "new testament greek". One JSON line per
check; exit 1 on any failure.
"""
import json
import sys
import urllib.error
import urllib.parse
import urllib.request

import datumesh

BASE = (sys.argv[1] if len(sys.argv) > 1 else "https://datumesh.com").rstrip("/")
NS = sys.argv[2] if len(sys.argv) > 2 else "datumesh.com/scripture"
HOST = NS.split("/")[0]
failed = []


def get(path):
    # Cloudflare's Browser Integrity Check answers Python's default user agent with 403 (1010).
    req = urllib.request.Request(BASE + path, headers={"User-Agent": "datumesh-smoke/1.7a (datumesh.com)"})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def check(name, ok, **detail):
    print(json.dumps({"check": name, "ok": bool(ok), **detail}, ensure_ascii=False))
    if not ok:
        failed.append(name)


s, kd = get("/.well-known/datumesh.json")
keys = datumesh.keys_from_response(HOST, s, kd)
fetched = datumesh.fetch_keys(HOST).ok if BASE == f"https://{HOST}" else None
check("key document", keys.ok and fetched is not False, fetched_by_library=fetched)


def envelope(call):
    s, b = get("/cell/" + urllib.parse.quote(call, safe="/"))
    e = json.loads(b)
    rec = e.get("receipt")
    k = datumesh.check_envelope(s, b, [keys], {"cell_id": datumesh.cell_hash(rec["cell"]), "mesh": HOST}) if rec else None
    return e, rec, k


FIRST_LAST = [("tanakh", "Gen", 1, 1, "uxlc"), ("tanakh", "2Chr", 36, 23, "uxlc"), ("tanakh", "Neh", 13, 31, "uxlc"),
              ("nt", "Matt", 1, 1, "sblgnt"), ("nt", "Rev", 22, 21, "sblgnt"),
              ("quran", "Quran", 1, 1, "uthmani"), ("quran", "Quran", 114, 6, "uthmani"),
              ("quran", "Quran", 1, 1, "simple"), ("quran", "Quran", 114, 6, "simple")]
for corpus, book, ch, v, ed in FIRST_LAST:
    m = f'{NS}/verse({corpus}, "{book}", {ch}, {v}).text({ed})'  # a book code starting with a digit is quoted (5.8)
    p = f'{NS}/verse/text(corpus: "{corpus}", book: "{book}", chapter: {ch}, verse: {v}, edition: "{ed}")'
    e1, r1, k1 = envelope(m)
    e2, r2, k2 = envelope(p)
    content = None
    if r1 and r1.get("type") == "blob":
        content = get("/blob/" + r1["value"].split(":")[0])[1]
    vr = datumesh.verify(json.dumps(r1), [keys], blob=content) if r1 else None
    check(f"{corpus} {book} {ch}:{v} {ed}", r1 and r2 and datumesh.receipt_hash(r1) == datumesh.receipt_hash(r2) and k1.ok and k2.ok
          and k1.trust == "primary" and vr.ok and vr.level == "signed" and vr.blob_checked,
          receipt=r1 and datumesh.receipt_hash(r1)[:16], bytes=content and len(content), trust=k1 and k1.trust,
          blob_checked=vr and vr.blob_checked)



def members(call):
    """The members of a slice (5.6), every page, each receipt verified; paths of `next` are read
    from this base (a rehearsal's mesh names its own host in them)."""
    path, out = "/slice/" + urllib.parse.quote(call, safe="/"), []
    while path:
        s, b = get(path)
        if s != 200:
            return None
        page = json.loads(b)
        for m in page.get("members", []):
            r = m["receipt"]
            if not datumesh.verify(json.dumps(r), [keys]).ok:
                return None
            out.append(r)
        nxt = page.get("next")
        path = urllib.parse.urlsplit(nxt)._replace(scheme="", netloc="").geturl() if nxt else None
    return out


for corpus, (nv, nc, nb) in (("tanakh", (23213, 929, 39)), ("nt", (7939, 260, 27)), ("quran", (6236, 114, 1))):
    cc = members(f'{NS}/chapter_count(corpus: "{corpus}", book: *)') or []
    vc = members(f'{NS}/verse_count(corpus: "{corpus}", book: *, chapter: *)') or []
    chapters, verses = sum(int(r["value"]) for r in cc), sum(int(r["value"]) for r in vc)
    check(f"{corpus}: verse and chapter counts equal the source files' own", verses == nv and chapters == nc and len(cc) == nb
          and len(vc) == nc, verses=verses, want_verses=nv, chapters=chapters, want_chapters=nc, books=len(cc))

for corpus, code in (("tanakh", "uxlc"), ("nt", "sblgnt"), ("quran", "uthmani"), ("quran", "simple")):
    e, r, k = envelope(f"{NS}/edition({corpus}, {code}).file_hash()")
    e2, r2, k2 = envelope(f"{NS}/edition({corpus}, {code}).notice()")
    n = get("/blob/" + r2["value"].split(":")[0])[1] if r2 else None
    v2 = datumesh.verify(json.dumps(r2), [keys], blob=n) if r2 else None
    check(f"edition({corpus}, {code}): file_hash and notice()", r and k.ok and len(r["value"]) == 64 and v2 and v2.ok and v2.blob_checked,
          file_hash=r and r["value"][:16], notice_bytes=n and len(n))

for q in ("quran", "torah", "bible verse", "new testament greek"):
    s, b = get("/find?q=" + urllib.parse.quote(q))
    rs = json.loads(b).get("results", []) if s == 200 else []
    check(f"find {q!r} returns the class", any(r.get("kind") == "class" and r.get("name") == f"{NS}/verse" for r in rs),
          names=[r.get("name") for r in rs][:6])
print(json.dumps({"python": f"datumesh {datumesh.__version__ if hasattr(datumesh, '__version__') else '0.5.0'}", "failed": failed}))
sys.exit(1 if failed else 0)
