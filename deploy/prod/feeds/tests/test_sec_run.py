"""sec.run on fixtures and synthetic companies, no network: scope, the first pass, the rate, 403, conformance (the frames
API for the relay method company/fact, the submissions API for the filing class, pinned samples of
<host>/sec/company/annual_report), the second run (K1 to K3, K7, N1 to N3; CONTRACT-sec-classes C1 to C3: two files, the
relay's and the subclass's)."""
import json
import pathlib
import re

import pytest

import common
import relay
import sec
from sec_helpers import names  # noqa: F401  (the SEC names: sec.names)
from common import ConformanceError, ShapeError
from helpers import check_summary
from sec_helpers import (CIK_CAT, CIK_GE, CIK_JPM, CIKS, FILING_METHODS, FRAMES_ASSETS, IN_SCOPE, NOW, SYNTH_CELLS, TICKERS,
                         FakeClock, annual_fn, check_order_all, company_cls, dirs, fact_fn, facts_file, filing_cls, kinds,
                         max_in_any_second, read_lines, sec_fixture, submissions_file, synth_company, synth_submissions)

BUDGET = 2300  # with the three real companies (rank 0, 57, 108) and synthetic ones of 6 cells: N = 125, 2220 cells
STAMP = relay._stamp(NOW)
FRAME_BY_CIK = {f"{d['cik']:010d}": d for d in json.loads(sec_fixture(FRAMES_ASSETS))["data"]}


class SecFetcher:
    """What data.sec.gov and www.sec.gov would send: the real tickers and Assets frame, the real companyfacts of the
    three fixture companies, a synthetic file for any other company in the frame, 404 for the other frames."""

    def __init__(self, names, clock=None, mutate=None, missing=(), refuse_at=None, facts_override=None, frames=None):
        self.names, self.clock, self.mutate = names, clock, mutate or {}
        self.missing, self.refuse_at, self.override = set(missing), refuse_at, facts_override or {}
        self.frames = frames or sec_fixture(FRAMES_ASSETS)
        self.urls, self.times = [], []

    def __call__(self, url):
        self.urls.append(url)
        if self.clock:
            self.times.append(self.clock())
        if self.refuse_at is not None and len(self.urls) == self.refuse_at:
            raise sec.SecRefused(url, "automated tools outside of the acceptable policy", 403)
        s = self.names["sec"]
        if url == s["tickers_url"]:
            return sec_fixture(TICKERS)
        if url == s["frames_url"].format(taxonomy="us-gaap", concept="Assets", unit="USD", ccp="CY2023Q4I"):
            return self.frames
        m = re.fullmatch(re.escape(s["companyfacts_url"]).replace(r"\{cik\}", r"(\d{10})"), url)
        if m:
            cik = m.group(1)
            if cik in self.missing:
                raise common.NotPublished(url)
            if cik in self.override:
                return self.override[cik]
            data = facts_file(cik) if cik in CIKS else self._synth(cik)
            return self.mutate[cik](data) if cik in self.mutate else data
        m = re.fullmatch(re.escape(s["submissions_url"]).replace(r"\{cik\}", r"(\d{10})"), url)
        if m:
            cik = m.group(1)
            if cik in self.missing:
                raise common.NotPublished(url)
            if cik in CIKS:
                return submissions_file(cik)
            d = FRAME_BY_CIK[cik]
            return synth_submissions(d["cik"], d["accn"])
        if "/frames/" in url:
            raise common.NotPublished(url)
        raise AssertionError(f"unexpected url {url}")

    def _synth(self, cik):
        d = FRAME_BY_CIK[cik]
        return synth_company(d["cik"], d["entityName"], d["accn"], d["end"], d["val"])

    def company_urls(self):
        return [u for u in self.urls if "/companyfacts/" in u]


def go(names, tmp_path, fetcher=None, fc=None, **kw):
    fc = fc or FakeClock()
    f = fetcher or SecFetcher(names, fc.clock)
    s, o = dirs(tmp_path) if not (tmp_path / "state").exists() else (tmp_path / "state", tmp_path / "out")
    kw.setdefault("budget", BUDGET)
    path = sec.run(names, fetch=f, now=NOW, state_dir=s, out_dir=o, sleep=fc.sleep, clock=fc.clock, **kw)
    return path, f, s, o


def jsonl(o):
    return sorted(p for p in o.iterdir() if p.suffix == ".jsonl")


def company_path(o, stamp=None):
    """The second file (C3): sec_company-<stamp>.jsonl, next to the relay file."""
    return o / f"sec_company-{stamp or STAMP}.jsonl"


@pytest.fixture(scope="module")
def first(names, tmp_path_factory):
    tmp = tmp_path_factory.mktemp("sec_run")
    fc = FakeClock()
    path, f, s, o = go(names, tmp, fc=fc)
    return {"path": path, "f": f, "s": s, "o": o, "fc": fc, "lines": read_lines(path), "tmp": tmp}


def test_run_writes_the_relay_file_and_the_company_file_named_by_the_stamp(first):
    assert first["path"] == first["o"] / f"sec-{STAMP}.jsonl" and first["path"].exists()
    assert jsonl(first["o"]) == [first["path"], company_path(first["o"])], "C3: the second file, published after the relay"
    check_order_all(first["lines"])
    check_order_all(read_lines(company_path(first["o"])))


def test_scope_chooses_n_125_and_the_cells_are_those_of_the_top_125(names, first):
    cells = [l for l in first["lines"] if "cell" in l and l["cell"]["fn"] == fact_fn(names)]
    assert len(cells) == 2220 == sum(IN_SCOPE.values()) + 122 * SYNTH_CELLS
    by_cik = {}
    for c in cells:
        by_cik.setdefault(c["cell"]["args"]["cik"]["value"], []).append(c)
    assert len(by_cik) == 125
    assert {k: len(v) for k, v in by_cik.items() if k in CIKS} == IN_SCOPE
    ranked = sec.rank_companies(sec_fixture(TICKERS), sec_fixture(FRAMES_ASSETS))["ranked"]
    assert set(by_cik) == {r["cik"] for r in ranked[:125]}
    assert CIK_CAT in by_cik and CIK_GE in by_cik and CIK_JPM in by_cik


def test_the_run_log_says_the_estimate_and_n(first, names, tmp_path, capsys):
    # the first run of the module fixture has run already; its stderr is gone, so run once more in a fresh dir
    capsys.readouterr()
    go(names, tmp_path)
    err = capsys.readouterr().err
    m = re.search(r"scope N=(\d+) estimated_cells=(\d+)", err)
    assert m and (int(m.group(1)), int(m.group(2))) == (125, 2220), err
    assert "budget=2300" in err and "ranked=4170" in err


def test_the_left_out_report_is_written_next_to_the_cells(first):
    p = first["o"] / f"sec-scope-{STAMP}.json"
    rep = json.loads(p.read_text(encoding="utf-8"))
    assert rep["n"] == 125 and rep["estimated_cells"] == 2220 and rep["budget"] == BUDGET
    assert rep["left_out"]["below_n"] == 4170 - 125 and rep["left_out"]["no_companyfacts"] == []
    assert rep["left_out"]["listed_without_assets"] == 1854 and rep["left_out"]["not_listed_on_nyse_or_nasdaq"] == 2260
    assert rep["left_out"]["left_out_period_end"] == {"count": 0, "accessions": []}, "every in-scope filing has a record"
    assert isinstance(rep["left_out"]["ambiguous_labels"], dict)


def test_the_relay_file_has_the_company_class_and_the_filing_class_and_no_function_line(names, first):
    k = kinds(first["lines"])
    assert sorted(l["class"] for l in k["class"]) == sorted([company_cls(names), filing_cls(names)]), (
        "register.py's class lines are not in this file")
    assert len(k["descriptor"]) == 7, "fact, and the six methods of the filing class"
    assert k["function"] == [], "annual_report moved to the company file; company_fact is a method now"
    assert not [l for l in first["lines"] if "catalog" in l and "register/dataset" in l["catalog"]]
    assert not [l for l in first["lines"] if "/sec/" in (l.get("catalog") or l.get("test") or (l["cell"]["fn"] if "cell" in l else ""))]


def test_tests_are_at_least_20_and_come_from_the_frames_the_submissions_and_the_selection(names, first):
    tests = kinds(first["lines"])["test"]
    by = {}
    for t in tests:
        by.setdefault(t["test"], []).append(t)
    cf = by[fact_fn(names)]
    assert len(cf) == 124, "122 synthetic + Caterpillar and GE; JPMorgan's frame report is out of scope"
    assert len(cf) >= 20
    methods = [f"{filing_cls(names)}/{m}" for m in FILING_METHODS]
    assert all(len(by[m]) == 18 + 122 * 2 for m in methods), "every filing, from the submissions of the chosen companies"
    ann = [t for t in kinds(read_lines(company_path(first["o"])))["test"] if t["test"] == annual_fn(names)]
    assert len(ann) == 6 + 122 * 2, "the earliest and the latest fiscal year of each company"
    assert annual_fn(names) not in by, "its pins are in the company file"
    assert set(by) == {fact_fn(names), *methods} | {t for t in by if "register/dataset" in t}
    assert len({t["call"] for t in tests}) == len(tests), "no test is written twice"


def test_run_calls_only_sec_urls_each_company_at_most_twice(names, first):
    f = first["f"]
    s = names["sec"]
    ok = (s["tickers_url"], s["frames_url"].format(taxonomy="us-gaap", concept="Assets", unit="USD", ccp="CY2023Q4I"))
    for u in f.urls:
        assert (u in ok or u.startswith("https://data.sec.gov/api/xbrl/") and u.endswith(".json")
                or re.fullmatch(r"https://data\.sec\.gov/submissions/CIK\d{10}\.json", u)), u
    cu = f.company_urls()
    assert 125 <= len(set(cu)) <= 150
    assert max(cu.count(u) for u in set(cu)) <= 2, "the counting pass and the emitting pass"
    su = [u for u in f.urls if "/submissions/" in u]
    assert len(su) == len(set(su)) == 125, "the second copy of the filing class: one request for each chosen company"
    assert f.urls[0] in ok, "the ranking files come first"


def test_the_rate_never_exceeds_5_a_second(first):
    assert max_in_any_second(first["f"].times) <= 5
    n = len(first["f"].times)
    assert first["fc"].clock() - 1000.0 >= (n - 1) / 5 - 1e-9


def test_state_is_a_plain_json_file_per_run_kind(first):
    st = first["s"] / "sec.json"
    assert st.exists()
    json.loads(st.read_text(encoding="utf-8"))


def test_second_run_emits_nothing_new(names, first):
    s, o = first["s"], first["o"]
    before_state = (s / "sec.json").read_text()
    fc = FakeClock()
    f = SecFetcher(names, fc.clock)
    again = sec.run(names, fetch=f, now="2026-11-21T12:00:00.000000Z", state_dir=s, out_dir=o, sleep=fc.sleep,
                    clock=fc.clock, budget=BUDGET)
    assert again is None and jsonl(o) == [first["path"], company_path(o)]
    assert (s / "sec.json").read_text() == before_state


def test_a_new_report_in_a_later_run_emits_only_its_cells(names, first, tmp_path):
    import shutil
    s, o = tmp_path / "state", tmp_path / "out"
    shutil.copytree(first["s"], s), shutil.copytree(first["o"], o)

    def add_amendment(data):
        raw = json.loads(data)
        raw["facts"]["us-gaap"]["Assets"]["units"]["USD"].append(
            {"end": "2024-12-31", "val": 177000000000, "accn": "0000040545-25-000099", "fy": 2024, "fp": "FY",
             "form": "10-K/A", "filed": "2025-06-02"})
        return json.dumps(raw).encode()
    fc = FakeClock()
    f = SecFetcher(names, fc.clock, mutate={CIK_GE: add_amendment})
    later = "2026-11-21T12:00:00.000000Z"
    path = sec.run(names, fetch=f, now=later, state_dir=s, out_dir=o, sleep=fc.sleep, clock=fc.clock, budget=BUDGET)
    assert path is not None and path.name == f"sec-{relay._stamp(later)}.jsonl"
    k = kinds(read_lines(path))
    assert [len(k[x]) for x in ("function", "descriptor", "class", "catalog")] == [0, 0, 0, 0]
    fns = sorted((c["cell"]["fn"], len(c["cell"]["args"])) for c in k["cell"])
    assert fns == sorted([(fact_fn(names), 6)] + [(f"{filing_cls(names)}/{m}", 1) for m in FILING_METHODS if m not in ("period_end", "filed")]), (
        "the new report has no record in the submissions: no period_end cell")
    rep = json.loads((o / f"sec-scope-{relay._stamp(later)}.json").read_text(encoding="utf-8"))
    assert rep["left_out"]["left_out_period_end"] == {"count": 1, "accessions": ["0000040545-25-000099"]}
    ck = kinds(read_lines(company_path(o, relay._stamp(later))))
    assert [len(ck[x]) for x in ("function", "catalog")] == [0, 0], "nothing new to bind"
    assert [(c["cell"]["fn"], c["value"]["value"]) for c in ck["cell"]] == [(annual_fn(names), "0000040545-25-000099")], (
        "the latest report of fiscal 2024 is now the amendment; no label or loaded cell is new")


def test_a_company_without_companyfacts_is_left_out_and_reported(names, tmp_path):
    wells = "0000072971"
    path, f, s, o = go(names, tmp_path, fetcher=SecFetcher(names, None, missing=[wells]))
    cells = [l for l in read_lines(path) if "cell" in l and l["cell"]["fn"] == fact_fn(names)]
    assert len(cells) == 2220 - SYNTH_CELLS
    rep = json.loads((o / f"sec-scope-{STAMP}.json").read_text(encoding="utf-8"))
    assert rep["left_out"]["no_companyfacts"] == [wells] and rep["n"] == 125 and rep["estimated_cells"] == 2214


# ---------- stops ----------

def test_a_403_stops_all_fetching_and_leaves_nothing(names, tmp_path):
    fc = FakeClock()
    f = SecFetcher(names, fc.clock, refuse_at=40)
    s, o = dirs(tmp_path)
    with pytest.raises(sec.SecRefused) as e:
        sec.run(names, fetch=f, now=NOW, state_dir=s, out_dir=o, sleep=fc.sleep, clock=fc.clock, budget=BUDGET)
    assert len(f.urls) == 40, "no request after the refusal"
    assert "automated tools" in str(e.value)
    assert list(o.iterdir()) == [] and list(s.iterdir()) == []


def test_a_403_on_the_first_request(names, tmp_path):
    fc = FakeClock()
    f = SecFetcher(names, fc.clock, refuse_at=1)
    s, o = dirs(tmp_path)
    with pytest.raises(sec.SecRefused):
        sec.run(names, fetch=f, now=NOW, state_dir=s, out_dir=o, sleep=fc.sleep, clock=fc.clock, budget=BUDGET)
    assert len(f.urls) == 1 and list(o.iterdir()) == []


def test_a_disagreement_with_the_frames_stops_the_run_and_publishes_nothing(names, tmp_path):
    data = sec_fixture(FRAMES_ASSETS)
    bad = data.replace(b'"val":87476000000', b'"val":87476000001')
    assert bad != data
    fc = FakeClock()
    s, o = dirs(tmp_path)
    published = []
    with pytest.raises(ConformanceError):
        sec.run(names, fetch=SecFetcher(names, fc.clock, frames=bad), now=NOW, state_dir=s, out_dir=o,
                sleep=fc.sleep, clock=fc.clock, budget=BUDGET, publish=published.append)
    assert published == [] and jsonl(o) == [] and not (s / "sec.json").exists()


def test_a_changed_shape_refuses_and_leaves_nothing(names, tmp_path):
    fc = FakeClock()
    s, o = dirs(tmp_path)
    mut = {CIK_JPM: lambda b: b.replace(b'"facts"', b'"fact"', 1)}
    with pytest.raises(ShapeError):
        sec.run(names, fetch=SecFetcher(names, fc.clock, mutate=mut), now=NOW, state_dir=s, out_dir=o,
                sleep=fc.sleep, clock=fc.clock, budget=BUDGET)
    assert jsonl(o) == [] and not (s / "sec.json").exists()


def test_publish_gets_the_file_and_the_state_is_kept_only_after_it_returns(names, tmp_path):
    fc = FakeClock()
    s, o = dirs(tmp_path)
    seen = []

    def publish(p):
        seen.append(p)
        assert p.exists() and not (s / "sec.json").exists(), "state is saved only after publish returns"
    path = sec.run(names, fetch=SecFetcher(names, fc.clock), now=NOW, state_dir=s, out_dir=o, sleep=fc.sleep,
                   clock=fc.clock, budget=BUDGET, publish=publish)
    assert seen == [path, company_path(o)] and (s / "sec.json").exists(), "the relay first, then the company file"


def test_publish_failure_removes_the_file_and_keeps_no_state(names, tmp_path):
    fc = FakeClock()
    s, o = dirs(tmp_path)

    def boom(p):
        raise RuntimeError("publish-stored failed")
    with pytest.raises(RuntimeError):
        sec.run(names, fetch=SecFetcher(names, fc.clock), now=NOW, state_dir=s, out_dir=o, sleep=fc.sleep,
                clock=fc.clock, budget=BUDGET, publish=boom)
    assert jsonl(o) == [] and not (s / "sec.json").exists()


def test_the_default_budget_and_step_are_the_contracts(names):
    import inspect
    p = inspect.signature(sec.run).parameters
    assert p["budget"].default == 300_000 and p["step"].default == 25


REG_CLASS = "mesh-test.datumesh.com/register/dataset"
REG_METHODS = ["name", "status", "licence_page", "licence_hash", "licence_clauses", "retrieved"]


def test_first_run_writes_the_register_cells_of_sec_gov_company_facts(first):
    """The licence register's six cells for the dataset (register.py's methods and texts). The class lines and the
    method descriptors belong to register.py, so this file carries cells only."""
    lines = first["lines"]
    cells = [l for l in lines if "cell" in l and l["cell"]["fn"].startswith(REG_CLASS + "/")]
    assert sorted(c["cell"]["fn"].rsplit("/", 1)[1] for c in cells) == sorted(REG_METHODS)
    for c in cells:
        assert c["cell"]["args"] == {"source": {"type": "string", "value": "sec.gov"},
                                     "code": {"type": "string", "value": "company_facts"}}
        assert c["licence"] == "CC0-1.0", "the register's own cells keep their licence"
        check_summary(c["summary"])
    by = {c["cell"]["fn"].rsplit("/", 1)[1]: c["value"] for c in cells}
    assert by["status"] == {"type": "string", "value": "relayed"}
    assert by["licence_page"] == {"type": "string", "value": "https://www.sec.gov/about/privacy-information"}
    assert by["licence_hash"] == {"type": "string",
                                  "value": "edd4ec41f3aec2594d25ead8e0a08681cb3bfe42414f6805fc538a6dc06a3723"}
    assert by["retrieved"] == {"type": "instant", "value": "2026-11-20T13:28:09.276628Z"}
    blob = by["licence_clauses"]
    assert blob["media_type"] == "text/plain" and blob["blob"].startswith("blobs/")
    text = (first["o"] / blob["blob"]).read_text(encoding="utf-8")
    assert "Please declare your user agent in request headers:" in text and " | " in text
    assert not [l for l in lines if "class" in l and REG_CLASS in l["class"]]
    assert not [l for l in lines if "catalog" in l and REG_CLASS in l["catalog"]]
    check_order_all(lines)


# ---------- the company file on the real fixtures (C3) ----------

def test_the_company_file_has_the_annual_report_cells_and_loaded_and_labels_of_the_real_companies(names, first):
    ck = read_lines(company_path(first["o"]))
    cells = lambda m: [l for l in ck if "cell" in l and l["cell"]["fn"] == f"{names['meta']['host']}/sec/company/{m}"]
    ann = {(c["cell"]["args"]["cik"]["value"], c["cell"]["args"]["fiscal_year"]["value"]): c["value"]["value"]
           for c in cells("annual_report")}
    assert ann[(CIK_GE, "2023")] == "0000040545-24-000027" and ann[(CIK_CAT, "2019")] == "0000018230-20-000056"
    assert len(ann) == 18 + 122 * 2
    loaded = {c["cell"]["args"]["cik"]["value"] for c in cells("loaded")}
    assert len(loaded) == 125 and {CIK_CAT, CIK_GE, CIK_JPM} <= loaded
    labels = {c["cell"]["args"]["label"]["value"]: c["value"]["value"] for c in cells("from_label")}
    enc = lambda cik: f'{names["meta"]["host"]}/sec/company(cik:"{cik}")'
    for label, cik in (("caterpillar inc", CIK_CAT), ("cat", CIK_CAT), ("caterpillar", CIK_CAT),
                       ("jpmorgan chase & co", CIK_JPM), ("jpm", CIK_JPM), ("jpmorgan chase", CIK_JPM),
                       ("general electric co", CIK_GE), ("ge", CIK_GE), ("general electric", CIK_GE)):  # folded (K8)
        assert labels[label] == enc(cik), label
    assert all(re.fullmatch(r".*/sec/company\(cik:\"\d{10}\"\)", v) for v in labels.values())
