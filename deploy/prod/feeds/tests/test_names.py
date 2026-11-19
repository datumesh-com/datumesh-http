"""names.toml is the only place a name lives (rehearsal spelling: option B, mesh-test only)."""
from conftest import FEEDS
from helpers import DNS, NAME, host_of

RELAY = ["ecb_fx", "estr", "policy_rates", "treasury", "boe"]


def segs(ns):
    return ns.split("/")


def test_marked_rehearsal_on_mesh_test(names):
    assert names["meta"]["status"] == "rehearsal"
    assert names["meta"]["host"] == "mesh-test.datumesh.com"
    for k in RELAY + ["chirps", "register"]:
        assert host_of(names[k]["namespace"]) == names["meta"]["host"], k  # nothing under production's host


def test_relay_names_are_option_b_and_valid_today(names):
    for k in RELAY:
        s = names[k]
        parts = segs(s["namespace"])
        assert parts[1] == "relay" and len(parts) == 3
        assert parts[2] == s["source_name"].replace(".", "_")  # option B: dots become underscores
        assert all(NAME.match(p) for p in parts[1:]) and DNS.match(parts[0])
        assert NAME.match(s["function"])
        assert DNS.match(s["source_name"]) and s["source_name"] != parts[0]
        assert s["takeover"].startswith("https://") and len(s["takeover"]) <= 512
        assert s["takeover"].startswith(names["meta"]["takeover_base"])
        assert s["unit"] in ("1", "percent")


def test_computed_and_register_names_are_not_relay_names(names):
    for k in ("chirps", "register"):
        parts = segs(names[k]["namespace"])
        assert "relay" not in parts and all(NAME.match(p) for p in parts[1:])
    assert NAME.match(names["chirps"]["function"]) and names["chirps"]["unit"] == "mm"
    assert NAME.match(names["register"]["text_function"]) and NAME.match(names["register"]["page_function"])
    assert names["register"]["text_function"] != names["register"]["page_function"]
    assert DNS.match(names["chirps"]["source_name"])
    assert all(r for r in names["chirps"]["regions"].values()) and "US-KS" in names["chirps"]["regions"]


def test_all_urls_are_https_and_short(names):
    found = []

    def walk(x, key=""):
        if isinstance(x, dict):
            for k, v in x.items():
                walk(v, k if "url" in k else key)
        elif isinstance(x, str) and "url" in key:
            found.append(x)
    walk(names)
    assert len(found) >= 10
    for u in found:
        assert u.startswith("https://") and u.isascii() and len(u) <= 512


def test_licence_is_the_registers_and_register_domain_exists(names, register):
    by = {r["source_domain"]: r for r in register}
    for k in RELAY + ["chirps"]:
        r = by[names[k]["register"]]
        assert r["verdict"] == "RELAY"
        assert names[k]["licence"] == r["spdx_or_url"]
    assert names["boe"]["licence"] == "OGL-UK-3.0"


def test_policy_series_cover_the_three_rates(names):
    p = names["policy_rates"]
    assert set(p["series"]) == set(p["urls"]) == {"mro", "dfr", "mlfr"}
    assert p["series"] == {"mro": "MRR_FR", "dfr": "DFR", "mlfr": "MLFR"}
    for rate, key in p["series"].items():
        assert f".{key}.LEV" in p["urls"][rate]


def test_no_script_hardcodes_a_name():
    """The scripts read every name from the table: no function name, namespace or mesh host in a string literal."""
    import ast
    import re
    banned = ("fx_reference", "par_yield", "bank_rate", "policy_rate", "chirps_region_rain", "licence_text_sha256",
              "licence_page", "/relay/", "ecb_europa_eu", "mesh-test.datumesh.com")
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
                    assert not re.search(r"(?<![a-z_])" + re.escape(lit) + r"(?![a-z_])", n.value), f"{path.name} hardcodes {lit}: {n.value[:60]!r}"
