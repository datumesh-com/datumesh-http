# SPDX-License-Identifier: Apache-2.0
"""Checks the output of scripture_prod.py before anything is loaded (CONTRACT-1.7a section S).

    python -I check_scripture_prod.py <download dir> <out dir> [<mesh-test seed out dir>]

Independent of the generator where it can be:
* counts: verses and chapters per edition from the source files' own markup (UXLC <v>/<c> elements
  counted as text, SBLGNT verse lines, Tanzil <aya>/<sura> elements), against the cells;
* every verse blob: its sha256 is the name of a file in blobs/ whose bytes hash to it, and, with
  the mesh-test seed (scripture.py of 1.8a, verified live in Python and JS), each verse's bytes
  equal mesh-test's for the same verse (Quran keys mapped: book "s", chapter 1, verse a ->
  book Quran, chapter s, verse a); Tanzil and SBLGNT blobs are also found verbatim in the file;
* the write order: 00 holds no cell; 10's first cell is Genesis 1:1; every edition cell follows
  its corpus' verses; no line has tx, asof or bound; no formula line anywhere (verbatim only);
* summaries at most 1,024 bytes, naming the file hash and the edition's notice(); the Quran's
  naming the Tanzil Project, tanzil.net and the no-changes term;
* the classes' method hashes are the descriptor lines' hashes; licences per corpus.
Prints one JSON line per check; exit 1 on any failure.
"""
import hashlib
import io
import json
import os
import re
import sys
import tarfile
import zipfile

dl, out = sys.argv[1], sys.argv[2]
seed = sys.argv[3] if len(sys.argv) > 3 else None
failed = []


def check(name, ok, **detail):
    print(json.dumps({"check": name, "ok": bool(ok), **detail}, ensure_ascii=False))
    if not ok:
        failed.append(name)


def jcs(v):
    return json.dumps(v, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


files = {}
for n in ("00-classes", "10-tanakh", "20-nt", "30-quran"):
    files[n] = [json.loads(l) for l in open(os.path.join(out, f"{n}.jsonl"), encoding="utf-8")]
NS = next(l["class"] for l in files["00-classes"] if "class" in l).rsplit("/", 1)[0]


def args(c):
    return {k: v["value"] for k, v in c["cell"]["args"].items()}


# ---- shape and order ----
check("no tx, asof, bound or issued anywhere", not any(k in l for ls in files.values() for l in ls for k in ("tx", "asof", "bound", "issued")))
check("no formula line anywhere (verbatim only)", not any("formula" in l for ls in files.values() for l in ls))
check("00 holds no cell", not any("cell" in l for l in files["00-classes"]))
first = next(l for l in files["10-tanakh"] if "cell" in l)
check("the first cell is Genesis 1:1 (uxlc)", first["cell"]["fn"] == f"{NS}/verse/text" and args(first) ==
      {"corpus": "tanakh", "book": "Gen", "chapter": "1", "verse": "1", "edition": "uxlc"}, first=args(first))
for n, corpus in (("10-tanakh", "tanakh"), ("20-nt", "nt"), ("30-quran", "quran")):
    kinds = [l["cell"]["fn"].rsplit("/", 1)[-1] if "/edition/" not in l["cell"]["fn"] else "edition" for l in files[n]]
    last_verse = max(i for i, k in enumerate(kinds) if k == "text")
    first_edition = min(i for i, k in enumerate(kinds) if k == "edition")
    check(f"{n}: verses, then counts, then edition cells", last_verse < first_edition and all(
        args(l)["corpus"] == corpus for l in files[n]), last_verse=last_verse, first_edition=first_edition)

# ---- classes ----
descs = {hashlib.sha256(jcs(l["descriptor"])).hexdigest(): l["descriptor"] for l in files["00-classes"] if "descriptor" in l and "class" not in l}
check("descriptor lines are stored descriptors", all(d["code"] == {"body": "", "lang": "stored"} and d["inputs"] == [] for d in descs.values()))
classes = {l["class"]: l["descriptor"] for l in files["00-classes"] if "class" in l}
check("two classes, verse and edition", set(classes) == {f"{NS}/verse", f"{NS}/edition"})
check("verse(corpus, book, chapter, verse) with text", classes[f"{NS}/verse"]["order"] == ["corpus", "book", "chapter", "verse"]
      and set(classes[f"{NS}/verse"]["methods"]) == {"text"})
check("edition(corpus, code) with five methods", classes[f"{NS}/edition"]["order"] == ["corpus", "code"]
      and set(classes[f"{NS}/edition"]["methods"]) == {"version", "source_url", "file_hash", "notice", "file"})
check("every method hash is a descriptor line's", all(h in descs for c in classes.values() for h in c["methods"].values()))
check("string keys have key_normalisation none", all(k.get("key_normalisation", "none") == "none" for c in classes.values()
                                                    for k in c["keys"].values() if k["type"] == "string"))
fns = {l["function"] for l in files["00-classes"] if "function" in l}
check("counts are plain functions", fns == {f"{NS}/chapter_count", f"{NS}/verse_count"})

# ---- blobs ----
bad = []
for name in os.listdir(os.path.join(out, "blobs")):
    if hashlib.sha256(open(os.path.join(out, "blobs", name), "rb").read()).hexdigest() != name:
        bad.append(name)
check("every blob file hashes to its name", not bad, bad=bad[:5])


def blob(l):
    return open(os.path.join(out, l["value"]["blob"]), "rb").read()


# ---- the sources' own counts ----
z = zipfile.ZipFile(os.path.join(dl, "Tanach.xml.zip"))
uv = sum(len(re.findall(rb"<v n=", z.read(n))) for n in z.namelist() if n.startswith("Books/") and n.count(".") == 1
         and not any(x in n for x in ("Header", "Index")))
uc = sum(len(re.findall(rb"<c n=", z.read(n))) for n in z.namelist() if n.startswith("Books/") and n.count(".") == 1
         and not any(x in n for x in ("Header", "Index")))
tgz_name = next(n for n in os.listdir(dl) if n.startswith("sblgnt-"))
tf = tarfile.open(os.path.join(dl, tgz_name))
nt_lines = {}
for m in tf.getmembers():
    if "/data/sblgnt/text/" in m.name and m.name.endswith(".txt"):
        for line in tf.extractfile(m).read().split(b"\n"):
            mm = re.match(rb"^(\S+) (\d+):(\d+)\t(.*)$", line, re.S)
            if mm:
                nt_lines[(mm.group(1).decode(), int(mm.group(2)), int(mm.group(3)))] = mm.group(4)
nt_ch = len({(b, c) for b, c, v in nt_lines})
tz = {e: open(os.path.join(dl, f"quran-{e}.xml"), "rb").read() for e in ("uthmani", "simple")}
own = {"tanakh/uxlc": (uv, uc), "nt/sblgnt": (len(nt_lines), nt_ch),
       **{f"quran/{e}": (len(re.findall(rb"<aya ", d)), len(re.findall(rb"<sura ", d))) for e, d in tz.items()}}
verses = {}
for n in ("10-tanakh", "20-nt", "30-quran"):
    for l in files[n]:
        if l["cell"]["fn"] == f"{NS}/verse/text":
            a = args(l)
            verses[(a["corpus"], a["book"], int(a["chapter"]), int(a["verse"]), a["edition"])] = l
for ed, (nv, nc) in own.items():
    corpus, code = ed.split("/")
    mine = [k for k in verses if k[0] == corpus and k[4] == code]
    check(f"{ed}: verse and chapter counts equal the source file's own", len(mine) == nv and len({k[1:3] for k in mine}) == nc,
          cells=len(mine), source_verses=nv, chapters=len({k[1:3] for k in mine}), source_chapters=nc)
cc = {(args(l)["corpus"], args(l)["book"]): int(l["value"]["value"]) for ls in files.values() for l in ls
      if "cell" in l and l["cell"]["fn"] == f"{NS}/chapter_count"}
vc = {(args(l)["corpus"], args(l)["book"], int(args(l)["chapter"])): int(l["value"]["value"]) for ls in files.values() for l in ls
      if "cell" in l and l["cell"]["fn"] == f"{NS}/verse_count"}
ref = {k[:4] for k in verses if k[4] in ("uxlc", "sblgnt", "uthmani")}
check("chapter_count and verse_count agree with the verses", all(
    cc[(c, b)] == len({k[2] for k in ref if k[:2] == (c, b)}) for c, b in cc) and all(
    vc[(c, b, ch)] == len([k for k in ref if k[:3] == (c, b, ch)]) for c, b, ch in vc) and len(vc) == 929 + 260 + 114,
    chapter_counts=len(cc), verse_counts=len(vc))
check("Quran: one book, Quran, chapters 1-114", cc.get(("quran", "Quran")) == 114 and {k[1] for k in verses if k[0] == "quran"} == {"Quran"})

# ---- verbatim ----
bad = [k for k, l in verses.items() if k[0] == "nt" and blob(l) != nt_lines[(k[1], k[2], k[3])]]
check("every SBLGNT verse is its line's bytes after the tab", not bad, bad=bad[:3])
bad = []
for e, d in tz.items():
    atts = {(int(s), int(a)): t for s, body in re.findall(rb'<sura index="(\d+)"[^>]*>(.*?)</sura>', d, re.S)
            for a, t in re.findall(rb'<aya index="(\d+)" text="([^"]*)"', body)}
    bad += [k for k, l in verses.items() if k[0] == "quran" and k[4] == e and blob(l) != atts[(k[2], k[3])]]
check("every Tanzil verse is its aya's text attribute, byte for byte", not bad, bad=bad[:3])
if seed:
    old = {}
    for line in open(os.path.join(seed, "scripture.jsonl"), encoding="utf-8"):
        if '"verse_text"' not in line and "/verse_text" not in line:
            continue
        l = json.loads(line)
        if "cell" not in l:
            continue
        a = args(l)
        k = (a["corpus"], a["book"], int(a["chapter"]), int(a["verse"]), a["edition"])
        if a["corpus"] == "quran":
            k = ("quran", "Quran", int(a["book"]), int(a["verse"]), a["edition"])
        old[k] = l["value"]["blob"].split("/")[-1]
    diff = [k for k, l in verses.items() if old.get(k) != l["value"]["blob"].split("/")[-1]]
    check("every verse equals mesh-test's verified seed, byte for byte (all corpora)", not diff and len(old) == len(verses),
          verses=len(verses), mesh_test=len(old), differ=[list(k) for k in diff[:3]])

# ---- summaries and licences ----
lic = {"tanakh": "https://tanach.us/License.html", "nt": "CC-BY-4.0", "quran": "https://tanzil.net/docs/text_license"}
cells = [l for ls in files.values() for l in ls if "cell" in l]
check("every summary at most 1,024 bytes", all(len(l["summary"].encode()) <= 1024 for l in cells),
      longest=max(len(l["summary"].encode()) for l in cells))
check("licences per corpus", all(l["licence"] == lic[args(l)["corpus"]] for l in cells))
eh = {}
for l in cells:
    if l["cell"]["fn"] == f"{NS}/edition/file_hash":
        eh[(args(l)["corpus"], args(l)["code"])] = l["value"]["value"]
bad = [k for k, l in verses.items() if eh[(k[0], k[4])] not in l["summary"]
       or f"{NS}/edition({k[0]}, {k[4]}).notice()" not in l["summary"]]
check("every verse summary names its file hash and its edition's notice()", not bad, bad=bad[:3])
bad = [k for k, l in verses.items() if k[0] == "quran" and not all(
    s in l["summary"] for s in ("Tanzil Project", "tanzil.net", "CHANGING IT IS NOT ALLOWED"))]
check("Quran summaries name the Tanzil Project, tanzil.net and the no-changes term", not bad, bad=bad[:3])

# ---- editions ----
ed = {}
for l in cells:
    if "/edition/" in l["cell"]["fn"]:
        ed.setdefault((args(l)["corpus"], args(l)["code"]), {})[l["cell"]["fn"].rsplit("/", 1)[1]] = l
want = {("tanakh", "uxlc"), ("nt", "sblgnt"), ("quran", "uthmani"), ("quran", "simple")}
check("four editions", set(ed) == want)
check("version, source_url, file_hash, notice for all four; file for all but uxlc", all(
    set(m) == ({"version", "source_url", "file_hash", "notice"} | (set() if k == ("tanakh", "uxlc") else {"file"})) for k, m in ed.items()))
srcfile = {("tanakh", "uxlc"): "Tanach.xml.zip", ("nt", "sblgnt"): tgz_name, ("quran", "uthmani"): "quran-uthmani.xml",
           ("quran", "simple"): "quran-simple.xml"}
check("file_hash is the downloaded file's sha256", all(
    m["file_hash"]["value"]["value"] == hashlib.sha256(open(os.path.join(dl, srcfile[k]), "rb").read()).hexdigest() for k, m in ed.items()))
check("file() is the downloaded file, byte for byte", all(
    blob(m["file"]) == open(os.path.join(dl, srcfile[k]), "rb").read() for k, m in ed.items() if "file" in m))
for e, d in tz.items():
    head = d[:d.index(b"-->")].decode()
    n = blob(ed[("quran", e)]["notice"]).decode()
    check(f"Tanzil {e} notice() is the file's copyright block verbatim", n.rstrip("\n") in head and n.startswith("# PLEASE DO NOT REMOVE")
          and n.rstrip().endswith("#" + "=" * 68), bytes=len(n.encode()))
n = blob(ed[("tanakh", "uxlc")]["notice"]).decode()
check("UXLC notice() quotes tanach.us/License.html", "without restriction" in n and "not the Westminster Leningrad Codex" in n
      and "written permission" in n, bytes=len(n.encode()))
n = blob(ed[("nt", "sblgnt")]["notice"]).decode()
check("SBLGNT notice() has the README licence section and the LICENSE file", "Creative Commons Attribution 4.0" in n
      and "Society of Biblical Literature" in n and "Attribution 4.0 International" in n, bytes=len(n.encode()))
print(json.dumps({"failed": failed, "cells": len(cells), "verses": len(verses)}))
sys.exit(1 if failed else 0)
