# SPDX-License-Identifier: Apache-2.0
"""The scripture seed: the JSON lines that `datumesh-mesh publish-stored` loads, from the
downloads of download.sh (checked against SOURCES), without changing a character of the text.

    python -I scripture.py <download dir> <spec dir> <out dir> <host>

Writes <out dir>/scripture.jsonl (the functions, every verse_text, chapter_count and
verse_count cell, and the strip_niqqud formula), <out dir>/blobs/<sha256> (each verse's bytes,
the blob a verse_text value names) and <out dir>/scripture-report.json (rows, bytes, non-NFC
counts per corpus and edition). The archives are read in memory; nothing is extracted to disk.

Editions (the user's decisions of 2026-10-26):
* tanakh/uxlc: UXLC 2.6 (build 27.7), Books/*.xml of Tanach.xml.zip. UXLC gives words, not
  verse strings: a verse's text is its words (<w>, ketiv <k>, qere <q>) joined by one space,
  none after a word ending in maqaf, a ketiv marked `*` and a qere `**` (the form of
  tanach.us's own text files); notes (<x>) and the paragraph marks (<pe/>, <samekh/>) are not
  text; the inverted nun (<reversednun/>) is the character U+05C6 after a space. Book keys:
  OSIS, mapped from UXLC's abbreviations. Hebrew versification, as UXLC numbers it.
* nt/sblgnt: SBLGNT v1.2 at Faithlife/SBLGNT d7f35ad, data/sblgnt/text/<OSIS>.txt: a verse is
  the bytes after the tab of its `<Book> <c>:<v>` line, up to the line end, as published.
* quran/uthmani and quran/simple: Tanzil 1.1 XML: a verse is the `text` attribute of its
  <aya>, as published (the bytes between the quotes; the file has no character references
  there); verse 1 without the bismillah attribute. Book: the surah number; chapter: 1.
"""
import hashlib
import io
import json
import os
import re
import sys
import tarfile
import unicodedata
import xml.etree.ElementTree as ET
import zipfile

dl, spec, out, host = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
NS = f"{host}/scripture"
TX = "2026-11-02T00:00:00.000000Z"  # tx, asof and the bindings' start: one load
SBLGNT_COMMIT = "d7f35adca9ec410e1d1499c63b4dd989d76c8d4d"
LIC_UXLC = "https://tanach.us/License.html"
LIC_TANZIL = "https://tanzil.net/docs/text_license"
MAQAF = "־"
OSIS = {"Gen": "Gen", "Ex": "Exod", "Lev": "Lev", "Num": "Num", "Deut": "Deut", "Josh": "Josh", "Judg": "Judg",
        "1 Sam": "1Sam", "2 Sam": "2Sam", "1 Kings": "1Kgs", "2 Kings": "2Kgs", "Isa": "Isa", "Jer": "Jer",
        "Ezek": "Ezek", "Hos": "Hos", "Joel": "Joel", "Am": "Amos", "Ob": "Obad", "Jon": "Jonah", "Mic": "Mic",
        "Nah": "Nah", "Hab": "Hab", "Zeph": "Zeph", "Hag": "Hag", "Zech": "Zech", "Mal": "Mal", "Ps": "Ps",
        "Prov": "Prov", "Job": "Job", "Song": "Song", "Ruth": "Ruth", "Lam": "Lam", "Eccl": "Eccl", "Esth": "Esth",
        "Dan": "Dan", "Ezra": "Ezra", "Neh": "Neh", "1 Chr": "1Chr", "2 Chr": "2Chr"}
NT = ["Matt", "Mark", "Luke", "John", "Acts", "Rom", "1Cor", "2Cor", "Gal", "Eph", "Phil", "Col", "1Thess", "2Thess",
      "1Tim", "2Tim", "Titus", "Phlm", "Heb", "Jas", "1Pet", "2Pet", "1John", "2John", "3John", "Jude", "Rev"]

os.makedirs(os.path.join(out, "blobs"), exist_ok=True)
lines, report = [], {"tx": TX, "editions": {}}


def S(x):
    return {"type": "string", "value": x}


def N(x):
    return {"type": "number", "value": str(x)}


def put_blob(data):
    h = hashlib.sha256(data).hexdigest()
    p = os.path.join(out, "blobs", h)
    if not os.path.exists(p):
        with open(p, "wb") as f:
            f.write(data)
    return "blobs/" + h


def verse(corpus, edition, book, ch, v, data, licence, summary):
    assert len(summary.encode()) <= 1024, summary
    data.decode("utf-8")  # strict UTF-8
    lines.append({"cell": {"fn": f"{NS}/verse_text", "args": {"book": S(book), "chapter": N(ch), "corpus": S(corpus),
                                                              "edition": S(edition), "verse": N(v)}},
                  "value": {"blob": put_blob(data), "media_type": "text/plain"},
                  "tx": TX, "asof": TX, "licence": licence, "summary": summary})
    r = report["editions"].setdefault(f"{corpus}/{edition}", {"verses": 0, "bytes": 0, "non_nfc": 0, "over_1024_bytes": 0,
                                                              "books": 0, "chapters": 0, "licence": licence})
    r["verses"] += 1
    r["bytes"] += len(data)
    r["non_nfc"] += not unicodedata.is_normalized("NFC", data.decode("utf-8"))
    r["over_1024_bytes"] += len(data) > 1024


def extents(corpus, edition, structure, release, licence):
    """chapter_count and verse_count cells from the verses loaded: {book: {chapter: verses}}."""
    for book, chs in structure.items():
        lines.append({"cell": {"fn": f"{NS}/chapter_count", "args": {"book": S(book), "corpus": S(corpus)}},
                      "value": {"type": "number", "value": str(len(chs))}, "tx": TX, "asof": TX, "licence": licence,
                      "summary": f"The number of chapters of {book} in {release}, counted from its loaded verse_text cells. Licence {licence}."})
        for ch, n in chs.items():
            lines.append({"cell": {"fn": f"{NS}/verse_count", "args": {"book": S(book), "chapter": N(ch), "corpus": S(corpus)}},
                          "value": {"type": "number", "value": str(n)}, "tx": TX, "asof": TX, "licence": licence,
                          "summary": f"The number of verses of {book} {ch} in {release}, counted from its loaded verse_text cells. Licence {licence}."})
    report["editions"][f"{corpus}/{edition}"]["books"] = len(structure)
    report["editions"][f"{corpus}/{edition}"]["chapters"] = sum(len(c) for c in structure.values())


sig = lambda args, order, returns: {"args": args, "order": order, "returns": returns}
str_arg = {"type": "string", "valid": False}
num_arg = {"type": "number", "unit": "1", "valid": False}
lines.append({"function": f"{NS}/verse_text", "bound": TX, "signature": sig(
    {"book": str_arg, "chapter": num_arg, "corpus": str_arg, "edition": str_arg, "verse": num_arg},
    ["corpus", "book", "chapter", "verse", "edition"], "blob")})
lines.append({"function": f"{NS}/chapter_count", "bound": TX, "signature": sig(
    {"book": str_arg, "corpus": str_arg}, ["corpus", "book"], {"type": "number", "unit": "count"})})
lines.append({"function": f"{NS}/verse_count", "bound": TX, "signature": sig(
    {"book": str_arg, "chapter": num_arg, "corpus": str_arg}, ["corpus", "book", "chapter"], {"type": "number", "unit": "count"})})

# ---- UXLC 2.6 ----
z = zipfile.ZipFile(os.path.join(dl, "Tanach.xml.zip"))
header = z.read("Books/TanachHeader.xml").decode("utf-8")
version = re.search(r"<version>([^<]*)</version>", header).group(1)
build = re.search(r"<build>([^<]*)</build>", header).group(1)
date = re.search(r"<date>([^<]*)</date>", header).group(1)
assert (version, build) == ("UXLC 2.6", "27.7"), (version, build)
UXLC = f"the Unicode/XML Leningrad Codex, {version} (build {build}, {date}, tanach.us)"


def wtext(el):
    s = el.text or ""
    for ch in el:
        if ch.tag != "x":
            s += wtext(ch)
        s += ch.tail or ""
    return s


index = ET.fromstring(z.read("Books/TanachIndex.xml"))
structure = {}
for b in index.iter("book"):
    fname = b.find(".//filename").text
    root = ET.fromstring(z.read(f"Books/{fname}.xml"))
    bk = root.find(".//tanach/book")
    abbrev, name = bk.find("names/abbrev").text, bk.find("names/name").text
    osis = OSIS[abbrev]
    structure[osis] = {}
    for c in bk.findall("c"):
        cn = int(c.get("n"))
        for v in c.findall("v"):
            vn = int(v.get("n"))
            text, glue = "", True
            for e in v:
                if e.tag in ("w", "k", "q"):
                    t = {"w": "", "k": "*", "q": "**"}[e.tag] + wtext(e)
                    if text and not glue:
                        text += " "
                    text += t
                    glue = t.endswith(MAQAF)
                elif e.tag == "reversednun":
                    text += " ׆"
                    glue = False
                else:
                    assert e.tag in ("x", "pe", "samekh"), e.tag
            structure[osis][cn] = structure[osis].get(cn, 0) + 1
            verse("tanakh", "uxlc", osis, cn, vn, text.encode("utf-8"), LIC_UXLC,
                  f"The text of {name} {cn}:{vn} (OSIS {osis}.{cn}.{vn}) in {UXLC}, pointed and accented, as UTF-8: "
                  f"the verse's words in Books/{fname}.xml joined by spaces (none after a maqaf; a ketiv marked * and its qere **, "
                  f"as in tanach.us's text files; notes and paragraph marks omitted). UXLC is not the WLC. Licence: {LIC_UXLC}.")
extents("tanakh", "uxlc", structure, f"UXLC 2.6 (build {build})", LIC_UXLC)

# ---- SBLGNT v1.2 ----
t = tarfile.open(os.path.join(dl, f"sblgnt-{SBLGNT_COMMIT}.tar.gz"))
prefix = f"SBLGNT-{SBLGNT_COMMIT}/"
readme = t.extractfile(prefix + "README.md").read().decode("utf-8")
assert "<td>v1.2</td><td>2023-07-10</td>" in readme
structure, other = {}, []
for book in NT:
    raw = t.extractfile(prefix + f"data/sblgnt/text/{book}.txt").read()
    assert not raw.startswith(b"\xef\xbb\xbf")
    structure[book] = {}
    for line in raw.split(b"\n"):
        m = re.match(rb"^(\S+) (\d+):(\d+)\t(.*)$", line, re.S)
        if not m:
            if line.strip():
                other.append(line[:40].decode("utf-8"))
            continue
        assert m.group(1).decode() == book and not line.endswith(b"\r")
        cn, vn = int(m.group(2)), int(m.group(3))
        structure[book][cn] = structure[book].get(cn, 0) + 1
        verse("nt", "sblgnt", book, cn, vn, m.group(4), "CC-BY-4.0",
              f"The text of {book} {cn}:{vn} in the SBL Greek New Testament (SBLGNT) v1.2 (2023-07-10; github.com/Faithlife/SBLGNT "
              f"commit {SBLGNT_COMMIT}, data/sblgnt/text/{book}.txt): the line's bytes after the tab, as published. "
              f"Copyright 2010 Society of Biblical Literature and Logos Bible Software. Licence CC-BY-4.0.")
report["sblgnt_other_lines"] = other
extents("nt", "sblgnt", structure, "SBLGNT v1.2", "CC-BY-4.0")

# ---- Tanzil 1.1 ----
first = None
for edition, label in (("uthmani", "Uthmani"), ("simple", "Simple")):
    raw = open(os.path.join(dl, f"quran-{edition}.xml"), "rb").read()
    head = raw[:raw.index(b"-->")].decode("utf-8")
    notice = " ".join(re.search(p, head).group(0).strip() for p in (r"Tanzil Quran Text \([^)]*\)", r"Copyright \(C\) [^\n]*", r"License: [^\n]*"))
    assert f"({label}, Version 1.1)" in notice, notice
    body = raw[raw.index(b"<quran>"):]
    parsed = {(int(s.get("index")), int(a.get("index"))): a.get("text") for s in ET.fromstring(body).iter("sura") for a in s.iter("aya")}
    structure, n = {}, 0
    for sm in re.finditer(rb'<sura index="(\d+)" name="[^"]*">(.*?)</sura>', body, re.S):
        s = int(sm.group(1))
        structure[str(s)] = {1: 0}
        for am in re.finditer(rb'<aya index="(\d+)" text="([^"]*)"(?: bismillah="[^"]*")? />', sm.group(2)):
            a, data = int(am.group(1)), am.group(2)
            assert b"&" not in data and parsed[(s, a)] == data.decode("utf-8")
            structure[str(s)][1] += 1
            n += 1
            verse("quran", edition, str(s), 1, a, data, LIC_TANZIL,
                  f"The text of surah {s}, ayah {a} of the Quran (chapter 1 by convention) in the Tanzil Quran Text ({label}, Version 1.1, "
                  f"February 2021, https://tanzil.net), XML form: the aya's text as published; verse 1 is without the bismillah, which precedes "
                  f"surahs 2-8 and 10-114. {notice}. Permission is granted to copy and distribute verbatim copies of this text, but CHANGING IT "
                  f"IS NOT ALLOWED. Source: Tanzil Project, https://tanzil.net (updates: http://tanzil.net/updates/). Licence: {LIC_TANZIL}.")
    assert n == len(parsed) == 6236, (n, len(parsed))
    if first is None:
        first = structure
        extents("quran", edition, structure, "the Tanzil Quran Text 1.1 (the same in its Uthmani and Simple editions)", LIC_TANZIL)
    else:
        assert structure == first
        report["editions"]["quran/simple"].update(books=len(structure), chapters=len(structure))

# ---- uxlc-consonantal: the strip_niqqud formula, with the characters of the spec vector ----
vec = json.load(open(os.path.join(spec, "vectors/formula-eval/hebrew-consonantal.json"), encoding="utf-8"))
vbody = next(iter(vec["world"]["functions"].values()))["descriptor"]["code"]["body"]
chars = vbody.rsplit(', "', 1)[1].split('"')[0]
assert len(chars) == 51 and MAQAF not in chars and "׀" not in chars and "׃" not in chars and "׆" not in chars
lines.append({"formula": f"{NS}/strip_niqqud", "bound": TX,
              "signature": sig({"book": str_arg, "chapter": num_arg, "verse": num_arg}, ["book", "chapter", "verse"], "string"),
              "source": f'remove_chars(text_of({NS}/verse_text(corpus: "tanakh", book: book, chapter: chapter, verse: verse, edition: "uxlc")), "{chars}")'})

with open(os.path.join(out, "scripture.jsonl"), "w", encoding="utf-8") as f:
    for l in lines:
        f.write(json.dumps(l, ensure_ascii=False, sort_keys=True) + "\n")
report["cells"] = sum("cell" in l for l in lines)
report["verses"] = sum(e["verses"] for e in report["editions"].values())
report["bytes"] = sum(e["bytes"] for e in report["editions"].values())
report["blobs"] = len(os.listdir(os.path.join(out, "blobs")))
json.dump(report, open(os.path.join(out, "scripture-report.json"), "w"), indent=1)
print(json.dumps(report, indent=1))
