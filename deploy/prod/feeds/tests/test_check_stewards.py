"""CONTRACT-classes-round K12: the steward's change-process address. Before any class reaches production, its
`steward.process` must be a real page; for production it is https://datumesh.com/classes/change, which the site serves
(HTTP 200). `check_stewards.py` is the operator check:

    python check_stewards.py FILE [FILE ...]

It reads publish files (JSON lines), fetches the `steward.process` of each class line with a GET, requires HTTP 200 and a
`text/html` content type, and exits 0 when every class line passes and 1 when one does not (every failure is reported,
naming the file and line, the URL and the reason). Rehearsal hosts (the hosts with status "rehearsal" in names.toml and
sec_names.toml) are exempt: their addresses are not real pages, no request is made to them, and they are listed in the
output. It is run before any production publish of a class. Exit code 2 is a usage error (no file).

Tested with a fake server on 127.0.0.1 (an ephemeral port), the script run as a subprocess. Standard library only."""
import http.server
import json
import subprocess
import sys
import threading
import tomllib

import pytest

from conftest import FEEDS, PRODUCTION, REHEARSAL

PAGES = {
    "/html": (200, "text/html", b"<!doctype html><title>Change process</title><p>How a class changes.</p>"),
    "/html-utf8": (200, "text/html; charset=utf-8", b"<!doctype html><title>Change process</title>"),
    "/html-upper": (200, "Text/HTML; Charset=UTF-8", b"<!doctype html><title>Change process</title>"),
    "/missing": (404, "text/html", b"<!doctype html><title>Not found</title>"),
    "/gone": (410, "text/html", b"<!doctype html><title>Gone</title>"),
    "/plain": (200, "text/plain", b"How a class changes."),
    "/json": (200, "application/json", b'{"process": "how a class changes"}'),
    "/error": (500, "text/html", b"<!doctype html><title>Error</title>"),
}


class Site:
    """A fake site that serves PAGES and records the methods and paths it was asked."""

    def __init__(self):
        self.requests = []
        site = self

        class H(http.server.BaseHTTPRequestHandler):
            def _answer(self, head):
                site.requests.append((self.command, self.path))
                code, ctype, body = PAGES.get(self.path.split("?")[0], (404, "text/html", b"<title>Not found</title>"))
                self.send_response(code)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                if not head:
                    self.wfile.write(body)

            def do_GET(self):
                self._answer(False)

            def do_HEAD(self):
                self._answer(True)

            def log_message(self, *a):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *a):
        self.server.shutdown()
        self.server.server_close()


def class_line(name, process, namespace="example.org/x"):
    return {"class": name, "descriptor": {
        "keys": {"k": {"type": "string", "key_normalisation": "none"}}, "order": ["k"], "methods": {},
        "steward": {"namespace": namespace, "process": process}}}


def write(tmp_path, name, lines):
    p = tmp_path / name
    p.write_text("".join(json.dumps(l, sort_keys=True) + "\n" for l in lines), encoding="utf-8")
    return p


def run(tmp_path, *files):
    assert (FEEDS / "check_stewards.py").exists(), "check_stewards.py is not written yet"
    r = subprocess.run([sys.executable, "-B", str(FEEDS / "check_stewards.py"), *map(str, files)], capture_output=True,
                       text=True, cwd=tmp_path, timeout=60)
    return r, r.stdout + r.stderr


def rehearsal_hosts():
    hosts = set()
    for f in ("names.toml", "sec_names.toml"):
        with open(FEEDS / f, "rb") as fh:
            hosts |= {h for h, v in tomllib.load(fh)["hosts"].items() if v.get("status") == "rehearsal"}
    return hosts


def test_the_rehearsal_hosts_are_the_two_the_tables_list():
    assert rehearsal_hosts() == {REHEARSAL, "scratch.invalid"}


@pytest.mark.parametrize("page", ["/html", "/html-utf8", "/html-upper"])
def test_a_200_html_page_passes(tmp_path, page):
    with Site() as s:
        f = write(tmp_path, "c.jsonl", [class_line("example.org/x/c", s.base + page)])
        r, out = run(tmp_path, f)
        assert r.returncode == 0, out
        assert [m for m, _ in s.requests] == ["GET"], "a GET, once: " + str(s.requests)


@pytest.mark.parametrize("page,why", [("/missing", "404"), ("/gone", "410"), ("/error", "500"),
                                      ("/plain", "text/plain"), ("/json", "application/json")])
def test_a_page_that_is_not_200_html_fails_and_says_why(tmp_path, page, why):
    with Site() as s:
        url = s.base + page
        f = write(tmp_path, "c.jsonl", [class_line("example.org/x/c", url)])
        r, out = run(tmp_path, f)
        assert r.returncode == 1, out
        assert url in out and why in out, out
        assert "c.jsonl" in out and ":1" in out.replace(" line 1", ":1"), "the file and the line: " + out


def test_every_failure_is_reported_not_only_the_first(tmp_path):
    with Site() as s:
        lines = [class_line("example.org/x/a", s.base + "/missing"), class_line("example.org/x/b", s.base + "/html"),
                 class_line("example.org/x/c", s.base + "/plain")]
        f = write(tmp_path, "c.jsonl", lines)
        r, out = run(tmp_path, f)
        assert r.returncode == 1, out
        assert s.base + "/missing" in out and s.base + "/plain" in out, out
        assert sorted(p for _, p in s.requests) == ["/html", "/missing", "/plain"], "every class line is fetched: " + str(s.requests)


def test_only_class_lines_are_read_and_several_files_are_read(tmp_path):
    with Site() as s:
        other = [{"function": "example.org/x/f", "signature": {"args": {}, "order": [], "returns": "string"}},
                 {"catalog": "example.org/x/f", "kind": "function", "label": "f", "description": "d",
                  "steward": {"process": s.base + "/missing"}},
                 {"cell": {"fn": "example.org/x/f", "args": {}}, "value": {"type": "string", "value": "v"}}]
        a = write(tmp_path, "a.jsonl", other + [class_line("example.org/x/c", s.base + "/html")])
        b = write(tmp_path, "b.jsonl", [class_line("example.org/x/d", s.base + "/plain")])
        r, out = run(tmp_path, a, b)
        assert r.returncode == 1, out
        assert "b.jsonl" in out and s.base + "/plain" in out, out
        assert s.base + "/missing" not in out and "/missing" not in [p for _, p in s.requests], "a catalogue line is not a class line: " + out
        r, out = run(tmp_path, a)
        assert r.returncode == 0, out
    # a file with no class line has nothing to check
    n = write(tmp_path, "n.jsonl", other)
    r, out = run(tmp_path, n)
    assert r.returncode == 0, out


def test_rehearsal_hosts_are_exempt_and_listed(tmp_path):
    urls = [f"https://{h}/classes/change" for h in sorted(rehearsal_hosts())]
    lines = [class_line(f"{h}/x/c", f"https://{h}/classes/change") for h in sorted(rehearsal_hosts())]
    f = write(tmp_path, "c.jsonl", lines)
    r, out = run(tmp_path, f)
    assert r.returncode == 0, out
    for u in urls:
        assert u in out, "an exempt address is listed: " + out
    assert "exempt" in out.lower() and "rehearsal" in out.lower(), out


def test_an_exempt_host_does_not_hide_a_failure(tmp_path):
    with Site() as s:
        f = write(tmp_path, "c.jsonl", [class_line(f"{REHEARSAL}/x/c", f"https://{REHEARSAL}/classes/change"),
                                        class_line("example.org/x/c", s.base + "/missing")])
        r, out = run(tmp_path, f)
        assert r.returncode == 1, out
        assert s.base + "/missing" in out and "404" in out, out
        assert [m for m, _ in s.requests] == ["GET"]


def test_production_is_not_exempt_and_an_unreachable_page_fails(tmp_path):
    # datumesh.com is production: its address is fetched, not exempt. The harness has no route to it, so what is asserted is
    # that it is not listed as exempt and that a page that cannot be fetched fails (a closed local port stands for it).
    f = write(tmp_path, "c.jsonl", [class_line("example.org/x/c", "http://127.0.0.1:9/classes/change")])
    r, out = run(tmp_path, f)
    assert r.returncode == 1 and "127.0.0.1:9" in out, out
    assert PRODUCTION not in rehearsal_hosts()
    g = write(tmp_path, "g.jsonl", [class_line(f"{PRODUCTION}/x/c", f"https://{PRODUCTION}/classes/change")])
    r, out = run(tmp_path, g)
    assert "exempt" not in out.lower() or f"https://{PRODUCTION}/classes/change" not in out.lower().split("exempt")[-1], out


def test_a_class_line_without_a_process_fails(tmp_path):
    bad = class_line("example.org/x/c", "x")
    del bad["descriptor"]["steward"]["process"]
    f = write(tmp_path, "c.jsonl", [bad])
    r, out = run(tmp_path, f)
    assert r.returncode == 1 and "c.jsonl" in out, out


def test_no_file_is_a_usage_error(tmp_path):
    r, out = run(tmp_path)
    assert r.returncode == 2, out


# ---- the SEC feeds spell the production address; the rehearsal ones are noted as not real pages -------------------------

PROD_PROCESS = "https://datumesh.com/classes/change"


def test_sec_py_spells_the_production_address_for_the_production_host_and_keeps_the_rehearsal_one():
    import sec
    table = dict(sec.sec_table(REHEARSAL)["sec"])
    for host, want in [(PRODUCTION, PROD_PROCESS), (REHEARSAL, f"https://{REHEARSAL}/classes/change")]:
        t = dict(table, namespace=f"{host}/relay/sec.gov")
        classes = [l for l in sec._class_lines(t) if "class" in l]
        assert classes and all(l["descriptor"]["steward"]["process"] == want for l in classes), (host, classes)


def test_the_one_step_class_line_spells_the_production_address_for_the_production_host():
    """R13: sec.py writes the class line itself (no describe round trip), with K12's address."""
    import sec
    table = dict(sec.sec_table(REHEARSAL)["sec"])
    for host, want in [(PRODUCTION, PROD_PROCESS), (REHEARSAL, f"https://{REHEARSAL}/classes/change")]:
        t = dict(table, namespace=f"{host}/relay/sec.gov")
        cls = sec.company_class_line(t, "0" * 64)
        assert cls["descriptor"]["steward"]["process"] == want, host


@pytest.mark.parametrize("source", ["sec.py"])
def test_the_rehearsal_addresses_are_noted_as_not_real_pages(source):
    text = (FEEDS / source).read_text(encoding="utf-8").lower()
    assert "not a real page" in text or "not real pages" in text, f"{source} does not say that the rehearsal addresses are not real pages"
    assert PROD_PROCESS in (FEEDS / source).read_text(encoding="utf-8"), f"{source} spells the production address"


def test_the_readme_says_to_run_the_check_before_a_production_publish_of_a_class():
    text = (FEEDS / "README.md").read_text(encoding="utf-8")
    assert "check_stewards.py" in text
