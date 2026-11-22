# SPDX-License-Identifier: Apache-2.0
"""The production scripture load for datumesh.com (step 1.7a, CONTRACT-1.7a section S): the JSON
lines that `datumesh-mesh publish-stored` (http 0.3.1) loads, from a fresh download checked against
SOURCES, without changing a byte of any verse.

    python -I scripture_prod.py <download dir> <out dir> [<host> [<namespace>]] [--catalog-only]

--catalog-only writes only the two catalogue lines of the classes (00-classes.jsonl): no source is read, no
blob, no report. The full run writes the same two lines in 00-classes.jsonl after the class lines.

<host> defaults to datumesh.com and <namespace> to <host>/scripture; a rehearsal passes a scratch
namespace (e.g. mesh-test.datumesh.com/scratch_1_7a/scripture). Writes, in the order they are to be loaded (the user's order of first writes):

  00-classes.jsonl  descriptor lines for the stored methods, the two count functions, the classes
                    <host>/scripture/verse and <host>/scripture/edition
  10-tanakh.jsonl   Genesis 1:1 first, every other UXLC verse, the Tanakh counts, edition(tanakh, uxlc)
  20-nt.jsonl       the SBLGNT verses, the NT counts, edition(nt, sblgnt)
  30-quran.jsonl    the Tanzil uthmani verses, then simple, the Quran counts, both editions
  blobs/<sha256>    every blob a value names
  report.json       counts, bytes, file hashes, non-NFC counts per edition

No line carries tx, asof or bound: each publish-stored run takes its clock's now (rule A, H4).
Names (the user's decisions of 2026-11-16, permanent): one class verse(corpus, book, chapter,
verse); corpus tanakh | nt | quran; book OSIS exactly, or Quran; for the Quran chapter = surah and
verse = ayah; text(edition) a stored method; editions uxlc, sblgnt, uthmani, simple; counts the
plain functions scripture/chapter_count(corpus, book) and scripture/verse_count(corpus, book,
chapter); the edition class edition(corpus, code) with version, source_url, file_hash (strings),
notice (text blob) and file (the exact source file; not for uxlc).
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
from html.parser import HTMLParser

ARGS = [a for a in sys.argv[1:] if a != "--catalog-only"]
CATALOG_ONLY = len(ARGS) != len(sys.argv) - 1
dl, out = ARGS[0], ARGS[1]
host = ARGS[2] if len(ARGS) > 2 else "datumesh.com"
NS = ARGS[3] if len(ARGS) > 3 else f"{host}/scripture"
HERE = os.path.dirname(os.path.abspath(__file__))
STEWARD = {"namespace": NS, "process": f"https://{host}/classes/change"}
LIC_UXLC = "https://tanach.us/License.html"
LIC_SBLGNT = "CC-BY-4.0"
LIC_TANZIL = "https://tanzil.net/docs/text_license"
SBLGNT_COMMIT = "d7f35adca9ec410e1d1499c63b4dd989d76c8d4d"
MAQAF = "־"
OSIS = {"Gen": "Gen", "Ex": "Exod", "Lev": "Lev", "Num": "Num", "Deut": "Deut", "Josh": "Josh", "Judg": "Judg",
        "1 Sam": "1Sam", "2 Sam": "2Sam", "1 Kings": "1Kgs", "2 Kings": "2Kgs", "Isa": "Isa", "Jer": "Jer",
        "Ezek": "Ezek", "Hos": "Hos", "Joel": "Joel", "Am": "Amos", "Ob": "Obad", "Jon": "Jonah", "Mic": "Mic",
        "Nah": "Nah", "Hab": "Hab", "Zeph": "Zeph", "Hag": "Hag", "Zech": "Zech", "Mal": "Mal", "Ps": "Ps",
        "Prov": "Prov", "Job": "Job", "Song": "Song", "Ruth": "Ruth", "Lam": "Lam", "Eccl": "Eccl", "Esth": "Esth",
        "Dan": "Dan", "Ezra": "Ezra", "Neh": "Neh", "1 Chr": "1Chr", "2 Chr": "2Chr"}
NT = ["Matt", "Mark", "Luke", "John", "Acts", "Rom", "1Cor", "2Cor", "Gal", "Eph", "Phil", "Col", "1Thess", "2Thess",
      "1Tim", "2Tim", "Titus", "Phlm", "Heb", "Jas", "1Pet", "2Pet", "1John", "2John", "3John", "Jude", "Rev"]

# ---- the classes' catalogue lines (the facets of the table; no source_name: the publisher's own data) ----
def _facets(domain, thing, measure, region, frequency):
    return {"domain": [domain], "thing": [thing], "measure": [measure], "region": [region], "frequency": [frequency]}


def catalogue():
    verse_line = {"catalog": f"{NS}/verse", "kind": "class",
                  "label": "Scripture verses: the text of each verse of the Tanakh, the New Testament and the Quran",
                  "description": ("One verse, by corpus (tanakh, nt or quran), book, chapter and verse. The method "
                                  "text(edition) returns the verse's text as a UTF-8 blob in the edition named, byte for "
                                  "byte as the source gives it. Books are OSIS names; for the Quran the chapter is the "
                                  "surah and the verse the ayah. Each cell carries its edition's licence."),
                  "args": {"corpus": "tanakh, nt or quran", "book": "the OSIS book name, or Quran",
                           "chapter": "the chapter number (for the Quran, the surah)",
                           "verse": "the verse number (for the Quran, the ayah)"},
                  "examples": [], "facets": _facets("scripture", "verse", "text", "world", "static")}
    edition_line = {"catalog": f"{NS}/edition", "kind": "class",
                    "label": "Scripture editions: version, source, file hash, notice and source file",
                    "description": ("The edition of a corpus: the Tanakh (UXLC), the New Testament (SBLGNT) or the Quran "
                                    "(Tanzil, Uthmani and Simple). Methods: version, source_url, file_hash (SHA-256 of the "
                                    "file the verses were checked against), notice (the licence or copyright notice, a "
                                    "text blob), and file (the source file itself, a blob; not for UXLC)."),
                    "args": {"corpus": "tanakh, nt or quran", "code": "the edition's code, for example uxlc or sblgnt"},
                    "examples": [], "facets": _facets("scripture", "edition", "provenance", "world", "static")}
    return [verse_line, edition_line]


if CATALOG_ONLY:
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "00-classes.jsonl"), "w", encoding="utf-8") as f:
        for l in catalogue():
            f.write(json.dumps(l, ensure_ascii=False, sort_keys=True) + "\n")
    sys.exit(0)

# ---- the sources, checked against SOURCES (download.sh checked them too; checked again here) ----
SOURCES = {}
for line in open(os.path.join(HERE, "..", "seed", "SOURCES"), encoding="utf-8"):
    if line.strip() and not line.startswith("#"):
        want, name, url = line.split()
        SOURCES[name] = (want, url)
raw = {}
for name, (want, url) in SOURCES.items():
    data = open(os.path.join(dl, name), "rb").read()
    got = hashlib.sha256(data).hexdigest()
    if got != want:
        sys.exit(f"{name}: sha256 {got}, SOURCES says {want}: stop (a new release arrives as corrections)")
    raw[name] = data
H = {name: hashlib.sha256(data).hexdigest() for name, data in raw.items()}

os.makedirs(os.path.join(out, "blobs"), exist_ok=True)
files = {k: [] for k in ("00-classes", "10-tanakh", "20-nt", "30-quran")}
report = {"host": host, "files": {n: {"sha256": H[n], "bytes": len(raw[n]), "url": SOURCES[n][1]} for n in raw}, "editions": {}}


def S(x):
    return {"type": "string", "value": x}


def N(x):
    return {"type": "number", "value": str(x)}


def jcs(v):
    return json.dumps(v, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def put_blob(data):
    h = hashlib.sha256(data).hexdigest()
    p = os.path.join(out, "blobs", h)
    if not os.path.exists(p):
        with open(p, "wb") as f:
            f.write(data)
    return "blobs/" + h


def summary(text):
    assert "\n" not in text and len(text.encode("utf-8")) <= 1024, (len(text.encode("utf-8")), text[:120])
    assert unicodedata.is_normalized("NFC", text), text[:120]
    return text


def cell(file, fn, args, value, licence, text):
    files[file].append({"cell": {"fn": f"{NS}/{fn}", "args": args}, "value": value, "licence": licence,
                        "summary": summary(text)})


# ---- 00: descriptors, count functions, classes ----
STR = {"type": "string", "valid": False}
NUM = {"type": "number", "unit": "1", "valid": False}


def stored(args, order, returns):
    return {"code": {"body": "", "lang": "stored"}, "inputs": [], "signature": {"args": args, "order": order, "returns": returns}}


def fn_hash(d):
    return hashlib.sha256(jcs(d)).hexdigest()


VKEYS = {"corpus": STR, "book": STR, "chapter": NUM, "verse": NUM}
EKEYS = {"corpus": STR, "code": STR}
text_d = stored({**VKEYS, "edition": STR}, ["corpus", "book", "chapter", "verse", "edition"], "blob")
edition_d = {m: stored(dict(EKEYS), ["corpus", "code"], r) for m, r in
             (("version", "string"), ("source_url", "string"), ("file_hash", "string"), ("notice", "blob"), ("file", "blob"))}
for d in [text_d, *edition_d.values()]:
    files["00-classes"].append({"descriptor": d})
files["00-classes"].append({"function": f"{NS}/chapter_count", "signature": {
    "args": {"corpus": STR, "book": STR}, "order": ["corpus", "book"], "returns": {"type": "number", "unit": "count"}}})
files["00-classes"].append({"function": f"{NS}/verse_count", "signature": {
    "args": {"corpus": STR, "book": STR, "chapter": NUM}, "order": ["corpus", "book", "chapter"],
    "returns": {"type": "number", "unit": "count"}}})
none = {"type": "string", "key_normalisation": "none"}
one = {"type": "number", "unit": "1"}
verse_class = {"keys": {"corpus": none, "book": none, "chapter": one, "verse": one},
               "order": ["corpus", "book", "chapter", "verse"], "methods": {"text": fn_hash(text_d)}, "steward": STEWARD}
edition_class = {"keys": {"corpus": none, "code": none}, "order": ["corpus", "code"],
                 "methods": {m: fn_hash(d) for m, d in edition_d.items()}, "steward": STEWARD}
files["00-classes"].append({"class": f"{NS}/verse", "descriptor": verse_class})
files["00-classes"].append({"class": f"{NS}/edition", "descriptor": edition_class})
files["00-classes"] += catalogue()
report["hashes"] = {"verse/text": fn_hash(text_d), **{f"edition/{m}": fn_hash(d) for m, d in edition_d.items()}}


def stats(key, data):
    r = report["editions"].setdefault(key, {"verses": 0, "bytes": 0, "non_nfc": 0, "over_1024_bytes": 0})
    r["verses"] += 1
    r["bytes"] += len(data)
    r["non_nfc"] += not unicodedata.is_normalized("NFC", data.decode("utf-8"))
    r["over_1024_bytes"] += len(data) > 1024


def verse(file, corpus, book, ch, v, edition, data, licence, text):
    data.decode("utf-8")  # strict UTF-8
    stats(f"{corpus}/{edition}", data)
    cell(file, "verse/text", {"corpus": S(corpus), "book": S(book), "chapter": N(ch), "verse": N(v), "edition": S(edition)},
         {"blob": put_blob(data), "media_type": "text/plain"}, licence, text)


def counts(file, corpus, structure, source, licence):
    for book, chs in structure.items():
        cell(file, "chapter_count", {"corpus": S(corpus), "book": S(book)}, {"type": "number", "value": str(len(chs))}, licence,
             f"The number of chapters of {book} in the corpus {corpus}, counted from {source}. Licence {licence}.")
        for ch, n in chs.items():
            cell(file, "verse_count", {"corpus": S(corpus), "book": S(book), "chapter": N(ch)}, {"type": "number", "value": str(n)},
                 licence, f"The number of verses of {book} {ch} in the corpus {corpus}, counted from {source}. Licence {licence}.")


def edition(file, corpus, code, version, url, file_name, notice, licence, media_type=None):
    ref = f"{NS}/edition({corpus}, {code})"
    args = {"corpus": S(corpus), "code": S(code)}
    cell(file, "edition/version", args, {"type": "string", "value": version}, licence,
         f"The version of the edition {code} of the corpus {corpus}: {version}. Licence {licence}.")
    cell(file, "edition/source_url", args, {"type": "string", "value": url}, licence,
         f"The URL the edition {code} of the corpus {corpus} ({version}) was downloaded from, the file {file_name}. Licence {licence}.")
    cell(file, "edition/file_hash", args, {"type": "string", "value": H[file_name]}, licence,
         f"The SHA-256 of the file {file_name} ({len(raw[file_name])} bytes) of the edition {code} of the corpus {corpus} ({version}), "
         f"the file every verse of {ref} was checked against before it was signed. Licence {licence}.")
    assert unicodedata.is_normalized("NFC", notice.decode("utf-8"))
    cell(file, "edition/notice", args, {"blob": put_blob(notice), "media_type": "text/plain"}, licence,
         f"The licence or copyright notice of the edition {code} of the corpus {corpus} ({version}) in full, as its source gives it. Licence {licence}.")
    if media_type:
        cell(file, "edition/file", args, {"blob": put_blob(raw[file_name]), "media_type": media_type}, licence,
             f"The source file {file_name} of the edition {code} of the corpus {corpus} ({version}), byte for byte as downloaded from {url}; "
             f"sha256 {H[file_name]}. Licence {licence}.")


# ---- 10: UXLC ----
ZIP = "Tanach.xml.zip"
z = zipfile.ZipFile(io.BytesIO(raw[ZIP]))
header = z.read("Books/TanachHeader.xml").decode("utf-8")
uv, ub, udate = (re.search(rf"<{t}>([^<]*)</{t}>", header).group(1) for t in ("version", "build", "date"))
assert (uv, ub) == ("UXLC 2.6", "27.7"), (uv, ub)
UVER = f"{uv} (build {ub}, {udate})"


def wtext(el):
    s = el.text or ""
    for ch in el:
        if ch.tag != "x":
            s += wtext(ch)
        s += ch.tail or ""
    return s


def uxlc_verse(v):
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
    return text


class Second(HTMLParser):
    """A second, independent reading of a UXLC book file (the standard library's HTML/SGML tokenizer,
    not ElementTree): {(chapter, verse): text} by the same joining rule."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.out, self.c, self.v, self.stack, self.parts, self.cur = {}, None, None, [], None, None

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "c":
            self.c = int(a["n"])
        elif tag == "v":
            self.v, self.parts = int(a["n"]), []
        elif self.parts is not None:
            if tag in ("w", "k", "q") and not self.stack:
                self.cur = [{"w": "", "k": "*", "q": "**"}[tag]]
            elif tag == "reversednun" and not self.stack:
                self.parts.append((" ׆", False))
            if tag in ("w", "k", "q", "x") or self.stack:
                self.stack.append(tag)

    def handle_startendtag(self, tag, attrs):
        if self.parts is not None and tag == "reversednun" and not self.stack:
            self.parts.append((" ׆", False))

    def handle_endtag(self, tag):
        if tag == "v" and self.parts is not None:
            text, glue = "", True
            for t, word in self.parts:
                if word and text and not glue:
                    text += " "
                text += t
                glue = t.endswith(MAQAF) if word else False
            self.out[(self.c, self.v)] = text
            self.parts = None
        elif self.stack and self.stack[-1] == tag:
            self.stack.pop()
            if not self.stack and self.cur is not None:
                self.parts.append(("".join(self.cur), True))
                self.cur = None

    def handle_data(self, d):
        if self.cur is not None and "x" not in self.stack:
            self.cur.append(d)


index = ET.fromstring(z.read("Books/TanachIndex.xml"))
structure, tanakh = {}, []
for b in index.iter("book"):
    fname = b.find(".//filename").text
    data = z.read(f"Books/{fname}.xml")
    bk = ET.fromstring(data).find(".//tanach/book")
    abbrev, name = bk.find("names/abbrev").text, bk.find("names/name").text
    osis = OSIS[abbrev]
    second = Second()
    second.feed(data.decode("utf-8"))
    structure[osis] = {}
    for c in bk.findall("c"):
        cn = int(c.get("n"))
        for v in c.findall("v"):
            vn = int(v.get("n"))
            text = uxlc_verse(v)
            assert second.out.get((cn, vn)) == text, (osis, cn, vn, second.out.get((cn, vn)), text)
            structure[osis][cn] = structure[osis].get(cn, 0) + 1
            tanakh.append((osis, cn, vn, text.encode("utf-8"),
                           f"The text of {name} {cn}:{vn} (OSIS {osis}.{cn}.{vn}) in the Unicode/XML Leningrad Codex, {UVER}, "
                           f"tanach.us; file {ZIP} sha256 {H[ZIP]}. Pointed and accented, as UTF-8: the verse's words in Books/{fname}.xml "
                           f"joined by spaces (none after a maqaf; a ketiv marked * and its qere **; notes and paragraph marks omitted). "
                           f"It is not the Westminster Leningrad Codex. Notice: {NS}/edition(tanakh, uxlc).notice(). Licence: {LIC_UXLC}."))
    assert len(second.out) == sum(structure[osis].values()), osis
assert tanakh[0][:3] == ("Gen", 1, 1)
for osis, cn, vn, data, text in tanakh:  # Genesis 1:1 is first: the first cell written to the store
    verse("10-tanakh", "tanakh", osis, cn, vn, "uxlc", data, LIC_UXLC, text)
counts("10-tanakh", "tanakh", structure, f"the Unicode/XML Leningrad Codex {UVER} (sha256 {H[ZIP]})", LIC_UXLC)


class Text(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.parts, self.skip = [], 0

    # tanach.us opens with <header> and closes with </head>: only script, style and title are skipped.
    def handle_starttag(self, tag, attrs):
        self.skip += tag in ("script", "style", "title")
        if tag in ("p", "br", "div", "h1", "h2", "h3", "li", "tr", "center"):
            self.parts.append("\n")

    def handle_endtag(self, tag):
        self.skip = max(0, self.skip - (tag in ("script", "style", "title")))

    def handle_data(self, d):
        if not self.skip:
            self.parts.append(d)


t = Text()
t.feed(open(os.path.join(dl, "uxlc-license.html"), encoding="utf-8").read())
lines = [re.sub(r"\s+", " ", x).strip() for x in "".join(t.parts).split("\n")]
uxlc_notice = "\n".join(x for x in lines if x)
assert "without restriction" in uxlc_notice and "not the Westminster Leningrad Codex" in uxlc_notice, uxlc_notice
uxlc_notice = (f"{uxlc_notice}\n\n(Quoted from {LIC_UXLC}, fetched with the download of {ZIP}; the file itself carries no notice.)\n")
edition("10-tanakh", "tanakh", "uxlc", UVER, SOURCES[ZIP][1], ZIP, unicodedata.normalize("NFC", uxlc_notice).encode("utf-8"), LIC_UXLC)
report["editions"]["tanakh/uxlc"].update(books=len(structure), chapters=sum(len(c) for c in structure.values()))

# ---- 20: SBLGNT ----
TGZ = f"sblgnt-{SBLGNT_COMMIT}.tar.gz"
tf = tarfile.open(fileobj=io.BytesIO(raw[TGZ]))
prefix = f"SBLGNT-{SBLGNT_COMMIT}/"
readme = tf.extractfile(prefix + "README.md").read().decode("utf-8")
licence_file = tf.extractfile(prefix + "LICENSE").read().decode("utf-8")
assert "<td>v1.2</td><td>2023-07-10</td>" in readme
SVER = f"SBLGNT v1.2 (2023-07-10), github.com/Faithlife/SBLGNT commit {SBLGNT_COMMIT}"
structure = {}
for book in NT:
    data = tf.extractfile(prefix + f"data/sblgnt/text/{book}.txt").read()
    assert not data.startswith(b"\xef\xbb\xbf")
    structure[book] = {}
    seen = 0
    for line in data.split(b"\n"):
        m = re.match(rb"^(\S+) (\d+):(\d+)\t(.*)$", line, re.S)
        if not m:
            continue
        assert m.group(1).decode() == book and not line.endswith(b"\r")
        cn, vn, vb = int(m.group(2)), int(m.group(3)), m.group(4)
        assert line == f"{book} {cn}:{vn}\t".encode() + vb  # the blob is the line's bytes after the tab, exactly
        structure[book][cn] = structure[book].get(cn, 0) + 1
        seen += 1
        verse("20-nt", "nt", book, cn, vn, "sblgnt", vb, LIC_SBLGNT,
              f"The text of {book} {cn}:{vn} in the SBL Greek New Testament (SBLGNT) v1.2 (2023-07-10; github.com/Faithlife/SBLGNT commit "
              f"{SBLGNT_COMMIT}; file {TGZ} sha256 {H[TGZ]}), data/sblgnt/text/{book}.txt: the line's bytes after the tab, as published. "
              f"Copyright 2010 Society of Biblical Literature and Logos Bible Software. Notice: {NS}/edition(nt, sblgnt).notice(). "
              f"Licence CC-BY-4.0.")
    assert seen == len(re.findall(rb"(?m)^\S+ \d+:\d+\t", data)), book
counts("20-nt", "nt", structure, f"{SVER} (sha256 {H[TGZ]})", LIC_SBLGNT)
sec = readme[readme.index("## License"):]
nxt = re.search(r"\n## ", sec[3:])
sblgnt_notice = (f"From README.md of {SVER}:\n\n{sec[:nxt.start() + 3] if nxt else sec}".rstrip()
                 + f"\n\n----- LICENSE (in the same tarball) -----\n\n{licence_file}")
edition("20-nt", "nt", "sblgnt", SVER, SOURCES[TGZ][1], TGZ, unicodedata.normalize("NFC", sblgnt_notice).encode("utf-8"), LIC_SBLGNT,
        "application/gzip")
report["editions"]["nt/sblgnt"].update(books=len(structure), chapters=sum(len(c) for c in structure.values()))

# ---- 30: Tanzil ----
first = None
for code, label in (("uthmani", "Uthmani"), ("simple", "Simple")):
    XML = f"quran-{code}.xml"
    data = raw[XML]
    head = data[:data.index(b"-->")].decode("utf-8")
    block = head[head.index("# PLEASE"):head.rindex("#===") + len("#" + "=" * 68)]
    block = block[:block.index("\n", block.rindex("#===")) if "\n" in block[block.rindex("#==="):] else len(block)].rstrip()
    assert f"Tanzil Quran Text ({label}, Version 1.1)" in block and "CHANGING IT IS NOT ALLOWED" in block, block
    copyright_line = re.search(r"Copyright \(C\) [^\n]*", block).group(0).strip()
    licence_line = re.search(r"License: [^\n]*", block).group(0).strip()
    body = data[data.index(b"<quran>"):]
    parsed = {(int(s.get("index")), int(a.get("index"))): a.get("text") for s in ET.fromstring(body).iter("sura") for a in s.iter("aya")}
    structure, n = {"Quran": {}}, 0
    for sm in re.finditer(rb'<sura index="(\d+)" name="[^"]*">(.*?)</sura>', body, re.S):
        s = int(sm.group(1))
        structure["Quran"][s] = 0
        for am in re.finditer(rb'<aya index="(\d+)" text="([^"]*)"(?: bismillah="[^"]*")? />', sm.group(2)):
            a, vb = int(am.group(1)), am.group(2)
            assert b"&" not in vb and parsed[(s, a)] == vb.decode("utf-8")  # two readings agree, byte for byte
            structure["Quran"][s] += 1
            n += 1
            verse("30-quran", "quran", "Quran", s, a, code, vb, LIC_TANZIL,
                  f"The text of surah {s}, ayah {a} of the Quran in the Tanzil Quran Text ({label}, Version 1.1; file {XML} sha256 {H[XML]}), "
                  f"from the Tanzil Project, https://tanzil.net: the aya's text as published, byte for byte; the bismillah that precedes "
                  f"surahs 2-8 and 10-114 is not part of ayah 1. {copyright_line}. {licence_line}. Permission is granted to copy and "
                  f"distribute verbatim copies of this text, but CHANGING IT IS NOT ALLOWED. Full notice: {NS}/edition(quran, {code}).notice(). "
                  f"Updates: http://tanzil.net/updates/. Licence: {LIC_TANZIL}.")
    assert n == len(parsed) == 6236 and len(structure["Quran"]) == 114, (n, len(parsed))
    if first is None:
        first = structure
    else:
        assert structure == first
    report["editions"][f"quran/{code}"].update(books=1, chapters=114)
    files["30-quran"].append(("edition", code, label, XML, block))
counts_q = []
eds = [x for x in files["30-quran"] if isinstance(x, tuple)]
files["30-quran"] = [x for x in files["30-quran"] if not isinstance(x, tuple)]
counts("30-quran", "quran", first, "the Tanzil Quran Text, Version 1.1 (its Uthmani and Simple editions agree)", LIC_TANZIL)
for _, code, label, XML, block in eds:
    edition("30-quran", "quran", code, f"Tanzil Quran Text ({label}, Version 1.1)", SOURCES[XML][1], XML,
            unicodedata.normalize("NFC", block + "\n").encode("utf-8"), LIC_TANZIL, "application/xml")

# ---- write ----
for name, lines_ in files.items():
    with open(os.path.join(out, f"{name}.jsonl"), "w", encoding="utf-8") as f:
        for l in lines_:
            f.write(json.dumps(l, ensure_ascii=False, sort_keys=True) + "\n")
report["lines"] = {k: len(v) for k, v in files.items()}
report["cells"] = sum(1 for v in files.values() for l in v if "cell" in l)
report["blobs"] = len(os.listdir(os.path.join(out, "blobs")))
json.dump(report, open(os.path.join(out, "report.json"), "w"), indent=1)
print(json.dumps(report, indent=1))
