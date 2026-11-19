"""names.toml is the only place a name lives. The host is a parameter: the rehearsal host or production, and
production names cannot be produced unless the host is explicitly datumesh.com."""
import re

import pytest

import common
from conftest import FEEDS, PRODUCTION, REHEARSAL
from helpers import DNS, NAME, host_of

RELAY = {  # section -> (source domain, function)
    "ecb_fx": ("ecb.europa.eu", "fx_reference"),
    "estr": ("ecb.europa.eu", "estr"),
    "policy_rates": ("ecb.europa.eu", "policy_rate"),
    "treasury": ("treasury.gov", "par_yield"),
    "boe": ("bankofengland.co.uk", "bank_rate"),
}
HOSTS = [REHEARSAL, PRODUCTION]
METHODS = ["name", "status", "licence_page", "licence_hash", "licence_clauses", "retrieved"]


def segs(ns):
    return ns.split("/")


def strings(x, path=""):
    if isinstance(x, dict):
        for k, v in x.items():
            yield from strings(v, f"{path}.{k}")
    elif isinstance(x, list):
        for i, v in enumerate(x):
            yield from strings(v, f"{path}[{i}]")
    elif isinstance(x, str):
        yield path, x


@pytest.mark.parametrize("host", HOSTS)
def test_relay_names_have_the_domain_as_a_segment(host):
    names = common.names(host)
    for k, (domain, function) in RELAY.items():
        s = names[k]
        assert s["namespace"] == f"{host}/relay/{domain}", k
        parts = segs(s["namespace"])
        assert DNS.match(parts[0]) and parts[1] == "relay" and DNS.match(parts[2])  # a DNS name is a label only
        assert NAME.match(parts[1]) and s["function"] == function and NAME.match(function)
        assert s["source_name"] == domain and s["register_source"] == domain
        assert s["takeover"] == f"https://{host}/takeover/{domain}"
        assert len(s["takeover"]) <= 512
        assert s["unit"] in ("1", "percent")


@pytest.mark.parametrize("host", HOSTS)
def test_register_class_and_rainfall_names(host):
    names = common.names(host)
    r = names["register"]
    assert r["namespace"] == f"{host}/register" and r["class"] == "dataset"
    assert r["keys"] == ["source", "code"] and r["methods"] == METHODS
    assert r["steward_process"] == f"https://{host}/classes/change"
    assert all(NAME.match(m) for m in r["methods"]) and r["licence"]
    c = names["chirps"]
    assert c["namespace"] == f"{host}/earth" and c["function"] == "monthly_rainfall" and c["dataset"] == "chirps"
    assert all("relay" not in p and "chirps" not in p for p in segs(c["namespace"]) + [c["function"]])
    assert c["unit"] == "mm" and DNS.match(c["source_name"]) and c["source_name"] == "chc.ucsb.edu"
    assert "us_kansas" in c["regions"]
    for region, spec in c["regions"].items():
        assert NAME.match(region) and spec["label"] and spec["ne_id"] and spec["file"]


@pytest.mark.parametrize("host", HOSTS)
def test_every_namespace_is_under_the_given_host_and_only_it(host):
    names = common.names(host)
    other = [h for h in HOSTS if h != host][0]
    assert names["meta"]["host"] == host
    assert names["meta"]["status"] == ("production" if host == PRODUCTION else "rehearsal")
    assert names["meta"]["takeover_base"] == f"https://{host}/takeover"
    for k in list(RELAY) + ["chirps", "register"]:
        assert host_of(names[k]["namespace"]) == host, k
    for path, s in strings(names):
        # the other host never appears as a host: not as a prefix, not after https://, not after a slash
        if host == PRODUCTION:
            assert "mesh-test" not in s, (path, s)
        else:
            assert not re.search(r"(?<![A-Za-z0-9.-])datumesh\.com", s), (path, s)


def test_the_two_tables_differ_only_in_the_host():
    a = {p: s.replace(REHEARSAL, "HOST") for p, s in strings(common.names(REHEARSAL))}
    b = {p: s.replace(PRODUCTION, "HOST") for p, s in strings(common.names(PRODUCTION))}
    assert a == b


@pytest.mark.parametrize("bad", [None, "", " ", "datumesh.com ", " datumesh.com", "DATUMESH.COM", "www.datumesh.com",
                                 "datumesh.com.", "datumesh.com/", "https://datumesh.com", "mesh-test.datumesh.com.",
                                 "MESH-TEST.datumesh.com", "example.com", "datumesh.org", "prod", "production",
                                 "rehearsal", b"datumesh.com", 1])
def test_an_unknown_or_loosely_spelled_host_is_refused(bad):
    with pytest.raises(common.UnknownHost):
        common.names(bad)


def test_there_is_no_default_host():
    with pytest.raises(TypeError):
        common.names()


def test_production_never_appears_in_the_table_text_except_as_a_host_key():
    """Every name in names.toml is spelled with {host}; the literal host appears only as a key of [hosts]."""
    text = (FEEDS / "names.toml").read_text(encoding="utf-8")
    body = "\n".join(l for l in text.splitlines() if not l.startswith("[hosts.") and not l.lstrip().startswith("#"))
    assert "datumesh.com" not in body
    assert "{host}" in body
    assert sorted(re.findall(r'^\[hosts\."([^"]+)"\]', text, re.M)) == sorted(HOSTS)


SCRIPTS = ["ecb_fx", "estr", "policy_rates", "treasury", "boe", "chirps", "register"]


@pytest.mark.parametrize("module", SCRIPTS)
def test_every_script_needs_a_known_host_on_the_command_line(module, tmp_path):
    import importlib
    mod = importlib.import_module(module)
    base = ["--state-dir", str(tmp_path), "--out-dir", str(tmp_path)]
    extra = ["--register", str(tmp_path / "none.json")] if module == "register" else []
    for args in (base + extra, base + extra + ["--host", "example.com"], base + extra + ["--host", "DATUMESH.COM"]):
        with pytest.raises(SystemExit) as e:
            mod.main(args)  # refused before any fetch or file
        assert e.value.code not in (0, None)
    assert list(tmp_path.iterdir()) == []


def test_policy_series_cover_the_three_rates(names):
    p = names["policy_rates"]
    assert set(p["series"]) == set(p["urls"]) == {"mro", "dfr", "mlfr"}
    assert p["series"] == {"mro": "MRR_FR", "dfr": "DFR", "mlfr": "MLFR"}
    for rate, key in p["series"].items():
        assert f".{key}.LEV" in p["urls"][rate]


def test_all_urls_are_https_and_short(names):
    found = [s for path, s in strings(names) if "url" in path.rsplit(".", 1)[-1] or path.endswith(".takeover")]
    assert len(found) >= 10
    for u in found:
        assert u.startswith("https://") and u.isascii() and len(u) <= 512


def test_register_table_matches_the_s4_list(names):
    got = [(d["source"], d["code"], d["status"]) for d in names["register"]["datasets"]]
    assert got == [
        ("ecb.europa.eu", "reference_rates", "relayed"),
        ("ecb.europa.eu", "estr", "relayed"),
        ("ecb.europa.eu", "policy_rates", "relayed"),
        ("treasury.gov", "par_yield_curve", "relayed"),
        ("bankofengland.co.uk", "bank_rate", "relayed"),
        ("chc.ucsb.edu", "chirps", "computed_from"),
        ("worldbank.org", "commodity_prices", "not_relayed"),
        ("fao.org", "asis", "not_relayed"),
    ]
    for d in names["register"]["datasets"]:
        assert DNS.match(d["source"]) and NAME.match(d["code"]) and d["name"] and d["input"]
        assert ("reason" in d) == (d["status"] == "not_relayed")


def test_relay_sections_point_at_their_register_entry(names, register):
    by = {r["source_domain"]: r for r in register}
    datasets = {(d["source"], d["code"]): d for d in names["register"]["datasets"]}
    for k in RELAY:
        d = datasets[(names[k]["register_source"], names[k]["register_code"])]
        assert d["status"] == "relayed"
        r = by[d["input"]]
        assert r["verdict"] == "RELAY" and names[k]["licence"] == r["spdx_or_url"]
    assert names["boe"]["licence"] == "OGL-UK-3.0"
    assert (names["chirps"]["register_source"], names["chirps"]["register_code"]) == ("chc.ucsb.edu", "chirps")


def test_no_script_hardcodes_a_name():
    """The scripts read every name from the table: no function name, namespace or mesh host in a string literal."""
    import ast
    banned = ("fx_reference", "par_yield", "bank_rate", "policy_rate", "monthly_rainfall", "chirps_region_rain",
              "licence_text_sha256", "licence_hash", "licence_clauses", "/relay/", "/register", "/earth",
              "mesh-test.datumesh.com", "ecb.europa.eu", "treasury.gov", "bankofengland.co.uk")
    scripts = sorted(FEEDS.glob("*.py"))
    assert scripts, "no scripts yet"
    for path in scripts:
        tree = ast.parse(path.read_text())
        docs = {id(n.body[0].value) for n in ast.walk(tree)
                if isinstance(n, (ast.Module, ast.FunctionDef, ast.ClassDef)) and n.body
                and isinstance(n.body[0], ast.Expr) and isinstance(n.body[0].value, ast.Constant)}
        for n in ast.walk(tree):
            if isinstance(n, ast.Constant) and isinstance(n.value, str) and id(n) not in docs:
                for lit in banned:
                    assert not re.search(r"(?<![a-z_.])" + re.escape(lit) + r"(?![a-z_])", n.value), \
                        f"{path.name} hardcodes {lit}: {n.value[:60]!r}"
