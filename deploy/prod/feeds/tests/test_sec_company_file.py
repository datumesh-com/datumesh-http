"""CONTRACT-sec-classes C3: sec.run writes a second file, sec_company-<stamp>.jsonl, with the stored methods of the
subclass <host>/sec/company: (R13, one step) the class line with its methods inline, then their catalogue entries, cells
and tests. The relay file holds nothing under <host>/sec/. Labels come from the tickers file the scope
rule already fetches (no new request): the title, each ticker, and a short name; ambiguity (after folding) drops a label; labels are
folded (K8: casefold, NFC, white space collapsed).

Run over the synthetic world of sec_class_helpers. The reference label rules below are written from the contract, not
copied from an implementation."""
import json
import re
import shutil
import unicodedata

import pytest

import relay
import sec
from helpers import check_summary
from sec_helpers import (FakeClock, annual_fn, args_of, check_order_all, dirs, item, kinds, read_lines, sec_company)
from sec_helpers import names  # noqa: F401
from sec_class_helpers import (AMBIGUOUS, ATHENE, ATHENE_10KA, FIRST_THREE, Fetcher, LABEL_CASES, NOW, WORLD, World, company_file, fold,
                               is_nfc, key_encoding, run_world)

PUBLIC = "public:https://www.sec.gov/about/privacy-information"
STAMP = relay._stamp(NOW)
S = {"type": "string", "valid": False}

WORDS = ["Inc", "Inc.", "Incorporated", "Corp", "Corp.", "Corporation", "Co", "Co.", "Company", "Ltd", "Ltd.", "Limited",
         "plc", "PLC", "N.V.", "NV", "S.A.", "SA", "AG", "SE", "LP", "L.P.", "LLC", "& Co", "& Co."]


def ref_short(title):
    """(c) of the contract: trailing legal-form words removed repeatedly (case-insensitive whole words, the longest first so
    that `& Co` goes before `Co`), a trailing /XX/ removed, then trailing spaces, commas and periods trimmed."""
    t = title
    words = sorted(WORDS, key=len, reverse=True)
    while True:
        before = t
        t = t.rstrip()
        m = re.search(r"\s*/[A-Z]{2}/$", t)
        if m:
            t = t[:m.start()]
        for w in words:
            m = re.search(r"(?i)(?:^|\s)" + re.escape(w) + r"$", t)
            if m:
                t = t[:m.start()]
                break
        if t == before:
            break
    return t.rstrip(" ,.")


def ref_labels(cos):
    """label -> set of ciks, from titles, tickers and short names (a short name that is empty or the title adds nothing)."""
    out = {}
    for c in cos:
        labels = [c.title, *c.tickers]
        s = ref_short(c.title)
        if s and s != c.title:
            labels.append(s)
        for l in labels:
            out.setdefault(l, set()).add(c.cik10)
    return out


REF = ref_labels(WORLD)
REF_AMBIGUOUS = {l: sorted(c) for l, c in REF.items() if len(c) > 1}


def ref_folded(cos):
    """K8: label -> ciks after folding (casefold, NFC, white space collapsed): the lookup's labels are these."""
    out = {}
    for l, ciks in ref_labels(cos).items():
        out.setdefault(fold(l), set()).update(ciks)
    return out


FOLDED = ref_folded(WORLD)
FOLDED_AMBIGUOUS = {l: sorted(c) for l, c in FOLDED.items() if len(c) > 1}
NFD_CIK = "0001000030"


def test_the_reference_agrees_with_the_cases_of_the_world():
    for cik, title, tickers, short in LABEL_CASES:
        if cik == 1000030:
            continue  # the NFD title: only "every label is NFC" is pinned
        got = ref_short(title)
        assert (got if got and got != title else None) == short, title
    assert REF_AMBIGUOUS == AMBIGUOUS


@pytest.fixture(scope="module")
def ran(names, tmp_path_factory):
    fc = FakeClock()
    path, f, s, o = run_world(names, tmp_path_factory.mktemp("cf"), fc=fc)
    cp = company_file(o)
    return {"path": path, "cpath": cp, "f": f, "s": s, "o": o, "lines": read_lines(path),
            "clines": read_lines(cp) if cp.exists() else [], "names": names}


def fn(names, m):
    return f"{sec_company(names)}/{m}"


def cells(ran, m):
    return [l for l in ran["clines"] if "cell" in l and l["cell"]["fn"] == fn(ran["names"], m)]


def labels_of(ran):
    return {args_of(c)["label"]: c["value"]["value"] for c in cells(ran, "from_label")}


# ---------- the two files ----------

def test_the_run_writes_the_relay_file_and_the_company_file_and_returns_the_relay_path(ran):
    o = ran["o"]
    assert ran["path"] == o / f"sec-{STAMP}.jsonl"
    assert ran["cpath"] == o / f"sec_company-{STAMP}.jsonl" and ran["cpath"].exists()
    assert sorted(p.name for p in o.iterdir() if p.suffix == ".jsonl") == [f"sec-{STAMP}.jsonl", f"sec_company-{STAMP}.jsonl"]
    check_order_all(ran["lines"])
    check_order_all(ran["clines"])


def test_the_relay_file_names_nothing_under_sec_and_no_file_says_company_fact(names, ran):
    own = names["meta"]["host"] + "/sec/"
    for l in ran["lines"]:
        name = l.get("function") or l.get("class") or l.get("catalog") or l.get("test") or (l["cell"]["fn"] if "cell" in l else "")
        assert not name.startswith(own), name
    for l in ran["lines"] + ran["clines"]:
        assert not re.search(r"company_fact(?!s)", json.dumps(l)), l


def test_the_company_file_has_only_names_of_the_subclass(names, ran):
    ns = sec_company(names) + "/"
    assert ran["clines"]
    for l in ran["clines"]:
        name = l.get("class") or l.get("catalog") or l.get("test") or l["cell"]["fn"]
        assert name == ns[:-1] or name.startswith(ns), name
    k = kinds(ran["clines"])
    assert len(k["descriptor"]) == 0 and len(k["class"]) == 1 and len(k["function"]) == 0, (
        "R13: one step: the class line with its methods inline, and no function line (test_sec_one_step.py pins it)")


# ---------- annual_report ----------

def test_annual_report_cells_moved_here_unsourced_and_our_selection(names, ran):
    cs = cells(ran, "annual_report")
    assert len(cs) == 2 * (len(WORLD) - 1) + 1, "fiscal 2023 and 2022 for the synthetic companies, fiscal 2020 for Athene"
    got = {(args_of(c)["cik"], args_of(c)["fiscal_year"]): c["value"] for c in cs}
    assert got[(ATHENE.cik10, "2020")] == {"type": "string", "value": ATHENE_10KA}, "the 10-K/A, filed later"
    assert got[(WORLD[0].cik10, "2023")] == {"type": "string", "value": WORLD[0].accn}
    assert not [l for l in ran["lines"] if "cell" in l and l["cell"]["fn"].endswith("/annual_report")]
    for c in cs:
        assert set(c) == {"cell", "value", "licence", "summary"}, "no source: trust primary, signed by the operator"
        assert c["licence"] == PUBLIC
        assert list(c["cell"]["args"]) == ["cik", "fiscal_year"]
        assert c["cell"]["args"]["fiscal_year"]["type"] == "number" and c["value"]["type"] == "string"
        t = c["summary"]
        check_summary(t)
        assert "selection" in t and "Datumesh" in t and "SEC" in t and "relayed by datumesh.com" not in t


def test_annual_report_tests_are_todays_pins_under_the_new_name(names, ran):
    w = World()
    its = [item(names, c.cik10, data=w.facts(c)) for c in WORLD]
    want = sec.annual_report_tests(its, names)
    assert want and all(t["test"] == annual_fn(names) == fn(names, "annual_report") for t in want)
    got = [t for t in ran["clines"] if "test" in t and t["test"] == fn(names, "annual_report")]
    assert sorted(json.dumps(t, sort_keys=True) for t in got) == sorted(json.dumps(t, sort_keys=True) for t in want)


# ---------- loaded ----------

def test_loaded_is_true_for_each_company_in_scope_primary_with_the_sec_licence(names, ran):
    cs = cells(ran, "loaded")
    assert sorted(args_of(c)["cik"] for c in cs) == sorted(c.cik10 for c in WORLD), "the N loaded companies"
    for c in cs:
        assert c["value"] == {"type": "boolean", "value": True}
        assert c["cell"]["args"] == {"cik": {"type": "string", "value": args_of(c)["cik"]}}
        assert set(c) == {"cell", "value", "licence", "summary"} and c["licence"] == PUBLIC, "primary: no source"
        check_summary(c["summary"])


def test_loaded_tests_are_three_that_give_true(names, ran):
    ts = [t for t in ran["clines"] if "test" in t and t["test"] == fn(names, "loaded")]
    assert len(ts) == 3
    assert sorted(t["call"] for t in ts) == sorted(f'{fn(names, "loaded")}(cik: "{c}")' for c in FIRST_THREE)
    assert all(t["expect"] == {"type": "boolean", "value": True} for t in ts)


# ---------- from_label: derivation ----------

def test_the_labels_are_the_titles_tickers_and_short_names_minus_the_ambiguous_ones(names, ran):
    got = labels_of(ran)
    want = {l: key_encoding(names, next(iter(c))) for l, c in FOLDED.items() if len(c) == 1 and next(iter(c)) != NFD_CIK}
    assert {l: v for l, v in got.items() if not v.endswith(f'"{NFD_CIK}")')} == want, "folded labels (K8)"


@pytest.mark.parametrize("cik,title,tickers,short", [c for c in LABEL_CASES if c[0] != 1000030], ids=lambda x: str(x)[:24])
def test_each_kind_of_label_resolves_to_the_company(names, ran, cik, title, tickers, short):
    got = labels_of(ran)
    enc = key_encoding(names, f"{cik:010d}")
    assert got[fold(title)] == enc, "(a) the title as listed, folded"
    for t in tickers:
        assert got[fold(t)] == enc, "(b) each ticker as listed, folded"
    if short is None:
        assert not [l for l, v in got.items() if v == enc and l not in (fold(title), *map(fold, tickers))], "(c) adds nothing"
    else:
        assert got[fold(short)] == enc, "(c) the short name, folded"


def test_short_name_cases_the_contract_names(names, ran):
    got = labels_of(ran)
    assert "nvidia" in got and "nvidia corp" in got and "nvda" in got
    assert "acme" in got and "acme corp /de/" in got and "acme corp" not in got, "the /DE/ suffix goes with the word"
    assert got["foo"].endswith('"0001000002")') and got["bar"].endswith('"0001000003")'), "repeated: Co Inc, Corp Ltd."
    assert got["meta platforms"].endswith('"0001326801")'), "a trailing comma is trimmed"
    assert got["jpmorgan chase"].endswith('"0000019617")') and got["eli lilly"].endswith('"0000059478")'), "& CO and & Co"
    assert "incredible" in got, "whole words only: Inc is not a prefix"
    assert got["tesco"].endswith('"0001000005")') and "tes" not in got, "Co is not a suffix of a word"
    assert "" not in got and "corp." in got and "corp" not in got, "an empty short name adds nothing"


def test_a_label_of_more_than_one_cik_is_emitted_for_none_and_listed_in_the_report(names, ran):
    got = labels_of(ran)
    for l in FOLDED_AMBIGUOUS:
        assert l not in got, l
    assert "delta corp" in got and "delta inc" in got and "orion" not in got and "orion inc" in got
    rep = json.loads((ran["o"] / f"sec-scope-{STAMP}.json").read_text(encoding="utf-8"))
    assert rep["left_out"]["ambiguous_labels"] == FOLDED_AMBIGUOUS, "listed by the folded label"
    assert set(FOLDED_AMBIGUOUS) == {"delta", "orion", "sun"}
    assert FOLDED_AMBIGUOUS["sun"] == ["0001000044", "0001000045", "0001000046"], "SUN, SUN Inc and Sun Corp fold together"


def test_labels_are_folded_so_case_and_extra_spaces_never_make_two_labels(names, ran):
    got = labels_of(ran)
    assert all(l == fold(l) for l in got)
    assert "sun" not in got and "SUN" not in got and "Sun" not in got, "the three fold to one label that names three companies"
    assert "apple inc." in got and "apple" in got and "aapl" in got
    assert "nvidia" in got and "Apple Inc." not in got and "AAPL" not in got and "apple  inc." not in got


def test_every_label_is_a_nonempty_nfc_string(ran):
    for l in labels_of(ran):
        assert l and is_nfc(l) and "\n" not in l


def test_values_are_key_encodings_of_the_subclass_and_cells_are_primary_with_the_sec_licence(names, ran):
    cs = cells(ran, "from_label")
    assert cs
    for c in cs:
        assert re.fullmatch(re.escape(names["meta"]["host"]) + r'/sec/company\(cik:"\d{10}"\)', c["value"]["value"])
        assert c["value"]["type"] == "string"
        assert c["cell"]["args"] == {"label": {"type": "string", "value": args_of(c)["label"]}}
        assert set(c) == {"cell", "value", "licence", "summary"} and c["licence"] == PUBLIC
        check_summary(c["summary"])
        assert "company_tickers_exchange" in c["summary"], "the summary says which listing the label came from"
    assert labels_of(ran)["apple inc."] == '%s/sec/company(cik:"0000320193")' % names["meta"]["host"]


def test_labels_cost_no_new_request(names, ran):
    f = ran["f"]
    assert f.urls.count(names["sec"]["tickers_url"]) == 1


# ---------- from_label: tests ----------

def test_label_tests_are_one_per_kind_of_label_of_the_first_three_loaded(names, ran):
    ts = [t for t in ran["clines"] if "test" in t and t["test"] == fn(names, "from_label")]
    want = {}
    for cik in FIRST_THREE:
        co = next(c for c in WORLD if c.cik10 == cik)
        for l in {fold(x) for x in (co.title, *co.tickers, ref_short(co.title))} - {""}:
            want[f'{fn(names, "from_label")}(label: "{l}")'] = key_encoding(names, cik)
    assert {t["call"]: t["expect"] for t in ts} == {c: {"type": "string", "value": v} for c, v in want.items()}
    assert len(ts) == len(want) == 10, "Apple: 3; NVIDIA: 3; JPMORGAN: title, JPM, JPM-PC, short name (folded)"
    for t in ts:
        assert set(t) == {"test", "call", "expect"}


def test_every_test_is_unique_and_in_the_company_file_or_the_relay_file(names, ran):
    calls = [t["call"] for t in ran["clines"] + ran["lines"] if "test" in t]
    assert len(calls) == len(set(calls))
    assert all(t["test"].startswith(sec_company(names) + "/") for t in ran["clines"] if "test" in t)


# ---------- catalogue ----------

def cat(lines, name):
    (c,) = [l for l in lines if l.get("catalog") == name]
    return c


def test_the_company_file_catalogue_has_one_entry_for_each_function(names, ran):
    cats = [l for l in ran["clines"] if "catalog" in l]
    assert sorted(c["catalog"] for c in cats) == sorted([sec_company(names)] + [fn(names, m) for m in ("annual_report", "loaded", "from_label")])
    cats = [c for c in cats if c["catalog"] != sec_company(names)]   # the class's own entry: test_sec_one_step.py
    for c in cats:
        assert c["kind"] == "function" and c["label"].strip() and c["description"].strip() and c["examples"]
        assert 0 < len(c["description"].encode()) <= 1024 and len(c["label"].encode()) <= 120
    fl = cat(ran["clines"], fn(names, "from_label"))
    assert "case-sensitive" not in fl["description"], "labels ignore case and extra spaces now (K8)"
    assert "case" in fl["description"] and "space" in fl["description"]
    assert set(fl["args"]) == {"label"}
    cs = labels_of(ran)
    for x in fl["examples"]:
        label = x["call"].split(chr(34))[1]
        assert x["call"] == f'{fn(names, "from_label")}(label: "{label}")' and x["expected"] == {"type": "string", "value": cs[label]}
    ld = cat(ran["clines"], fn(names, "loaded"))
    assert set(ld["args"]) == {"cik"}
    for x in ld["examples"]:
        assert x["expected"] == {"type": "boolean", "value": True}
        assert x["call"].startswith(fn(names, "loaded") + '(cik: "')


def test_the_annual_report_catalogue_states_the_selection_rule(names, ran):
    c = cat(ran["clines"], annual_fn(names))
    assert set(c["args"]) == {"cik", "fiscal_year"}
    d = c["description"]
    for part in ("10-K", "10-K/A", "latest", "filed", "accession", "tie", "fiscal year", "Datumesh"):
        assert part in d, part
    assert not d.startswith("Relayed") and "open_keys" not in c
    assert len(c["examples"]) >= 2
    got = {(args_of(l)["cik"], args_of(l)["fiscal_year"]): l["value"] for l in cells(ran, "annual_report")}
    for e in c["examples"]:
        assert e["call"].startswith(annual_fn(names) + '(cik: "') and 'fiscal_year: "' not in e["call"]
        cik, fy = e["call"].split(chr(34))[1], e["call"].rstrip(")").split("fiscal_year: ")[1]
        assert e["expected"] == got[(cik, fy)]
    nfc = lambda t: unicodedata.normalize("NFC", t)
    assert {e["value"]: nfc(e["label"]) for e in c["dictionary"]["cik"]} == {co.cik10: nfc(co.title) for co in WORLD}


# ---------- a second run, and publishing ----------

def test_a_second_run_with_nothing_new_writes_no_file_of_either_kind(names, ran, tmp_path):
    s, o = tmp_path / "state", tmp_path / "out"
    shutil.copytree(ran["s"], s), shutil.copytree(ran["o"], o)
    fc = FakeClock()
    again = sec.run(names, fetch=Fetcher(names, World(), fc.clock), now="2026-11-21T12:00:00.000000Z", state_dir=s, out_dir=o,
                    sleep=fc.sleep, clock=fc.clock)
    assert again is None
    assert sorted(p.name for p in o.iterdir() if p.suffix == ".jsonl") == [f"sec-{STAMP}.jsonl", f"sec_company-{STAMP}.jsonl"]


def test_publish_gets_the_relay_file_and_then_the_company_file(names, tmp_path):
    seen = []
    path, f, s, o = run_world(names, tmp_path, publish=lambda p: seen.append((p, (s_state(tmp_path)).exists())))
    assert [p for p, _ in seen] == [path, company_file(o)], "the relay is published first, then the subclass's methods"
    assert not any(saved for _, saved in seen), "state is saved only after both returned"
    assert s_state(tmp_path).exists()


def s_state(tmp_path):
    return tmp_path / "state" / "sec.json"


def test_a_failure_of_the_second_publish_removes_both_files_and_keeps_no_state(names, tmp_path):
    calls = []

    def publish(p):
        calls.append(p)
        if len(calls) == 2:
            raise RuntimeError("publish-stored failed")
    with pytest.raises(RuntimeError):
        run_world(names, tmp_path, publish=publish)
    o = tmp_path / "out"
    assert len(calls) == 2
    assert [p for p in o.iterdir() if p.suffix == ".jsonl"] == [] and not s_state(tmp_path).exists()
