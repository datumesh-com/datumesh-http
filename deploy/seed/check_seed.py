# SPDX-License-Identifier: Apache-2.0
"""The seed data's tests over HTTPS, with the Python library datumesh 0.4.3 (check_seed.mjs
repeats the per-corpus verification with @datumesh/verify 0.4.3):

* every verse_text cell of the seed answers by name (POST /cells, 1,000 a batch, checked with
  check_batch against its cell id and the host asked; an item `pending` past the mesh's budget
  is polled to its envelope, checked against the same request with its job): ok, primary, the
  value the blob of the source's bytes;
* verse_count and chapter_count agree with the loaded verse_text cells for every book and
  chapter;
* the slice scripture/verse_text(corpus: "quran", book: *, chapter: *, verse: *, edition:
  "uthmani") pages through /slice (each page checked with check_envelope against the slice id,
  the key after which it starts and the host asked) to its 6,236 members, each the seed's;
* one verse of each corpus (and edition): the envelope, the receipt verified at `signed` with
  its blob content (10.1 step 7.7), the blob from /blob/<sha256> the source's bytes;
* the derived edition: strip_niqqud of the first verse of every book, computed by the mesh
  (trust computed), equals the UXLC verse with NFC and the vector's characters removed;
* the synthetic ECB cells (fx_rate, holiday) answer by name with their licences, the restricted
  cell among them, and fx_cross over them is computed by the mesh.

    python check_seed.py <base URL> <seed dir> <API key file> <out dir> [CA file]

The seed dir is what the generators wrote (ecb.jsonl, scripture.jsonl, blobs/). JSON results
to stdout, the verses for the JavaScript part to <out dir>/verses.json; exit 1 on a failure.
"""
import json
import os
import pathlib
import ssl
import sys
import time
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
from decimal import Decimal

import datumesh

base, seed, keyfile, out = sys.argv[1].rstrip("/"), pathlib.Path(sys.argv[2]), sys.argv[3], pathlib.Path(sys.argv[4])
cafile = sys.argv[5] if len(sys.argv) > 5 else None
# The mesh's host (its key document's): the base URL's, unless DATUMESH_HOST names it (a mesh
# reached by another name, e.g. over plain HTTP inside a Docker network).
host = os.environ.get("DATUMESH_HOST") or base.split("//", 1)[1].split("/")[0]
ctx = ssl.create_default_context(cafile=cafile)
KEY = open(keyfile).read().strip()
results = []
started = time.time()


def check(name, ok, detail=""):
    results.append({"check": name, "ok": bool(ok), "detail": detail})
    return ok


def req(path, method="GET", body=None, headers=None):
    h = dict(headers or {})
    h["Authorization"] = "Bearer " + KEY
    for _ in range(20):
        r = urllib.request.Request(base + path, data=body, method=method, headers=h)
        try:
            with urllib.request.urlopen(r, context=ctx, timeout=120) as resp:
                return resp.status, dict(resp.headers), resp.read()
        except urllib.error.HTTPError as e:
            if e.code == 429:
                time.sleep(int(e.headers.get("Retry-After", "2")) + 0.5)
                continue
            return e.code, dict(e.headers), e.read()
    raise RuntimeError("rate limited 20 times: " + path)


def lines(name):
    for line in open(seed / name, encoding="utf-8"):
        if line.strip():
            yield json.loads(line)


s, _, kd = req("/.well-known/datumesh.json")
keys = datumesh.keys_from_response(host, s, kd)
check("key document", keys.ok, getattr(keys, "detail", ""))


def binding(name):
    s, _, b = req("/bindings?name=" + urllib.parse.quote(name, safe=""))
    return json.loads(b)["bindings"][name]["fn_hash"] if s == 200 else None


def uri_path(cell):
    ns, fn = cell["fn"].rsplit("/", 1)
    return datumesh.format_uri({"ns": ns, "fn": fn, "args": cell["args"]})[len("cell://"):]


def cell_id(cell, fn_hash):
    ns, fn = cell["fn"].rsplit("/", 1)
    return datumesh.cell_hash({"ns": ns, "fn": fn, "fn_hash": fn_hash, "args": cell["args"]})


def batch(cells, hashes):
    """Each cell's checked envelope (None when its item fails the check), by name, 1,000 a batch."""
    got = []
    for i in range(0, len(cells), 1000):
        part = cells[i:i + 1000]
        s, _, b = req("/cells", "POST", json.dumps([uri_path(c) for c in part]).encode(), {"Content-Type": "application/json"})
        requests = [{"cell_id": cell_id(c, hashes[c["fn"]]), "mesh": host} for c in part]
        r = datumesh.check_batch(s, b, [keys], requests)
        if not r.ok:
            check(f"batch at {i}", False, f"{r.code}: {r.detail}")
            got.extend([None] * len(part))
            continue
        items = json.loads(b)
        for item, res, rq in zip(items, r.items, requests):
            e = item["envelope"]
            if res.ok and e.get("outcome") == "pending":
                # Past the mesh's budget (a batch answers within it, 5.7): the job, polled to
                # its envelope, which is bound to the original request.
                job = e["job"]
                for _ in range(600):
                    s2, _, b2 = req(f"/jobs/{job}")
                    if s2 != 202:
                        break
                    time.sleep(0.5)
                res = datumesh.check_envelope(s2, b2, [keys], dict(rq, job=job))
                e = json.loads(b2)
                pending[0] += 1
            got.append(e if res.ok else {"failed": res.code, "detail": res.detail})
    return got


pending = [0]


def blob_value(cell_line):
    v = cell_line["value"]
    data = (seed / v["blob"]).read_bytes()
    return f"{v['blob'].rsplit('/', 1)[1]}:{len(data)}:{v['media_type']}", data


# Scripture: every verse_text cell by name.
scripture = list(lines("scripture.jsonl"))
fns = {l["function"] for l in scripture if "function" in l} | {l["formula"] for l in scripture if "formula" in l}
hashes = {f: binding(f) for f in fns}
check("scripture functions bound", all(hashes.values()), json.dumps(hashes))
verses = [l for l in scripture if "cell" in l and l["cell"]["fn"].endswith("/verse_text")]
t0 = time.time()
envs = batch([v["cell"] for v in verses], hashes)
by_edition, bad = {}, []
for v, e in zip(verses, envs):
    a = v["cell"]["args"]
    key = f"{a['corpus']['value']}/{a['edition']['value']}"
    want, _ = blob_value(v)
    ok = e is not None and e.get("outcome") == "ok" and e.get("trust") == "primary" and e["receipt"]["value"] == want and e["receipt"]["type"] == "blob"
    by_edition.setdefault(key, [0, 0])[0 if ok else 1] += 1
    if not ok and len(bad) < 5:
        bad.append({"cell": v["cell"]["args"], "envelope": e})
check("every verse_text cell answers by name (ok, primary, the source's blob)", not bad and len(envs) == len(verses),
      json.dumps({"by corpus/edition [answered, otherwise]": by_edition, "pending, then polled": pending[0], "seconds": round(time.time() - t0, 1), "first failures": bad}, ensure_ascii=False)[:2000])

# verse_count and chapter_count against the loaded verse_text cells.
chapters, versen = {}, {}
for v in verses:
    a = v["cell"]["args"]
    if a["corpus"]["value"] == "quran" and a["edition"]["value"] != "uthmani":
        continue
    ck = (a["corpus"]["value"], a["book"]["value"])
    chapters.setdefault(ck, set()).add(int(a["chapter"]["value"]))
    vk = ck + (int(a["chapter"]["value"]),)
    versen[vk] = versen.get(vk, 0) + 1
s_ = lambda x: {"type": "string", "value": x}
n_ = lambda x: {"type": "number", "value": str(x)}
cc = [{"fn": f"{host}/scripture/chapter_count", "args": {"corpus": s_(c), "book": s_(b)}} for (c, b) in sorted(chapters)]
vc = [{"fn": f"{host}/scripture/verse_count", "args": {"corpus": s_(c), "book": s_(b), "chapter": n_(ch)}} for (c, b, ch) in sorted(versen)]
want_cc = [len(chapters[k]) for k in sorted(chapters)]
want_vc = [versen[k] for k in sorted(versen)]
for name, cells, want in (("chapter_count", cc, want_cc), ("verse_count", vc, want_vc)):
    envs = batch(cells, hashes)
    wrong = [(c["args"], e and e.get("receipt", {}).get("value"), w) for c, e, w in zip(cells, envs, want)
             if not (e and e.get("outcome") == "ok" and e["receipt"]["value"] == str(w))]
    check(f"{name} agrees with the loaded verses for every book" + (" and chapter" if name == "verse_count" else ""),
          not wrong, f"{len(cells)} cells; " + json.dumps(wrong[:5], ensure_ascii=False))

# The Quran (Uthmani) slice through /slice, page by page.
quran = {}
for v in verses:
    a = v["cell"]["args"]
    if a["corpus"]["value"] == "quran" and a["edition"]["value"] == "uthmani":
        quran[cell_id(v["cell"], hashes[v["cell"]["fn"]])] = blob_value(v)[0]
path = f"/slice/{host}/scripture/verse_text?book=*&chapter=*&corpus=quran&edition=uthmani&verse=*"
seen, pages, after, page_fail, t0 = {}, 0, None, [], time.time()
url = path
while url:
    s, _, b = req(url)
    pages += 1
    page = json.loads(b)
    sid = datumesh.slice_id(page["slice"]) if s == 200 else None
    rq = {"slice_id": sid, "mesh": host}
    if after is not None:
        rq["after"] = after
    r = datumesh.check_envelope(s, b, [keys], rq)
    if not r.ok:
        page_fail.append(f"page {pages}: {r.code} {r.detail}")
        break
    for m in page["members"]:
        rec = m["receipt"]
        seen[datumesh.cell_hash(rec["cell"])] = rec["value"]
    if page["members"]:
        last = page["members"][-1]["receipt"]["cell"]["args"]
        after = {k: last[k] for k in ("book", "chapter", "verse")}
    nxt = page.get("next")
    url = (lambda u: u.path + ("?" + u.query if u.query else ""))(urllib.parse.urlsplit(nxt)) if nxt else None
    if pages > 100:
        page_fail.append("more than 100 pages")
        break
check("the Quran (Uthmani) slice pages through /slice: 6,236 members, each the seed's verse",
      not page_fail and len(seen) == 6236 and seen == quran,
      f"{len(seen)} members in {pages} pages, {round(time.time() - t0, 1)} s; " + "; ".join(page_fail))

# One verse of each corpus and edition: envelope, receipt at signed with its blob (7.7).
samples = {"tanakh/uxlc": ("Gen", 1, 1), "nt/sblgnt": ("John", 1, 1), "quran/uthmani": ("1", 1, 1), "quran/simple": ("1", 1, 1)}
for_js = []
for key, (book, ch, vn) in samples.items():
    corpus, edition = key.split("/")
    line = next(v for v in verses if v["cell"]["args"]["corpus"]["value"] == corpus and v["cell"]["args"]["edition"]["value"] == edition
                and v["cell"]["args"]["book"]["value"] == book and v["cell"]["args"]["chapter"]["value"] == str(ch) and v["cell"]["args"]["verse"]["value"] == str(vn))
    s, _, b = req("/cell/" + uri_path(line["cell"]))
    r = datumesh.check_envelope(s, b, [keys], {"cell_id": cell_id(line["cell"], hashes[line["cell"]["fn"]]), "mesh": host})
    rec = json.loads(b).get("receipt", {})
    want, data = blob_value(line)
    s2, _, blob = req("/blob/" + want[:64])
    v = datumesh.verify(json.dumps(rec), [keys], None, blob)
    check(f"{key} {book} {ch}:{vn}: envelope (Python 0.4.3), receipt signed with its blob (7.7), the source's bytes",
          r.ok and r.trust == "primary" and v.ok and v.level == "signed" and getattr(v, "blob_checked", False) and s2 == 200 and blob == data,
          f"{r.code} {v.code} {getattr(v, 'detail', '')} licence {rec.get('licence')}; {rec.get('summary', '')[:160]}")
    for_js.append({"corpus": key, "path": "/cell/" + uri_path(line["cell"]), "cell_id": cell_id(line["cell"], hashes[line["cell"]["fn"]]), "blob": want[:64]})

# The derived edition: strip_niqqud of each book's first verse, computed by the mesh.
strip_line = next(l for l in scripture if l.get("formula", "").endswith("/strip_niqqud"))
chars = strip_line["source"].rsplit(', "', 1)[1].split('"')[0]
firsts = {}
for v in verses:
    a = v["cell"]["args"]
    if a["edition"]["value"] == "uxlc" and a["chapter"]["value"] == "1" and a["verse"]["value"] == "1":
        firsts[a["book"]["value"]] = v
cells = [{"fn": strip_line["formula"], "args": {"book": s_(b), "chapter": n_(1), "verse": n_(1)}} for b in sorted(firsts)]
envs = batch(cells, hashes)
wrong = []
for b, e in zip(sorted(firsts), envs):
    text = unicodedata.normalize("NFC", blob_value(firsts[b])[1].decode("utf-8"))
    want = "".join(ch for ch in text if ch not in chars)
    if not (e and e.get("outcome") == "ok" and e.get("trust") == "computed" and e["receipt"]["value"] == want):
        wrong.append({"book": b, "envelope": e})
check("strip_niqqud (uxlc-consonantal) of every book's first verse: computed, the UXLC text with NFC and the points removed",
      not wrong, f"{len(cells)} cells; " + json.dumps(wrong[:3], ensure_ascii=False)[:1500])

# The synthetic ECB cells.
ecb = list(lines("ecb.jsonl"))
efns = {l["function"] for l in ecb if "function" in l} | {l["formula"] for l in ecb if "formula" in l}
ehash = {f: binding(f) for f in efns}
check("ecb functions bound", all(ehash.values()), json.dumps(ehash))
ecells = [l for l in ecb if "cell" in l]
envs = batch([l["cell"] for l in ecells], ehash)
wrong, licences = [], {}
for l, e in zip(ecells, envs):
    ok = e and e.get("outcome") == "ok" and e.get("trust") == "primary" and e["receipt"]["value"] == l["value"]["value"] \
        and e["receipt"]["licence"] == l["licence"] and "test data" in e["receipt"]["summary"]
    licences[l["licence"]] = licences.get(l["licence"], 0) + 1
    if not ok:
        wrong.append({"cell": l["cell"], "envelope": e})
check("ecb cells answer by name with their values, licences and 'test data' summaries", not wrong,
      f"{len(ecells)} cells, licences {json.dumps(licences)}; " + json.dumps(wrong[:3])[:1500])
restricted = [l for l in ecells if l["licence"] == f"https://{host}/licences/restricted-test"]
check("one cell has the restricted test licence", len(restricted) == 1, json.dumps([l["cell"] for l in restricted]))
rates = {}
for l in ecells:
    if l["cell"]["fn"].endswith("/fx_rate"):
        a = l["cell"]["args"]
        rates[(a["quote"]["value"], a["date"]["value"])] = Decimal(l["value"]["value"])
cross = next(l["formula"] for l in ecb if "formula" in l and l["formula"].endswith("/fx_cross"))
picks = [("USD", "GBP"), ("GBP", "JPY")]
day = max(d for (q, d) in rates if q == "USD")
nok_day = next(d for (q, d) in rates if q == "NOK")
cells = [{"fn": cross, "args": {"base": s_(b), "date": {"type": "date", "value": day}, "quote": s_(q)}} for b, q in picks]
cells.append({"fn": cross, "args": {"base": s_("USD"), "date": {"type": "date", "value": nok_day}, "quote": s_("NOK")}})
envs = batch(cells, ehash)
wrong = []
for c, e in zip(cells, envs):
    a = c["args"]
    want = rates[(a["quote"]["value"], a["date"]["value"])] / rates[(a["base"]["value"], a["date"]["value"])]
    got = e and e.get("outcome") == "ok" and e.get("trust") == "computed" and abs(Decimal(e["receipt"]["value"]) - want) < Decimal("1e-20")
    if not got:
        wrong.append({"args": a, "envelope": e, "want": str(want)})
check("fx_cross over the fx_rate cells (the restricted one included) is computed by the mesh", not wrong, json.dumps(wrong[:3])[:1500])

out.mkdir(parents=True, exist_ok=True)
(out / "verses.json").write_text(json.dumps(for_js))
failed = [r for r in results if not r["ok"]]
print(json.dumps({"base": base, "checks": len(results), "failed": len(failed), "seconds": round(time.time() - started, 1), "results": results}, indent=1, ensure_ascii=False))
sys.exit(1 if failed else 0)
