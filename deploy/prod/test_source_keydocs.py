#!/usr/bin/env python3
"""Tests of source_keydocs.py (CONTRACT-guard-patch P3), against a local fixture mesh and fixture sources.

Run: python3 -I deploy/prod/test_source_keydocs.py   (standard library only; no network beyond 127.0.0.1)

The script under test:
  python3 -I source_keydocs.py
  env DATUMESH_MESH        the mesh origin (default https://datumesh.com)
      DATUMESH_STATUS_DIR  where status files go (default /root/datumesh-prod/status)
      DATUMESH_SOURCE_URL  the key document URL of a source, with {domain} (default https://{domain}/.well-known/datumesh.json)
It finds the relayed domains from the catalogue (/catalog, /catalog/<d>: links /fn/<host>/relay/<domain>/<fn>), keeps those whose
GET /takeover/<domain> (Accept: application/json) is 200, fetches each source's key document and writes
  source-keydocs.json  {"checked", "mesh", "ok", "domains": [{"domain", "keydoc": "present"|"absent"|"invalid", "http",
                        "sha256" (null unless present), "first_seen" (null unless present)}], "new": [...], "changed": [...]}
  source-keydocs.txt   one line: "<time> OK: ..." | "<time> NEW: <domains> ..." | "<time> WARN: ..."
  source-keydocs.state the last present documents (domain, sha256, first_seen), kept across runs.
"present" is a 200 JSON object whose "host" is the domain. A mesh that cannot be read is WARN, exit 1, and the state is kept.
"""
import http.server
import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "source_keydocs.py")


class Fixture:
    """The mesh's catalogue and takeover pages, and the sources' key documents, served on one port."""

    def __init__(self):
        self.relayed = {"ecb.europa.eu": ["estr", "fx_reference"], "treasury.gov": ["par_yield"]}
        # A domain linked from the catalogue whose takeover page says the mesh relays nothing from it.
        self.unconfirmed = "stale.example"
        self.keydocs = {}  # domain -> bytes served at its key document URL
        self.mesh_up = True
        self.requests = []
        fx = self

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def send(self, code, body, ctype="text/html; charset=utf-8"):
                b = body if isinstance(body, bytes) else body.encode()
                self.send_response(code)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(b)))
                self.end_headers()
                self.wfile.write(b)

            def do_GET(self):
                p = self.path
                fx.requests.append(p)
                if p.startswith("/src/"):
                    domain = p.split("/")[2]
                    if domain in fx.keydocs:
                        return self.send(200, fx.keydocs[domain], "application/json")
                    return self.send(404, '{"error":"not found"}', "application/json")
                if not fx.mesh_up:
                    return self.send(502, "bad gateway")
                if p == "/catalog":
                    return self.send(200, '<ul><li><a href="/catalog/finance">finance</a></li>'
                                          '<li><a href="/catalog/registry">registry</a></li></ul><a href="/catalog/rule">rule</a>')
                if p == "/catalog/finance":
                    links = "".join(f'<li><a href="/catalog/finance/subject?key=x">s</a> <a href="/fn/datumesh.com/relay/{d}/{f}">{f}</a></li>'
                                    for d, fs in fx.relayed.items() for f in fs)
                    links += f'<li><a href="/fn/datumesh.com/relay/{fx.unconfirmed}/old">old</a></li>'
                    return self.send(200, f"<ul>{links}</ul><a href=\"/catalog\">back</a>")
                if p == "/catalog/registry":
                    return self.send(200, '<ul><li><a href="/fn/datumesh.com/registry/list">list</a></li></ul>')
                if p == "/catalog/rule":
                    return self.send(200, "<p>the rule</p>")
                if p.startswith("/takeover/"):
                    d = p[len("/takeover/"):]
                    if d in fx.relayed:
                        fns = [{"name": f"datumesh.com/relay/{d}/{f}"} for f in fx.relayed[d]]
                        return self.send(200, json.dumps({"domain": d, "functions": fns}), "application/json")
                    return self.send(404, json.dumps({"outcome": "not_covered"}), "application/json")
                return self.send(404, "no")

        self.srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.port = self.srv.server_address[1]
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()

    def keydoc(self, domain, extra=""):
        return json.dumps({"host": domain, "proto": "datumesh/1", "keys": [{"id": "k1" + extra}]}).encode()


class SourceKeydocs(unittest.TestCase):
    def setUp(self):
        self.fx = Fixture()
        self.tmp = tempfile.TemporaryDirectory(prefix="source-keydocs-")
        self.dir = self.tmp.name

    def tearDown(self):
        self.fx.srv.shutdown()
        self.tmp.cleanup()

    def run_it(self):
        env = dict(os.environ)
        env.update({
            "DATUMESH_MESH": f"http://127.0.0.1:{self.fx.port}",
            "DATUMESH_STATUS_DIR": self.dir,
            "DATUMESH_SOURCE_URL": f"http://127.0.0.1:{self.fx.port}/src/{{domain}}/.well-known/datumesh.json",
        })
        p = subprocess.run([sys.executable, "-I", SCRIPT], env=env, capture_output=True, text=True, timeout=60)
        return p

    def status(self):
        with open(os.path.join(self.dir, "source-keydocs.json"), encoding="utf-8") as f:
            return json.load(f)

    def line(self):
        with open(os.path.join(self.dir, "source-keydocs.txt"), encoding="utf-8") as f:
            return f.read().strip()

    def by_domain(self):
        return {d["domain"]: d for d in self.status()["domains"]}

    def test_the_relayed_domains_come_from_the_catalogue_confirmed_by_takeover(self):
        p = self.run_it()
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(sorted(self.by_domain()), ["ecb.europa.eu", "treasury.gov"])
        self.assertIn("/takeover/stale.example", self.fx.requests, "a linked domain is confirmed by its takeover page")

    def test_no_key_document_is_absent_and_ok(self):
        p = self.run_it()
        self.assertEqual(p.returncode, 0, p.stderr)
        s = self.status()
        self.assertTrue(s["ok"])
        self.assertEqual(s["new"], [])
        for d in s["domains"]:
            self.assertEqual(d["keydoc"], "absent", d)
            self.assertEqual(d["http"], 404, d)
            self.assertIsNone(d["sha256"])
            self.assertIsNone(d["first_seen"])
        self.assertRegex(self.line(), r"^\S+ OK: ")
        self.assertIn("2 relayed", self.line())

    def test_a_new_key_document_is_reported_once(self):
        self.run_it()
        self.fx.keydocs["treasury.gov"] = self.fx.keydoc("treasury.gov")
        p = self.run_it()
        self.assertEqual(p.returncode, 0, p.stderr)
        s = self.status()
        self.assertEqual(s["new"], ["treasury.gov"])
        t = self.by_domain()["treasury.gov"]
        self.assertEqual(t["keydoc"], "present")
        self.assertEqual(t["http"], 200)
        self.assertRegex(t["sha256"], r"^[0-9a-f]{64}$")
        self.assertEqual(t["first_seen"], s["checked"])
        self.assertRegex(self.line(), r"^\S+ NEW: .*treasury\.gov")
        # The next run: still present, not new; first_seen kept.
        first = t["first_seen"]
        self.run_it()
        s = self.status()
        self.assertEqual(s["new"], [])
        self.assertEqual(self.by_domain()["treasury.gov"]["first_seen"], first)
        self.assertRegex(self.line(), r"^\S+ OK: ")
        self.assertIn("treasury.gov", self.line(), "the OK line names the domains that publish a key document")

    def test_a_changed_key_document_is_reported(self):
        self.fx.keydocs["ecb.europa.eu"] = self.fx.keydoc("ecb.europa.eu")
        self.run_it()
        self.fx.keydocs["ecb.europa.eu"] = self.fx.keydoc("ecb.europa.eu", extra="-rotated")
        self.run_it()
        s = self.status()
        self.assertEqual(s["new"], [])
        self.assertEqual(s["changed"], ["ecb.europa.eu"])
        self.assertRegex(self.line(), r"^\S+ NEW: .*ecb\.europa\.eu.*changed")

    def test_a_document_for_another_host_or_not_json_is_invalid(self):
        self.fx.keydocs["ecb.europa.eu"] = self.fx.keydoc("someone.else")
        self.fx.keydocs["treasury.gov"] = b"<html>not json</html>"
        p = self.run_it()
        self.assertEqual(p.returncode, 0, p.stderr)
        d = self.by_domain()
        self.assertEqual(d["ecb.europa.eu"]["keydoc"], "invalid")
        self.assertEqual(d["treasury.gov"]["keydoc"], "invalid")
        self.assertEqual(self.status()["new"], [])

    def test_a_mesh_that_cannot_be_read_is_warn_and_keeps_the_state(self):
        self.fx.keydocs["treasury.gov"] = self.fx.keydoc("treasury.gov")
        self.run_it()
        first = self.by_domain()["treasury.gov"]["first_seen"]
        self.fx.mesh_up = False
        p = self.run_it()
        self.assertEqual(p.returncode, 1)
        self.assertRegex(self.line(), r"^\S+ WARN: ")
        self.assertFalse(self.status()["ok"])
        # Back up: treasury.gov is not new again, and its first sighting is kept.
        self.fx.mesh_up = True
        self.run_it()
        self.assertEqual(self.status()["new"], [])
        self.assertEqual(self.by_domain()["treasury.gov"]["first_seen"], first)

    def test_status_files_are_not_world_writable(self):
        self.run_it()
        for n in ["source-keydocs.json", "source-keydocs.txt", "source-keydocs.state"]:
            mode = os.stat(os.path.join(self.dir, n)).st_mode & 0o777
            self.assertEqual(mode & 0o022, 0, f"{n}: {oct(mode)}")


if __name__ == "__main__":
    unittest.main(verbosity=2)
