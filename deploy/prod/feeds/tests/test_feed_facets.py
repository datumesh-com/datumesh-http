"""CONTRACT-round-0.7 R6: each feed writes its facets (the C6 table, with scripture/edition measure `provenance` and calendar/*
measure `date`) and its `source_name` (R8) in its OWN catalogue lines. Own data names no source. Facet values follow the C2
bounds: keys among domain, thing, measure, region, frequency; 1 to 8 values a key; each value at most 32 bytes; values
ascending and unique. `source_name` is plain words of 1 to 48 bytes.

facets_c6.py (the scratch converter) goes: nothing here reads it."""
import json
import subprocess
import sys

import pytest

import chirps
import register as reg
from conftest import FEEDS, REHEARSAL, fixture
from helpers import split_lines
from sec_helpers import CIK_CAT, item
from test_chirps import RECT, grid, src as chirps_src
from test_relay_sources import emit_all, parsed
from test_register import entries
from specs import SPECS

FACET_KEYS = ("domain", "thing", "measure", "region", "frequency")


def F(domain, thing, measure, region, frequency):
    return {k: [v] for k, v in zip(FACET_KEYS, (domain, thing, measure, region, frequency))}


C6 = {
    "scripture/verse": F("scripture", "verse", "text", "world", "static"),
    "scripture/edition": F("scripture", "edition", "provenance", "world", "static"),
    "register/dataset": F("registry", "dataset", "licence", "world", "on_change"),
    "relay/ecb.europa.eu/fx_reference": F("finance", "currency", "exchange_rate", "ea", "daily"),
    "relay/ecb.europa.eu/estr": F("finance", "rate", "interest_rate", "ea", "daily"),
    "relay/ecb.europa.eu/policy_rate": F("finance", "rate", "interest_rate", "ea", "on_change"),
    "relay/bankofengland.co.uk/bank_rate": F("finance", "rate", "interest_rate", "gb", "daily"),
    "relay/treasury.gov/par_yield": F("finance", "yield", "yield", "us", "daily"),
    "earth/monthly_rainfall": F("earth", "region", "rainfall", "world", "monthly"),
    "sec/company": F("finance", "company", "fundamentals", "us", "annual"),
    "relay/sec.gov/company": F("finance", "company", "fundamentals", "us", "annual"),
    "relay/sec.gov/filing": F("finance", "filing", "fundamentals", "us", "annual"),
}
RATIO = F("tools", "none", "ratio", "none", "static")
CALENDAR = F("tools", "none", "date", "none", "static")
SOURCE_NAMES = {"ecb_fx": "the ECB", "estr": "the ECB", "policy_rates": "the ECB", "boe": "the Bank of England",
                "treasury": "the US Treasury", "chirps": "CHIRPS", "sec": "SEC filings"}


def check_bounds(facets):
    """C2: keys only domain/thing/measure/region/frequency; 1 to 8 values; each at most 32 bytes; ascending, unique."""
    assert isinstance(facets, dict) and facets and set(facets) <= set(FACET_KEYS), facets
    for k, vs in facets.items():
        assert isinstance(vs, list) and 1 <= len(vs) <= 8, (k, vs)
        assert all(isinstance(v, str) and 0 < len(v.encode()) <= 32 for v in vs), (k, vs)
        assert vs == sorted(set(vs)), (k, "ascending and unique")


def check_source_name(name):
    assert isinstance(name, str) and name.strip() and 1 <= len(name.encode()) <= 48, name


def catalog_line(lines, full):
    (c,) = [l for l in lines if l.get("catalog") == full]
    return c


def check_entry(c, short, source_name):
    assert c["facets"] == C6[short], (short, c.get("facets"))
    check_bounds(c["facets"])
    if source_name is None:
        assert "source_name" not in c, "the publisher's own data names no source"
    else:
        assert c["source_name"] == source_name
        check_source_name(c["source_name"])


# ---------- the relay sources ----------

RELAY_SHORT = {"ecb_fx": "relay/ecb.europa.eu/fx_reference", "estr": "relay/ecb.europa.eu/estr",
               "policy_rates": "relay/ecb.europa.eu/policy_rate", "boe": "relay/bankofengland.co.uk/bank_rate",
               "treasury": "relay/treasury.gov/par_yield"}


@pytest.mark.parametrize("key", RELAY_SHORT)
def test_a_relay_feed_writes_its_facets_and_source_name_in_its_catalogue_line(key, names):
    mod, inputs, spec = parsed(key, names)
    lines, _ = emit_all(mod, inputs, {}, names)
    _, cats, _, _ = split_lines(lines)
    assert len(cats) == 1
    short = RELAY_SHORT[key]
    assert cats[0]["catalog"] == f"{REHEARSAL}/{short}"
    check_entry(cats[0], short, SOURCE_NAMES[key])
    assert set(cats[0]) >= {"catalog", "kind", "label", "description", "args", "examples", "dictionary", "facets", "source_name"}


@pytest.mark.parametrize("key", RELAY_SHORT)
def test_a_later_run_writes_no_catalogue_line(key, names):
    mod, inputs, spec = parsed(key, names)
    _, state = emit_all(mod, inputs, {}, names)
    lines, _ = emit_all(mod, inputs, state, names)
    assert not [l for l in lines if "catalog" in l]


# ---------- chirps and the register ----------

def test_chirps_writes_its_facets_and_source_name(names):
    rows = [chirps.region_row("us_kansas", "2024-06-01", grid(), RECT)]
    lines, _ = chirps.emit(rows, {}, names, chirps_src(names))
    _, cats, _, _ = split_lines(lines)
    (c,) = cats
    assert c["catalog"] == f"{REHEARSAL}/earth/monthly_rainfall"
    check_entry(c, "earth/monthly_rainfall", "CHIRPS")


def test_the_register_writes_its_facets_on_the_class_entry_and_no_source_name(names):
    lines, _ = reg.emit(entries(), {}, names, blobs={})
    cats = [l for l in lines if "catalog" in l]
    cls = f"{REHEARSAL}/register/dataset"
    check_entry(catalog_line(cats, cls), "register/dataset", None)
    for c in cats:
        assert "source_name" not in c, "the register is Datumesh's own data"
        if c["catalog"] != cls:
            assert "facets" not in c, "facets are the class's: its methods carry none"


# ---------- the SEC relay ----------

def test_the_sec_relay_gives_the_company_and_filing_classes_their_facets_and_source_name():
    import sec
    sec_names = sec.names(REHEARSAL)
    lines, _ = sec.emit([item(sec_names, CIK_CAT)], {}, sec_names)
    cats = [l for l in lines if "catalog" in l]
    ns = sec_names["sec"]["namespace"]
    check_entry(catalog_line(cats, ns + "/company"), "relay/sec.gov/company", "SEC filings")
    check_entry(catalog_line(cats, ns + "/filing"), "relay/sec.gov/filing", "SEC filings")
    company_fact = catalog_line(cats, ns + "/company/fact")
    assert "facets" not in company_fact, "only the two classes are in the table"


# ---------- the views file ----------

def views():
    return [json.loads(l) for l in (FEEDS / "sec_views.jsonl").read_text(encoding="utf-8").splitlines() if l.strip()]


def test_every_ratio_and_calendar_catalogue_entry_carries_the_tools_facets_and_no_source_name():
    seen = {"ratio": [], "calendar": []}
    for l in views():
        c = l.get("catalog", "")
        for group in seen:
            if c.startswith(f"{REHEARSAL}/{group}/"):
                seen[group].append(c)
                assert l["facets"] == (RATIO if group == "ratio" else CALENDAR), c
                check_bounds(l["facets"])
                assert "source_name" not in l, c
    assert sorted(seen["ratio"]) == sorted(f"{REHEARSAL}/ratio/{n}" for n in ("margin", "percent_change", "cagr", "percent_of_total"))
    assert sorted(seen["calendar"]) == sorted(f"{REHEARSAL}/calendar/{n}" for n in ("age_at", "days_between"))


def test_the_calendar_measure_is_date_and_the_ratio_measure_is_ratio():
    assert CALENDAR["measure"] == ["date"] and RATIO["measure"] == ["ratio"]


# ---------- scripture_prod.py --catalog-only ----------

PROD = FEEDS.parent / "scripture_prod.py"


def run_catalog_only(tmp_path, *args):
    out = tmp_path / "out"
    out.mkdir()
    dl = tmp_path / "no-such-download-dir"   # --catalog-only reads no source file
    r = subprocess.run([sys.executable, "-I", "-B", str(PROD), str(dl), str(out), *args, "--catalog-only"],
                       capture_output=True, text=True, cwd=tmp_path, timeout=120)
    return r, out


def written(out):
    files = sorted(p for p in out.rglob("*") if p.is_file())
    lines = [json.loads(l) for p in files if p.suffix == ".jsonl" for l in p.read_text(encoding="utf-8").splitlines()]
    return files, lines


@pytest.mark.parametrize("args,ns", [((), "datumesh.com/scripture"),
                                     (("datumesh.com",), "datumesh.com/scripture"),
                                     ((REHEARSAL, f"{REHEARSAL}/scratch_r6/scripture"), f"{REHEARSAL}/scratch_r6/scripture")])
def test_scripture_prod_catalog_only_writes_the_two_class_catalogue_lines_and_nothing_else(tmp_path, args, ns):
    r, out = run_catalog_only(tmp_path, *args)
    assert r.returncode == 0, r.stderr
    files, lines = written(out)
    assert all(p.suffix == ".jsonl" for p in files), [p.name for p in files]   # no blobs, no report
    assert all(set(l) >= {"catalog", "kind", "label", "description", "args", "facets"} for l in lines)
    assert all("catalog" in l and not {"cell", "class", "descriptor", "function", "test"} & set(l) for l in lines)
    assert sorted(l["catalog"] for l in lines) == [f"{ns}/edition", f"{ns}/verse"]
    by = {l["catalog"]: l for l in lines}
    for kind in ("verse", "edition"):
        c = by[f"{ns}/{kind}"]
        assert c["kind"] == "class" and c["label"].strip() and len(c["label"].encode()) <= 120
        assert 0 < len(c["description"].encode()) <= 1024
        check_entry(c, f"scripture/{kind}", None)
    assert set(by[f"{ns}/verse"]["args"]) == {"corpus", "book", "chapter", "verse"}
    assert set(by[f"{ns}/edition"]["args"]) == {"corpus", "code"}


def test_scripture_prod_catalog_only_needs_no_download_and_makes_no_blob_directory(tmp_path):
    r, out = run_catalog_only(tmp_path)
    assert r.returncode == 0, r.stderr
    assert not (out / "blobs").exists() or not list((out / "blobs").iterdir())
    assert not (out / "report.json").exists()


def test_scripture_prod_spells_the_two_facet_rows_of_the_table():
    """The full run writes the same catalogue lines (in 00-classes.jsonl, after the class lines): its source carries the
    table row of each, with the measure `provenance` for the edition."""
    text = PROD.read_text(encoding="utf-8")
    assert "--catalog-only" in text and "provenance" in text and "facets" in text
