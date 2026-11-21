"""CONTRACT-sec-classes C5: sec_class.py writes the class line of <host>/sec/company, a subclass of
<host>/relay/sec.gov/company. A class line holds method hashes, and only the mesh's define computes a formula's hash, so this
is a separate step after the views are published: it asks the mesh (`GET <mesh>/describe/<name>`) for the parent's class hash
and for each method's function hash, and writes the class line and its catalogue line.

    python sec_class.py --host H --mesh URL --out FILE

Tested with a fake describe server on 127.0.0.1 (an ephemeral port), the script run as a subprocess. Standard library only."""
import ast
import hashlib
import http.server
import json
import subprocess
import sys
import threading
import urllib.parse

import pytest

from conftest import FEEDS, PRODUCTION, REHEARSAL

SCRATCH = "scratch.invalid"
FORMULA_METHODS = ["fact_in", "reported", "latest", "revenue", "cost_of_revenue", "net_income", "operating_income",
                   "total_assets", "current_assets", "current_liabilities", "inventory", "ppe_net", "accounts_payable",
                   "accounts_receivable", "current_ratio", "return_on_assets", "inventory_turnover", "fixed_asset_turnover",
                   "days_payable_outstanding", "cash_conversion_cycle", "gross_margin", "revenue_growth"]
METHODS = ["fact", "annual_report", "loaded", "from_label"] + FORMULA_METHODS
assert len(FORMULA_METHODS) == 22 and len(set(METHODS)) == 26


def h(name):
    return hashlib.sha256(name.encode()).hexdigest()


def names_for(host):
    """name -> (kind, hash) the fake mesh binds: the relay class, the relay's stored fact (a decoy: the class's fact is the
    override), and the 26 methods under <host>/sec/company/."""
    out = {f"{host}/relay/sec.gov/company": ("class", h("class:" + host)),
           f"{host}/relay/sec.gov/company/fact": ("function", h("decoy-relay-fact"))}
    for m in METHODS:
        out[f"{host}/sec/company/{m}"] = ("function", h(f"{host}/sec/company/{m}"))
    return out


class Mesh:
    """A fake mesh that answers GET /describe/<name> (percent-encoded or not) for the names it binds, 404 for the others."""

    def __init__(self, bound):
        self.bound, self.requests = dict(bound), []
        mesh = self

        class H(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                path = urllib.parse.unquote(self.path.split("?")[0])
                mesh.requests.append(path)
                name = path[len("/describe/"):] if path.startswith("/describe/") else None
                hit = mesh.bound.get(name)
                if hit is None:
                    body, code = json.dumps({"error": "E_NAME_UNBOUND", "name": name}).encode(), 404
                elif hit[0] == "class":
                    body, code = json.dumps({"kind": "class", "name": name, "class_hash": hit[1],
                                             "keys": {}, "methods": {}}).encode(), 200
                else:
                    body, code = json.dumps({"kind": "function", "name": name, "fn_hash": hit[1],
                                             "tests": {"count": 0}}).encode(), 200
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *a):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *a):
        self.server.shutdown()
        self.server.server_close()

    def described(self):
        return [r[len("/describe/"):] for r in self.requests]


def run(tmp_path, host, mesh_url, out=None, extra=()):
    assert (FEEDS / "sec_class.py").exists(), "sec_class.py is not written yet"
    out = out or tmp_path / "sec_class.jsonl"
    r = subprocess.run([sys.executable, "-B", str(FEEDS / "sec_class.py"), "--host", host, "--mesh", mesh_url, "--out", str(out),
                        *extra], capture_output=True, text=True, cwd=tmp_path, timeout=60)
    return r, out


def lines(path):
    return [json.loads(l) for l in path.read_text(encoding="utf-8").splitlines()]


@pytest.fixture(params=[REHEARSAL, SCRATCH])
def host(request):
    return request.param


def test_writes_the_class_line_with_the_parent_every_method_the_lookup_the_extent_and_the_steward(tmp_path, host):
    bound = names_for(host)
    with Mesh(bound) as m:
        r, out = run(tmp_path, host, m.url)
    assert r.returncode == 0, r.stderr
    ls = lines(out)
    assert len(ls) == 2 and "class" in ls[0] and "catalog" in ls[1], "the class line, then its catalogue line"
    cls = ls[0]
    assert cls["class"] == f"{host}/sec/company"
    d = cls["descriptor"]
    assert set(d) == {"keys", "order", "parent", "methods", "lookup", "extent", "steward"}
    assert d["keys"] == {} and d["order"] == []
    assert d["parent"] == bound[f"{host}/relay/sec.gov/company"][1], "the class_hash of the relay company class"
    assert d["methods"] == {m: h(f"{host}/sec/company/{m}") for m in METHODS}, (
        "the 4 stored methods and the 22 views; fact is the override's hash, not the relay's")
    assert d["lookup"] == "from_label"
    assert d["extent"] == {"keys": h(f"{host}/sec/company/loaded")}, "the loaded function's hash"
    assert d["steward"] == {"namespace": f"{host}/sec", "process": f"https://{host}/classes/change"}


def test_it_reads_the_describe_of_the_relay_class_and_each_method_and_nothing_else(tmp_path, host):
    with Mesh(names_for(host)) as m:
        r, _ = run(tmp_path, host, m.url)
        assert r.returncode == 0, r.stderr
        assert sorted(set(m.described())) == sorted([f"{host}/relay/sec.gov/company"] + [f"{host}/sec/company/{x}" for x in METHODS])
        assert all(q.startswith("/describe/") for q in m.requests)


def test_the_catalogue_line_of_the_class(tmp_path):
    host = REHEARSAL
    with Mesh(names_for(host)) as m:
        r, out = run(tmp_path, host, m.url)
    assert r.returncode == 0, r.stderr
    c = lines(out)[1]
    assert c["catalog"] == f"{host}/sec/company" and c["kind"] == "class"
    assert {"label", "description", "args"} <= set(c)
    assert "SEC company (Datumesh views over the SEC relay)" in (c["label"], c["description"]) or \
        "SEC company (Datumesh views over the SEC relay)" in c["label"] + c["description"]
    assert set(c["args"]) == {"cik"} and all(isinstance(v, str) and v.strip() for v in c["args"].values())
    assert "exact" in json.dumps(c), "it says the labels are exact"
    assert len(c["description"].encode()) <= 1024 and len(c["label"].encode()) <= 120


def test_every_line_is_one_json_line_in_nfc_utf8(tmp_path):
    with Mesh(names_for(REHEARSAL)) as m:
        r, out = run(tmp_path, REHEARSAL, m.url)
    text = out.read_bytes().decode("utf-8")
    assert text.endswith("\n") and len(text.splitlines()) == 2
    assert not text.endswith("\n\n")


def test_a_trailing_slash_on_the_mesh_url_is_fine(tmp_path):
    with Mesh(names_for(REHEARSAL)) as m:
        r, out = run(tmp_path, REHEARSAL, m.url + "/")
    assert r.returncode == 0, r.stderr
    assert len(lines(out)) == 2


UNBOUND = ([f"{REHEARSAL}/relay/sec.gov/company"] + [f"{REHEARSAL}/sec/company/{m}" for m in
                                                     ("fact", "annual_report", "loaded", "from_label", "fact_in", "current_ratio",
                                                      "revenue_growth")])


@pytest.mark.parametrize("missing", UNBOUND)
def test_an_unbound_name_is_an_error_that_names_it_and_nothing_is_written(tmp_path, missing):
    bound = names_for(REHEARSAL)
    del bound[missing]
    with Mesh(bound) as m:
        r, out = run(tmp_path, REHEARSAL, m.url)
    assert r.returncode != 0
    assert missing in r.stderr, r.stderr
    assert not out.exists() and list(tmp_path.iterdir()) == []


def test_two_unbound_names_are_both_named_or_the_first_is(tmp_path):
    bound = names_for(REHEARSAL)
    a, b = f"{REHEARSAL}/sec/company/loaded", f"{REHEARSAL}/sec/company/revenue"
    del bound[a], bound[b]
    with Mesh(bound) as m:
        r, out = run(tmp_path, REHEARSAL, m.url)
    assert r.returncode != 0 and (a in r.stderr or b in r.stderr) and not out.exists()


@pytest.mark.parametrize("bad", [PRODUCTION, "example.com", "mesh-test.datumesh.com."])
def test_a_host_that_is_not_a_rehearsal_host_is_refused_before_any_request(tmp_path, bad):
    with Mesh(names_for(REHEARSAL)) as m:
        r, out = run(tmp_path, bad, m.url)
        assert m.requests == []
    assert r.returncode != 0 and not out.exists()


def test_an_unreachable_mesh_is_an_error_and_nothing_is_written(tmp_path):
    with Mesh(names_for(REHEARSAL)) as m:
        url = m.url
    r, out = run(tmp_path, REHEARSAL, url)  # the server is gone
    assert r.returncode != 0 and not out.exists()


def test_the_script_needs_the_three_arguments(tmp_path):
    assert (FEEDS / "sec_class.py").exists(), "sec_class.py is not written yet"
    for args in ([], ["--host", REHEARSAL], ["--host", REHEARSAL, "--mesh", "http://127.0.0.1:1"]):
        r = subprocess.run([sys.executable, "-B", str(FEEDS / "sec_class.py"), *args], capture_output=True, text=True,
                           cwd=tmp_path, timeout=60)
        assert r.returncode != 0
    assert list(tmp_path.iterdir()) == []


def test_it_imports_only_the_standard_library_and_the_feeds_own_modules():
    tree = ast.parse((FEEDS / "sec_class.py").read_text(encoding="utf-8"))
    mine = {p.stem for p in FEEDS.glob("*.py")}
    tops = set()
    for n in ast.walk(tree):
        if isinstance(n, ast.Import):
            tops |= {a.name.split(".")[0] for a in n.names}
        elif isinstance(n, ast.ImportFrom) and n.level == 0:
            tops.add(n.module.split(".")[0])
    assert tops, "it imports something"
    assert all(t in sys.stdlib_module_names or t in mine for t in tops), tops
