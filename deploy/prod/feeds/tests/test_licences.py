"""licences.toml: the licence clauses of the register are verbatim quotes of the licence check (LICENCES.md)."""
import json

import common
import register as reg
from conftest import fixture
from helpers import NAME

RAW = fixture("LICENCES.md")


def licences():
    return common.licences()


def entries():
    return reg.parse(fixture("register.json"))


def test_every_quote_is_in_licences_md_byte_for_byte():
    for e in licences():
        assert e["quotes"], (e["source"], e["code"])
        for q in e["quotes"]:
            assert isinstance(q, str) and q and q == q.strip() and "\n" not in q
            assert q.encode("utf-8") in RAW, (e["source"], e["code"], q[:80])


def test_one_entry_per_register_dataset_with_the_registers_page(names):
    got = {(e["source"], e["code"]): e for e in licences()}
    assert len(got) == len(licences()) == 8
    inputs = {r["source_domain"]: r for r in entries()}
    for d in names["register"]["datasets"]:
        e = got[(d["source"], d["code"])]
        assert e["page"] == inputs[d["input"]]["licence_url"] and e["page"].startswith("https://")
        assert ("reason" in e) == (d["status"] == "not_relayed")
        assert "reason" not in d, "the reason moved to licences.toml"
        if "reason" in e:
            assert e["reason"].strip()


def test_the_quotes_cover_what_each_licence_grants_and_asks():
    got = {(e["source"], e["code"]): e["quotes"] for e in licences()}
    join = lambda k: " ".join(got[k])
    for k in (("ecb.europa.eu", "reference_rates"), ("ecb.europa.eu", "policy_rates"), ("ecb.europa.eu", "estr")):
        t = join(k)
        assert "make free use of the information" in t and "must be cited as the source" in t
        assert "this must be stated explicitly" in t  # modification
    assert "The ECB does not charge for the €STR or license its use." in got[("ecb.europa.eu", "estr")]
    assert "adapt the Information; exploit the Information commercially" in join(("bankofengland.co.uk", "bank_rate"))
    assert "Contains public sector information licensed under the Open Government Licence v3.0." in join(("bankofengland.co.uk", "bank_rate"))
    assert "not available for any work of the United States Government" in join(("treasury.gov", "par_yield_curve"))
    assert "CHIRPS3 is in the public domain" in join(("chc.ucsb.edu", "chirps"))
    assert "non-commercial" in join(("fao.org", "asis"))
    assert "non-commercial" in join(("worldbank.org", "commodity_prices"))


def test_licence_clauses_is_a_text_blob_of_the_quotes_and_nothing_else(names):
    from test_register import blob_text
    cls = f"{names['register']['namespace']}/{names['register']['class']}"
    blobs = {}
    lines, _ = reg.emit(entries(), {}, names, blobs=blobs)
    cells = {(c["cell"]["args"]["source"]["value"], c["cell"]["args"]["code"]["value"]): c
             for c in lines if "cell" in c and c["cell"]["fn"] == f"{cls}/licence_clauses"}
    assert len(cells) == 8
    inputs = {r["source_domain"]: r for r in entries()}
    over = 0
    for d in names["register"]["datasets"]:
        e = {(x["source"], x["code"]): x for x in licences()}[(d["source"], d["code"])]
        v = cells[(d["source"], d["code"])]["value"]
        text = blob_text(v, blobs)
        want = " | ".join(e["quotes"])
        if "reason" in e:
            want = f"Not relayed. Reason: {e['reason']} Clauses: {want}"
        assert text == want and blobs[v["blob"]] == want.encode("utf-8")
        over += len(want.encode("utf-8")) > 1024
        for cond in inputs[d["input"]]["conditions"]:
            assert cond not in text
    assert over >= 1, "the reason this is a blob: a clauses text over the 1024-byte string limit"


def test_the_clauses_method_returns_a_blob_and_the_others_keep_their_types(names):
    lines, _ = reg.emit(entries(), {}, names, blobs={})
    returns = {m: l["descriptor"]["signature"]["returns"] for m, l in zip(names["register"]["methods"], lines[:6])}
    assert returns == {"name": "string", "status": "string", "licence_page": "string", "licence_hash": "string",
                       "licence_clauses": "blob", "retrieved": "instant"}


def test_every_string_value_fits_the_1024_byte_limit(names):
    lines, _ = reg.emit(entries(), {}, names, blobs={})
    n = 0
    for l in lines:
        if "cell" in l and l["value"].get("type") == "string":
            n += 1
            assert len(l["value"]["value"].encode("utf-8")) <= 1024, (l["cell"]["fn"], l["cell"]["args"])
    assert n == 8 * 4
